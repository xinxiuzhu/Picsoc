use std::{
    ffi::OsStr,
    path::{Component, MAIN_SEPARATOR, Path},
};

use anyhow::{Result, bail};

pub const PHOTOS_LIBRARY_ERROR: &str = "Apple Photos 图库不能作为普通素材文件夹导入，请先在「照片」中导出为 JPEG、PNG 或 TIFF，再添加导出文件夹";

/// Apply the same dot-directory rule to the picker, scanner, and indexed tree.
/// Checking the encoded leading ASCII dot also works for names that are not Unicode.
pub fn is_hidden_directory_name(name: &OsStr) -> bool {
    name.as_encoded_bytes().first() == Some(&b'.')
}

/// Paths here are relative to the explicitly selected library root. A dot-named
/// root is allowed, but a hidden directory anywhere beneath it is not browsable.
pub fn is_hidden_subdirectory(path: &Path) -> bool {
    path.components().any(
        |component| matches!(component, Component::Normal(name) if is_hidden_directory_name(name)),
    )
}

/// Detect a package name without opening it. Callers decide whether it is a directory.
pub fn is_photos_library(path: &Path) -> bool {
    const SUFFIX: &str = ".photoslibrary";
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.get(name.len().saturating_sub(SUFFIX.len())..))
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(SUFFIX))
}

/// Relative library paths stay platform native. Preserve names but normalize separators.
pub fn normalize_folder(folder: &str) -> Result<String> {
    if folder.is_empty() {
        return Ok(String::new());
    }
    if folder.len() > 4096 || folder.contains('\0') {
        bail!("素材子目录路径无效");
    }
    let mut parts = Vec::new();
    for component in Path::new(folder).components() {
        match component {
            Component::Normal(name) => parts.push(
                name.to_str()
                    .ok_or_else(|| anyhow::anyhow!("素材子目录路径无效"))?,
            ),
            _ => bail!("素材子目录路径无效"),
        }
    }
    Ok(parts.join(std::path::MAIN_SEPARATOR_STR))
}

/// A keyset cursor names a sibling in this listing; it may have been deleted since the last page.
pub fn normalize_cursor(parent: &str, cursor: &str) -> Result<String> {
    let cursor = normalize_folder(cursor)?;
    if cursor.is_empty() || Path::new(&cursor).parent().and_then(Path::to_str) != Some(parent) {
        bail!("目录分页游标必须属于当前目录");
    }
    Ok(cursor)
}

/// The separator is ASCII, so its successor is a precise binary upper bound for this subtree.
pub fn subtree_bounds(folder: &str) -> (String, String) {
    (
        format!("{folder}{MAIN_SEPARATOR}"),
        format!("{folder}{}", char::from(MAIN_SEPARATOR as u8 + 1)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hidden_directory_rule_checks_components_without_hiding_ordinary_dotted_names() {
        for name in [".git", ".svn", ".hg", ".cache", ".中文"] {
            assert!(is_hidden_directory_name(OsStr::new(name)), "{name}");
            assert!(is_hidden_subdirectory(Path::new(name)), "{name}");
            assert!(is_hidden_subdirectory(&Path::new("普通").join(name)));
            assert!(is_hidden_subdirectory(&Path::new(name).join("ordinary")));
        }
        for name in ["", "git", "release.v1", "文件夹", "ordinary.git"] {
            assert!(!is_hidden_directory_name(OsStr::new(name)), "{name}");
            assert!(!is_hidden_subdirectory(Path::new(name)), "{name}");
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert!(is_hidden_directory_name(OsStr::from_bytes(b".\xff")));
        }
    }

    #[test]
    fn photos_package_detection_preserves_unicode_and_checks_only_the_final_name() {
        for name in [
            "Photos Library.photoslibrary",
            "相片图库.photoslibrary",
            "图库.PHOTOSLIBRARY",
            "照片.pHoToSlIbRaRy",
            ".photoslibrary",
        ] {
            assert!(is_photos_library(&Path::new("素材").join(name)), "{name}");
        }
        for name in [
            "photoslibrary",
            "图库.photoslibrary.backup",
            "图库.photoslibraries",
            "photoslibrary.png",
            "相片图库",
            "图",
            "图库.photoslibrary/exports",
        ] {
            assert!(!is_photos_library(Path::new(name)), "{name}");
        }
        assert!(!is_photos_library(Path::new("/")));
    }
    #[test]
    fn relative_paths_preserve_unicode_and_reject_traversal() {
        let folder = Path::new("中文").join("壁纸");
        assert_eq!(
            normalize_folder(folder.to_str().unwrap()).unwrap(),
            folder.to_str().unwrap()
        );
        assert_eq!(normalize_folder("").unwrap(), "");
        #[cfg(unix)]
        assert_eq!(normalize_folder("合法\\目录").unwrap(), "合法\\目录");
        for path in ["..", "../private", "/private", ".", "bad\0path"] {
            assert!(normalize_folder(path).is_err(), "{path}");
        }
    }
}
