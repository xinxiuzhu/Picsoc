use std::{
    fs::{self, File},
    io::{BufReader, BufWriter, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result, bail};
use image::{
    DynamicImage, ImageDecoder, ImageEncoder, ImageReader, Limits,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};

use crate::models::Asset;

pub const MAX_DECODE_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_SOURCE_BYTES: u64 = 256 * 1024 * 1024;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn supported_format(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => Some("jpg"),
        "png" => Some("png"),
        "gif" => Some("gif"),
        "webp" => Some("webp"),
        "bmp" => Some("bmp"),
        "tif" | "tiff" => Some("tiff"),
        _ => None,
    }
}

/// Resolve only an indexed relative path, rejecting traversal and symlink escapes.
pub fn secure_path(root: &Path, relative_path: &str) -> Result<PathBuf> {
    let relative = Path::new(relative_path);
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        bail!("素材路径无效");
    }
    let canonical_root = root.canonicalize().context("素材目录不可访问")?;
    // Libraries are stored canonically. Replacing the root with a symlink must not grant new access.
    if canonical_root != root {
        bail!("素材目录已改变，请重新添加目录");
    }
    let resolved = root
        .join(relative)
        .canonicalize()
        .context("原图不存在或不可访问")?;
    if !resolved.starts_with(&canonical_root) || !resolved.is_file() {
        bail!("素材路径超出了素材目录");
    }
    Ok(resolved)
}

pub fn cache_path(cache_root: &Path, asset: &Asset) -> PathBuf {
    cache_root
        .join(asset.library_id.to_string())
        .join(format!("{}.png", asset.cache_key()))
}

pub fn create_thumbnail(cache_root: &Path, asset: &Asset) -> Result<(u32, u32)> {
    let path = secure_path(Path::new(&asset.library_path), &asset.relative_path)?;
    let source = File::open(&path)?;
    if source.metadata()?.len() > MAX_SOURCE_BYTES {
        bail!("图片超过 256 MiB，已跳过缩略图");
    }
    let mut reader = ImageReader::new(BufReader::new(source)).with_guessed_format()?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    limits.max_image_width = Some(32_768);
    limits.max_image_height = Some(32_768);
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    if decoder.total_bytes() > MAX_DECODE_BYTES {
        bail!("图片解码超过 128 MiB，已跳过缩略图");
    }
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut decoded = DynamicImage::from_decoder(decoder)?;
    decoded.apply_orientation(orientation);
    let (width, height) = (decoded.width(), decoded.height());
    let thumbnail = if width <= 384 && height <= 384 {
        // Keep small images at their original size instead of enlarging their cache files.
        decoded.into_rgba8()
    } else {
        let thumbnail = decoded.thumbnail(384, 384).into_rgba8();
        // Drop the full image before encoding to keep peak memory low.
        drop(decoded);
        thumbnail
    };
    let target = cache_path(cache_root, asset);
    let parent = target.parent().context("缩略图缓存路径错误")?;
    fs::create_dir_all(parent)?;
    let temp = target.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut output = BufWriter::new(File::create(&temp)?);
        PngEncoder::new_with_quality(&mut output, CompressionType::Fast, FilterType::Adaptive)
            .write_image(
                thumbnail.as_raw(),
                thumbnail.width(),
                thumbnail.height(),
                image::ExtendedColorType::Rgba8,
            )?;
        output.flush()?;
        // Another request may have already produced this thumbnail. On Windows rename cannot replace a file.
        if target.exists() {
            fs::remove_file(&temp)?;
        } else if let Err(error) = fs::rename(&temp, &target) {
            if target.exists() {
                let _ = fs::remove_file(&temp);
            } else {
                return Err(error.into());
            }
        }
        Ok::<_, anyhow::Error>((width, height))
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

pub fn placeholder() -> &'static str {
    r##"<svg xmlns="http://www.w3.org/2000/svg" width="384" height="288" viewBox="0 0 384 288"><rect width="384" height="288" rx="18" fill="#e9eef2"/><g fill="none" stroke="#9aa8b3" stroke-width="8"><rect x="128" y="86" width="128" height="106" rx="12"/><path d="m132 168 39-39 30 27 25-24 26 29"/><circle cx="218" cy="114" r="8"/></g></svg>"##
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_and_absolute_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::write(root.join("image.png"), b"image").unwrap();
        assert!(secure_path(&root, "image.png").is_ok());
        for path in ["../image.png", "/etc/passwd", ".", ""] {
            assert!(secure_path(&root, path).is_err(), "{path}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("library");
        fs::create_dir(&root).unwrap();
        let outside = temp.path().join("private.png");
        fs::write(&outside, b"private").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link.png")).unwrap();
        assert!(secure_path(&root.canonicalize().unwrap(), "link.png").is_err());
    }

    fn test_asset(root: &Path, id: i64, name: &str) -> Asset {
        Asset {
            id,
            library_id: 1,
            name: name.into(),
            relative_path: name.into(),
            format: "png".into(),
            size: 100,
            width: None,
            height: None,
            modified_at: 1,
            favorite: false,
            tags: Vec::new(),
            thumbnail_url: String::new(),
            original_url: String::new(),
            mtime_ns: 1,
            library_path: root.to_str().unwrap().into(),
            thumbnail_error: None,
        }
    }

    #[test]
    fn simultaneous_thumbnail_requests_write_valid_atomic_cache() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        image::RgbaImage::from_pixel(80, 60, image::Rgba([15, 30, 60, 128]))
            .save(root.join("透明.png"))
            .unwrap();
        let asset = test_asset(&root, 1, "透明.png");
        let cache = root.join("cache");
        let tasks = (0..4)
            .map(|_| {
                let cache = cache.clone();
                let asset = asset.clone();
                std::thread::spawn(move || create_thumbnail(&cache, &asset).unwrap())
            })
            .collect::<Vec<_>>();
        for task in tasks {
            assert_eq!(task.join().unwrap(), (80, 60));
        }
        let thumbnail = image::open(cache_path(&cache, &asset))
            .unwrap()
            .into_rgba8();
        assert_eq!(thumbnail.dimensions(), (80, 60));
        assert_eq!(thumbnail.get_pixel(0, 0).0, [15, 30, 60, 128]);
        assert_eq!(fs::read_dir(cache.join("1")).unwrap().count(), 1);
    }

    #[test]
    fn large_and_narrow_images_shrink_preserving_alpha_and_original_dimensions() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let cache = root.join("cache");
        for (index, ((width, height), expected)) in [
            ((1536, 768), (384, 192)),
            ((768, 1536), (192, 384)),
            ((1, 1536), (1, 384)),
            ((1536, 1), (384, 1)),
        ]
        .into_iter()
        .enumerate()
        {
            let name = format!("透明-{index}.png");
            image::RgbaImage::from_pixel(width, height, image::Rgba([15, 30, 60, 128]))
                .save(root.join(&name))
                .unwrap();
            let asset = test_asset(&root, index as i64 + 1, &name);
            assert_eq!(create_thumbnail(&cache, &asset).unwrap(), (width, height));
            let thumbnail = image::open(cache_path(&cache, &asset))
                .unwrap()
                .into_rgba8();
            assert_eq!(thumbnail.dimensions(), expected);
            assert!(thumbnail.width() <= width && thumbnail.height() <= height);
            assert!(thumbnail.pixels().all(|pixel| pixel.0 == [15, 30, 60, 128]));
        }
    }
}
