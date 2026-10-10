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
    folders, media,
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
        if root.ancestors().any(folders::is_photos_library) {
            bail!(folders::PHOTOS_LIBRARY_ERROR);
        }
        if root.canonicalize().context("素材目录不可访问")? != root {
            bail!("素材目录已改变，请重新添加");
        }
        let generation = self.generation.fetch_add(1, Ordering::Relaxed);
        let mut records = Vec::with_capacity(100);
        let mut directory_records = Vec::with_capacity(100);
        let mut traversal_error = None;
        let data_dir = self.data_dir.clone();
        let mut entries = WalkDir::new(&root)
            .follow_links(false)
            .max_open(8)
            .into_iter();
        while let Some(entry) = entries.next() {
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
            if entry.file_type().is_dir() {
                if entry.depth() > 0 && folders::is_hidden_directory_name(entry.file_name()) {
                    // Prune old hidden indexes only after this traversal completes successfully.
                    // Skipping now also discards any buffered permission error for this directory.
                    entries.skip_current_dir();
                    continue;
                }
                if entry.path() == data_dir {
                    entries.skip_current_dir();
                    continue;
                }
                if entry.depth() > 0 && folders::is_photos_library(entry.path()) {
                    entries.skip_current_dir();
                    // An excluded package is not a deleted folder. Keep annotations from
                    // older versions without reading any of its descendants.
                    let Some(relative) = entry.path().strip_prefix(&root)?.to_str() else {
                        traversal_error = Some("文件名无法转换为 Unicode".into());
                        continue;
                    };
                    if let Err(error) = self.db.retain_subtree(id, generation, relative) {
                        traversal_error = Some(error.to_string());
                    }
                    continue;
                }
                let Some(relative) = entry.path().strip_prefix(&root)?.to_str() else {
                    entries.skip_current_dir();
                    traversal_error = Some("文件名无法转换为 Unicode".into());
                    continue;
                };
                directory_records.push(relative.to_owned());
                if directory_records.len() == 100 {
                    self.flush_folders(id, generation, &mut directory_records)?;
                }
            }
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
        self.flush_folders(id, generation, &mut directory_records)?;
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
        while !self.cancelled(id) {
            if self.db.remove_unseen_folders_batch(id, generation)? == 0 {
                break;
            }
        }
        Ok(())
    }

    fn flush_folders(&self, id: i64, generation: i64, records: &mut Vec<String>) -> Result<()> {
        if records.is_empty() || self.cancelled(id) {
            return Ok(());
        }
        self.db.index_folders(id, generation, records)?;
        records.clear();
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

    async fn scan_status(scanner: &Scanner, id: i64) -> ScanStatus {
        tokio::time::timeout(Duration::from_secs(10), async {
            while scanner.status(id).state == "scanning" {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        scanner.status(id)
    }

    async fn wait_scan(scanner: &Scanner, id: i64) {
        let status = scan_status(scanner, id).await;
        assert_eq!(status.state, "idle", "{status:?}");
    }

    fn fixture(name: &str) -> (tempfile::TempDir, PathBuf, Db, i64, Scanner) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join(name);
        let data = temp.path().join("data");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        let root = root.canonicalize().unwrap();
        let data = data.canonicalize().unwrap();
        let db = Db::open(&data.join("test.sqlite")).unwrap();
        let id = db.add_library("素材", root.to_str().unwrap()).unwrap().id;
        let scanner = Scanner::new(db.clone(), data, 1);
        (temp, root, db, id, scanner)
    }

    fn write_png(path: &Path) {
        image::RgbaImage::from_pixel(8, 6, image::Rgba([12, 24, 48, 255]))
            .save(path)
            .unwrap();
    }

    fn old_record(path: &Path) -> FileRecord {
        FileRecord {
            relative_path: path.to_str().unwrap().to_owned(),
            name: path.file_name().unwrap().to_str().unwrap().to_owned(),
            format: "png".into(),
            size: 1,
            modified_at: 1,
            mtime_ns: 1,
        }
    }

    #[tokio::test]
    async fn hidden_subtrees_are_skipped_and_legacy_indexes_are_pruned_after_success() {
        // A root explicitly selected by the user remains valid even with a leading dot.
        let (_temp, root, db, id, scanner) = fixture(".selected");
        let hidden = [
            Path::new(".git").join("objects"),
            Path::new("普通").join(".cache").join("深层"),
        ];
        for path in &hidden {
            std::fs::create_dir_all(root.join(path)).unwrap();
            write_png(&root.join(path).join("hidden.png"));
        }
        std::fs::create_dir(root.join("空目录")).unwrap();
        write_png(&root.join(".preview.png"));
        write_png(&root.join("普通").join("visible.png"));
        let legacy = db
            .index_batch(
                id,
                0,
                &[
                    old_record(&hidden[0].join("hidden.png")),
                    old_record(&hidden[1].join("hidden.png")),
                    old_record(&Path::new("普通").join("visible.png")),
                ],
            )
            .unwrap();
        let visible_id = legacy[2].0.id;
        db.patch_asset(
            visible_id,
            &AssetPatch {
                favorite: Some(true),
                tags: Some(vec!["保留标签".into()]),
            },
        )
        .unwrap();
        assert_eq!(db.stats().unwrap().total_assets, 3);
        scanner.start(id);
        wait_scan(&scanner, id).await;
        assert_eq!(scanner.status(id).processed, 2);
        assert_eq!(db.stats().unwrap().total_assets, 2);
        for (asset, _) in &legacy[..2] {
            assert!(db.asset(asset.id).is_err());
        }
        let retained = db.asset(visible_id).unwrap();
        assert!(retained.favorite);
        assert_eq!(retained.tags, vec!["保留标签"]);
        assert!(
            db.assets(&AssetQuery::default())
                .unwrap()
                .assets
                .iter()
                .any(|asset| asset.name == ".preview.png")
        );
        let tree = db.folders(id, "").unwrap();
        assert_eq!(tree.folders.len(), 2);
        assert!(tree.folders.iter().all(|folder| !folder.has_children));
        assert!(tree.folders.iter().any(|folder| folder.name == "空目录"));
        for path in hidden {
            assert!(root.join(path).join("hidden.png").exists());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unreadable_hidden_directory_does_not_fail_the_scan() {
        use std::os::unix::fs::PermissionsExt;

        let (_temp, root, db, id, scanner) = fixture("素材");
        write_png(&root.join("可读.png"));
        let hidden = root.join(".git");
        std::fs::create_dir(&hidden).unwrap();
        write_png(&hidden.join("跳过.png"));
        let permissions = std::fs::metadata(&hidden).unwrap().permissions();
        std::fs::set_permissions(&hidden, std::fs::Permissions::from_mode(0o0)).unwrap();
        scanner.start(id);
        let status = scan_status(&scanner, id).await;
        std::fs::set_permissions(&hidden, permissions).unwrap();
        assert_eq!(status.state, "idle", "{status:?}");
        assert_eq!(db.stats().unwrap().total_assets, 1);
        assert_eq!(status.processed, 1);
        assert!(db.folders(id, "").unwrap().folders.is_empty());
    }

    #[tokio::test]
    async fn skips_nested_photos_packages_without_skipping_similar_directories() {
        let (_temp, root, db, id, scanner) = fixture("素材");
        write_png(&root.join("普通.png"));
        for name in ["旅行.photoslibrary", "中文.PHOTOSLIBRARY", ".photoslibrary"] {
            let package = root.join(name).join("originals");
            std::fs::create_dir_all(&package).unwrap();
            write_png(&package.join("不导入.png"));
        }
        let ordinary = root.join("旅行.photoslibrary.backup");
        std::fs::create_dir(&ordinary).unwrap();
        write_png(&ordinary.join("导入.png"));
        std::fs::write(root.join("普通文件.photoslibrary"), b"not a directory").unwrap();
        scanner.start(id);
        wait_scan(&scanner, id).await;
        let assets = db.assets(&AssetQuery::default()).unwrap().assets;
        assert_eq!(assets.len(), 2);
        assert!(assets.iter().any(|a| a.name == "普通.png"));
        assert!(assets.iter().any(|a| a.name == "导入.png"));
        assert_eq!(scanner.status(id).processed, 2);
        let indexed_folders = db.folders(id, "").unwrap().folders;
        assert_eq!(indexed_folders.len(), 1);
        assert_eq!(indexed_folders[0].name, "旅行.photoslibrary.backup");
    }

    #[tokio::test]
    async fn skipped_package_retains_annotations_but_deleted_ordinary_assets_are_pruned() {
        let (_temp, root, db, id, scanner) = fixture("素材");
        let package = root.join("中文.PHOTOSLIBRARY");
        std::fs::create_dir(&package).unwrap();
        write_png(&package.join("包内.png"));
        write_png(&root.join("普通.png"));
        let legacy = db
            .index_batch(
                id,
                0,
                &[
                    old_record(&Path::new("中文.PHOTOSLIBRARY").join("包内.png")),
                    old_record(Path::new("已删除.png")),
                    old_record(Path::new("普通文件.photoslibrary")),
                    old_record(&Path::new("普通文件.photoslibrary").join("旧包内.png")),
                ],
            )
            .unwrap();
        let package_asset = &legacy[0].0;
        let deleted_id = legacy[1].0.id;
        let file_id = legacy[2].0.id;
        let former_package_id = legacy[3].0.id;
        // Replacing an old package directory with a regular file must not retain its subtree.
        std::fs::write(root.join("普通文件.photoslibrary"), b"ordinary file").unwrap();
        db.patch_asset(
            package_asset.id,
            &AssetPatch {
                favorite: Some(true),
                tags: Some(vec!["保留标签".into()]),
            },
        )
        .unwrap();
        scanner.start(id);
        wait_scan(&scanner, id).await;
        let retained = db.asset(package_asset.id).unwrap();
        assert!(retained.favorite);
        assert_eq!(retained.tags, vec!["保留标签"]);
        assert_eq!(
            retained.size, 1,
            "excluded package metadata must not be read"
        );
        assert!(db.asset(deleted_id).is_err());
        assert!(db.asset(file_id).is_err());
        assert!(db.asset(former_package_id).is_err());
        assert_eq!(db.stats().unwrap().total_assets, 2);
        let indexed_folders = db.folders(id, "").unwrap().folders;
        assert_eq!(indexed_folders.len(), 1);
        assert_eq!(indexed_folders[0].name, "中文.PHOTOSLIBRARY");
        assert_eq!(indexed_folders[0].asset_count, 1);
        assert!(package.join("包内.png").exists());
    }

    #[tokio::test]
    async fn photos_package_roots_and_inner_roots_fail_without_pruning() {
        let (_temp, root, db, id, scanner) = fixture("中文.photoslibrary");
        let legacy = db
            .index_batch(id, 0, &[old_record(Path::new("旧图片.png"))])
            .unwrap();
        scanner.start(id);
        let status = scan_status(&scanner, id).await;
        assert_eq!(status.state, "error");
        assert_eq!(status.error.as_deref(), Some(folders::PHOTOS_LIBRARY_ERROR));
        assert!(db.asset(legacy[0].0.id).is_ok());

        let inner = root.join("originals");
        std::fs::create_dir(&inner).unwrap();
        let inner_id = db
            .add_library("包内目录", inner.to_str().unwrap())
            .unwrap()
            .id;
        scanner.start(inner_id);
        let status = scan_status(&scanner, inner_id).await;
        assert_eq!(status.state, "error");
        assert_eq!(status.error.as_deref(), Some(folders::PHOTOS_LIBRARY_ERROR));
    }

    #[tokio::test]
    async fn package_retention_database_failure_prevents_pruning() {
        let (_temp, root, db, id, scanner) = fixture("素材");
        std::fs::create_dir(root.join("旧.photoslibrary")).unwrap();
        write_png(&root.join("新图片.png"));
        let legacy = db
            .index_batch(
                id,
                0,
                &[
                    old_record(&Path::new("旧.photoslibrary").join("legacy.png")),
                    old_record(Path::new("已删除.png")),
                ],
            )
            .unwrap();
        let connection =
            rusqlite::Connection::open(root.parent().unwrap().join("data/test.sqlite")).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_retention BEFORE UPDATE OF seen_generation ON assets
                 WHEN OLD.name='legacy.png'
                 BEGIN SELECT RAISE(ABORT,'retention blocked'); END;",
            )
            .unwrap();
        scanner.start(id);
        let status = scan_status(&scanner, id).await;
        assert_eq!(status.state, "error");
        assert!(status.error.unwrap().contains("retention blocked"));
        assert!(db.asset(legacy[0].0.id).is_ok());
        assert!(db.asset(legacy[1].0.id).is_ok());
        assert_eq!(db.stats().unwrap().total_assets, 3);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unreadable_nested_photos_package_does_not_fail_the_scan() {
        use std::os::unix::fs::PermissionsExt;

        let (_temp, root, db, id, scanner) = fixture("素材");
        write_png(&root.join("可读.png"));
        let package = root.join("Photos Library.photoslibrary");
        std::fs::create_dir(&package).unwrap();
        write_png(&package.join("跳过.png"));
        let permissions = std::fs::metadata(&package).unwrap().permissions();
        std::fs::set_permissions(&package, std::fs::Permissions::from_mode(0o0)).unwrap();
        scanner.start(id);
        let status = scan_status(&scanner, id).await;
        std::fs::set_permissions(&package, permissions).unwrap();
        assert_eq!(status.state, "idle", "{status:?}");
        assert_eq!(db.stats().unwrap().total_assets, 1);
        assert_eq!(status.processed, 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ordinary_permission_error_keeps_previous_index() {
        use std::os::unix::fs::PermissionsExt;

        let (_temp, root, db, id, scanner) = fixture("素材");
        write_png(&root.join("可读.png"));
        let locked = root.join("不可读目录");
        std::fs::create_dir(&locked).unwrap();
        write_png(&locked.join("不可读.png"));
        let legacy = db
            .index_batch(id, 0, &[old_record(Path::new("已删除.png"))])
            .unwrap();
        db.index_folders(id, 0, &["旧空目录".into()]).unwrap();
        let permissions = std::fs::metadata(&locked).unwrap().permissions();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o0)).unwrap();
        // Root can bypass mode bits; only exercise the permission regression when denied.
        if std::fs::read_dir(&locked).is_ok() {
            std::fs::set_permissions(&locked, permissions).unwrap();
            return;
        }
        scanner.start(id);
        let status = scan_status(&scanner, id).await;
        std::fs::set_permissions(&locked, permissions).unwrap();
        assert_eq!(status.state, "error");
        assert!(
            status
                .error
                .unwrap()
                .starts_with("扫描未完成，保留原有索引：")
        );
        assert!(db.asset(legacy[0].0.id).is_ok());
        assert!(
            db.folders(id, "")
                .unwrap()
                .folders
                .iter()
                .any(|folder| folder.name == "旧空目录")
        );
        assert_eq!(db.stats().unwrap().total_assets, 2);
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

    #[tokio::test]
    async fn directory_scans_include_empty_folders_and_prune_removed_branches() {
        let (_temp, root, db, id, scanner) = fixture("素材");
        std::fs::create_dir_all(root.join("空目录").join("深层空目录")).unwrap();
        std::fs::create_dir_all(root.join("相册").join("嵌套")).unwrap();
        std::fs::create_dir(root.join("相册2")).unwrap();
        write_png(&root.join("root.png"));
        write_png(&root.join("相册").join("a.png"));
        write_png(&root.join("相册").join("嵌套").join("b.png"));
        scanner.start(id);
        wait_scan(&scanner, id).await;
        let folders = db.folders(id, "").unwrap();
        assert_eq!(folders.folders.len(), 3);
        assert_eq!(folders.direct_asset_count, 1);
        let empty = folders
            .folders
            .iter()
            .find(|folder| folder.name == "空目录")
            .unwrap();
        assert_eq!(empty.asset_count, 0);
        assert!(empty.has_children);
        assert_eq!(
            db.folders(id, "空目录").unwrap().folders[0].name,
            "深层空目录"
        );
        let album = folders
            .folders
            .iter()
            .find(|folder| folder.name == "相册")
            .unwrap();
        assert_eq!((album.asset_count, album.direct_asset_count), (2, 1));
        std::fs::remove_dir_all(root.join("空目录")).unwrap();
        std::fs::remove_dir_all(root.join("相册").join("嵌套")).unwrap();
        scanner.start(id);
        wait_scan(&scanner, id).await;
        let folders = db.folders(id, "").unwrap();
        assert_eq!(folders.folders.len(), 2);
        let album = folders
            .folders
            .iter()
            .find(|folder| folder.name == "相册")
            .unwrap();
        assert!(!album.has_children);
        assert_eq!((album.asset_count, album.direct_asset_count), (1, 1));
        assert!(root.join("相册2").exists());
    }

    #[tokio::test]
    async fn cancelling_a_queued_scan_keeps_existing_folders_and_assets() {
        let (_temp, root, db, id, scanner) = fixture("素材");
        db.index_folders(id, 0, &["旧空目录".into()]).unwrap();
        let indexed = db
            .index_batch(
                id,
                0,
                &[
                    old_record(&Path::new("旧目录").join("旧图.png")),
                    old_record(&Path::new(".git").join("旧图.png")),
                ],
            )
            .unwrap();
        // Keep the traversal queued so cancellation deterministically happens before any pruning.
        let permit = scanner.gate.clone().acquire_owned().await.unwrap();
        scanner.start(id);
        scanner.cancel(id);
        drop(permit);
        wait_scan(&scanner, id).await;
        assert!(db.asset(indexed[0].0.id).is_ok());
        assert!(db.asset(indexed[1].0.id).is_ok());
        assert_eq!(db.folders(id, "").unwrap().folders.len(), 2);
        assert!(!root.join("旧目录").exists());
    }
}
