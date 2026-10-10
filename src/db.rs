use std::{
    collections::BTreeSet,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use rusqlite::{
    Connection, OptionalExtension, Row, Transaction, params, params_from_iter, types::Value,
};

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

fn substring_pattern(value: &str) -> String {
    let literal = value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{literal}%")
}

fn index_folder(
    tx: &Transaction<'_>,
    library_id: i64,
    generation: i64,
    folder: &str,
) -> Result<()> {
    // Index ancestors too, so legacy indexes and image batches can construct the same tree.
    let mut path = Path::new(folder);
    loop {
        let relative = path.to_str().context("素材子目录路径无效")?;
        let parent = path.parent().and_then(Path::to_str).unwrap_or_default();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        tx.prepare_cached(
            "INSERT INTO library_folders(library_id,relative_path,parent,name,seen_generation)
             VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(library_id,relative_path) DO UPDATE SET seen_generation=excluded.seen_generation
             WHERE library_folders.seen_generation<>excluded.seen_generation",
        )?.execute(params![library_id, relative, parent, name, generation])?;
        if relative.is_empty() {
            break;
        }
        path = path.parent().unwrap_or_else(|| Path::new(""));
    }
    Ok(())
}

fn migrate_folders(conn: &mut Connection) -> Result<()> {
    // Older installations only recorded files. Backfill their parents without reading the disk.
    // Paging keeps migration memory bounded even when a library contains millions of images.
    let tx = conn.transaction()?;
    tx.execute(
        "ALTER TABLE assets ADD COLUMN parent_folder TEXT NOT NULL DEFAULT ''",
        [],
    )?;
    let mut cursor = 0;
    loop {
        let paths = {
            let mut statement = tx.prepare("SELECT id,library_id,relative_path,seen_generation FROM assets WHERE id>?1 ORDER BY id LIMIT 500")?;
            statement
                .query_map([cursor], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        if paths.is_empty() {
            break;
        }
        for (id, library_id, relative, generation) in paths {
            let parent = Path::new(&relative)
                .parent()
                .and_then(Path::to_str)
                .unwrap_or_default();
            tx.execute(
                "UPDATE assets SET parent_folder=?1 WHERE id=?2",
                params![parent, id],
            )?;
            index_folder(&tx, library_id, generation, parent)?;
            cursor = id;
        }
    }
    tx.execute_batch("PRAGMA user_version=2;")?;
    tx.commit()?;
    Ok(())
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path).context("无法打开数据库")?;
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
            CREATE TABLE IF NOT EXISTS library_folders (
                library_id INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
                relative_path TEXT NOT NULL,
                parent TEXT NOT NULL,
                name TEXT NOT NULL,
                seen_generation INTEGER NOT NULL,
                PRIMARY KEY(library_id,relative_path)
            );
            CREATE INDEX IF NOT EXISTS assets_modified ON assets(modified_at DESC,id DESC);
            CREATE INDEX IF NOT EXISTS assets_library ON assets(library_id,modified_at DESC,id DESC);
            CREATE INDEX IF NOT EXISTS assets_favorite ON assets(favorite,modified_at DESC,id DESC);
            CREATE INDEX IF NOT EXISTS asset_tags_name ON asset_tags(tag,asset_id);
            CREATE INDEX IF NOT EXISTS library_folders_parent ON library_folders(library_id,parent,relative_path);")?;
        let has_parent = {
            let mut statement = conn.prepare("PRAGMA table_info(assets)")?;
            statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?
                .iter()
                .any(|name| name == "parent_folder")
        };
        if !has_parent {
            migrate_folders(&mut conn)?;
        }
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS assets_folder ON assets(library_id,parent_folder,id);
            CREATE INDEX IF NOT EXISTS assets_dimensions ON assets(width,height);
            CREATE INDEX IF NOT EXISTS assets_size ON assets(size,id);",
        )?;
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
        query.validate()?;
        let offset = query.offset.unwrap_or(0);
        let limit = query.limit.unwrap_or(100).clamp(1, 200);
        let mut filters = vec!["1=1".to_owned()];
        let mut values = Vec::<Value>::new();
        if let Some(id) = query.library_id {
            filters.push("a.library_id=?".into());
            values.push(id.into());
        }
        if query.folder.is_some() || query.folder_recursive.is_some() {
            let folder = folders::normalize_folder(query.folder.as_deref().unwrap_or_default())?;
            if !query.folder_recursive.unwrap_or(true) {
                filters.push("a.parent_folder=?".into());
                values.push(folder.into());
            } else if !folder.is_empty() {
                let (lower, upper) = folders::subtree_bounds(&folder);
                filters.push("a.relative_path>=? AND a.relative_path<?".into());
                values.extend([lower.into(), upper.into()]);
            }
        }
        if let Some(orientation) = query.orientation.as_deref() {
            filters.push("a.width>0 AND a.height>0".into());
            filters.push(
                match orientation {
                    "landscape" => "a.width>a.height",
                    "portrait" => "a.width<a.height",
                    _ => "a.width=a.height",
                }
                .into(),
            );
        }
        if let Some(ratio) = query.aspect_ratio_value()? {
            filters.push(
                "a.width>0 AND a.height>0 AND CAST(a.width AS REAL)/a.height BETWEEN ? AND ?"
                    .into(),
            );
            values.extend([Value::Real(ratio * 0.98), Value::Real(ratio * 1.02)]);
        }
        for (column, min, max) in [
            (
                "a.width",
                query.min_width.map(i64::from),
                query.max_width.map(i64::from),
            ),
            (
                "a.height",
                query.min_height.map(i64::from),
                query.max_height.map(i64::from),
            ),
            ("a.size", query.min_size, query.max_size),
        ] {
            if let Some(min) = min {
                filters.push(format!("{column}>=?"));
                values.push(min.into());
            }
            if let Some(max) = max {
                filters.push(format!("{column}<=?"));
                values.push(max.into());
            }
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
        for name in query.normalize_excluded_names()? {
            filters.push("a.name NOT LIKE ? ESCAPE '\\'".into());
            values.push(substring_pattern(&name).into());
        }
        if let Some(q) = query.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
            let pattern = substring_pattern(q);
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
            Some("name_desc") => "a.name COLLATE NOCASE DESC,a.id DESC",
            Some("size") => "a.size DESC,a.id DESC",
            Some("size_asc") => "a.size ASC,a.id ASC",
            Some("width") => "a.width DESC,a.id DESC",
            Some("height") => "a.height DESC,a.id DESC",
            // SQLite promotes overflowing integer products to REAL.
            Some("pixels") => "a.width*a.height DESC,a.id DESC",
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
        self.folders_page(library_id, parent, None, 1000)
    }

    pub fn folders_page(
        &self,
        library_id: i64,
        parent: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<FolderList> {
        // Validate the library before returning an empty result for an unknown id.
        self.library(library_id)?;
        let parent = folders::normalize_folder(parent)?;
        anyhow::ensure!(
            (1..=1000).contains(&limit),
            "每页目录数量需要在 1–1000 之间"
        );
        let cursor = cursor
            .map(|cursor| folders::normalize_cursor(&parent, cursor))
            .transpose()?;
        if folders::is_hidden_subdirectory(Path::new(&parent)) {
            return Ok(FolderList {
                folders: Vec::new(),
                parent,
                truncated: false,
                direct_asset_count: 0,
                separator: std::path::MAIN_SEPARATOR.to_string(),
                next_cursor: None,
            });
        }
        self.with(|conn| {
            let direct_asset_count = conn.query_row(
                "SELECT COUNT(*) FROM assets WHERE library_id=?1 AND parent_folder=?2",
                params![library_id, parent], |row| row.get(0),
            )?;
            let children = {
                let mut sql =
                    "SELECT relative_path,name,
                     EXISTS(SELECT 1 FROM library_folders child WHERE child.library_id=f.library_id AND child.parent=f.relative_path AND child.relative_path<>'' AND child.name NOT GLOB '.*')
                     FROM library_folders f WHERE library_id=? AND parent=? AND relative_path<>'' AND name NOT GLOB '.*'".to_owned();
                let mut values = vec![Value::from(library_id), Value::from(parent.clone())];
                if let Some(cursor) = &cursor {
                    sql.push_str(" AND relative_path>?");
                    values.push(cursor.clone().into());
                }
                sql.push_str(" ORDER BY relative_path LIMIT ?");
                values.push(i64::from(limit + 1).into());
                let mut statement = conn.prepare(&sql)?;
                statement.query_map(params_from_iter(values.iter()), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, bool>(2)?))
                })?.collect::<rusqlite::Result<Vec<_>>>()?
            };
            let truncated = children.len() > limit as usize;
            let mut folders = Vec::with_capacity(children.len().min(limit as usize));
            let mut count = conn.prepare("SELECT COUNT(*) FROM assets WHERE library_id=?1 AND relative_path>=?2 AND relative_path<?3")?;
            let mut direct_count = conn.prepare("SELECT COUNT(*) FROM assets WHERE library_id=?1 AND parent_folder=?2")?;
            for (path, name, has_children) in children.into_iter().take(limit as usize) {
                let (lower, upper) = folders::subtree_bounds(&path);
                let asset_count = count.query_row(params![library_id, lower, upper], |row| row.get(0))?;
                let direct_asset_count = direct_count.query_row(params![library_id, path], |row| row.get(0))?;
                folders.push(Folder {
                    path, name, parent: (!parent.is_empty()).then(|| parent.clone()),
                    asset_count, direct_asset_count, has_children,
                });
            }
            let next_cursor = truncated.then(|| folders.last().unwrap().path.clone());
            Ok(FolderList { folders, parent, truncated, direct_asset_count, separator: std::path::MAIN_SEPARATOR.to_string(), next_cursor })
        })
    }

    pub fn index_folders(&self, library_id: i64, generation: i64, paths: &[String]) -> Result<()> {
        self.with(|conn| {
            let tx = conn.transaction()?;
            for path in paths {
                index_folder(&tx, library_id, generation, path)?;
            }
            tx.commit()?;
            Ok(())
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
            let mut indexed_parents = BTreeSet::new();
            for r in records {
                let parent = Path::new(&r.relative_path).parent().and_then(Path::to_str).unwrap_or_default();
                if indexed_parents.insert(parent) {
                    index_folder(&tx, library_id, generation, parent)?;
                }
                let previous = tx.query_row("SELECT id,mtime_ns,size FROM assets WHERE library_id=?1 AND relative_path=?2",params![library_id,r.relative_path],|row| Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?,row.get::<_,i64>(2)?))).optional()?;
                let obsolete_key=previous.filter(|(_,mtime,size)| *mtime!=r.mtime_ns || *size!=r.size).map(|(id,mtime,size)|format!("{id}-{mtime}-{size}"));
                let id = tx.query_row("INSERT INTO assets(library_id,relative_path,name,format,size,modified_at,mtime_ns,seen_generation,parent_folder)
                    VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
                    ON CONFLICT(library_id,relative_path) DO UPDATE SET
                    name=excluded.name,format=excluded.format,parent_folder=excluded.parent_folder,
                    width=CASE WHEN assets.mtime_ns=excluded.mtime_ns AND assets.size=excluded.size THEN assets.width ELSE NULL END,
                    height=CASE WHEN assets.mtime_ns=excluded.mtime_ns AND assets.size=excluded.size THEN assets.height ELSE NULL END,
                    thumbnail_error=CASE WHEN assets.mtime_ns=excluded.mtime_ns AND assets.size=excluded.size THEN assets.thumbnail_error ELSE NULL END,
                    size=excluded.size,modified_at=excluded.modified_at,mtime_ns=excluded.mtime_ns,seen_generation=excluded.seen_generation
                    RETURNING id", params![library_id,r.relative_path,r.name,r.format,r.size,r.modified_at,r.mtime_ns,generation,parent], |row| row.get(0))?;
                result.push((query_asset(&tx,id)?,obsolete_key));
            }
            tx.commit()?;
            Ok(result)
        })
    }

    pub fn retain_subtree(&self, library_id: i64, generation: i64, folder: &str) -> Result<()> {
        let (lower, upper) = folders::subtree_bounds(folder);
        self.with(|conn| {
            let tx = conn.transaction()?;
            tx.execute(
                "UPDATE assets SET seen_generation=?2 WHERE library_id=?1 AND relative_path>=?3 AND relative_path<?4",
                params![library_id, generation, lower, upper],
            )?;
            tx.execute(
                "UPDATE library_folders SET seen_generation=?2 WHERE library_id=?1 AND (relative_path=?3 OR (relative_path>=?4 AND relative_path<?5))",
                params![library_id, generation, folder, lower, upper],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn remove_unseen_folders_batch(&self, library_id: i64, generation: i64) -> Result<usize> {
        self.with(|conn| Ok(conn.execute(
            "DELETE FROM library_folders WHERE library_id=?1 AND relative_path IN
             (SELECT relative_path FROM library_folders WHERE library_id=?1 AND seen_generation<>?2 ORDER BY relative_path LIMIT 200)",
            params![library_id, generation],
        )?))
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

    #[test]
    fn retained_subtree_uses_exact_native_prefix_and_preserves_annotations() {
        let temp = tempfile::tempdir().unwrap();
        let db = Db::open(&temp.path().join("test.sqlite")).unwrap();
        let library = db.add_library("素材", "library-one").unwrap();
        let other_library = db.add_library("其他素材", "library-two").unwrap();
        let package = "中文.PHOTOSLIBRARY";
        let paths = [
            Path::new(package).join("保留.png"),
            Path::new(package).join("originals").join("深层.png"),
            Path::new("中文.photoslibrary").join("不同大小写.png"),
            Path::new("中文.PHOTOSLIBRARY.backup").join("不同目录.png"),
            Path::new(package).to_path_buf(),
        ];
        let records = paths
            .iter()
            .map(|path| record(path.to_str().unwrap(), 100, 1))
            .collect::<Vec<_>>();
        let indexed = db.index_batch(library.id, 1, &records).unwrap();
        let other = db.index_batch(other_library.id, 1, &records[..1]).unwrap();
        db.patch_asset(
            indexed[0].0.id,
            &AssetPatch {
                favorite: Some(true),
                tags: Some(vec!["原有标签".into()]),
            },
        )
        .unwrap();
        db.retain_subtree(library.id, 2, package).unwrap();
        assert_eq!(db.remove_unseen_batch(library.id, 2).unwrap().len(), 3);
        let retained = db.asset(indexed[0].0.id).unwrap();
        assert!(retained.favorite);
        assert_eq!(retained.tags, vec!["原有标签"]);
        assert!(db.asset(indexed[1].0.id).is_ok());
        for asset in &indexed[2..] {
            assert!(db.asset(asset.0.id).is_err());
        }
        assert!(db.asset(other[0].0.id).is_ok());
    }

    #[test]
    fn folder_tree_tracks_empty_directories_and_current_folder_counts() {
        let temp = tempfile::tempdir().unwrap();
        let db = Db::open(&temp.path().join("test.sqlite")).unwrap();
        let library = db
            .add_library("素材", temp.path().to_str().unwrap())
            .unwrap();
        let native = |path: &str| path.replace('/', std::path::MAIN_SEPARATOR_STR);
        db.index_folders(
            library.id,
            1,
            &[native("空/深层"), native("相册"), native("相册2")],
        )
        .unwrap();
        db.index_batch(
            library.id,
            1,
            &[
                record("root.png", 1, 1),
                record(&native("相册/a.png"), 1, 1),
                record(&native("相册/嵌套/b.png"), 1, 1),
                record(&native("相册2/c.png"), 1, 1),
            ],
        )
        .unwrap();
        let root = db.folders(library.id, "").unwrap();
        assert_eq!(root.direct_asset_count, 1);
        assert_eq!(root.separator, std::path::MAIN_SEPARATOR.to_string());
        let album = root
            .folders
            .iter()
            .find(|folder| folder.name == "相册")
            .unwrap();
        assert_eq!(
            (
                album.asset_count,
                album.direct_asset_count,
                album.has_children
            ),
            (2, 1, true)
        );
        let empty = root
            .folders
            .iter()
            .find(|folder| folder.name == "空")
            .unwrap();
        assert_eq!(
            (
                empty.asset_count,
                empty.direct_asset_count,
                empty.has_children
            ),
            (0, 0, true)
        );
        let child = &db.folders(library.id, "空").unwrap().folders[0];
        assert_eq!(child.name, "深层");
        assert!(!child.has_children);
        for (folder, count) in [("", 1), ("相册", 1), ("空", 0)] {
            assert_eq!(
                db.assets(&AssetQuery {
                    library_id: Some(library.id),
                    folder: Some(folder.into()),
                    folder_recursive: Some(false),
                    ..Default::default()
                })
                .unwrap()
                .total,
                count
            );
        }
        assert_eq!(
            db.assets(&AssetQuery {
                library_id: Some(library.id),
                folder: Some("相册".into()),
                ..Default::default()
            })
            .unwrap()
            .total,
            2
        );
        db.index_folders(library.id, 2, &[native("相册")]).unwrap();
        db.retain_subtree(library.id, 2, "空").unwrap();
        assert!(db.remove_unseen_folders_batch(library.id, 2).unwrap() > 0);
        let updated = db.folders(library.id, "").unwrap();
        assert!(updated.folders.iter().any(|folder| folder.name == "空"));
        assert!(!updated.folders.iter().any(|folder| folder.name == "相册2"));
        assert_eq!(db.folders(library.id, "空").unwrap().folders.len(), 1);
    }

    #[test]
    fn old_file_only_indexes_migrate_to_folders_without_losing_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("legacy.sqlite");
        let relative = Path::new("旅行").join("海边").join("照片.png");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE libraries (id INTEGER PRIMARY KEY AUTOINCREMENT,name TEXT NOT NULL,path TEXT NOT NULL UNIQUE);
            CREATE TABLE assets (id INTEGER PRIMARY KEY AUTOINCREMENT,library_id INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
                relative_path TEXT NOT NULL,name TEXT NOT NULL,format TEXT NOT NULL,size INTEGER NOT NULL,width INTEGER,height INTEGER,
                modified_at INTEGER NOT NULL,mtime_ns INTEGER NOT NULL,favorite INTEGER NOT NULL DEFAULT 0,
                seen_generation INTEGER NOT NULL,thumbnail_error TEXT,UNIQUE(library_id,relative_path));
            CREATE TABLE asset_tags (asset_id INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,tag TEXT NOT NULL,PRIMARY KEY(asset_id,tag));
            PRAGMA user_version=1;").unwrap();
        conn.execute(
            "INSERT INTO libraries(id,name,path) VALUES(1,'旧图库',?1)",
            [temp.path().to_str().unwrap()],
        )
        .unwrap();
        conn.execute("INSERT INTO assets(id,library_id,relative_path,name,format,size,width,height,modified_at,mtime_ns,favorite,seen_generation)
            VALUES(7,1,?1,'照片.png','png',100,1600,900,1,1,1,42)", [relative.to_str().unwrap()]).unwrap();
        conn.execute("INSERT INTO asset_tags(asset_id,tag) VALUES(7,'保留')", [])
            .unwrap();
        drop(conn);
        let db = Db::open(&path).unwrap();
        let asset = db.asset(7).unwrap();
        assert!(asset.favorite);
        assert_eq!(asset.tags, vec!["保留"]);
        assert_eq!((asset.width, asset.height), (Some(1600), Some(900)));
        let root = db.folders(1, "").unwrap();
        assert_eq!(root.folders[0].name, "旅行");
        assert_eq!(root.folders[0].asset_count, 1);
        assert!(root.folders[0].has_children);
        let parent = relative.parent().unwrap().to_str().unwrap();
        assert_eq!(
            db.assets(&AssetQuery {
                library_id: Some(1),
                folder: Some(parent.into()),
                folder_recursive: Some(false),
                ..Default::default()
            })
            .unwrap()
            .total,
            1
        );
        drop(db);
        assert_eq!(
            Db::open(&path)
                .unwrap()
                .folders(1, "旅行")
                .unwrap()
                .folders
                .len(),
            1
        );
    }

    #[test]
    fn geometry_and_size_filters_compose_with_folder_and_pagination() {
        let temp = tempfile::tempdir().unwrap();
        let db = Db::open(&temp.path().join("test.sqlite")).unwrap();
        let library = db
            .add_library("素材", temp.path().to_str().unwrap())
            .unwrap();
        let native = |path: &str| path.replace('/', std::path::MAIN_SEPARATOR_STR);
        let records = [
            record(&native("宽/a.png"), 1000, 1),
            record(&native("宽/b.png"), 1500, 1),
            record(&native("宽/c.png"), 1700, 1),
            record("portrait.png", 2000, 1),
            record("square.png", 3000, 1),
            record("unknown.png", 4000, 1),
        ];
        let indexed = db.index_batch(library.id, 1, &records).unwrap();
        for ((asset, _), (width, height)) in indexed.iter().zip([
            (1600, 900),
            (1632, 900),
            (1568, 900),
            (900, 1600),
            (1000, 1000),
        ]) {
            db.set_thumbnail_result(asset, Some(width), Some(height), None)
                .unwrap();
        }
        for (orientation, count) in [("landscape", 3), ("portrait", 1), ("square", 1)] {
            assert_eq!(
                db.assets(&AssetQuery {
                    orientation: Some(orientation.into()),
                    ..Default::default()
                })
                .unwrap()
                .total,
                count
            );
        }
        assert_eq!(
            db.assets(&AssetQuery {
                aspect_ratio: Some("16:9".into()),
                ..Default::default()
            })
            .unwrap()
            .total,
            3
        );
        assert_eq!(
            db.assets(&AssetQuery {
                max_width: Some(u32::MAX),
                ..Default::default()
            })
            .unwrap()
            .total,
            5
        );
        let combined = db
            .assets(&AssetQuery {
                library_id: Some(library.id),
                folder: Some("宽".into()),
                folder_recursive: Some(false),
                orientation: Some("landscape".into()),
                aspect_ratio: Some("16:9".into()),
                min_width: Some(1600),
                max_width: Some(1632),
                min_height: Some(900),
                max_height: Some(900),
                min_size: Some(1000),
                max_size: Some(2000),
                format: Some("png".into()),
                sort: Some("name".into()),
                offset: Some(1),
                limit: Some(1),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(combined.total, 2);
        assert_eq!(combined.assets.len(), 1);
        assert_eq!(combined.assets[0].name, "b.png");
        assert_eq!(
            db.assets(&AssetQuery {
                sort: Some("pixels".into()),
                ..Default::default()
            })
            .unwrap()
            .assets[0]
                .name,
            "b.png"
        );
        assert_eq!(
            db.assets(&AssetQuery {
                sort: Some("size_asc".into()),
                ..Default::default()
            })
            .unwrap()
            .assets[0]
                .name,
            "a.png"
        );
    }

    #[test]
    fn legacy_hidden_folders_do_not_consume_pages_or_create_empty_expanders() {
        let temp = tempfile::tempdir().unwrap();
        let db = Db::open(&temp.path().join("test.sqlite")).unwrap();
        let library = db
            .add_library("素材", temp.path().to_str().unwrap())
            .unwrap();
        let native = |path: &str| path.replace('/', std::path::MAIN_SEPARATOR_STR);
        let mut paths = (0..1100)
            .map(|index| format!(".hidden{index:04}/objects"))
            .map(|path| native(&path))
            .collect::<Vec<_>>();
        let visible = ["alpha", "beta", "empty", "only-hidden", "release.v1"];
        paths.extend(visible.into_iter().map(str::to_owned));
        paths.push(native("only-hidden/.git/objects"));
        for batch in paths.chunks(100) {
            db.index_folders(library.id, 1, batch).unwrap();
        }
        let mut cursor = None;
        let mut visited = Vec::new();
        loop {
            let page = db
                .folders_page(library.id, "", cursor.as_deref(), 2)
                .unwrap();
            assert!(!page.folders.is_empty());
            assert!(page.folders.iter().all(|folder| !folder.has_children));
            visited.extend(page.folders.into_iter().map(|folder| folder.path));
            if !page.truncated {
                assert!(page.next_cursor.is_none());
                break;
            }
            cursor = page.next_cursor;
        }
        assert_eq!(visited, visible);
        let only_hidden = db.folders(library.id, "only-hidden").unwrap();
        assert!(only_hidden.folders.is_empty());
        assert!(!only_hidden.truncated);
        let hidden_parent = db.folders(library.id, ".hidden0000").unwrap();
        assert!(hidden_parent.folders.is_empty());
        assert_eq!(hidden_parent.direct_asset_count, 0);
        assert!(!hidden_parent.truncated);
        assert!(hidden_parent.next_cursor.is_none());
        assert!(
            db.folders(library.id, &native("only-hidden/.git"))
                .unwrap()
                .folders
                .is_empty()
        );
        // Hidden legacy cursors remain usable just like a deleted sibling cursor.
        let page = db
            .folders_page(library.id, "", Some(".hidden1099"), 2)
            .unwrap();
        assert_eq!(page.folders[0].path, "alpha");
        assert_eq!(page.next_cursor.as_deref(), Some("beta"));
    }

    #[test]
    fn folder_pages_are_complete_stable_and_accept_deleted_cursors() {
        let temp = tempfile::tempdir().unwrap();
        let db = Db::open(&temp.path().join("test.sqlite")).unwrap();
        let library = db
            .add_library("素材", temp.path().to_str().unwrap())
            .unwrap();
        let paths = (0..1001)
            .map(|index| format!("目录{index:04}"))
            .collect::<Vec<_>>();
        for batch in paths.chunks(100) {
            db.index_folders(library.id, 1, batch).unwrap();
        }
        let first = db.folders(library.id, "").unwrap();
        assert_eq!(first.folders.len(), 1000);
        assert!(first.truncated);
        assert_eq!(first.next_cursor.as_deref(), Some("目录0999"));
        let last = db
            .folders_page(library.id, "", first.next_cursor.as_deref(), 1000)
            .unwrap();
        assert_eq!(last.folders.len(), 1);
        assert_eq!(last.folders[0].path, "目录1000");
        assert!(!last.truncated);
        assert!(last.next_cursor.is_none());
        let mut cursor = None;
        let mut visited = Vec::new();
        loop {
            let page = db
                .folders_page(library.id, "", cursor.as_deref(), 23)
                .unwrap();
            visited.extend(page.folders.into_iter().map(|folder| folder.path));
            if !page.truncated {
                assert!(page.next_cursor.is_none());
                break;
            }
            cursor = page.next_cursor;
        }
        assert_eq!(visited, paths);
        db.with(|conn| {
            conn.execute(
                "DELETE FROM library_folders WHERE library_id=?1 AND relative_path='目录0999'",
                [library.id],
            )?;
            Ok(())
        })
        .unwrap();
        let after_deletion = db
            .folders_page(library.id, "", Some("目录0999"), 2)
            .unwrap();
        assert_eq!(after_deletion.folders[0].path, "目录1000");
        assert!(db.folders_page(library.id, "", None, 0).is_err());
        assert!(db.folders_page(library.id, "", None, 1001).is_err());
        assert!(
            db.folders_page(
                library.id,
                "",
                Some(Path::new("other").join("child").to_str().unwrap()),
                2
            )
            .is_err()
        );
        assert!(
            db.folders_page(library.id, "目录0000", Some("目录0001"), 2)
                .is_err()
        );
        assert!(
            db.folders_page(library.id, "", Some("../outside"), 2)
                .is_err()
        );
    }

    #[test]
    fn excluded_names_are_literal_filename_filters_and_compose_with_other_conditions() {
        let temp = tempfile::tempdir().unwrap();
        let db = Db::open(&temp.path().join("test.sqlite")).unwrap();
        let library = db
            .add_library("素材", temp.path().to_str().unwrap())
            .unwrap();
        let native = |path: &str| path.replace('/', std::path::MAIN_SEPARATOR_STR);
        let mut records = [
            "map_diffuse.png",
            "MAP_normal.png",
            "普通噪音.png",
            "100%_mask.png",
            "100abmask.png",
            "keep.png",
            "map/clean.png",
            "safe/tagged.png",
            "safe/backslash.png",
            "safe/notfavorite.png",
            "safe/another.png",
            "safe/portrait.png",
        ]
        .iter()
        .map(|path| record(&native(path), 100, 1))
        .collect::<Vec<_>>();
        // Exercise literal LIKE escaping independently of the host filesystem's filename rules.
        records[8].name = "path\\noise.png".into();
        let indexed = db.index_batch(library.id, 1, &records).unwrap();
        for (index, (asset, _)) in indexed.iter().enumerate() {
            let dimensions = if index == 11 {
                (900, 1600)
            } else {
                (1600, 900)
            };
            db.set_thumbnail_result(asset, Some(dimensions.0), Some(dimensions.1), None)
                .unwrap();
            db.patch_asset(
                asset.id,
                &AssetPatch {
                    favorite: Some(index != 9),
                    tags: Some(vec!["map".into()]),
                },
            )
            .unwrap();
        }
        let map = db
            .assets(&AssetQuery {
                exclude_names: Some(" map \nmap\n\n".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(map.total, 10);
        assert!(
            map.assets
                .iter()
                .any(|asset| asset.relative_path == native("map/clean.png"))
        );
        assert!(map.assets.iter().any(|asset| asset.name == "tagged.png"));
        assert!(
            map.assets
                .iter()
                .all(|asset| !asset.name.to_ascii_lowercase().contains("map"))
        );
        for keyword in ["%_", "\\", "噪音"] {
            assert_eq!(
                db.assets(&AssetQuery {
                    exclude_names: Some(keyword.into()),
                    ..Default::default()
                })
                .unwrap()
                .total,
                11
            );
        }
        let multiple = db
            .assets(&AssetQuery {
                exclude_names: Some("map\n噪音\n%_\n\\\nmap".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(multiple.total, 7);
        assert!(
            multiple
                .assets
                .iter()
                .any(|asset| asset.name == "100abmask.png")
        );
        assert_eq!(
            db.assets(&AssetQuery {
                exclude_names: Some(" \n\t\r\n".into()),
                ..Default::default()
            })
            .unwrap()
            .total,
            12
        );
        let combined = db
            .assets(&AssetQuery {
                library_id: Some(library.id),
                folder: Some("safe".into()),
                folder_recursive: Some(false),
                favorite: Some(true),
                tag: Some("map".into()),
                aspect_ratio: Some("16:9".into()),
                exclude_names: Some("map\n\\".into()),
                sort: Some("name".into()),
                offset: Some(1),
                limit: Some(1),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(combined.total, 2);
        assert_eq!(combined.assets.len(), 1);
        assert_eq!(combined.assets[0].name, "tagged.png");
    }
}
