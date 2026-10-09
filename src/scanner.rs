use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use tokio::sync::{Semaphore, mpsc, oneshot};
use walkdir::WalkDir;

use crate::{
    db::{Db, FileRecord},
    media,
    models::{Asset, ScanStatus},
};

pub struct ThumbnailJob {
    pub asset: Asset,
    pub complete: Option<oneshot::Sender<()>>,
}

#[derive(Clone)]
pub struct Scanner {
    db: Db,
    cache_root: PathBuf,
    data_dir: PathBuf,
    statuses: Arc<Mutex<HashMap<i64, ScanStatus>>>,
    gate: Arc<Semaphore>,
    generation: Arc<AtomicI64>,
    pub thumbnails: mpsc::Sender<ThumbnailJob>,
}

impl Scanner {
    pub fn new(db: Db, data_dir: PathBuf, workers: usize) -> Self {
        let cache_root = data_dir.join("thumbnails");
        let (sender, receiver) = mpsc::channel::<ThumbnailJob>(64);
        let receiver = Arc::new(tokio::sync::Mutex::new(receiver));
        for _ in 0..workers {
            let receiver = receiver.clone();
            let db = db.clone();
            let cache_root = cache_root.clone();
            tokio::spawn(async move {
                loop {
                    let Some(job) = receiver.lock().await.recv().await else {
                        break;
                    };
                    let db = db.clone();
                    let cache_root = cache_root.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        // Re-read the row so queued work cannot serve a removed or changed image.
                        if let Ok(current) = db.asset(job.asset.id)
                            && current.cache_key() == job.asset.cache_key()
                            && !media::cache_path(&cache_root, &current).exists()
                            && current.thumbnail_error.is_none()
                        {
                            match media::create_thumbnail(&cache_root, &current) {
                                Ok((w, h)) => {
                                    if db.asset(current.id).is_ok_and(|latest| {
                                        latest.cache_key() == current.cache_key()
                                    }) {
                                        let _ = db.set_thumbnail_result(
                                            &current,
                                            Some(w),
                                            Some(h),
                                            None,
                                        );
                                    } else {
                                        let _ = std::fs::remove_file(media::cache_path(
                                            &cache_root,
                                            &current,
                                        ));
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(asset_id=current.id,error=%e,"缩略图生成失败");
                                    let _ = db.set_thumbnail_result(
                                        &current,
                                        None,
                                        None,
                                        Some(&e.to_string()),
                                    );
                                }
                            }
                        }
                        if let Some(done) = job.complete {
                            let _ = done.send(());
                        }
                    })
                    .await;
                }
            });
        }
        Self {
            db,
            cache_root,
            data_dir,
            statuses: Arc::new(Mutex::new(HashMap::new())),
            gate: Arc::new(Semaphore::new(1)),
            generation: Arc::new(AtomicI64::new(now_ns())),
            thumbnails: sender,
        }
    }

    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }

    pub fn status(&self, id: i64) -> ScanStatus {
        self.statuses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn forget(&self, id: i64) {
        // Removing status cancels an active traversal at its next batch boundary.
        self.statuses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
    }

    pub fn cancel(&self, id: i64) {
        if let Some(status) = self
            .statuses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&id)
        {
            status.cancelled = true;
        }
    }

    fn cancelled(&self, id: i64) -> bool {
        self.statuses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .is_none_or(|s| s.cancelled)
    }

    pub fn start(&self, id: i64) {
        {
            let mut statuses = self.statuses.lock().unwrap_or_else(|e| e.into_inner());
            if statuses.get(&id).is_some_and(|s| s.state == "scanning") {
                return;
            }
            statuses.insert(
                id,
                ScanStatus {
                    state: "scanning".into(),
                    ..Default::default()
                },
            );
        }
        let scanner = self.clone();
        tokio::spawn(async move {
            let Ok(_permit) = scanner.gate.clone().acquire_owned().await else {
                return;
            };
            let copy = scanner.clone();
            let result = tokio::task::spawn_blocking(move || copy.scan(id)).await;
            let error = match result {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some(e.to_string()),
                Err(e) => Some(e.to_string()),
            };
            if let Some(status) = scanner
                .statuses
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_mut(&id)
            {
                status.state = if error.is_some() && !status.cancelled {
                    "error"
                } else {
                    "idle"
                }
                .into();
                status.error = if status.cancelled { None } else { error };
                status.total = Some(status.processed);
                status.cancelled = false;
            }
        });
    }

    fn scan(&self, id: i64) -> Result<()> {
        if self.cancelled(id) {
            return Ok(());
        }
        let library = self.db.library(id)?;
        let root = PathBuf::from(&library.path);
        if root.canonicalize().context("素材目录不可访问")? != root {
            bail!("素材目录已改变，请重新添加");
        }
        let generation = self.generation.fetch_add(1, Ordering::Relaxed);
        let mut records = Vec::with_capacity(100);
        let mut traversal_error = None;
        let data_dir = self.data_dir.clone();
        let entries = WalkDir::new(&root)
            .follow_links(false)
            .max_open(8)
            .into_iter()
            .filter_entry(|entry| entry.path() != data_dir);
        for entry in entries {
            if self.cancelled(id) {
                return Ok(());
            }
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    traversal_error = Some(e.to_string());
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let Some(format) = media::supported_format(entry.path()) else {
                continue;
            };
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(e) => {
                    traversal_error = Some(e.to_string());
                    continue;
                }
            };
            let Some(relative) = entry.path().strip_prefix(&root)?.to_str() else {
                traversal_error = Some("文件名无法转换为 Unicode".into());
                continue;
            };
            let modified = metadata
                .modified()
                .unwrap_or(UNIX_EPOCH)
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            records.push(FileRecord {
                relative_path: relative.into(),
                name: entry.file_name().to_string_lossy().into_owned(),
                format: format.into(),
                size: metadata.len().min(i64::MAX as u64) as i64,
                modified_at: modified.as_secs().min(i64::MAX as u64) as i64,
                mtime_ns: modified.as_nanos().min(i64::MAX as u128) as i64,
            });
            if records.len() == 100 {
                self.flush(id, generation, &mut records)?;
            }
        }
        if self.cancelled(id) {
            return Ok(());
        }
        self.flush(id, generation, &mut records)?;
        // Never prune on incomplete traversal: an offline/unreadable folder must not erase metadata.
        if let Some(error) = traversal_error {
            bail!("扫描未完成，保留原有索引：{error}");
        }
        while !self.cancelled(id) {
            let removed = self.db.remove_unseen_batch(id, generation)?;
            if removed.is_empty() {
                break;
            }
            for key in removed {
                let _ = std::fs::remove_file(
                    self.cache_root
                        .join(id.to_string())
                        .join(format!("{key}.png")),
                );
            }
        }
        Ok(())
    }

    fn flush(&self, id: i64, generation: i64, records: &mut Vec<FileRecord>) -> Result<()> {
        if records.is_empty() || self.cancelled(id) {
            return Ok(());
        }
        let assets = self.db.index_batch(id, generation, records)?;
        if let Some(status) = self
            .statuses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&id)
        {
            status.processed += assets.len() as u64;
        }
        records.clear();
        for (asset, obsolete_key) in assets {
            if let Some(key) = obsolete_key {
                let _ = std::fs::remove_file(
                    self.cache_root
                        .join(asset.library_id.to_string())
                        .join(format!("{key}.png")),
                );
            }
            if self.cancelled(id) {
                break;
            }
            if !media::cache_path(&self.cache_root, &asset).exists()
                && asset.thumbnail_error.is_none()
            {
                self.thumbnails
                    .blocking_send(ThumbnailJob {
                        asset,
                        complete: None,
                    })
                    .context("缩略图队列已停止")?;
            }
        }
        Ok(())
    }
}

fn now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(i64::MAX as u128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AssetPatch, AssetQuery};
    use std::time::Duration;

    async fn wait_scan(scanner: &Scanner, id: i64) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while scanner.status(id).state == "scanning" {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(scanner.status(id).state, "idle", "{:?}", scanner.status(id));
    }

    #[tokio::test]
    async fn incremental_scan_retains_annotations_and_removes_deleted_files() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        let root = temp.path().join("素材");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let data = data.canonicalize().unwrap();
        image::RgbaImage::from_pixel(32, 24, image::Rgba([12, 24, 48, 255]))
            .save(root.join("中文.png"))
            .unwrap();
        std::fs::write(root.join("忽略.txt"), b"not an image").unwrap();
        let db = Db::open(&data.join("test.sqlite")).unwrap();
        let library = db.add_library("素材", root.to_str().unwrap()).unwrap();
        let scanner = Scanner::new(db.clone(), data, 2);
        scanner.start(library.id);
        wait_scan(&scanner, library.id).await;
        let assets = db.assets(&AssetQuery::default()).unwrap().assets;
        assert_eq!(assets.len(), 1);
        let asset = assets[0].clone();
        let (done, wait) = oneshot::channel();
        scanner
            .thumbnails
            .send(ThumbnailJob {
                asset: asset.clone(),
                complete: Some(done),
            })
            .await
            .unwrap();
        wait.await.unwrap();
        assert!(media::cache_path(scanner.cache_root(), &asset).exists());
        assert_eq!(db.asset(asset.id).unwrap().width, Some(32));
        db.patch_asset(
            asset.id,
            &AssetPatch {
                favorite: Some(true),
                tags: Some(vec!["标签".into()]),
            },
        )
        .unwrap();
        scanner.start(library.id);
        wait_scan(&scanner, library.id).await;
        assert!(db.asset(asset.id).unwrap().favorite);
        image::RgbaImage::from_pixel(48, 24, image::Rgba([50, 70, 90, 255]))
            .save(root.join("中文.png"))
            .unwrap();
        scanner.start(library.id);
        wait_scan(&scanner, library.id).await;
        let changed = db.asset(asset.id).unwrap();
        assert_ne!(changed.cache_key(), asset.cache_key());
        assert!(!media::cache_path(scanner.cache_root(), &asset).exists());
        assert_eq!(changed.tags, vec!["标签"]);
        std::fs::remove_file(root.join("中文.png")).unwrap();
        scanner.start(library.id);
        wait_scan(&scanner, library.id).await;
        assert_eq!(db.stats().unwrap().total_assets, 0);
        assert!(root.join("忽略.txt").exists());
    }
}
