use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize)]
pub struct Asset {
    pub id: i64,
    pub library_id: i64,
    pub name: String,
    pub relative_path: String,
    pub format: String,
    pub size: i64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub modified_at: i64,
    pub favorite: bool,
    pub tags: Vec<String>,
    pub thumbnail_url: String,
    pub original_url: String,
    #[serde(skip)]
    pub mtime_ns: i64,
    #[serde(skip)]
    pub library_path: String,
    #[serde(skip)]
    pub thumbnail_error: Option<String>,
}

impl Asset {
    pub fn cache_key(&self) -> String {
        format!("{}-{}-{}", self.id, self.mtime_ns, self.size)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ScanStatus {
    pub state: String,
    pub processed: u64,
    pub total: Option<u64>,
    pub error: Option<String>,
    #[serde(skip)]
    pub cancelled: bool,
}

impl Default for ScanStatus {
    fn default() -> Self {
        Self {
            state: "idle".into(),
            processed: 0,
            total: None,
            error: None,
            cancelled: false,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Library {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub asset_count: i64,
    pub scan: ScanStatus,
}

#[derive(Debug, Serialize)]
pub struct Stats {
    pub total_assets: i64,
    pub total_size: i64,
    pub total_favorites: i64,
    pub total_libraries: i64,
}

#[derive(Clone, Default, Debug, Deserialize)]
pub struct AssetQuery {
    pub q: Option<String>,
    pub exclude_names: Option<String>,
    pub library_id: Option<i64>,
    pub favorite: Option<bool>,
    pub format: Option<String>,
    pub tag: Option<String>,
    pub folder: Option<String>,
    pub folder_recursive: Option<bool>,
    pub orientation: Option<String>,
    pub aspect_ratio: Option<String>,
    pub min_width: Option<u32>,
    pub max_width: Option<u32>,
    pub min_height: Option<u32>,
    pub max_height: Option<u32>,
    pub min_size: Option<i64>,
    pub max_size: Option<i64>,
    pub sort: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}

impl AssetQuery {
    pub fn normalize_excluded_names(&self) -> anyhow::Result<Vec<String>> {
        let Some(input) = &self.exclude_names else {
            return Ok(Vec::new());
        };
        const ERROR: &str = "排除关键词最多 50 个，每个最多 100 个字符，总长度最多 4096 字节";
        anyhow::ensure!(input.len() <= 4096, ERROR);
        let mut names = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for name in input.lines().map(str::trim).filter(|name| !name.is_empty()) {
            anyhow::ensure!(name.chars().count() <= 100 && !name.contains('\0'), ERROR);
            if seen.insert(name) {
                names.push(name.to_owned());
                anyhow::ensure!(names.len() <= 50, ERROR);
            }
        }
        Ok(names)
    }

    pub fn aspect_ratio_value(&self) -> anyhow::Result<Option<f64>> {
        let Some(value) = &self.aspect_ratio else {
            return Ok(None);
        };
        let invalid = || anyhow::anyhow!("宽高比需要两个大于零的数，例如 16:9");
        let (width, height) = value.split_once(':').ok_or_else(invalid)?;
        let width = width.trim().parse::<f64>().map_err(|_| invalid())?;
        let height = height.trim().parse::<f64>().map_err(|_| invalid())?;
        let ratio = width / height;
        anyhow::ensure!(
            width.is_finite()
                && height.is_finite()
                && width > 0.0
                && height > 0.0
                && ratio.is_finite()
                && ratio > 0.0
                && (ratio * 1.02).is_finite(),
            "宽高比需要两个大于零的数，例如 16:9"
        );
        Ok(Some(ratio))
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        self.normalize_excluded_names()?;
        anyhow::ensure!(
            (self.folder.is_none() && self.folder_recursive.is_none()) || self.library_id.is_some(),
            "按子目录筛选时必须指定素材库"
        );
        if let Some(folder) = &self.folder {
            crate::folders::normalize_folder(folder)?;
        }
        anyhow::ensure!(
            self.q.as_ref().is_none_or(|q| q.len() <= 1024)
                && self.tag.as_ref().is_none_or(|q| q.len() <= 200),
            "搜索内容过长"
        );
        anyhow::ensure!(
            self.sort.as_deref().is_none_or(|sort| [
                "modified",
                "name",
                "name_desc",
                "size",
                "size_asc",
                "width",
                "height",
                "pixels"
            ]
            .contains(&sort)),
            "排序方式无效"
        );
        anyhow::ensure!(
            self
                .format
                .as_deref()
                .is_none_or(
                    |format| ["jpg", "png", "gif", "webp", "bmp", "tiff"].contains(&format)
                ),
            "图片格式无效"
        );
        anyhow::ensure!(
            self.orientation.as_deref().is_none_or(|orientation| [
                "landscape",
                "portrait",
                "square"
            ]
            .contains(&orientation)),
            "图片方向无效"
        );
        self.aspect_ratio_value()?;
        anyhow::ensure!(
            self.min_width
                .zip(self.max_width)
                .is_none_or(|(min, max)| min <= max)
                && self
                    .min_height
                    .zip(self.max_height)
                    .is_none_or(|(min, max)| min <= max),
            "尺寸下限不能大于上限"
        );
        anyhow::ensure!(
            self.min_size.is_none_or(|size| size >= 0)
                && self.max_size.is_none_or(|size| size >= 0)
                && self
                    .min_size
                    .zip(self.max_size)
                    .is_none_or(|(min, max)| min <= max),
            "文件大小必须非负，且下限不能大于上限"
        );
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct AssetList {
    pub assets: Vec<Asset>,
    pub total: i64,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Debug, Deserialize)]
pub struct AssetPatch {
    pub favorite: Option<bool>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct AssetBatch {
    pub ids: Vec<i64>,
    pub favorite: Option<bool>,
    pub add_tags: Option<Vec<String>>,
    pub remove_tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct Folder {
    pub path: String,
    pub name: String,
    pub parent: Option<String>,
    pub asset_count: i64,
    pub direct_asset_count: i64,
    pub has_children: bool,
}

#[derive(Debug, Serialize)]
pub struct FolderList {
    pub folders: Vec<Folder>,
    pub parent: String,
    pub truncated: bool,
    pub direct_asset_count: i64,
    pub separator: String,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Tag {
    pub name: String,
    pub count: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_filter_values_before_querying_the_database() {
        let invalid = [
            AssetQuery {
                aspect_ratio: Some("16:0".into()),
                ..Default::default()
            },
            AssetQuery {
                aspect_ratio: Some("NaN:1".into()),
                ..Default::default()
            },
            AssetQuery {
                aspect_ratio: Some("1:inf".into()),
                ..Default::default()
            },
            AssetQuery {
                aspect_ratio: Some("16:9:1".into()),
                ..Default::default()
            },
            AssetQuery {
                aspect_ratio: Some("1e308:1e-308".into()),
                ..Default::default()
            },
            AssetQuery {
                orientation: Some("diagonal".into()),
                ..Default::default()
            },
            AssetQuery {
                min_width: Some(101),
                max_width: Some(100),
                ..Default::default()
            },
            AssetQuery {
                min_height: Some(101),
                max_height: Some(100),
                ..Default::default()
            },
            AssetQuery {
                min_size: Some(-1),
                ..Default::default()
            },
            AssetQuery {
                min_size: Some(101),
                max_size: Some(100),
                ..Default::default()
            },
            AssetQuery {
                folder_recursive: Some(false),
                ..Default::default()
            },
            AssetQuery {
                library_id: Some(1),
                folder: Some("../outside".into()),
                ..Default::default()
            },
        ];
        for query in invalid {
            assert!(query.validate().is_err(), "{query:?}");
        }
        assert_eq!(
            AssetQuery {
                aspect_ratio: Some(" 1.6: 0.9 ".into()),
                ..Default::default()
            }
            .aspect_ratio_value()
            .unwrap(),
            Some(1.6 / 0.9)
        );
        assert!(
            AssetQuery {
                min_width: Some(0),
                max_width: Some(u32::MAX),
                min_size: Some(0),
                max_size: Some(i64::MAX),
                ..Default::default()
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn excluded_name_limits_apply_after_trimming_and_deduplication() {
        let query = AssetQuery {
            exclude_names: Some(" map \n\nmap\r\n 中文 \n".into()),
            ..Default::default()
        };
        assert_eq!(
            query.normalize_excluded_names().unwrap(),
            vec!["map", "中文"]
        );
        let duplicates = AssetQuery {
            exclude_names: Some("map\n".repeat(60)),
            ..Default::default()
        };
        assert_eq!(duplicates.normalize_excluded_names().unwrap(), vec!["map"]);
        let fifty = (0..50)
            .map(|index| format!("word{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            AssetQuery {
                exclude_names: Some(fifty.clone()),
                ..Default::default()
            }
            .normalize_excluded_names()
            .unwrap()
            .len(),
            50
        );
        for input in [
            format!("{fifty}\nextra"),
            "中".repeat(101),
            "x".repeat(4097),
            "bad\0name".into(),
        ] {
            assert!(
                AssetQuery {
                    exclude_names: Some(input),
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        let boundary = format!("{}map", " ".repeat(4093));
        assert_eq!(
            AssetQuery {
                exclude_names: Some(boundary),
                ..Default::default()
            }
            .normalize_excluded_names()
            .unwrap(),
            vec!["map"]
        );
        assert_eq!(
            AssetQuery {
                exclude_names: Some("中".repeat(100)),
                ..Default::default()
            }
            .normalize_excluded_names()
            .unwrap()[0]
                .chars()
                .count(),
            100
        );
    }
}
