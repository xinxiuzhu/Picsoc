use std::path::{Component, MAIN_SEPARATOR, Path};

use anyhow::{Result, bail};

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
