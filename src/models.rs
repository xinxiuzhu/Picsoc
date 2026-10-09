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
    pub library_id: Option<i64>,
    pub favorite: Option<bool>,
    pub format: Option<String>,
    pub tag: Option<String>,
    pub folder: Option<String>,
    pub sort: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
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
}

#[derive(Debug, Serialize)]
pub struct FolderList {
    pub folders: Vec<Folder>,
    pub parent: String,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct Tag {
    pub name: String,
    pub count: i64,
}
