mod api;
mod auth;
mod config;
mod db;
mod design;
mod folders;
mod i18n;
mod mcp;
mod mcp_auth;
mod mcp_tools;
mod media;
mod models;
mod network;
mod scanner;

use anyhow::{Context, Result};
use clap::Parser;
use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

#[derive(Parser, Debug)]
#[command(version, about = "Picsoc · 轻量中文图片素材库")]
struct Args {
    #[arg(
        long,
        help = "TOML 配置文件，默认位于数据目录 config.toml；不存在时自动生成"
    )]
    config: Option<PathBuf>,
    #[arg(long, help = "临时覆盖配置中的 HTTP 监听地址")]
    bind: Option<SocketAddr>,
    #[arg(
        long,
        help = "数据目录及默认配置文件位置，默认使用当前用户应用数据目录"
    )]
    data_dir: Option<PathBuf>,
    #[arg(long, help = "启动后不自动打开浏览器")]
    no_open: bool,
    #[arg(long,value_parser=clap::value_parser!(u8).range(1..=4),help="临时覆盖图片解码并发数（1–4）")]
    workers: Option<u8>,
    #[arg(long, help = "临时覆盖增量扫描间隔（秒），0 为关闭周期扫描")]
    scan_interval: Option<u64>,
    #[arg(
        long,
        num_args = 0..=1,
        default_missing_value = "true",
        help = "临时覆盖 MCP 开关（--mcp 或 --mcp=false）"
    )]
    mcp: Option<bool>,
    #[arg(long, help = "临时覆盖 MCP OAuth 的 HTTPS 根地址")]
    public_url: Option<String>,
    #[arg(
        long,
        value_delimiter = ',',
        help = "额外允许的 OAuth 完整回调地址（逗号分隔）"
    )]
    mcp_redirect_uris: Vec<String>,
}

impl Args {
    fn apply(&self, settings: &mut config::AppConfig) {
        if let Some(bind) = self.bind {
            settings.bind = bind;
        }
        if let Some(data_dir) = &self.data_dir {
            settings.data_dir.clone_from(data_dir);
        }
        if self.no_open {
            settings.open_browser = false;
        }
        if let Some(workers) = self.workers {
            settings.workers = workers;
        }
        if let Some(scan_interval) = self.scan_interval {
            settings.scan_interval = scan_interval;
        }
        if let Some(enabled) = self.mcp {
            settings.mcp.enabled = enabled;
        }
        if let Some(public_url) = &self.public_url {
            settings.mcp.public_url.clone_from(public_url);
        }
        if !self.mcp_redirect_uris.is_empty() {
            settings
                .mcp
                .redirect_uris
                .clone_from(&self.mcp_redirect_uris);
        }
    }
}

fn absolute_path(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn default_data_dir() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        Ok(PathBuf::from(
            std::env::var_os("LOCALAPPDATA").context("请通过 --data-dir 设置数据目录")?,
        )
        .join("Picsoc"))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(
            PathBuf::from(std::env::var_os("HOME").context("请通过 --data-dir 设置数据目录")?)
                .join("Library/Application Support/Picsoc"),
        )
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Some(path) = std::env::var_os("XDG_DATA_HOME").filter(|p| !p.is_empty()) {
            return Ok(PathBuf::from(path).join("picsoc"));
        }
        Ok(
            PathBuf::from(std::env::var_os("HOME").context("请通过 --data-dir 设置数据目录")?)
                .join(".local/share/picsoc"),
        )
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "picsoc=info".into()),
        )
        .init();
    let args = Args::parse();
    let initial_data_dir = absolute_path(args.data_dir.clone().map_or_else(default_data_dir, Ok)?)?;
    let config_path = absolute_path(
        args.config
            .clone()
            .unwrap_or_else(|| initial_data_dir.join("config.toml")),
    )?;
    let mut initial = config::AppConfig::defaults(initial_data_dir);
    args.apply(&mut initial);
    let loaded = config::load_or_create(&config_path, &initial)?;
    println!("配置文件：{}", config_path.display());
    if loaded.created {
        println!("已生成 config.toml；按 Ctrl+C 停止服务，编辑配置后重新启动即可生效。");
    }
    let mut settings = loaded.config;
    args.apply(&mut settings);
    let public_url = Some(settings.mcp.public_url.clone()).filter(|value| !value.trim().is_empty());
    let mcp_redirect_uris: Vec<_> = settings
        .mcp
        .redirect_uris
        .iter()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .collect();
    let data_dir = settings.data_dir;
    std::fs::create_dir_all(&data_dir).context("无法创建数据目录")?;
    let data_dir = data_dir.canonicalize()?;
    let database = db::Db::open(&data_dir.join("picsoc.sqlite3"))?;
    let scanner =
        scanner::Scanner::new(database.clone(), data_dir.clone(), settings.workers.into());
    let designs = design::DesignService::new(database.clone(), data_dir.clone())?;
    let password = Some(settings.password)
        .filter(|p| !p.is_empty())
        .map(Arc::<str>::from);
    let mcp_token = Some(settings.mcp.token)
        .filter(|value| !value.is_empty())
        .map(Arc::<str>::from);
    let mcp_auth = Arc::new(mcp_auth::McpAuth::new(
        settings.mcp.enabled,
        password.clone(),
        mcp_token,
        public_url.as_deref(),
        &mcp_redirect_uris,
        data_dir.join("mcp-oauth.json"),
    )?);
    let listener = tokio::net::TcpListener::bind(settings.bind)
        .await
        .context("监听端口失败，可能已有 Picsoc 实例在运行")?;
    let bind = listener.local_addr()?;
    let state = api::AppState {
        db: database.clone(),
        scanner: scanner.clone(),
        data_dir: data_dir.clone(),
        bind,
        auth: auth::Auth::new(password.clone()),
        designs: designs.clone(),
    };
    let url = network::local_url(bind);
    println!(
        "Picsoc {}\n监听地址：{bind}\n本机网页：{url}\n数据目录：{}",
        env!("CARGO_PKG_VERSION"),
        data_dir.display()
    );
    if bind.is_ipv4() && bind.ip().is_unspecified() {
        match network::ipv4_addresses() {
            Ok(addresses) if !addresses.is_empty() => {
                for address in addresses {
                    println!(
                        "网络 IPv4：http://{}:{}（{}）",
                        address.ip,
                        bind.port(),
                        address.interface
                    );
                }
            }
            Ok(_) => println!("网络 IPv4：未检测到可用地址，请检查网卡连接"),
            Err(error) => {
                tracing::warn!(%error, "无法读取网卡 IPv4 地址，请使用服务器实际 IP 访问")
            }
        }
    } else if bind.ip().is_unspecified() {
        println!(
            "网络 IPv6：请使用服务器实际 IPv6 地址和端口 {} 访问",
            bind.port()
        );
    } else if bind.ip().is_loopback() {
        println!(
            "其他设备访问：在 config.toml 设置 bind = \"0.0.0.0:{}\"、open_browser = false",
            bind.port()
        );
    } else {
        println!("网络访问：{url}");
    }
    println!("按 Ctrl+C 退出");
    if settings.open_browser {
        let browser_url = url.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = webbrowser::open(&browser_url) {
                tracing::warn!(error=%e,"无法自动打开浏览器，请手动访问网页地址");
            }
        });
    }
    let background_db = database.clone();
    let background_scanner = scanner.clone();
    tokio::spawn(async move {
        loop {
            let db = background_db.clone();
            if let Ok(Ok(libraries)) = tokio::task::spawn_blocking(move || db.libraries()).await {
                for library in libraries {
                    background_scanner.start(library.id);
                }
            }
            if settings.scan_interval == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_secs(settings.scan_interval)).await;
        }
    });
    if mcp_auth.enabled() {
        println!(
            "MCP：{}",
            mcp_auth.endpoint().unwrap_or_else(|| format!("{url}/mcp"))
        );
        println!(
            "MCP 授权：{}",
            if mcp_auth.oauth_enabled() {
                "OAuth（使用 Picsoc 密码授权）"
            } else {
                "独立 Bearer token"
            }
        );
    }
    let app = api::router(state).merge(mcp::router(mcp::McpState {
        auth: mcp_auth,
        tools: Arc::new(mcp_tools::PicsocTools {
            db: database.clone(),
            designs,
            public_url,
        }),
    }));
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    for library in database.libraries()? {
        scanner.cancel(library.id);
    }
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _=ctrl_c=>{},_=terminate=>{} }
}
