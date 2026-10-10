mod api;
mod auth;
mod db;
mod folders;
mod i18n;
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
        env = "PICSOC_BIND",
        default_value = "127.0.0.1:3210",
        help = "HTTP 监听地址"
    )]
    bind: SocketAddr,
    #[arg(
        long,
        env = "PICSOC_DATA_DIR",
        help = "数据库和缩略图目录，默认使用当前用户应用数据目录"
    )]
    data_dir: Option<PathBuf>,
    #[arg(long, help = "启动后不自动打开浏览器")]
    no_open: bool,
    #[arg(long,env="PICSOC_WORKERS",default_value_t=1,value_parser=clap::value_parser!(u8).range(1..=4),help="图片解码并发数（1–4）")]
    workers: u8,
    #[arg(
        long,
        env = "PICSOC_SCAN_INTERVAL",
        default_value_t = 300,
        help = "增量扫描间隔（秒），0 为关闭周期扫描"
    )]
    scan_interval: u64,
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
    let data_dir = args.data_dir.map_or_else(default_data_dir, Ok)?;
    std::fs::create_dir_all(&data_dir).context("无法创建数据目录")?;
    let data_dir = data_dir.canonicalize()?;
    let database = db::Db::open(&data_dir.join("picsoc.sqlite3"))?;
    let scanner = scanner::Scanner::new(database.clone(), data_dir.clone(), args.workers.into());
    let password = std::env::var("PICSOC_PASSWORD")
        .ok()
        .filter(|p| !p.is_empty())
        .map(Arc::<str>::from);
    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .context("监听端口失败，可能已有 Picsoc 实例在运行")?;
    let bind = listener.local_addr()?;
    let state = api::AppState {
        db: database.clone(),
        scanner: scanner.clone(),
        data_dir: data_dir.clone(),
        bind,
        auth: auth::Auth::new(password),
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
            "其他设备访问：添加 --bind 0.0.0.0:{} --no-open",
            bind.port()
        );
    } else {
        println!("网络访问：{url}");
    }
    println!("按 Ctrl+C 退出");
    if !args.no_open {
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
            if args.scan_interval == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_secs(args.scan_interval)).await;
        }
    });
    axum::serve(listener, api::router(state))
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
