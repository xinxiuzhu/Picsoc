//! Bounded, deterministic PNG composition and immutable, editable scene revisions.
//! Originals are always addressed through the existing index; generated files live separately.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    fs::{self, File, OpenOptions},
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    time::{SystemTime, UNIX_EPOCH},
};

use ab_glyph::{Font, FontArc, FontVec, ScaleFont, point};
use anyhow::{Context, Result, anyhow, bail};
use image::{
    DynamicImage, ImageDecoder, ImageEncoder, ImageReader, Limits, Rgba, RgbaImage,
    codecs::png::{CompressionType, FilterType, PngEncoder},
    imageops,
};
use serde::{Deserialize, Serialize};

use crate::{db::Db, media, models::Asset};

pub const MAX_CANVAS_EDGE: u32 = 4096;
pub const MAX_CANVAS_PIXELS: u64 = 16_777_216;
pub const MAX_LAYERS: usize = 64;
pub const MAX_CONTACT_ASSETS: usize = 24;
const MAX_SCENE_BYTES: u64 = 1024 * 1024;
const MAX_RENDER_DECODE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_FONT_BYTES: u64 = 64 * 1024 * 1024;
const QUEUE_CAPACITY: usize = 4;
const PREVIEW_EDGE: u32 = 1280;

#[derive(Debug)]
pub enum DesignError {
    Invalid(String),
    NotFound(String),
    Conflict(String),
    Busy,
}

impl fmt::Display for DesignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(s) | Self::NotFound(s) | Self::Conflict(s) => f.write_str(s),
            Self::Busy => f.write_str("合成队列已满，请稍后重试"),
        }
    }
}
impl std::error::Error for DesignError {}

pub fn error_status(error: &anyhow::Error) -> u16 {
    match error.downcast_ref::<DesignError>() {
        Some(DesignError::Invalid(_)) => 400,
        Some(DesignError::NotFound(_)) => 404,
        Some(DesignError::Conflict(_)) => 409,
        Some(DesignError::Busy) => 429,
        None => 500,
    }
}

fn invalid(message: impl Into<String>) -> anyhow::Error {
    DesignError::Invalid(message.into()).into()
}
fn not_found(message: impl Into<String>) -> anyhow::Error {
    DesignError::NotFound(message.into()).into()
}
fn one() -> f32 {
    1.0
}
fn default_version() -> u32 {
    1
}
fn default_background() -> String {
    "transparent".into()
}
fn default_font() -> String {
    "default".into()
}
fn default_line_height() -> f32 {
    1.2
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    #[serde(default = "default_version")]
    pub version: u32,
    pub name: String,
    pub canvas: Canvas,
    pub layers: Vec<Layer>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    #[serde(default = "default_background")]
    pub background: String,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    #[default]
    Contain,
    Cover,
    Stretch,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Layer {
    Image {
        asset_id: i64,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        #[serde(default)]
        fit: Fit,
        #[serde(default = "one")]
        opacity: f32,
    },
    Rect {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        color: String,
        #[serde(default)]
        radius: u32,
        #[serde(default = "one")]
        opacity: f32,
    },
    Text {
        text: String,
        x: i32,
        y: i32,
        font_size: f32,
        color: String,
        #[serde(default = "default_font")]
        font_id: String,
        #[serde(default)]
        max_width: Option<u32>,
        #[serde(default)]
        align: TextAlign,
        #[serde(default = "default_line_height")]
        line_height: f32,
        #[serde(default = "one")]
        opacity: f32,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveDesign {
    #[serde(default)]
    pub design_id: Option<String>,
    #[serde(default)]
    pub expected_revision: Option<u64>,
    pub scene: Scene,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssetSource {
    pub asset_id: i64,
    pub cache_key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredDesign {
    pub design_id: String,
    pub revision: u64,
    pub created_at: u64,
    pub name: String,
    pub scene: Scene,
    pub asset_sources: Vec<AssetSource>,
    #[serde(default)]
    pub latest_job: Option<RenderJob>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DesignSummary {
    pub design_id: String,
    pub revision: u64,
    pub name: String,
    pub updated_at: u64,
    pub width: u32,
    pub height: u32,
    pub layer_count: usize,
    pub latest_job: Option<RenderJob>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderQuality {
    #[default]
    Preview,
    Final,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderRequest {
    pub design_id: String,
    #[serde(default)]
    pub revision: Option<u64>,
    #[serde(default)]
    pub quality: RenderQuality,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenderJob {
    pub job_id: String,
    pub design_id: String,
    pub revision: u64,
    pub quality: RenderQuality,
    pub status: String,
    pub created_at: u64,
    pub finished_at: Option<u64>,
    pub error: Option<String>,
    pub width: u32,
    pub height: u32,
    pub output_url: Option<String>,
    pub preview_url: Option<String>,
    pub layout_url: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FontInfo {
    pub id: String,
    pub name: String,
    pub supports_chinese: bool,
}

#[derive(Debug, Serialize)]
pub struct MissingAsset {
    pub asset_id: i64,
    pub error: String,
}

pub struct AssetPreview {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub asset_ids: Vec<i64>,
    pub missing: Vec<MissingAsset>,
}

#[derive(Clone)]
struct LoadedFont {
    info: FontInfo,
    font: FontArc,
}

struct Work {
    design: StoredDesign,
    job: RenderJob,
}

#[derive(Clone)]
pub struct DesignService {
    db: Db,
    root: PathBuf,
    fonts: Arc<Vec<LoadedFont>>,
    save_lock: Arc<Mutex<()>>,
    render_lock: Arc<Mutex<()>>,
    sender: mpsc::SyncSender<Work>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn new_id(prefix: &str) -> Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| anyhow!("无法生成设计编号：{e}"))?;
    let suffix = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    Ok(format!("{prefix}_{suffix}"))
}

fn valid_id(id: &str, prefix: &str) -> bool {
    id.strip_prefix(prefix)
        .and_then(|s| s.strip_prefix('_'))
        .is_some_and(|s| {
            s.len() == 32
                && s.bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        })
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_SCENE_BYTES {
        bail!("布局或任务文件超过限制");
    }
    serde_json::from_reader(BufReader::new(file)).context("布局或任务文件无效")
}

fn safe_directory(root: &Path, parts: &[&str]) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    for part in parts {
        if Path::new(part).components().count() != 1
            || !matches!(
                Path::new(part).components().next(),
                Some(std::path::Component::Normal(_))
            )
        {
            return Err(invalid("生成文件夹路径无效"));
        }
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(invalid("生成文件夹不能是符号链接或普通文件"));
            }
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(&path)?,
            Err(error) => return Err(error.into()),
        }
        if path.canonicalize()? != path {
            return Err(invalid("生成文件夹超出了数据目录"));
        }
    }
    Ok(path)
}

fn publish_new(temporary: &Path, target: &Path) -> Result<()> {
    // Unix rename replaces an existing revision; link publishes the complete file with create-new semantics.
    #[cfg(unix)]
    {
        fs::hard_link(temporary, target)?;
        fs::remove_file(temporary)?;
        if let Some(parent) = target.parent() {
            File::open(parent)?.sync_all()?;
        }
    }
    #[cfg(not(unix))]
    fs::rename(temporary, target)?;
    Ok(())
}

/// Publish a previously nonexistent artifact. Every revision and job state is immutable.
fn atomic_bytes(target: &Path, bytes: &[u8]) -> Result<()> {
    let parent = target.parent().context("生成文件路径无效")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".{}.tmp", new_id("write")?));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if target.exists() {
            bail!("生成文件已存在，禁止覆盖不可变版本");
        }
        publish_new(&temporary, target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn atomic_json<T: Serialize>(target: &Path, data: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(data)?;
    if bytes.len() as u64 > MAX_SCENE_BYTES {
        bail!("布局或任务文件超过限制");
    }
    atomic_bytes(target, &bytes)
}

fn job_at(root: &Path, job_id: &str) -> Result<RenderJob> {
    if !valid_id(job_id, "j") {
        return Err(invalid("任务编号无效"));
    }
    let directory = root.join("jobs").join(job_id);
    for phase in ["failed", "succeeded", "running", "queued"] {
        let path = directory.join(format!("{phase}.json"));
        if path.exists() {
            let resolved = media::secure_path(root, &format!("jobs/{job_id}/{phase}.json"))
                .map_err(|_| not_found("合成任务不可访问"))?;
            return read_json(&resolved);
        }
    }
    Err(not_found("合成任务不存在"))
}

fn persist_job(root: &Path, job: &RenderJob) -> Result<()> {
    let directory = safe_directory(root, &["jobs", &job.job_id])?;
    atomic_json(&directory.join(format!("{}.json", job.status)), job)
}

impl DesignService {
    pub fn new(db: Db, data_dir: PathBuf) -> Result<Self> {
        let generated = data_dir.join("generated");
        if fs::symlink_metadata(&generated).is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(invalid("generated 目录不能是符号链接"));
        }
        fs::create_dir_all(&generated)?;
        let root = generated.canonicalize()?;
        safe_directory(&root, &["designs"])?;
        safe_directory(&root, &["jobs"])?;
        // A restart must not leave clients polling an abandoned job indefinitely.
        for entry in fs::read_dir(root.join("jobs"))? {
            let entry = entry?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if !valid_id(&id, "j") {
                continue;
            }
            if let Ok(mut job) = job_at(&root, &id)
                && matches!(job.status.as_str(), "queued" | "running")
            {
                job.status = "failed".into();
                job.finished_at = Some(now());
                job.error = Some("服务重新启动，未完成的合成已中止，请重新提交".into());
                persist_job(&root, &job)?;
            }
        }
        let fonts = Arc::new(load_fonts(&data_dir));
        let render_lock = Arc::new(Mutex::new(()));
        let (sender, receiver) = mpsc::sync_channel::<Work>(QUEUE_CAPACITY);
        let worker_root = root.clone();
        let worker_db = db.clone();
        let worker_fonts = fonts.clone();
        let worker_lock = render_lock.clone();
        std::thread::Builder::new()
            .name("picsoc-render".into())
            .spawn(move || {
                for work in receiver {
                    let mut job = work.job;
                    job.status = "running".into();
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        persist_job(&worker_root, &job).and_then(|_| {
                            let _guard = worker_lock
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                            render_job(&worker_root, &worker_db, &worker_fonts, &work.design, &job)
                        })
                    }))
                    .unwrap_or_else(|_| Err(anyhow!("图片处理异常，合成已中止")));
                    job.finished_at = Some(now());
                    match result {
                        Ok(()) => {
                            job.status = "succeeded".into();
                            let base = format!("/api/design-jobs/{}", job.job_id);
                            job.output_url = Some(format!("{base}/output.png"));
                            job.preview_url = Some(format!("{base}/preview.png"));
                            job.layout_url = Some(format!("{base}/layout.json"));
                        }
                        Err(error) => {
                            job.status = "failed".into();
                            job.error = Some(format!("{error:#}"));
                        }
                    }
                    if let Err(error) = persist_job(&worker_root, &job) {
                        tracing::error!(%error, job_id=%job.job_id, "无法保存合成结果状态");
                    }
                }
            })?;
        Ok(Self {
            db,
            root,
            fonts,
            save_lock: Arc::new(Mutex::new(())),
            render_lock,
            sender,
        })
    }

    pub fn fonts(&self) -> Vec<FontInfo> {
        self.fonts.iter().map(|font| font.info.clone()).collect()
    }

    fn revision_path(&self, design_id: &str, revision: Option<u64>) -> Result<PathBuf> {
        if !valid_id(design_id, "d") {
            return Err(invalid("设计编号无效"));
        }
        let directory = self.root.join("designs").join(design_id);
        if !directory.is_dir() {
            return Err(not_found("设计不存在"));
        }
        let revision = match revision {
            Some(0) => return Err(invalid("版本编号必须大于 0")),
            Some(revision) => revision,
            None => fs::read_dir(&directory)?
                .filter_map(|entry| {
                    let name = entry.ok()?.file_name().into_string().ok()?;
                    name.strip_prefix('r')?
                        .strip_suffix(".json")?
                        .parse::<u64>()
                        .ok()
                })
                .max()
                .ok_or_else(|| not_found("设计尚未保存"))?,
        };
        let relative = format!("designs/{design_id}/r{revision}.json");
        media::secure_path(&self.root, &relative).map_err(|_| not_found("设计版本不存在或不可访问"))
    }

    fn latest_job(&self, design_id: &str, revision: u64) -> Result<Option<RenderJob>> {
        let mut successful = None::<RenderJob>;
        let mut latest = None::<RenderJob>;
        for entry in fs::read_dir(self.root.join("jobs"))? {
            let id = entry?.file_name().to_string_lossy().into_owned();
            if !valid_id(&id, "j") {
                continue;
            }
            let Ok(job) = job_at(&self.root, &id) else {
                continue;
            };
            if job.design_id != design_id || job.revision != revision {
                continue;
            }
            if job.status == "succeeded"
                && successful
                    .as_ref()
                    .is_none_or(|old| old.created_at <= job.created_at)
            {
                successful = Some(job.clone());
            }
            if latest
                .as_ref()
                .is_none_or(|old| old.created_at <= job.created_at)
            {
                latest = Some(job);
            }
        }
        Ok(successful.or(latest))
    }

    pub fn get(&self, design_id: &str, revision: Option<u64>) -> Result<StoredDesign> {
        let mut design: StoredDesign = read_json(&self.revision_path(design_id, revision)?)?;
        design.latest_job = self.latest_job(design_id, design.revision)?;
        Ok(design)
    }

    pub fn list(&self, limit: u32, offset: u32) -> Result<Vec<DesignSummary>> {
        if limit == 0 || limit > 100 || offset > 100_000 {
            return Err(invalid("作品分页参数无效"));
        }
        let mut designs = Vec::new();
        for entry in fs::read_dir(self.root.join("designs"))? {
            let id = entry?.file_name().to_string_lossy().into_owned();
            if !valid_id(&id, "d") {
                continue;
            }
            // Damaged drafts do not prevent other designs from being listed.
            let Ok(design) = self.get(&id, None) else {
                continue;
            };
            designs.push(DesignSummary {
                design_id: design.design_id,
                revision: design.revision,
                name: design.name,
                updated_at: design.created_at,
                width: design.scene.canvas.width,
                height: design.scene.canvas.height,
                layer_count: design.scene.layers.len(),
                latest_job: design.latest_job,
            });
        }
        designs.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.design_id.cmp(&b.design_id))
        });
        Ok(designs
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect())
    }

    pub fn save(&self, request: SaveDesign) -> Result<StoredDesign> {
        validate_scene(&request.scene, &self.fonts)?;
        let _guard = self
            .save_lock
            .lock()
            .map_err(|_| anyhow!("布局保存锁不可用"))?;
        let (design_id, revision) = match request.design_id {
            Some(id) => {
                let current = self.get(&id, None)?;
                if request.expected_revision != Some(current.revision) {
                    return Err(
                        DesignError::Conflict("布局已更新，请读取最新版本后再修改".into()).into(),
                    );
                }
                if current.revision >= 10_000 {
                    return Err(invalid("布局版本数量超过限制"));
                }
                (id, current.revision + 1)
            }
            None => {
                if request.expected_revision.is_some() {
                    return Err(invalid("新设计不能指定旧版本"));
                }
                (new_id("d")?, 1)
            }
        };
        let mut sources = BTreeMap::new();
        for layer in &request.scene.layers {
            if let Layer::Image { asset_id, .. } = layer {
                let asset = self
                    .db
                    .asset(*asset_id)
                    .map_err(|_| invalid(format!("素材 {asset_id} 不存在")))?;
                sources.insert(*asset_id, asset.cache_key());
            }
        }
        let design = StoredDesign {
            design_id: design_id.clone(),
            revision,
            created_at: now(),
            name: request.scene.name.clone(),
            scene: request.scene,
            asset_sources: sources
                .into_iter()
                .map(|(asset_id, cache_key)| AssetSource {
                    asset_id,
                    cache_key,
                })
                .collect(),
            latest_job: None,
        };
        let directory = safe_directory(&self.root, &["designs", &design_id])?;
        atomic_json(&directory.join(format!("r{revision}.json")), &design)?;
        Ok(design)
    }

    pub fn submit(&self, request: RenderRequest) -> Result<RenderJob> {
        let design = self.get(&request.design_id, request.revision)?;
        validate_scene(&design.scene, &self.fonts)?;
        let (width, height) = output_dimensions(&design.scene.canvas, request.quality);
        let job = RenderJob {
            job_id: new_id("j")?,
            design_id: design.design_id.clone(),
            revision: design.revision,
            quality: request.quality,
            status: "queued".into(),
            created_at: now(),
            finished_at: None,
            error: None,
            width,
            height,
            output_url: None,
            preview_url: None,
            layout_url: None,
        };
        persist_job(&self.root, &job)?;
        match self.sender.try_send(Work {
            design,
            job: job.clone(),
        }) {
            Ok(()) => Ok(job),
            Err(mpsc::TrySendError::Full(_)) => {
                let _ = fs::remove_dir_all(self.root.join("jobs").join(&job.job_id));
                Err(DesignError::Busy.into())
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                let _ = fs::remove_dir_all(self.root.join("jobs").join(&job.job_id));
                Err(anyhow!("合成工作线程不可用"))
            }
        }
    }

    pub fn job(&self, job_id: &str) -> Result<RenderJob> {
        job_at(&self.root, job_id)
    }

    pub fn output_path(&self, job_id: &str, filename: &str) -> Result<PathBuf> {
        if !matches!(filename, "output.png" | "preview.png" | "layout.json") {
            return Err(invalid("生成文件名无效"));
        }
        let job = self.job(job_id)?;
        if job.status != "succeeded" {
            return Err(DesignError::Conflict("合成尚未成功完成".into()).into());
        }
        media::secure_path(&self.root, &format!("jobs/{job_id}/{filename}"))
            .map_err(|_| not_found("生成文件不存在或不可访问"))
    }

    pub fn preview_assets(&self, ids: &[i64]) -> Result<AssetPreview> {
        if ids.is_empty() || ids.len() > MAX_CONTACT_ASSETS || ids.iter().any(|id| *id <= 0) {
            return Err(invalid("请提供 1–24 个有效素材编号"));
        }
        if ids.iter().copied().collect::<BTreeSet<_>>().len() != ids.len() {
            return Err(invalid("预览素材编号不能重复"));
        }
        let _guard = match self.render_lock.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(DesignError::Busy.into()),
        };
        contact_sheet(&self.db, ids)
    }
}

fn parse_color(value: &str) -> Result<Rgba<u8>> {
    if value == "transparent" {
        return Ok(Rgba([0, 0, 0, 0]));
    }
    let Some(hex) = value.strip_prefix('#') else {
        return Err(invalid("颜色请使用 #RRGGBB、#RRGGBBAA 或 transparent"));
    };
    if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("颜色请使用 #RRGGBB、#RRGGBBAA 或 transparent"));
    }
    let mut channels = [0, 0, 0, 255];
    for (index, channel) in channels.iter_mut().take(hex.len() / 2).enumerate() {
        *channel = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)?;
    }
    Ok(Rgba(channels))
}

fn validate_box(x: i32, y: i32, width: u32, height: u32, opacity: f32) -> Result<()> {
    if x.unsigned_abs() > MAX_CANVAS_EDGE * 2 || y.unsigned_abs() > MAX_CANVAS_EDGE * 2 {
        return Err(invalid("图层坐标必须在 -8192 到 8192 范围内"));
    }
    if width == 0 || height == 0 || width > MAX_CANVAS_EDGE || height > MAX_CANVAS_EDGE {
        return Err(invalid("图层宽高必须在 1 到 4096 范围内"));
    }
    if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
        return Err(invalid("透明度必须在 0 到 1 范围内"));
    }
    Ok(())
}

fn has_glyph(font: &FontArc, ch: char) -> bool {
    ch.is_whitespace() || {
        let id = font.glyph_id(ch);
        id.0 != 0 && font.outline_glyph(id.with_scale(32.0)).is_some()
    }
}

fn select_font<'a>(fonts: &'a [LoadedFont], font_id: &str, text: &str) -> Result<&'a FontArc> {
    let supports = |font: &LoadedFont| text.chars().all(|ch| has_glyph(&font.font, ch));
    if font_id == "default" {
        return fonts
            .iter()
            .find(|font| supports(font))
            .map(|font| &font.font)
            .ok_or_else(|| {
                invalid(
                    "没有可绘制这些文字的字体；请安装中文字体或将字体放入数据目录 fonts 后重启服务",
                )
            });
    }
    let font = fonts
        .iter()
        .find(|font| font.info.id == font_id)
        .ok_or_else(|| invalid("字体编号无效，请从 get_fonts 的字体列表选择"))?;
    if !supports(font) {
        return Err(invalid("所选字体缺少文字字形，请选择支持这些文字的字体"));
    }
    Ok(&font.font)
}

fn validate_scene(scene: &Scene, fonts: &[LoadedFont]) -> Result<()> {
    if scene.version != 1 {
        return Err(invalid("目前只支持 version: 1 布局"));
    }
    if scene.name.trim().is_empty() || scene.name.chars().count() > 120 {
        return Err(invalid("设计名称不能为空，且不能超过 120 个字符"));
    }
    let canvas = &scene.canvas;
    if canvas.width == 0
        || canvas.height == 0
        || canvas.width > MAX_CANVAS_EDGE
        || canvas.height > MAX_CANVAS_EDGE
        || u64::from(canvas.width) * u64::from(canvas.height) > MAX_CANVAS_PIXELS
    {
        return Err(invalid(
            "画布宽高必须在 1 到 4096 范围内，总像素不能超过 16777216",
        ));
    }
    parse_color(&canvas.background)?;
    if scene.layers.len() > MAX_LAYERS {
        return Err(invalid("每个布局最多包含 64 个图层"));
    }
    let mut total_chars = 0;
    for layer in &scene.layers {
        match layer {
            Layer::Image {
                asset_id,
                x,
                y,
                width,
                height,
                opacity,
                ..
            } => {
                if *asset_id <= 0 {
                    return Err(invalid("素材编号必须大于 0"));
                }
                validate_box(*x, *y, *width, *height, *opacity)?;
            }
            Layer::Rect {
                x,
                y,
                width,
                height,
                color,
                radius,
                opacity,
            } => {
                validate_box(*x, *y, *width, *height, *opacity)?;
                if *radius > (*width).min(*height) / 2 {
                    return Err(invalid("圆角半径不能超过短边的一半"));
                }
                parse_color(color)?;
            }
            Layer::Text {
                text,
                x,
                y,
                font_size,
                color,
                font_id,
                max_width,
                line_height,
                opacity,
                ..
            } => {
                validate_box(*x, *y, max_width.unwrap_or(canvas.width), 1, *opacity)?;
                if !font_size.is_finite() || !(4.0..=512.0).contains(font_size) {
                    return Err(invalid("字号必须在 4 到 512 范围内"));
                }
                if !line_height.is_finite() || !(0.5..=4.0).contains(line_height) {
                    return Err(invalid("行高倍数必须在 0.5 到 4 范围内"));
                }
                let chars = text.chars().count();
                if chars == 0
                    || chars > 1024
                    || text
                        .chars()
                        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\t'))
                {
                    return Err(invalid("每个文字图层需要 1–1024 个字符，不支持控制字符"));
                }
                total_chars += chars;
                if total_chars > 4096 {
                    return Err(invalid("布局文字总数不能超过 4096 个字符"));
                }
                if font_id.len() > 160 {
                    return Err(invalid("字体编号无效"));
                }
                parse_color(color)?;
                select_font(fonts, font_id, text)?;
            }
        }
    }
    Ok(())
}

fn load_fonts(data_dir: &Path) -> Vec<LoadedFont> {
    let mut candidates = Vec::<(String, PathBuf)>::new();
    let custom = data_dir.join("fonts");
    if let Ok(entries) = fs::read_dir(custom) {
        let mut paths = entries
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .collect::<Vec<_>>();
        paths.sort();
        for path in paths.into_iter().take(8) {
            let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            if !matches!(
                extension.to_ascii_lowercase().as_str(),
                "ttf" | "otf" | "ttc"
            ) {
                continue;
            }
            let Some(name) = path.file_name().and_then(|p| p.to_str()) else {
                continue;
            };
            candidates.push((format!("custom:{name}"), path.clone()));
        }
    }
    for (id, path) in [
        (
            "noto-cjk",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        ),
        ("wqy-zenhei", "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc"),
        ("pingfang", "/System/Library/Fonts/PingFang.ttc"),
        ("heiti", "/System/Library/Fonts/STHeiti Medium.ttc"),
        ("hiragino-gb", "/System/Library/Fonts/Hiragino Sans GB.ttc"),
        ("arial", "/System/Library/Fonts/Supplemental/Arial.ttf"),
        ("dejavu", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
    ] {
        candidates.push((id.into(), path.into()));
    }
    if let Some(directory) = std::env::var_os("WINDIR") {
        for (id, name) in [
            ("microsoft-yahei", "msyh.ttc"),
            ("windows-arial", "arial.ttf"),
        ] {
            candidates.push((
                id.into(),
                PathBuf::from(&directory).join("Fonts").join(name),
            ));
        }
    }
    let mut fonts = Vec::new();
    let mut loaded_bytes = 0;
    let mut has_system_chinese = false;
    for (id, path) in candidates {
        let is_chinese_candidate = matches!(
            id.as_str(),
            "noto-cjk" | "wqy-zenhei" | "pingfang" | "heiti" | "hiragino-gb" | "microsoft-yahei"
        );
        if is_chinese_candidate && has_system_chinese {
            continue;
        }
        let font = (|| -> Result<FontArc> {
            let file = File::open(&path)?;
            let size = file.metadata()?.len();
            if size > MAX_FONT_BYTES || loaded_bytes + size > MAX_FONT_BYTES {
                bail!("字体文件超过内存限制");
            }
            let mut bytes = Vec::new();
            file.take(MAX_FONT_BYTES + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_FONT_BYTES {
                bail!("字体文件超过内存限制");
            }
            // Noto's collection orders JP, KR, SC, TC, HK. Select simplified Chinese explicitly.
            let index = if id == "noto-cjk" { 2 } else { 0 };
            let font = FontVec::try_from_vec_and_index(bytes, index)
                .map_err(|_| anyhow!("字体无法解析"))?;
            loaded_bytes += size;
            Ok(FontArc::from(font))
        })();
        if let Ok(font) = font {
            let supports_chinese = "星海中文".chars().all(|ch| has_glyph(&font, ch));
            if is_chinese_candidate && supports_chinese {
                has_system_chinese = true;
            }
            let name = path
                .file_name()
                .and_then(|p| p.to_str())
                .unwrap_or(&id)
                .to_owned();
            fonts.push(LoadedFont {
                info: FontInfo {
                    id,
                    name,
                    supports_chinese,
                },
                font,
            });
        }
    }
    fonts
}

fn output_dimensions(canvas: &Canvas, quality: RenderQuality) -> (u32, u32) {
    if matches!(quality, RenderQuality::Final) || canvas.width.max(canvas.height) <= PREVIEW_EDGE {
        return (canvas.width, canvas.height);
    }
    let ratio = PREVIEW_EDGE as f64 / canvas.width.max(canvas.height) as f64;
    (
        (canvas.width as f64 * ratio).round().max(1.0) as u32,
        (canvas.height as f64 * ratio).round().max(1.0) as u32,
    )
}

fn render_scale(canvas: &Canvas, quality: RenderQuality) -> f64 {
    if matches!(quality, RenderQuality::Preview) && canvas.width.max(canvas.height) > PREVIEW_EDGE {
        PREVIEW_EDGE as f64 / canvas.width.max(canvas.height) as f64
    } else {
        1.0
    }
}

fn decode_asset(asset: &Asset) -> Result<RgbaImage> {
    let path = media::secure_path(Path::new(&asset.library_path), &asset.relative_path)?;
    let source = File::open(path)?;
    let metadata = source.metadata()?;
    if metadata.len() > media::MAX_SOURCE_BYTES {
        bail!("素材 {} 超过 256 MiB 文件限制", asset.id);
    }
    let modified = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let mtime_ns = modified.as_nanos().min(i64::MAX as u128) as i64;
    if metadata.len() != asset.size as u64 || mtime_ns != asset.mtime_ns {
        bail!(
            "素材 {} 原文件已修改，请重新扫描素材库并保存新布局",
            asset.id
        );
    }
    let unchanged = source.try_clone()?;
    let mut reader = ImageReader::new(BufReader::new(source)).with_guessed_format()?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_RENDER_DECODE_BYTES);
    limits.max_image_width = Some(32_768);
    limits.max_image_height = Some(32_768);
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    if decoder.total_bytes() > MAX_RENDER_DECODE_BYTES {
        bail!("素材 {} 解码超过 64 MiB 限制", asset.id);
    }
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) * 4 > MAX_RENDER_DECODE_BYTES {
        bail!("素材 {} 的 RGBA 缓冲区超过 64 MiB 限制", asset.id);
    }
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    let after = unchanged.metadata()?;
    if after.len() != metadata.len() || after.modified()? != metadata.modified()? {
        bail!("素材 {} 在读取时发生变化，请稍后重试", asset.id);
    }
    Ok(image.into_rgba8())
}

/// Resample directly into the bounded output buffer using premultiplied colors.
/// imageops::resize uses a source_width × target_height floating-point intermediate,
/// which can be much larger than either input or output for very wide images.
fn resize_alpha(source: RgbaImage, width: u32, height: u32) -> RgbaImage {
    if source.dimensions() == (width, height) {
        return source;
    }
    let mut output = RgbaImage::new(width, height);
    let ratio_x = source.width() as f64 / width as f64;
    let ratio_y = source.height() as f64 / height as f64;
    let shrinking = width <= source.width() && height <= source.height();
    for y in 0..height {
        for x in 0..width {
            let mut total = [0.0_f64; 4];
            let mut weight_sum = 0.0;
            let mut add = |px: u32, py: u32, weight: f64| {
                let pixel = source.get_pixel(px, py).0;
                let alpha = f64::from(pixel[3]);
                for channel in 0..3 {
                    total[channel] += f64::from(pixel[channel]) * alpha * weight;
                }
                total[3] += alpha * weight;
                weight_sum += weight;
            };
            if shrinking {
                // Box averages give thin UI strokes and downscaled icons a proper area filter.
                let left = x as f64 * ratio_x;
                let right = (x + 1) as f64 * ratio_x;
                let top = y as f64 * ratio_y;
                let bottom = (y + 1) as f64 * ratio_y;
                for py in (top.floor() as u32)..(bottom.ceil() as u32).min(source.height()) {
                    let wy = ((py + 1) as f64).min(bottom) - (py as f64).max(top);
                    for px in (left.floor() as u32)..(right.ceil() as u32).min(source.width()) {
                        let wx = ((px + 1) as f64).min(right) - (px as f64).max(left);
                        add(px, py, wx * wy);
                    }
                }
            } else {
                let sx = ((x as f64 + 0.5) * ratio_x - 0.5)
                    .clamp(0.0, source.width().saturating_sub(1) as f64);
                let sy = ((y as f64 + 0.5) * ratio_y - 0.5)
                    .clamp(0.0, source.height().saturating_sub(1) as f64);
                let left = sx.floor() as u32;
                let top = sy.floor() as u32;
                let right = (left + 1).min(source.width() - 1);
                let bottom = (top + 1).min(source.height() - 1);
                let wx = sx - left as f64;
                let wy = sy - top as f64;
                add(left, top, (1.0 - wx) * (1.0 - wy));
                add(right, top, wx * (1.0 - wy));
                add(left, bottom, (1.0 - wx) * wy);
                add(right, bottom, wx * wy);
            }
            let mut pixel = [0_u8; 4];
            if total[3] > 0.0 {
                for channel in 0..3 {
                    pixel[channel] = (total[channel] / total[3]).round().clamp(0.0, 255.0) as u8;
                }
                pixel[3] = (total[3] / weight_sum).round().clamp(0.0, 255.0) as u8;
            }
            output.put_pixel(x, y, Rgba(pixel));
        }
    }
    output
}

/// Exact integer source-over, with rounding. Pixel::blend's float-to-u8 truncation can
/// turn an opaque destination into alpha 254 and accumulate transparency over many layers.
fn blend_pixel(destination: &mut Rgba<u8>, source: Rgba<u8>) {
    let sa = u64::from(source.0[3]);
    if sa == 0 {
        return;
    }
    if sa == 255 {
        *destination = source;
        return;
    }
    let da = u64::from(destination.0[3]);
    let alpha_numerator = sa * 255 + da * (255 - sa);
    let mut output = [0_u8; 4];
    for (channel, value) in output.iter_mut().enumerate().take(3) {
        let numerator = u64::from(source.0[channel]) * sa * 255
            + u64::from(destination.0[channel]) * da * (255 - sa);
        *value = ((numerator + alpha_numerator / 2) / alpha_numerator) as u8;
    }
    output[3] = ((alpha_numerator + 127) / 255) as u8;
    *destination = Rgba(output);
}

fn overlay(canvas: &mut RgbaImage, source: &RgbaImage, x: i64, y: i64) {
    let left = x.max(0).min(i64::from(canvas.width()));
    let top = y.max(0).min(i64::from(canvas.height()));
    let right = (x + i64::from(source.width()))
        .max(0)
        .min(i64::from(canvas.width()));
    let bottom = (y + i64::from(source.height()))
        .max(0)
        .min(i64::from(canvas.height()));
    for cy in top..bottom {
        for cx in left..right {
            blend_pixel(
                canvas.get_pixel_mut(cx as u32, cy as u32),
                *source.get_pixel((cx - x) as u32, (cy - y) as u32),
            );
        }
    }
}

fn fit_image(source: RgbaImage, width: u32, height: u32, fit: Fit) -> (RgbaImage, i64, i64) {
    match fit {
        Fit::Stretch => (resize_alpha(source, width, height), 0, 0),
        Fit::Contain => {
            let ratio =
                (width as f64 / source.width() as f64).min(height as f64 / source.height() as f64);
            let target_width = (source.width() as f64 * ratio)
                .round()
                .max(1.0)
                .min(width as f64) as u32;
            let target_height = (source.height() as f64 * ratio)
                .round()
                .max(1.0)
                .min(height as f64) as u32;
            (
                resize_alpha(source, target_width, target_height),
                i64::from((width - target_width) / 2),
                i64::from((height - target_height) / 2),
            )
        }
        Fit::Cover => {
            // Crop before resizing, otherwise a very narrow source can create a huge intermediate.
            let source_ratio = source.width() as f64 / source.height() as f64;
            let target_ratio = width as f64 / height as f64;
            let (crop_width, crop_height) = if source_ratio > target_ratio {
                (
                    (source.height() as f64 * target_ratio)
                        .round()
                        .max(1.0)
                        .min(source.width() as f64) as u32,
                    source.height(),
                )
            } else {
                (
                    source.width(),
                    (source.width() as f64 / target_ratio)
                        .round()
                        .max(1.0)
                        .min(source.height() as f64) as u32,
                )
            };
            let cropped = imageops::crop_imm(
                &source,
                (source.width() - crop_width) / 2,
                (source.height() - crop_height) / 2,
                crop_width,
                crop_height,
            )
            .to_image();
            drop(source);
            (resize_alpha(cropped, width, height), 0, 0)
        }
    }
}

fn with_opacity(mut color: Rgba<u8>, opacity: f32) -> Rgba<u8> {
    color.0[3] = (color.0[3] as f32 * opacity).round().clamp(0.0, 255.0) as u8;
    color
}

#[allow(clippy::too_many_arguments)]
fn draw_rect(
    canvas: &mut RgbaImage,
    x: i64,
    y: i64,
    width: u32,
    height: u32,
    radius: u32,
    color: Rgba<u8>,
) {
    let left = x.max(0).min(i64::from(canvas.width()));
    let top = y.max(0).min(i64::from(canvas.height()));
    let right = (x + i64::from(width)).max(0).min(i64::from(canvas.width()));
    let bottom = (y + i64::from(height))
        .max(0)
        .min(i64::from(canvas.height()));
    for cy in top..bottom {
        for cx in left..right {
            if radius > 0 {
                let r = f64::from(radius);
                let px = (cx - x) as f64 + 0.5;
                let py = (cy - y) as f64 + 0.5;
                let center_x = px.clamp(r, f64::from(width) - r);
                let center_y = py.clamp(r, f64::from(height) - r);
                let distance = ((px - center_x).powi(2) + (py - center_y).powi(2)).sqrt();
                if distance > r + 0.5 {
                    continue;
                }
                let coverage = (r + 0.5 - distance).clamp(0.0, 1.0) as f32;
                blend_pixel(
                    canvas.get_pixel_mut(cx as u32, cy as u32),
                    with_opacity(color, coverage),
                );
            } else {
                blend_pixel(canvas.get_pixel_mut(cx as u32, cy as u32), color);
            }
        }
    }
}

fn line_width(font: &FontArc, size: f32, text: &str) -> f32 {
    let scaled = font.as_scaled(size);
    let mut width = 0.0;
    let mut previous = None;
    for ch in text.chars() {
        let glyph = font.glyph_id(ch);
        if let Some(previous) = previous {
            width += scaled.kern(previous, glyph);
        }
        width += scaled.h_advance(glyph);
        previous = Some(glyph);
    }
    width
}

fn wrap_text(font: &FontArc, size: f32, text: &str, max_width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let scaled = font.as_scaled(size);
    for paragraph in text.replace('\t', "    ").split('\n') {
        let mut line = String::new();
        let mut width = 0.0;
        let mut previous = None;
        for ch in paragraph.chars() {
            let id = font.glyph_id(ch);
            let mut advance = scaled.h_advance(id) + previous.map_or(0.0, |p| scaled.kern(p, id));
            if !line.is_empty() && width + advance > max_width {
                lines.push(std::mem::take(&mut line));
                width = 0.0;
                advance = scaled.h_advance(id);
            }
            line.push(ch);
            width += advance;
            previous = Some(id);
        }
        lines.push(line);
    }
    lines
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    canvas: &mut RgbaImage,
    font: &FontArc,
    text: &str,
    x: f32,
    y: f32,
    size: f32,
    max_width: f32,
    align: TextAlign,
    line_height: f32,
    color: Rgba<u8>,
) {
    let scaled = font.as_scaled(size);
    for (line_index, line) in wrap_text(font, size, text, max_width).iter().enumerate() {
        let width = line_width(font, size, line);
        let mut cursor = x + match align {
            TextAlign::Left => 0.0,
            TextAlign::Center => (max_width - width) / 2.0,
            TextAlign::Right => max_width - width,
        };
        let baseline = y + scaled.ascent() + line_index as f32 * size * line_height;
        let mut previous = None;
        for ch in line.chars() {
            let id = font.glyph_id(ch);
            if let Some(previous) = previous {
                cursor += scaled.kern(previous, id);
            }
            if let Some(outline) =
                font.outline_glyph(id.with_scale_and_position(size, point(cursor, baseline)))
            {
                let bounds = outline.px_bounds();
                if bounds.max.x > 0.0
                    && bounds.max.y > 0.0
                    && bounds.min.x < canvas.width() as f32
                    && bounds.min.y < canvas.height() as f32
                {
                    outline.draw(|gx, gy, coverage| {
                        let px = bounds.min.x as i64 + i64::from(gx);
                        let py = bounds.min.y as i64 + i64::from(gy);
                        if px >= 0
                            && py >= 0
                            && px < i64::from(canvas.width())
                            && py < i64::from(canvas.height())
                        {
                            blend_pixel(
                                canvas.get_pixel_mut(px as u32, py as u32),
                                with_opacity(color, coverage),
                            );
                        }
                    });
                }
            }
            cursor += scaled.h_advance(id);
            previous = Some(id);
        }
    }
}

fn compose(
    db: &Db,
    fonts: &[LoadedFont],
    design: &StoredDesign,
    quality: RenderQuality,
) -> Result<RgbaImage> {
    validate_scene(&design.scene, fonts)?;
    let (width, height) = output_dimensions(&design.scene.canvas, quality);
    let scale = render_scale(&design.scene.canvas, quality);
    let sx = |value: i32| (f64::from(value) * scale).round() as i64;
    let dim = |value: u32| (f64::from(value) * scale).round().max(1.0) as u32;
    let mut canvas =
        RgbaImage::from_pixel(width, height, parse_color(&design.scene.canvas.background)?);
    for layer in &design.scene.layers {
        match layer {
            Layer::Image {
                asset_id,
                x,
                y,
                width,
                height,
                fit,
                opacity,
            } => {
                if *opacity == 0.0 {
                    continue;
                }
                let asset = db
                    .asset(*asset_id)
                    .with_context(|| format!("素材 {asset_id} 不存在"))?;
                let expected = design
                    .asset_sources
                    .iter()
                    .find(|s| s.asset_id == *asset_id)
                    .context("布局缺少素材来源记录，请重新保存布局")?;
                if expected.cache_key != asset.cache_key() {
                    bail!("素材 {asset_id} 已修改，请重新保存布局生成新版本");
                }
                let source =
                    decode_asset(&asset).with_context(|| format!("无法读取素材 {asset_id}"))?;
                let (mut resized, offset_x, offset_y) =
                    fit_image(source, dim(*width), dim(*height), *fit);
                if *opacity != 1.0 {
                    for pixel in resized.pixels_mut() {
                        *pixel = with_opacity(*pixel, *opacity);
                    }
                }
                overlay(&mut canvas, &resized, sx(*x) + offset_x, sx(*y) + offset_y);
            }
            Layer::Rect {
                x,
                y,
                width,
                height,
                color,
                radius,
                opacity,
            } => {
                if *opacity == 0.0 {
                    continue;
                }
                let w = dim(*width);
                let h = dim(*height);
                let r = (f64::from(*radius) * scale).round() as u32;
                draw_rect(
                    &mut canvas,
                    sx(*x),
                    sx(*y),
                    w,
                    h,
                    r.min(w.min(h) / 2),
                    with_opacity(parse_color(color)?, *opacity),
                );
            }
            Layer::Text {
                text,
                x,
                y,
                font_size,
                color,
                font_id,
                max_width,
                align,
                line_height,
                opacity,
            } => {
                if *opacity == 0.0 {
                    continue;
                }
                let font = select_font(fonts, font_id, text)?;
                draw_text(
                    &mut canvas,
                    font,
                    text,
                    sx(*x) as f32,
                    sx(*y) as f32,
                    *font_size * scale as f32,
                    dim(max_width.unwrap_or(design.scene.canvas.width)) as f32,
                    *align,
                    *line_height,
                    with_opacity(parse_color(color)?, *opacity),
                );
            }
        }
    }
    Ok(canvas)
}

fn png_bytes(image: &RgbaImage) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    PngEncoder::new_with_quality(&mut bytes, CompressionType::Fast, FilterType::Adaptive)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )?;
    Ok(bytes)
}

fn write_png(target: &Path, image: &RgbaImage) -> Result<()> {
    let temporary = target.with_extension(format!("{}.tmp", new_id("png")?));
    let result = (|| {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let mut writer = BufWriter::new(file);
        PngEncoder::new_with_quality(&mut writer, CompressionType::Fast, FilterType::Adaptive)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgba8,
            )?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        drop(writer);
        if target.exists() {
            bail!("生成图片已存在");
        }
        publish_new(&temporary, target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn render_job(
    root: &Path,
    db: &Db,
    fonts: &[LoadedFont],
    design: &StoredDesign,
    job: &RenderJob,
) -> Result<()> {
    let output = compose(db, fonts, design, job.quality)?;
    let directory = safe_directory(root, &["jobs", &job.job_id])?;
    write_png(&directory.join("output.png"), &output)?;
    if output.width().max(output.height()) > PREVIEW_EDGE {
        let ratio = PREVIEW_EDGE as f64 / output.width().max(output.height()) as f64;
        let width = (output.width() as f64 * ratio).round().max(1.0) as u32;
        let height = (output.height() as f64 * ratio).round().max(1.0) as u32;
        // This one additional buffer is limited to 1280²; originals have already been dropped.
        let preview = resize_alpha(output, width, height);
        write_png(&directory.join("preview.png"), &preview)?;
    } else {
        write_png(&directory.join("preview.png"), &output)?;
    }
    let mut layout = design.clone();
    layout.latest_job = None;
    atomic_json(&directory.join("layout.json"), &layout)?;
    Ok(())
}

fn draw_number(image: &mut RgbaImage, text: &str, x: u32, y: u32, color: Rgba<u8>) {
    const DIGITS: [[u8; 7]; 10] = [
        [14, 17, 19, 21, 25, 17, 14],
        [4, 12, 4, 4, 4, 4, 14],
        [14, 17, 1, 2, 4, 8, 31],
        [30, 1, 1, 14, 1, 1, 30],
        [2, 6, 10, 18, 31, 2, 2],
        [31, 16, 16, 30, 1, 1, 30],
        [14, 16, 16, 30, 17, 17, 14],
        [31, 1, 2, 4, 8, 8, 8],
        [14, 17, 17, 14, 17, 17, 14],
        [14, 17, 17, 15, 1, 1, 14],
    ];
    for (index, ch) in text.chars().enumerate() {
        let Some(digit) = ch.to_digit(10) else {
            continue;
        };
        for (row, bits) in DIGITS[digit as usize].iter().enumerate() {
            for column in 0..5 {
                if bits & (1 << (4 - column)) != 0 {
                    for dy in 0..2 {
                        for dx in 0..2 {
                            let px = x + index as u32 * 12 + column * 2 + dx;
                            let py = y + row as u32 * 2 + dy;
                            if px < image.width() && py < image.height() {
                                image.put_pixel(px, py, color);
                            }
                        }
                    }
                }
            }
        }
    }
}

fn contact_sheet(db: &Db, ids: &[i64]) -> Result<AssetPreview> {
    const TILE: u32 = 240;
    const HEIGHT: u32 = 216;
    let columns = ids.len().min(4) as u32;
    let rows = (ids.len() as u32).div_ceil(columns);
    let (width, height) = (columns * TILE, rows * HEIGHT);
    let mut sheet = RgbaImage::from_pixel(width, height, Rgba([246, 247, 250, 255]));
    let mut asset_ids = Vec::new();
    let mut missing = Vec::new();
    for (index, id) in ids.iter().enumerate() {
        let x = index as u32 % columns * TILE;
        let y = index as u32 / columns * HEIGHT;
        for py in y + 8..y + 180 {
            for px in x + 8..x + TILE - 8 {
                let shade = if ((px - x) / 12 + (py - y) / 12).is_multiple_of(2) {
                    230
                } else {
                    245
                };
                sheet.put_pixel(px, py, Rgba([shade, shade, shade, 255]));
            }
        }
        match db.asset(*id).and_then(|asset| decode_asset(&asset)) {
            Ok(source) => {
                let (thumbnail, offset_x, offset_y) =
                    fit_image(source, TILE - 24, 160, Fit::Contain);
                overlay(
                    &mut sheet,
                    &thumbnail,
                    i64::from(x + 12) + offset_x,
                    i64::from(y + 14) + offset_y,
                );
                asset_ids.push(*id);
            }
            Err(error) => {
                draw_rect(
                    &mut sheet,
                    i64::from(x + 12),
                    i64::from(y + 14),
                    TILE - 24,
                    160,
                    4,
                    Rgba([250, 226, 226, 255]),
                );
                missing.push(MissingAsset {
                    asset_id: *id,
                    error: format!("{error:#}"),
                });
            }
        }
        draw_number(
            &mut sheet,
            &id.to_string(),
            x + 12,
            y + 190,
            Rgba([25, 32, 48, 255]),
        );
    }
    let png = png_bytes(&sheet)?;
    Ok(AssetPreview {
        png,
        width,
        height,
        asset_ids,
        missing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::FileRecord;
    use std::time::Duration;

    fn fixture() -> (tempfile::TempDir, Db, DesignService, i64) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let library = root.join("library");
        fs::create_dir(&library).unwrap();
        RgbaImage::from_pixel(4, 2, Rgba([240, 60, 30, 128]))
            .save(library.join("透明素材.png"))
            .unwrap();
        let db = Db::open(&root.join("picsoc.sqlite3")).unwrap();
        let lib = db.add_library("测试", library.to_str().unwrap()).unwrap();
        let metadata = fs::metadata(library.join("透明素材.png")).unwrap();
        let modified = metadata
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap();
        let records = vec![FileRecord {
            relative_path: "透明素材.png".into(),
            name: "透明素材.png".into(),
            format: "png".into(),
            size: metadata.len() as i64,
            modified_at: modified.as_secs() as i64,
            mtime_ns: modified.as_nanos().min(i64::MAX as u128) as i64,
        }];
        let id = db.index_batch(lib.id, 1, &records).unwrap()[0].0.id;
        let service = DesignService::new(db.clone(), root).unwrap();
        (temp, db, service, id)
    }

    fn scene(id: i64) -> Scene {
        Scene {
            version: 1,
            name: "登录页".into(),
            canvas: Canvas {
                width: 8,
                height: 8,
                background: "transparent".into(),
            },
            layers: vec![Layer::Image {
                asset_id: id,
                x: 0,
                y: 0,
                width: 8,
                height: 8,
                fit: Fit::Contain,
                opacity: 0.5,
            }],
        }
    }

    fn wait(service: &DesignService, id: &str) -> RenderJob {
        for _ in 0..1000 {
            let job = service.job(id).unwrap();
            if matches!(job.status.as_str(), "succeeded" | "failed") {
                return job;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("render job timed out: {id}");
    }

    #[test]
    fn transparent_contain_and_opacity_preserve_asset_shape() {
        let (_temp, db, service, id) = fixture();
        let saved = service
            .save(SaveDesign {
                design_id: None,
                expected_revision: None,
                scene: scene(id),
            })
            .unwrap();
        let rendered = compose(&db, &service.fonts, &saved, RenderQuality::Final).unwrap();
        assert_eq!(rendered.dimensions(), (8, 8));
        assert_eq!(rendered.get_pixel(0, 0).0, [0, 0, 0, 0]);
        assert_eq!(rendered.get_pixel(4, 4).0, [240, 60, 30, 64]);
        assert_eq!(rendered.get_pixel(4, 7).0, [0, 0, 0, 0]);
        assert_eq!(
            image::open(Path::new(&db.asset(id).unwrap().library_path).join("透明素材.png"))
                .unwrap()
                .to_rgba8()
                .dimensions(),
            (4, 2)
        );
    }

    #[test]
    fn source_over_alpha_and_negative_coordinates_clip_correctly() {
        let mut canvas = RgbaImage::from_pixel(2, 2, Rgba([0, 0, 255, 255]));
        draw_rect(&mut canvas, -1, -1, 2, 2, 0, Rgba([255, 0, 0, 128]));
        assert_eq!(canvas.get_pixel(0, 0).0, [128, 0, 127, 255]);
        assert_eq!(canvas.get_pixel(1, 0).0, [0, 0, 255, 255]);
        let mut transparent = RgbaImage::new(1, 1);
        draw_rect(&mut transparent, 0, 0, 1, 1, 0, Rgba([60, 100, 180, 128]));
        assert_eq!(transparent.get_pixel(0, 0).0, [60, 100, 180, 128]);
    }

    #[test]
    fn cover_extreme_aspect_ratio_stays_bounded() {
        let narrow = RgbaImage::from_pixel(1, 16_000, Rgba([40, 60, 80, 255]));
        let (output, x, y) = fit_image(narrow, 100, 100, Fit::Cover);
        assert_eq!((output.width(), output.height(), x, y), (100, 100, 0, 0));
        assert_eq!(output.get_pixel(50, 50).0, [40, 60, 80, 255]);
    }

    #[test]
    fn premultiplied_resize_does_not_make_transparent_color_halo() {
        let mut source = RgbaImage::new(2, 1);
        source.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        source.put_pixel(1, 0, Rgba([0, 255, 255, 0]));
        let resized = resize_alpha(source, 1, 1);
        assert_eq!(resized.get_pixel(0, 0).0, [255, 0, 0, 128]);
    }

    #[test]
    fn scene_validation_rejects_unknown_fields_and_resource_exhaustion() {
        assert!(serde_json::from_value::<Scene>(serde_json::json!({
            "version":1,"name":"x","canvas":{"width":10,"height":10},
            "layers":[{"type":"image","asset_id":1,"x":0,"y":0,"width":10,"height":10,"path":"/etc/passwd"}]
        })).is_err());
        let mut value = scene(1);
        value.canvas.width = u32::MAX;
        assert_eq!(error_status(&validate_scene(&value, &[]).unwrap_err()), 400);
        value.canvas.width = 8;
        value.layers.resize(MAX_LAYERS + 1, value.layers[0].clone());
        assert!(validate_scene(&value, &[]).is_err());
        value.layers = vec![Layer::Text {
            text: "星海".into(),
            x: 0,
            y: 0,
            font_size: 32.0,
            color: "#FFFFFF".into(),
            font_id: "/etc/passwd".into(),
            max_width: None,
            align: TextAlign::Left,
            line_height: 1.2,
            opacity: 1.0,
        }];
        assert!(validate_scene(&value, &[]).is_err());
        assert!(parse_color("#ééé").is_err());
        assert!(validate_box(i32::MIN, 0, 1, 1, 1.0).is_err());
        assert!(validate_box(0, 0, 1, 1, f32::NAN).is_err());
    }

    #[test]
    fn immutable_revisions_survive_restart_and_prevent_lost_updates() {
        let (temp, db, service, id) = fixture();
        let first = service
            .save(SaveDesign {
                design_id: None,
                expected_revision: None,
                scene: scene(id),
            })
            .unwrap();
        let mut edited = first.scene.clone();
        edited.name = "第二版".into();
        let second = service
            .save(SaveDesign {
                design_id: Some(first.design_id.clone()),
                expected_revision: Some(1),
                scene: edited.clone(),
            })
            .unwrap();
        assert_eq!(second.revision, 2);
        let error = service
            .save(SaveDesign {
                design_id: Some(first.design_id.clone()),
                expected_revision: Some(1),
                scene: edited,
            })
            .unwrap_err();
        assert_eq!(error_status(&error), 409);
        assert_eq!(
            service.get(&first.design_id, Some(1)).unwrap().name,
            "登录页"
        );
        drop(service);
        let restored = DesignService::new(db, temp.path().canonicalize().unwrap()).unwrap();
        assert_eq!(restored.get(&first.design_id, None).unwrap().name, "第二版");
        assert_eq!(restored.list(10, 0).unwrap().len(), 1);
    }

    #[test]
    fn render_exports_png_preview_and_editable_layout_then_survives_restart() {
        let (temp, db, service, id) = fixture();
        let saved = service
            .save(SaveDesign {
                design_id: None,
                expected_revision: None,
                scene: scene(id),
            })
            .unwrap();
        let submitted = service
            .submit(RenderRequest {
                design_id: saved.design_id.clone(),
                revision: Some(1),
                quality: RenderQuality::Final,
            })
            .unwrap();
        let finished = wait(&service, &submitted.job_id);
        assert_eq!(finished.status, "succeeded", "{:?}", finished.error);
        assert_eq!(
            finished.output_url.as_deref(),
            Some(format!("/api/design-jobs/{}/output.png", submitted.job_id).as_str())
        );
        assert_eq!(
            image::open(
                service
                    .output_path(&submitted.job_id, "output.png")
                    .unwrap()
            )
            .unwrap()
            .to_rgba8()
            .dimensions(),
            (8, 8)
        );
        assert!(
            service
                .output_path(&submitted.job_id, "preview.png")
                .unwrap()
                .exists()
        );
        let layout: StoredDesign = read_json(
            &service
                .output_path(&submitted.job_id, "layout.json")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(layout.scene.name, saved.scene.name);
        assert_eq!(layout.asset_sources[0].asset_id, id);
        assert!(
            service
                .output_path(&submitted.job_id, "../picsoc.sqlite3")
                .is_err()
        );
        assert!(service.get("../../etc", None).is_err());
        drop(service);
        let restored = DesignService::new(db, temp.path().canonicalize().unwrap()).unwrap();
        assert_eq!(restored.job(&submitted.job_id).unwrap().status, "succeeded");
        assert_eq!(
            restored
                .get(&saved.design_id, None)
                .unwrap()
                .latest_job
                .unwrap()
                .job_id,
            submitted.job_id
        );
    }

    #[test]
    fn changed_index_source_fails_old_revision_instead_of_changing_design_silently() {
        let (_temp, db, service, id) = fixture();
        let saved = service
            .save(SaveDesign {
                design_id: None,
                expected_revision: None,
                scene: scene(id),
            })
            .unwrap();
        let asset = db.asset(id).unwrap();
        db.index_batch(
            asset.library_id,
            2,
            &[FileRecord {
                relative_path: asset.relative_path,
                name: asset.name,
                format: asset.format,
                size: asset.size + 1,
                modified_at: asset.modified_at,
                mtime_ns: asset.mtime_ns + 1,
            }],
        )
        .unwrap();
        let submitted = service
            .submit(RenderRequest {
                design_id: saved.design_id,
                revision: None,
                quality: RenderQuality::Preview,
            })
            .unwrap();
        let finished = wait(&service, &submitted.job_id);
        assert_eq!(finished.status, "failed");
        assert!(finished.error.unwrap().contains("已修改"));
    }

    #[test]
    fn changed_original_without_rescan_is_rejected_by_open_file_fingerprint() {
        let (_temp, db, service, id) = fixture();
        let saved = service
            .save(SaveDesign {
                design_id: None,
                expected_revision: None,
                scene: scene(id),
            })
            .unwrap();
        let asset = db.asset(id).unwrap();
        // Keep the index untouched: this is the race a DB-only cache-key check cannot detect.
        RgbaImage::from_pixel(6, 3, Rgba([0, 255, 0, 255]))
            .save(Path::new(&asset.library_path).join(&asset.relative_path))
            .unwrap();
        assert_eq!(
            db.asset(id).unwrap().cache_key(),
            saved.asset_sources[0].cache_key
        );
        let error = compose(&db, &service.fonts, &saved, RenderQuality::Final).unwrap_err();
        assert!(format!("{error:#}").contains("重新扫描"));
        let preview = service.preview_assets(&[id]).unwrap();
        assert!(preview.asset_ids.is_empty());
        assert!(preview.missing[0].error.contains("重新扫描"));
    }

    #[test]
    fn queue_saturation_returns_busy_and_creates_no_orphan_jobs() {
        let (_temp, _db, service, id) = fixture();
        let saved = service
            .save(SaveDesign {
                design_id: None,
                expected_revision: None,
                scene: scene(id),
            })
            .unwrap();
        let guard = service.render_lock.lock().unwrap();
        let mut submitted = Vec::new();
        let mut busy = 0;
        for _ in 0..12 {
            match service.submit(RenderRequest {
                design_id: saved.design_id.clone(),
                revision: None,
                quality: RenderQuality::Preview,
            }) {
                Ok(job) => submitted.push(job.job_id),
                Err(error) => {
                    assert_eq!(error_status(&error), 429);
                    busy += 1;
                }
            }
        }
        assert!(busy > 0);
        assert!(submitted.len() <= QUEUE_CAPACITY + 1);
        assert_eq!(
            fs::read_dir(service.root.join("jobs")).unwrap().count(),
            submitted.len()
        );
        drop(guard);
        for id in submitted {
            assert_eq!(wait(&service, &id).status, "succeeded");
        }
    }

    #[test]
    fn interrupted_job_becomes_failed_on_restart() {
        let (temp, db, service, id) = fixture();
        let saved = service
            .save(SaveDesign {
                design_id: None,
                expected_revision: None,
                scene: scene(id),
            })
            .unwrap();
        let unfinished = RenderJob {
            job_id: new_id("j").unwrap(),
            design_id: saved.design_id,
            revision: 1,
            quality: RenderQuality::Final,
            status: "running".into(),
            created_at: now(),
            finished_at: None,
            error: None,
            width: 8,
            height: 8,
            output_url: None,
            preview_url: None,
            layout_url: None,
        };
        persist_job(&service.root, &unfinished).unwrap();
        drop(service);
        let restarted = DesignService::new(db, temp.path().canonicalize().unwrap()).unwrap();
        let result = restarted.job(&unfinished.job_id).unwrap();
        assert_eq!(result.status, "failed");
        assert!(result.error.unwrap().contains("重新启动"));
    }

    #[test]
    fn contact_sheet_labels_actual_ids_and_reports_missing_assets() {
        let (_temp, _db, service, id) = fixture();
        let preview = service.preview_assets(&[id, 99999]).unwrap();
        let image = image::load_from_memory(&preview.png).unwrap().into_rgba8();
        assert_eq!((preview.width, preview.height), (480, 216));
        assert_eq!(image.dimensions(), (480, 216));
        assert_eq!(preview.asset_ids, [id]);
        assert_eq!(preview.missing[0].asset_id, 99999);
        assert!(service.preview_assets(&[id, id]).is_err());
        assert!(service.preview_assets(&[]).is_err());
        assert!(service.preview_assets(&[id; 25]).is_err());
    }

    #[test]
    fn preview_scales_canvas_coordinates_without_mutating_saved_scene() {
        let canvas = Canvas {
            width: 1920,
            height: 1080,
            background: "#000000".into(),
        };
        assert_eq!(
            output_dimensions(&canvas, RenderQuality::Preview),
            (1280, 720)
        );
        assert_eq!(
            output_dimensions(&canvas, RenderQuality::Final),
            (1920, 1080)
        );
        assert_eq!(
            output_dimensions(
                &Canvas {
                    width: 1,
                    height: 4096,
                    background: "transparent".into()
                },
                RenderQuality::Preview
            ),
            (1, 1280)
        );
        assert_eq!(
            render_scale(
                &Canvas {
                    width: 1,
                    height: 4096,
                    background: "transparent".into()
                },
                RenderQuality::Preview
            ),
            0.3125
        );
    }

    #[test]
    fn installed_chinese_font_draws_real_glyph_pixels_and_wraps() {
        let temp = tempfile::tempdir().unwrap();
        let fonts = load_fonts(temp.path());
        #[cfg(target_os = "macos")]
        assert!(
            fonts.iter().any(|font| font.info.supports_chinese),
            "macOS system Chinese fonts were not loaded"
        );
        let Some(font) = fonts.iter().find(|font| font.info.supports_chinese) else {
            return;
        };
        let mut canvas = RgbaImage::new(120, 100);
        draw_text(
            &mut canvas,
            &font.font,
            "星海中文",
            0.0,
            0.0,
            32.0,
            55.0,
            TextAlign::Left,
            1.2,
            Rgba([255, 255, 255, 255]),
        );
        assert!(canvas.pixels().filter(|p| p.0[3] > 0).count() > 100);
        assert!(
            canvas
                .enumerate_pixels()
                .any(|(_, y, p)| y > 40 && p.0[3] > 0)
        );
        assert!(select_font(&fonts, &font.info.id, "星海中文").is_ok());
        assert!(select_font(&fonts, "not-a-font", "星海").is_err());
        if let Some(latin) = fonts.iter().find(|font| !font.info.supports_chinese) {
            assert!(select_font(&fonts, &latin.info.id, "星海").is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_generated_directory_is_rejected_without_writing_outside() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let outside = root.join("outside");
        fs::create_dir(&outside).unwrap();
        let data = root.join("data");
        fs::create_dir(&data).unwrap();
        std::os::unix::fs::symlink(&outside, data.join("generated")).unwrap();
        let db = Db::open(&root.join("db.sqlite3")).unwrap();
        assert!(DesignService::new(db, data).is_err());
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_job_directory_cannot_read_or_publish_outside_generated() {
        let (temp, _db, service, _id) = fixture();
        let outside = temp.path().join("outside-job");
        fs::create_dir(&outside).unwrap();
        let job = RenderJob {
            job_id: new_id("j").unwrap(),
            design_id: new_id("d").unwrap(),
            revision: 1,
            quality: RenderQuality::Final,
            status: "queued".into(),
            created_at: now(),
            finished_at: None,
            error: None,
            width: 8,
            height: 8,
            output_url: None,
            preview_url: None,
            layout_url: None,
        };
        atomic_json(&outside.join("queued.json"), &job).unwrap();
        std::os::unix::fs::symlink(&outside, service.root.join("jobs").join(&job.job_id)).unwrap();
        assert!(service.job(&job.job_id).is_err());
        assert!(persist_job(&service.root, &job).is_err());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
    }
}
