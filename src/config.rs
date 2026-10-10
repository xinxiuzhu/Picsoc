//! File-based startup configuration. Credentials deliberately have no `Debug`
//! implementation, and TOML parser diagnostics never print input fragments.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{ErrorKind, Read, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Serialize)]
pub struct AppConfig {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub open_browser: bool,
    pub workers: u8,
    pub scan_interval: u64,
    pub password: String,
    pub mcp: McpConfig,
}

#[derive(Clone, Default, Serialize)]
pub struct McpConfig {
    pub enabled: bool,
    pub public_url: String,
    pub token: String,
    pub redirect_uris: Vec<String>,
}

pub struct LoadedConfig {
    pub config: AppConfig,
    pub created: bool,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    bind: Option<SocketAddr>,
    data_dir: Option<PathBuf>,
    open_browser: Option<bool>,
    workers: Option<u8>,
    scan_interval: Option<u64>,
    password: Option<String>,
    mcp: Option<FileMcpConfig>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileMcpConfig {
    enabled: Option<bool>,
    public_url: Option<String>,
    token: Option<String>,
    redirect_uris: Option<Vec<String>>,
}

impl AppConfig {
    pub fn defaults(data_dir: PathBuf) -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], 3210)),
            data_dir,
            open_browser: true,
            workers: 1,
            scan_interval: 300,
            password: String::new(),
            mcp: McpConfig::default(),
        }
    }

    fn validate(&self, path: &Path) -> Result<()> {
        if !(1..=4).contains(&self.workers) {
            bail!("配置文件 {}：workers 必须是 1 到 4", path.display());
        }
        if self.data_dir.as_os_str().is_empty() {
            bail!("配置文件 {}：data_dir 不能为空", path.display());
        }
        Ok(())
    }
}

/// Read a legacy configuration without creating or changing any files. Relative
/// data paths remain anchored to the legacy file before writing a new template.
pub fn load_existing(path: &Path, initial: &AppConfig) -> Result<Option<AppConfig>> {
    match File::open(path) {
        Ok(file) => read_config(file, path, initial).map(Some),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("无法读取配置文件 {}", path.display())),
    }
}

/// Read an existing file without changing it, or create a complete annotated
/// template exactly once. CLI overrides are applied by the caller after loading.
pub fn load_or_create(path: &Path, initial: &AppConfig) -> Result<LoadedConfig> {
    match File::open(path) {
        Ok(file) => read_config(file, path, initial).map(|config| LoadedConfig {
            config,
            created: false,
        }),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            initial.validate(path)?;
            let mut initial = initial.clone();
            if initial.data_dir.is_relative() {
                initial.data_dir = std::env::current_dir()
                    .context("无法读取当前目录")?
                    .join(&initial.data_dir);
            }
            let template = template(&initial)?;
            fs::create_dir_all(config_parent(path))
                .with_context(|| format!("无法创建配置目录 {}", config_parent(path).display()))?;
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(path) {
                Ok(mut file) => {
                    file.write_all(template.as_bytes())
                        .and_then(|()| file.sync_all())
                        .with_context(|| format!("无法保存配置文件 {}", path.display()))?;
                    Ok(LoadedConfig {
                        config: initial,
                        created: true,
                    })
                }
                // Another process may have created the configuration while the
                // template was prepared. Its contents always take precedence.
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    let file = File::open(path)
                        .with_context(|| format!("无法读取配置文件 {}", path.display()))?;
                    read_config(file, path, &initial).map(|config| LoadedConfig {
                        config,
                        created: false,
                    })
                }
                Err(error) => {
                    Err(error).with_context(|| format!("无法创建配置文件 {}", path.display()))
                }
            }
        }
        Err(error) => Err(error).with_context(|| format!("无法读取配置文件 {}", path.display())),
    }
}

fn config_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn read_config(file: File, path: &Path, initial: &AppConfig) -> Result<AppConfig> {
    let mut contents = String::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_string(&mut contents)
        .with_context(|| format!("无法读取 UTF-8 配置文件 {}", path.display()))?;
    if contents.len() as u64 > MAX_CONFIG_BYTES {
        bail!("配置文件 {}：文件大小不能超过 1 MiB", path.display());
    }
    let parsed: FileConfig = toml::from_str(&contents).map_err(|error| {
        // `toml::de::Error` includes the source line and may include credential
        // values in Display/Debug. Only its location is safe to report here.
        let offset = error.span().map_or(0, |span| span.start);
        let prefix = &contents[..floor_char_boundary(&contents, offset)];
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
        anyhow!(
            "配置文件 {}：第 {} 行第 {} 列 TOML 配置无效，请检查语法、字段名和字段类型",
            path.display(),
            line,
            column
        )
    })?;
    let mut config = initial.clone();
    if let Some(bind) = parsed.bind {
        config.bind = bind;
    }
    if let Some(data_dir) = parsed.data_dir {
        if data_dir.as_os_str().is_empty() {
            bail!("配置文件 {}：data_dir 不能为空", path.display());
        }
        config.data_dir = if data_dir.is_relative() {
            config_parent(path).join(data_dir)
        } else {
            data_dir
        };
    }
    if let Some(open_browser) = parsed.open_browser {
        config.open_browser = open_browser;
    }
    if let Some(workers) = parsed.workers {
        config.workers = workers;
    }
    if let Some(scan_interval) = parsed.scan_interval {
        config.scan_interval = scan_interval;
    }
    if let Some(password) = parsed.password {
        config.password = password;
    }
    if let Some(mcp) = parsed.mcp {
        if let Some(enabled) = mcp.enabled {
            config.mcp.enabled = enabled;
        }
        if let Some(public_url) = mcp.public_url {
            config.mcp.public_url = public_url;
        }
        if let Some(token) = mcp.token {
            config.mcp.token = token;
        }
        if let Some(redirect_uris) = mcp.redirect_uris {
            config.mcp.redirect_uris = redirect_uris;
        }
    }
    config.validate(path)?;
    Ok(config)
}

fn floor_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn template(config: &AppConfig) -> Result<String> {
    let mut result = String::from(
        "# Picsoc 启动配置\n\
         # 首次启动自动生成；按 Ctrl+C 退出后编辑，再重新启动生效。\n\
         # 已有配置不会被自动覆盖。密码和令牌写在此文件中，请妥善保管。\n\
         # 字符串需要使用 TOML 引号；Windows 路径可写为 'C:\\Picsoc\\data'。\n\n",
    );
    append_field(
        &mut result,
        "# HTTP 监听地址。其他设备访问时可改为 \"0.0.0.0:3210\"。",
        "bind",
        &config.bind,
    )?;
    append_field(
        &mut result,
        "# 数据库、缩略图和设计作品目录。相对路径以此配置文件所在目录为基准。",
        "data_dir",
        &config.data_dir,
    )?;
    append_field(
        &mut result,
        "# 启动时打开浏览器；服务器或 Docker 部署建议设为 false。",
        "open_browser",
        &config.open_browser,
    )?;
    append_field(
        &mut result,
        "# 图片解码并发数：1 到 4。低配置设备建议使用 1。",
        "workers",
        &config.workers,
    )?;
    append_field(
        &mut result,
        "# 增量扫描间隔（秒），0 表示关闭周期扫描。",
        "scan_interval",
        &config.scan_interval,
    )?;
    append_field(
        &mut result,
        "# 网页登录密码。空字符串表示不启用网页登录密码。",
        "password",
        &config.password,
    )?;
    result.push_str("\n# ChatGPT / MCP 素材搜索与设计接口\n[mcp]\n");
    append_field(
        &mut result,
        "# 是否启用 MCP；启用时需配置 OAuth（公网地址及登录密码）或独立令牌。",
        "enabled",
        &config.mcp.enabled,
    )?;
    append_field(
        &mut result,
        "# 反向代理的 HTTPS 根地址，例如 \"https://orionai.iepose.cn\"。",
        "public_url",
        &config.mcp.public_url,
    )?;
    append_field(
        &mut result,
        "# 可选独立 MCP Bearer 令牌，至少 32 个 ASCII 字符；OAuth 接入可留空。",
        "token",
        &config.mcp.token,
    )?;
    append_field(
        &mut result,
        "# 额外允许的完整 OAuth 回调地址列表。默认已允许 ChatGPT 稳定回调地址。",
        "redirect_uris",
        &config.mcp.redirect_uris,
    )?;
    Ok(result)
}

fn append_field<T: Serialize>(
    output: &mut String,
    comment: &str,
    name: &str,
    value: &T,
) -> Result<()> {
    // Serialize each entire value rather than inserting comments into serialized
    // text: a multiline password may itself contain text resembling a TOML key.
    let encoded = toml::to_string(&BTreeMap::from([(name, value)]))
        .map_err(|_| anyhow!("无法生成 TOML 配置，请检查数据目录是否为有效 UTF-8 路径"))?;
    output.push_str(comment);
    output.push('\n');
    output.push_str(&encoded);
    output.push('\n');
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults(dir: &Path) -> AppConfig {
        AppConfig::defaults(dir.join("data"))
    }

    fn load_error(path: &Path, initial: &AppConfig) -> String {
        match load_or_create(path, initial) {
            Ok(_) => panic!("invalid configuration was accepted"),
            Err(error) => format!("{error:#}"),
        }
    }

    #[test]
    fn creates_complete_template_and_loads_user_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings/config.toml");
        let initial = defaults(dir.path());
        let created = load_or_create(&path, &initial).unwrap();
        assert!(created.created);
        assert_eq!(created.config.bind, initial.bind);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("首次启动自动生成"));
        assert!(text.contains("[mcp]"));
        let loaded = load_or_create(&path, &initial).unwrap();
        assert!(!loaded.created);
        assert_eq!(loaded.config.data_dir, initial.data_dir);
        assert_eq!(loaded.config.scan_interval, 300);
        assert!(loaded.config.open_browser);
        assert_eq!(fs::read_to_string(&path).unwrap(), text);

        let edits = text
            .replace("127.0.0.1:3210", "0.0.0.0:4000")
            .replace("open_browser = true", "open_browser = false")
            .replace("password = \"\"", "password = \"中文密码\"")
            .replace("enabled = false", "enabled = true")
            .replace(
                "public_url = \"\"",
                "public_url = \"https://orionai.iepose.cn\"",
            );
        fs::write(&path, &edits).unwrap();
        let loaded = load_or_create(&path, &initial).unwrap();
        assert!(!loaded.created);
        assert_eq!(loaded.config.bind, "0.0.0.0:4000".parse().unwrap());
        assert!(!loaded.config.open_browser);
        assert_eq!(loaded.config.password, "中文密码");
        assert!(loaded.config.mcp.enabled);
        assert_eq!(loaded.config.mcp.public_url, "https://orionai.iepose.cn");
        assert_eq!(fs::read_to_string(path).unwrap(), edits);
    }

    #[test]
    fn partial_configuration_keeps_defaults_and_resolves_relative_paths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let initial = defaults(dir.path());
        fs::write(
            &path,
            "# 保留默认数据目录\nworkers = 2\n[mcp]\nenabled = false\n",
        )
        .unwrap();
        let loaded = load_or_create(&path, &initial).unwrap();
        assert_eq!(loaded.config.data_dir, initial.data_dir);
        assert_eq!(loaded.config.workers, 2);
        assert_eq!(loaded.config.bind, initial.bind);
        assert!(loaded.config.mcp.token.is_empty());
        fs::write(&path, "data_dir = '嵌套/data'\n").unwrap();
        let loaded = load_or_create(&path, &initial).unwrap();
        assert_eq!(loaded.config.data_dir, dir.path().join("嵌套/data"));
        assert!(!loaded.config.data_dir.exists());
    }

    #[test]
    fn serializer_roundtrips_quotes_backslashes_and_multiline_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut initial = defaults(dir.path());
        initial.password = "秘密\\\"\npassword = \"another\"\n".to_string();
        initial.mcp.token = "独立 \\ \" 凭据".to_string();
        initial.mcp.redirect_uris = vec!["https://example.test/callback?quoted=\"a\"".to_string()];
        load_or_create(&path, &initial).unwrap();
        let loaded = load_or_create(&path, &initial).unwrap();
        assert_eq!(loaded.config.password, initial.password);
        assert_eq!(loaded.config.mcp.token, initial.mcp.token);
        assert_eq!(loaded.config.mcp.redirect_uris, initial.mcp.redirect_uris);
        // TOML must encode backslashes safely even when generating on Unix for
        // a Windows-looking path; parsing must recover the exact original text.
        let mut snippet = String::new();
        append_field(&mut snippet, "# Windows", "data_dir", &r"C:\Picsoc\素材").unwrap();
        let parsed: FileConfig = toml::from_str(&snippet).unwrap();
        assert_eq!(parsed.data_dir.unwrap(), PathBuf::from(r"C:\Picsoc\素材"));
    }

    #[test]
    fn invalid_input_has_no_side_effects_and_never_reports_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let initial = defaults(dir.path());
        let cases = [
            "password = \"SECRET-PLAIN\"\nworkers = \"SECRET-TYPE\"\n",
            "password = \"SECRET-UNTERMINATED\n",
            "password = \"SECRET-PLAIN\"\nSECRET-TYPO = true\n",
            "[mcp]\ntoken = \"SECRET-TOKEN\"\nSECRET-UNKNOWN = true\n",
            "bind = \"SECRET-BAD-ADDRESS\"\n",
        ];
        for contents in cases {
            fs::write(&path, contents).unwrap();
            let error = load_error(&path, &initial);
            assert!(error.contains("第 "));
            assert!(error.contains(" 行第 "));
            assert!(!error.contains("SECRET"), "secret leaked: {error}");
            assert_eq!(fs::read_to_string(&path).unwrap(), contents);
            assert!(!initial.data_dir.exists());
        }
    }

    #[test]
    fn invalid_ranges_and_empty_directory_leave_file_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let initial = defaults(dir.path());
        for contents in ["workers = 0\n", "workers = 5\n", "data_dir = ''\n"] {
            fs::write(&path, contents).unwrap();
            assert!(load_or_create(&path, &initial).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), contents);
            assert!(!initial.data_dir.exists());
        }
        fs::remove_file(&path).unwrap();
        let mut invalid = initial.clone();
        invalid.workers = 0;
        assert!(load_or_create(&path, &invalid).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn directories_and_oversized_files_are_rejected_without_changes() {
        let dir = tempfile::tempdir().unwrap();
        let initial = defaults(dir.path());
        assert!(load_or_create(dir.path(), &initial).is_err());
        let path = dir.path().join("config.toml");
        fs::write(&path, " ".repeat(MAX_CONFIG_BYTES as usize + 1)).unwrap();
        assert!(load_error(&path, &initial).contains("1 MiB"));
        assert_eq!(fs::metadata(path).unwrap().len(), MAX_CONFIG_BYTES + 1);
    }

    #[test]
    fn generated_relative_initial_directory_is_anchored_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let initial = AppConfig::defaults(PathBuf::from("relative-picsoc-data"));
        let generated = load_or_create(&path, &initial).unwrap();
        let loaded = load_or_create(&path, &initial).unwrap();
        assert!(generated.config.data_dir.is_absolute());
        assert_eq!(loaded.config.data_dir, generated.config.data_dir);
    }

    #[cfg(unix)]
    #[test]
    fn generated_configuration_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        load_or_create(&path, &defaults(dir.path())).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        load_or_create(&path, &defaults(dir.path())).unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
}
