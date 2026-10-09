use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter, types::Value};

use crate::{
    folders,
    models::{
        Asset, AssetBatch, AssetList, AssetPatch, AssetQuery, Folder, FolderList, Library,
        ScanStatus, Stats, Tag,
    },
};

const ASSET_COLUMNS: &str = "a.id,a.library_id,a.name,a.relative_path,a.format,a.size,a.width,a.height,a.modified_at,a.favorite,a.mtime_ns,l.path,a.thumbnail_error,COALESCE((SELECT json_group_array(tag) FROM (SELECT tag FROM asset_tags WHERE asset_id=a.id ORDER BY tag)), '[]')";

#[derive(Clone)]
pub struct Db(Arc<Mutex<Connection>>);

#[derive(Debug)]
pub struct FileRecord {
    pub relative_path: String,
    pub name: String,
    pub format: String,
    pub size: i64,
    pub modified_at: i64,
    pub mtime_ns: i64,
}

fn asset_from_row(row: &Row<'_>) -> rusqlite::Result<Asset> {
    let id: i64 = row.get(0)?;
    let mtime_ns: i64 = row.get(10)?;
    let size: i64 = row.get(5)?;
    let width: Option<u32> = row.get(6)?;
    let height: Option<u32> = row.get(7)?;
    let thumbnail_error: Option<String> = row.get(12)?;
    let cache_key = format!("{id}-{mtime_ns}-{size}");
    let thumbnail_state = if thumbnail_error.is_some() {
        "error"
    } else if width.is_some() && height.is_some() {
        "ready"
    } else {
        "pending"
    };
    let tags: String = row.get(13)?;
    Ok(Asset {
        id,
        library_id: row.get(1)?,
        name: row.get(2)?,
        relative_path: row.get(3)?,
        format: row.get(4)?,
        size,
        width,
        height,
        modified_at: row.get(8)?,
        favorite: row.get(9)?,
        mtime_ns,
        library_path: row.get(11)?,
        thumbnail_error,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        thumbnail_url: format!("/api/assets/{id}/thumbnail?v={cache_key}-{thumbnail_state}"),
        original_url: format!("/api/assets/{id}/original?v={cache_key}"),
    })
}

fn query_asset(conn: &Connection, id: i64) -> Result<Asset> {
    conn.query_row(
        &format!("SELECT {ASSET_COLUMNS} FROM assets a JOIN libraries l ON l.id=a.library_id WHERE a.id=?1"),
        [id], asset_from_row,
    ).map_err(Into::into)
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).context("无法打开数据库")?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS libraries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                path TEXT NOT NULL UNIQUE
            );
            CREATE TABLE IF NOT EXISTS assets (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                library_id INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
                relative_path TEXT NOT NULL,
                name TEXT NOT NULL,
                format TEXT NOT NULL,
                size INTEGER NOT NULL,
                width INTEGER,
                height INTEGER,
                modified_at INTEGER NOT NULL,
                mtime_ns INTEGER NOT NULL,
                favorite INTEGER NOT NULL DEFAULT 0,
                seen_generation INTEGER NOT NULL,
                thumbnail_error TEXT,
                UNIQUE(library_id,relative_path)
            );
            CREATE TABLE IF NOT EXISTS asset_tags (
                asset_id INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
                tag TEXT NOT NULL,
                PRIMARY KEY(asset_id,tag)
            );
            CREATE INDEX IF NOT EXISTS assets_modified ON assets(modified_at DESC,id DESC);
            CREATE INDEX IF NOT EXISTS assets_library ON assets(library_id,modified_at DESC,id DESC);
            CREATE INDEX IF NOT EXISTS assets_favorite ON assets(favorite,modified_at DESC,id DESC);
            CREATE INDEX IF NOT EXISTS asset_tags_name ON asset_tags(tag,asset_id);
            PRAGMA user_version=1;")?;
        Ok(Self(Arc::new(Mutex::new(conn))))
    }

    fn with<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut conn = self.0.lock().map_err(|_| anyhow!("数据库锁异常"))?;
        f(&mut conn)
    }

    pub fn libraries(&self) -> Result<Vec<Library>> {
        self.with(|conn| {
            let mut stmt = conn.prepare("SELECT l.id,l.name,l.path,(SELECT COUNT(*) FROM assets WHERE library_id=l.id) FROM libraries l ORDER BY l.id")?;
            Ok(stmt.query_map([], |r| Ok(Library {
                id: r.get(0)?, name: r.get(1)?, path: r.get(2)?, asset_count: r.get(3)?, scan: ScanStatus::default(),
            }))?.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    pub fn library(&self, id: i64) -> Result<Library> {
        self.with(|conn| Ok(conn.query_row("SELECT l.id,l.name,l.path,(SELECT COUNT(*) FROM assets WHERE library_id=l.id) FROM libraries l WHERE l.id=?1", [id], |r| Ok(Library {
            id: r.get(0)?, name: r.get(1)?, path: r.get(2)?, asset_count: r.get(3)?, scan: ScanStatus::default(),
        }))?))
    }

    pub fn add_library(&self, name: &str, path: &str) -> Result<Library> {
        let id = self.with(|conn| {
            conn.execute(
                "INSERT INTO libraries(name,path) VALUES(?1,?2)",
                params![name, path],
            )?;
            Ok(conn.last_insert_rowid())
        })?;
        self.library(id)
    }

    pub fn delete_library(&self, id: i64) -> Result<bool> {
        self.with(|conn| Ok(conn.execute("DELETE FROM libraries WHERE id=?1", [id])? > 0))
    }

    pub fn stats(&self) -> Result<Stats> {
        self.with(|conn| Ok(conn.query_row("SELECT COUNT(*),COALESCE(SUM(size),0),COALESCE(SUM(favorite),0),(SELECT COUNT(*) FROM libraries) FROM assets", [], |r| Ok(Stats {
            total_assets: r.get(0)?, total_size: r.get(1)?, total_favorites: r.get(2)?, total_libraries: r.get(3)?,
        }))?))
    }

    pub fn asset(&self, id: i64) -> Result<Asset> {
        self.with(|conn| query_asset(conn, id))
    }

    pub fn assets(&self, query: &AssetQuery) -> Result<AssetList> {
        let offset = query.offset.unwrap_or(0);
        let limit = query.limit.unwrap_or(100).clamp(1, 200);
        let mut filters = vec!["1=1".to_owned()];
        let mut values = Vec::<Value>::new();
        if let Some(id) = query.library_id {
            filters.push("a.library_id=?".into());
            values.push(id.into());
        }
        if let Some(folder) = query.folder.as_deref().filter(|path| !path.is_empty()) {
            let (lower, upper) = folders::subtree_bounds(folder);
            filters.push("a.relative_path>=? AND a.relative_path<?".into());
            values.extend([lower.into(), upper.into()]);
        }
        if let Some(favorite) = query.favorite {
            filters.push("a.favorite=?".into());
            values.push(i64::from(favorite).into());
        }
        if let Some(format) = &query.format {
            filters.push("a.format=?".into());
            values.push(format.to_lowercase().into());
        }
        if let Some(tag) = &query.tag {
            filters.push("EXISTS(SELECT 1 FROM asset_tags WHERE asset_id=a.id AND tag=?)".into());
            values.push(tag.trim().to_owned().into());
        }
        if let Some(q) = query.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
            let literal = q
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            let pattern = format!("%{literal}%");
            filters.push("(a.name LIKE ? ESCAPE '\\' OR a.relative_path LIKE ? ESCAPE '\\' OR EXISTS(SELECT 1 FROM asset_tags WHERE asset_id=a.id AND tag LIKE ? ESCAPE '\\'))".into());
            values.extend([
                pattern.clone().into(),
                pattern.clone().into(),
                pattern.into(),
            ]);
        }
        let where_clause = filters.join(" AND ");
        let order = match query.sort.as_deref() {
            Some("name") => "a.name COLLATE NOCASE ASC,a.id ASC",
            Some("size") => "a.size DESC,a.id DESC",
            _ => "a.modified_at DESC,a.id DESC",
        };
        self.with(|conn| {
            let total = conn.query_row(&format!("SELECT COUNT(*) FROM assets a WHERE {where_clause}"), params_from_iter(values.iter()), |r| r.get(0))?;
            let mut page_values = values.clone();
            page_values.extend([i64::from(limit).into(),i64::from(offset).into()]);
            let mut stmt = conn.prepare(&format!("SELECT {ASSET_COLUMNS} FROM assets a JOIN libraries l ON l.id=a.library_id WHERE {where_clause} ORDER BY {order} LIMIT ? OFFSET ?"))?;
            let assets = stmt.query_map(params_from_iter(page_values.iter()),asset_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(AssetList { assets, total, offset, limit })
        })
    }

    pub fn patch_asset(&self, id: i64, patch: &AssetPatch) -> Result<Asset> {
        self.with(|conn| {
            let tx = conn.transaction()?;
            // Check existence before writing tags (and before reporting an empty patch as success).
            query_asset(&tx, id)?;
            if let Some(favorite) = patch.favorite {
                tx.execute(
                    "UPDATE assets SET favorite=?1 WHERE id=?2",
                    params![favorite, id],
                )?;
            }
            if let Some(tags) = &patch.tags {
                tx.execute("DELETE FROM asset_tags WHERE asset_id=?1", [id])?;
                for tag in tags {
                    let tag = tag.trim();
                    if !tag.is_empty() {
                        tx.execute(
                            "INSERT OR IGNORE INTO asset_tags(asset_id,tag) VALUES(?1,?2)",
                            params![id, tag],
                        )?;
                    }
                }
            }
            tx.commit()?;
            query_asset(conn, id)
        })
    }

    pub fn batch_assets(&self, batch: &AssetBatch) -> Result<usize> {
        self.with(|conn| {
            let tx = conn.transaction()?;
            for id in &batch.ids {
                tx.query_row("SELECT id FROM assets WHERE id=?1", [id], |row| {
                    row.get::<_, i64>(0)
                })?;
                if let Some(favorite) = batch.favorite {
                    tx.execute(
                        "UPDATE assets SET favorite=?1 WHERE id=?2",
                        params![favorite, id],
                    )?;
                }
                if let Some(tags) = &batch.remove_tags {
                    let mut statement =
                        tx.prepare_cached("DELETE FROM asset_tags WHERE asset_id=?1 AND tag=?2")?;
                    for tag in tags {
                        statement.execute(params![id, tag])?;
                    }
                }
                if let Some(tags) = &batch.add_tags {
                    let mut statement = tx.prepare_cached(
                        "INSERT OR IGNORE INTO asset_tags(asset_id,tag) VALUES(?1,?2)",
                    )?;
                    for tag in tags {
                        statement.execute(params![id, tag])?;
                    }
                }
                if batch.add_tags.is_some() {
                    let count = tx.query_row(
                        "SELECT COUNT(*) FROM asset_tags WHERE asset_id=?1",
                        [id],
                        |row| row.get::<_, i64>(0),
                    )?;
                    anyhow::ensure!(count <= 50, "批量操作后每张图片最多 50 个标签");
                }
            }
            tx.commit()?;
            Ok(batch.ids.len())
        })
    }

    pub fn folders(&self, library_id: i64, parent: &str) -> Result<FolderList> {
        // Validate the library before returning an empty result for an unknown id.
        self.library(library_id)?;
        let mut counts = BTreeMap::<String, i64>::new();
        let mut cursor = String::new();
        let mut truncated = false;
        let (lower, upper) = folders::subtree_bounds(parent);
        'pages: loop {
            let paths = self.with(|conn| {
                let mut sql = "SELECT relative_path FROM assets WHERE library_id=?".to_owned();
                let mut values = vec![Value::from(library_id)];
                if !parent.is_empty() {
                    sql.push_str(" AND relative_path>=? AND relative_path<?");
                    values.extend([lower.clone().into(), upper.clone().into()]);
                }
                if !cursor.is_empty() {
                    sql.push_str(" AND relative_path>?");
                    values.push(cursor.clone().into());
                }
                sql.push_str(" ORDER BY relative_path LIMIT 500");
                let mut stmt = conn.prepare(&sql)?;
                Ok(stmt
                    .query_map(params_from_iter(values.iter()), |row| {
                        row.get::<_, String>(0)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?)
            })?;
            if paths.is_empty() {
                break;
            }
            cursor = paths.last().unwrap().clone();
            for relative in paths {
                let tail = if parent.is_empty() {
                    relative.as_str()
                } else {
                    relative.strip_prefix(&lower).unwrap_or_default()
                };
                let mut components = Path::new(tail).components();
                let Some(name) = components.next().and_then(|c| c.as_os_str().to_str()) else {
                    continue;
                };
                if components.next().is_none() {
                    continue;
                }
                // Paths are ordered, so the first 1000 children's counts are complete when child 1001 starts.
                if !counts.contains_key(name) && counts.len() == 1000 {
                    truncated = true;
                    break 'pages;
                }
                *counts.entry(name.to_owned()).or_default() += 1;
            }
        }
        let folders = counts
            .into_iter()
            .map(|(name, asset_count)| Folder {
                path: Path::new(parent).join(&name).to_string_lossy().into_owned(),
                name,
                parent: (!parent.is_empty()).then(|| parent.to_owned()),
                asset_count,
            })
            .collect();
        Ok(FolderList {
            folders,
            parent: parent.to_owned(),
            truncated,
        })
    }

    pub fn tags(&self) -> Result<Vec<Tag>> {
        self.with(|conn| {
            let mut stmt = conn.prepare("SELECT tag,COUNT(*) FROM asset_tags GROUP BY tag ORDER BY COUNT(*) DESC,tag ASC LIMIT 2000")?;
            Ok(stmt.query_map([], |r| Ok(Tag { name: r.get(0)?, count: r.get(1)? }))?.collect::<rusqlite::Result<_>>()?)
        })
    }

    pub fn index_batch(
        &self,
        library_id: i64,
        generation: i64,
        records: &[FileRecord],
    ) -> Result<Vec<(Asset, Option<String>)>> {
        self.with(|conn| {
            let tx = conn.transaction()?;
            let mut result = Vec::with_capacity(records.len());
            for r in records {
                let previous = tx.query_row("SELECT id,mtime_ns,size FROM assets WHERE library_id=?1 AND relative_path=?2",params![library_id,r.relative_path],|row| Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?))).optional()?;
                let obsolete_key=previous.filter(|(_,mtime,size)| *mtime!=r.mtime_ns || *size!=r.size).map(|(id,mtime,size)|format!("{id}-{mtime}-{size}"));
                let id = tx.query_row("INSERT INTO assets(library_id,relative_path,name,format,size,modified_at,mtime_ns,seen_generation)
                    VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                    ON CONFLICT(library_id,relative_path) DO UPDATE SET
                    name=excluded.name,format=excluded.format,
                    width=CASE WHEN assets.mtime_ns=excluded.mtime_ns AND assets.size=excluded.size THEN assets.width ELSE NULL END,
                    height=CASE WHEN assets.mtime_ns=excluded.mtime_ns AND assets.size=excluded.size THEN assets.height ELSE NULL END,
                    thumbnail_error=CASE WHEN assets.mtime_ns=excluded.mtime_ns AND assets.size=excluded.size THEN assets.thumbnail_error ELSE NULL END,
                    size=excluded.size,modified_at=excluded.modified_at,mtime_ns=excluded.mtime_ns,seen_generation=excluded.seen_generation
                    RETURNING id", params![library_id,r.relative_path,r.name,r.format,r.size,r.modified_at,r.mtime_ns,generation], |row| row.get(0))?;
                result.push((query_asset(&tx,id)?,obsolete_key));
            }
            tx.commit()?;
            Ok(result)
        })
    }

    pub fn remove_unseen_batch(&self, library_id: i64, generation: i64) -> Result<Vec<String>> {
        self.with(|conn| {
            let tx = conn.transaction()?;
            let removed = {
                let mut stmt = tx.prepare("SELECT id,mtime_ns,size FROM assets WHERE library_id=?1 AND seen_generation<>?2 ORDER BY id LIMIT 200")?;
                stmt.query_map(params![library_id,generation],|row| {
                    let id:i64=row.get(0)?;
                    let mtime:i64=row.get(1)?;
                    let size:i64=row.get(2)?;
                    Ok(format!("{id}-{mtime}-{size}"))
                })?.collect::<rusqlite::Result<Vec<_>>>()?
            };
            tx.execute("DELETE FROM assets WHERE id IN (SELECT id FROM assets WHERE library_id=?1 AND seen_generation<>?2 ORDER BY id LIMIT 200)",params![library_id,generation])?;
            tx.commit()?;
            Ok(removed)
        })
    }

    pub fn set_thumbnail_result(
        &self,
        asset: &Asset,
        width: Option<u32>,
        height: Option<u32>,
        error: Option<&str>,
    ) -> Result<()> {
        self.with(|conn| {
            conn.execute("UPDATE assets SET width=COALESCE(?1,width),height=COALESCE(?2,height),thumbnail_error=?3 WHERE id=?4 AND mtime_ns=?5 AND size=?6",params![width,height,error,asset.id,asset.mtime_ns,asset.size])?;
            Ok(())
        })
    }

    pub fn retry_failed_thumbnails(&self, library_id: i64) -> Result<()> {
        self.with(|conn| {
            conn.execute("UPDATE assets SET thumbnail_error=NULL WHERE library_id=?1 AND thumbnail_error IS NOT NULL",[library_id])?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(path: &str, size: i64, mtime_ns: i64) -> FileRecord {
        FileRecord {
            relative_path: path.into(),
            name: Path::new(path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into(),
            format: "png".into(),
            size,
            modified_at: mtime_ns / 1_000_000_000,
            mtime_ns,
        }
    }

    #[test]
    fn indexes_unicode_paths_and_preserves_user_metadata_on_rescan() {
        let temp = tempfile::tempdir().unwrap();
        let db = Db::open(&temp.path().join("test.sqlite")).unwrap();
        let library = db
            .add_library("素材", temp.path().to_str().unwrap())
            .unwrap();
        let files = [
            record("风景/海边.png", 100, 1_000_000_000),
            record("人物/海边.png", 200, 2_000_000_000),
            record("100%完成.png", 300, 3_000_000_000),
        ];
        let initial = db.index_batch(library.id, 1, &files).unwrap();
        let id = initial[0].0.id;
        db.patch_asset(
            id,
            &AssetPatch {
                favorite: Some(true),
                tags: Some(vec![" 壁纸 ".into(), "壁纸".into(), "蓝色".into()]),
            },
        )
        .unwrap();
        db.set_thumbnail_result(&initial[0].0, Some(640), Some(480), None)
            .unwrap();
        db.index_batch(library.id, 2, &files).unwrap();
        let same = db.asset(id).unwrap();
        assert!(same.favorite);
        assert_eq!(same.tags, vec!["壁纸", "蓝色"]);
        assert_eq!((same.width, same.height), (Some(640), Some(480)));
        let found = db
            .assets(&AssetQuery {
                q: Some("海边".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(found.total, 2);
        assert_eq!(
            db.assets(&AssetQuery {
                q: Some("壁纸".into()),
                favorite: Some(true),
                ..Default::default()
            })
            .unwrap()
            .total,
            1
        );
        assert_eq!(
            db.assets(&AssetQuery {
                q: Some("%".into()),
                ..Default::default()
            })
            .unwrap()
            .total,
            1
        );
        let changed = db
            .index_batch(
                library.id,
                3,
                &[record("风景/海边.png", 150, 4_000_000_000)],
            )
            .unwrap();
        assert!(changed[0].1.is_some());
        assert_eq!(changed[0].0.id, id);
        assert_eq!(changed[0].0.width, None);
        assert!(changed[0].0.favorite);
        let removed = db.remove_unseen_batch(library.id, 3).unwrap();
        assert_eq!(removed.len(), 2);
        assert_eq!(db.stats().unwrap().total_assets, 1);
        assert_eq!(db.tags().unwrap()[0].count, 1);
        assert!(db.delete_library(library.id).unwrap());
        assert_eq!(db.stats().unwrap().total_assets, 0);
        assert!(temp.path().exists());
    }

    #[test]
    fn pagination_has_stable_order_and_bounded_limit() {
        let temp = tempfile::tempdir().unwrap();
        let db = Db::open(&temp.path().join("test.sqlite")).unwrap();
        let library = db
            .add_library("图库", temp.path().to_str().unwrap())
            .unwrap();
        db.index_batch(
            library.id,
            1,
            &[
                record("a.png", 100, 0),
                record("b.png", 100, 0),
                record("c.png", 100, 0),
            ],
        )
        .unwrap();
        let page1 = db
            .assets(&AssetQuery {
                limit: Some(1),
                sort: Some("name".into()),
                ..Default::default()
            })
            .unwrap();
        let page2 = db
            .assets(&AssetQuery {
                limit: Some(1),
                offset: Some(1),
                sort: Some("name".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page1.assets[0].name, "a.png");
        assert_eq!(page2.assets[0].name, "b.png");
        assert_eq!(page1.total, 3);
        assert_eq!(
            db.assets(&AssetQuery {
                limit: Some(u32::MAX),
                ..Default::default()
            })
            .unwrap()
            .limit,
            200
        );
    }

    #[test]
    fn failed_thumbnails_retry_only_when_explicitly_requested() {
        let temp = tempfile::tempdir().unwrap();
        let db = Db::open(&temp.path().join("test.sqlite")).unwrap();
        let library = db
            .add_library("图库", temp.path().to_str().unwrap())
            .unwrap();
        let records = [record("image.png", 100, 1_000_000_000)];
        let asset = db.index_batch(library.id, 1, &records).unwrap()[0]
            .0
            .clone();
        assert!(asset.thumbnail_url.ends_with("-pending"));
        db.set_thumbnail_result(&asset, None, None, Some("暂时无权限"))
            .unwrap();
        let failed_url = db.asset(asset.id).unwrap().thumbnail_url;
        assert!(failed_url.ends_with("-error"));
        assert_ne!(failed_url, asset.thumbnail_url);
        db.index_batch(library.id, 2, &records).unwrap();
        assert_eq!(
            db.asset(asset.id).unwrap().thumbnail_error.as_deref(),
            Some("暂时无权限")
        );
        db.retry_failed_thumbnails(library.id).unwrap();
        let retry = db.asset(asset.id).unwrap();
        assert!(retry.thumbnail_error.is_none());
        assert_eq!(retry.thumbnail_url, asset.thumbnail_url);
        assert_ne!(retry.thumbnail_url, failed_url);
        db.set_thumbnail_result(&retry, Some(32), Some(24), None)
            .unwrap();
        let ready = db.asset(asset.id).unwrap();
        assert!(ready.thumbnail_url.ends_with("-ready"));
        assert_ne!(ready.thumbnail_url, retry.thumbnail_url);
        assert_eq!(ready.original_url, asset.original_url);
    }

    #[test]
    fn pruning_uses_bounded_batches() {
        let temp = tempfile::tempdir().unwrap();
        let db = Db::open(&temp.path().join("test.sqlite")).unwrap();
        let library = db
            .add_library("图库", temp.path().to_str().unwrap())
            .unwrap();
        let records = (0..205)
            .map(|i| record(&format!("{i}.png"), 100, 1_000_000_000))
            .collect::<Vec<_>>();
        db.index_batch(library.id, 1, &records).unwrap();
        assert_eq!(db.remove_unseen_batch(library.id, 2).unwrap().len(), 200);
        assert_eq!(db.stats().unwrap().total_assets, 5);
        assert_eq!(db.remove_unseen_batch(library.id, 2).unwrap().len(), 5);
        assert!(db.remove_unseen_batch(library.id, 2).unwrap().is_empty());
    }

    #[test]
    fn batches_are_atomic_when_an_asset_is_missing_or_tags_overflow() {
        let temp = tempfile::tempdir().unwrap();
        let db = Db::open(&temp.path().join("test.sqlite")).unwrap();
        let library = db
            .add_library("素材", temp.path().to_str().unwrap())
            .unwrap();
        let indexed = db
            .index_batch(
                library.id,
                1,
                &[record("a.png", 100, 1), record("b.png", 100, 2)],
            )
            .unwrap();
        let ids = indexed
            .iter()
            .map(|(asset, _)| asset.id)
            .collect::<Vec<_>>();
        assert_eq!(
            db.batch_assets(&AssetBatch {
                ids: ids.clone(),
                favorite: Some(true),
                add_tags: Some(vec!["标签".into(), "保留".into()]),
                remove_tags: None
            })
            .unwrap(),
            2
        );
        assert!(
            db.batch_assets(&AssetBatch {
                ids: vec![ids[0], 999999],
                favorite: Some(false),
                add_tags: None,
                remove_tags: Some(vec!["保留".into()])
            })
            .is_err()
        );
        assert!(db.asset(ids[0]).unwrap().favorite);
        assert!(db.asset(ids[0]).unwrap().tags.contains(&"保留".into()));
        db.patch_asset(
            ids[1],
            &AssetPatch {
                favorite: None,
                tags: Some((0..50).map(|i| format!("tag{i}")).collect()),
            },
        )
        .unwrap();
        assert!(
            db.batch_assets(&AssetBatch {
                ids: ids.clone(),
                favorite: Some(false),
                add_tags: Some(vec!["新标签".into()]),
                remove_tags: None
            })
            .is_err()
        );
        assert!(db.asset(ids[0]).unwrap().favorite);
        assert!(!db.asset(ids[0]).unwrap().tags.contains(&"新标签".into()));
        db.batch_assets(&AssetBatch {
            ids: vec![ids[0]],
            favorite: Some(false),
            add_tags: Some(vec!["标签".into()]),
            remove_tags: Some(vec!["标签".into(), "保留".into()]),
        })
        .unwrap();
        let updated = db.asset(ids[0]).unwrap();
        assert!(!updated.favorite);
        assert_eq!(updated.tags, vec!["标签"]);
    }

    #[test]
    fn indexed_folder_navigation_counts_descendants_and_filters_exact_subtrees() {
        let temp = tempfile::tempdir().unwrap();
        let db = Db::open(&temp.path().join("test.sqlite")).unwrap();
        let library = db
            .add_library("素材", temp.path().to_str().unwrap())
            .unwrap();
        let paths = [
            "root.png",
            "旅行/a.png",
            "旅行/海边/b.png",
            "旅行2/c.png",
            "Travel/d.png",
            "travel/e.png",
        ];
        let records = paths
            .iter()
            .map(|path| {
                let native = path.replace('/', std::path::MAIN_SEPARATOR_STR);
                record(&native, 100, 1)
            })
            .collect::<Vec<_>>();
        db.index_batch(library.id, 1, &records).unwrap();
        let root = db.folders(library.id, "").unwrap();
        assert_eq!(root.folders.len(), 4);
        assert_eq!(
            root.folders
                .iter()
                .find(|f| f.name == "旅行")
                .unwrap()
                .asset_count,
            2
        );
        assert!(!root.truncated);
        let nested = db.folders(library.id, "旅行").unwrap();
        assert_eq!(nested.folders.len(), 1);
        assert_eq!(nested.folders[0].name, "海边");
        assert_eq!(nested.folders[0].parent.as_deref(), Some("旅行"));
        assert_eq!(nested.folders[0].asset_count, 1);
        let filtered = db
            .assets(&AssetQuery {
                library_id: Some(library.id),
                folder: Some("旅行".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(filtered.total, 2);
        assert_eq!(
            db.assets(&AssetQuery {
                library_id: Some(library.id),
                folder: Some("Travel".into()),
                ..Default::default()
            })
            .unwrap()
            .total,
            1
        );
        assert_eq!(
            db.assets(&AssetQuery {
                library_id: Some(library.id),
                folder: Some(String::new()),
                ..Default::default()
            })
            .unwrap()
            .total,
            6
        );
        assert!(db.folders(999999, "").is_err());
    }

    #[test]
    fn folder_navigation_limits_sibling_count_without_loading_assets() {
        let temp = tempfile::tempdir().unwrap();
        let db = Db::open(&temp.path().join("test.sqlite")).unwrap();
        let library = db
            .add_library("素材", temp.path().to_str().unwrap())
            .unwrap();
        let mut records = (0..1002)
            .map(|i| {
                record(
                    Path::new(&format!("folder{i:04}"))
                        .join("asset.png")
                        .to_str()
                        .unwrap(),
                    100,
                    1,
                )
            })
            .collect::<Vec<_>>();
        for i in 0..700 {
            records.push(record(
                Path::new("folder0999")
                    .join("nested")
                    .join(format!("child{i:04}.png"))
                    .to_str()
                    .unwrap(),
                100,
                1,
            ));
        }
        for records in records.chunks(100) {
            db.index_batch(library.id, 1, records).unwrap();
        }
        let result = db.folders(library.id, "").unwrap();
        assert_eq!(result.folders.len(), 1000);
        assert!(result.truncated);
        assert_eq!(
            result
                .folders
                .iter()
                .find(|f| f.name == "folder0999")
                .unwrap()
                .asset_count,
            701
        );
        assert!(
            result
                .folders
                .iter()
                .filter(|f| f.name != "folder0999")
                .all(|folder| folder.asset_count == 1)
        );
    }
}
