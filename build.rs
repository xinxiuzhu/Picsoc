use std::{
    collections::hash_map::DefaultHasher,
    env, fs,
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

const INPUTS: &[&str] = &[
    "src",
    "scripts",
    "public",
    "package.json",
    "package-lock.json",
    "index.html",
    "tsconfig.json",
    "vite.config.ts",
];

fn main() -> ExitCode {
    println!("cargo:rerun-if-env-changed=PICSOC_FRONTEND_PREBUILT");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=frontend/dist");
    for input in INPUTS {
        if *input != "public" || Path::new("frontend/public").exists() {
            println!("cargo:rerun-if-changed=frontend/{input}");
        }
    }
    if let Err(error) = build_frontend() {
        eprintln!("\nPicsoc 网页构建失败 / Frontend build failed:\n{error}\n");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn build_frontend() -> Result<(), String> {
    let root =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("Missing manifest directory")?);
    let frontend = root.join("frontend");
    let dist = frontend.join("dist");
    if env::var_os("PICSOC_FRONTEND_PREBUILT").as_deref() == Some("1".as_ref()) {
        return validate_dist(&dist);
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").ok_or("Missing build output directory")?);
    let fingerprint = fingerprint(&frontend, INPUTS).map_err(|error| error.to_string())?;
    let stamp = out.join("frontend-inputs");
    if validate_dist(&dist).is_ok()
        && fs::read_to_string(&stamp).ok().as_ref() == Some(&fingerprint)
    {
        return Ok(());
    }

    println!("正在构建 Picsoc 网页 / Building Picsoc frontend (first build or changed inputs)");
    let dependencies = fingerprint_dependencies(&frontend).map_err(|error| error.to_string())?;
    let dependency_stamp = out.join("frontend-dependencies");
    if !frontend.join("node_modules/typescript/bin/tsc").is_file()
        || !frontend.join("node_modules/vite/bin/vite.js").is_file()
        || fs::read_to_string(&dependency_stamp).ok().as_ref() != Some(&dependencies)
    {
        npm(&frontend, &["ci"])?;
        fs::write(&dependency_stamp, dependencies).map_err(|error| error.to_string())?;
    }
    npm(&frontend, &["run", "build"])?;
    validate_dist(&dist)?;
    fs::write(stamp, fingerprint).map_err(|error| error.to_string())?;
    Ok(())
}

fn npm(frontend: &Path, args: &[&str]) -> Result<(), String> {
    // Choose the host command, including when Rust cross-compiles for another OS.
    let mut command = if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/D", "/C", "npm.cmd"]);
        command
    } else {
        Command::new("npm")
    };
    let status = command
        .current_dir(frontend)
        .args(args)
        .status()
        .map_err(|error| {
            format!(
                "无法运行 npm / Cannot run npm: {error}\n\
             从源码构建需要 Node.js 和 npm（推荐 Node.js 22）。Debian 可安装 nodejs、npm。\n\
             Building from source needs Node.js and npm (Node.js 22 recommended).\n\
             也可以手动运行 / Or build manually:\n\
             npm --prefix frontend ci\n\
             npm --prefix frontend run build\n\
             PICSOC_FRONTEND_PREBUILT=1 cargo run --locked --release"
            )
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "npm {} 失败 / failed ({status}). 请查看上面的 npm 日志；推荐 Node.js 22。\n\
             Check the npm output above; Node.js 22 is recommended.",
            args.join(" ")
        ))
    }
}

fn validate_dist(dist: &Path) -> Result<(), String> {
    let html = fs::read_to_string(dist.join("index.html")).unwrap_or_default();
    let references: Vec<_> = ["src=\"", "href=\""]
        .iter()
        .flat_map(|prefix| html.split(prefix).skip(1))
        .filter_map(|value| value.split('"').next())
        .filter_map(|value| value.strip_prefix("/assets/"))
        .collect();
    if !references.is_empty()
        && references
            .iter()
            .all(|asset| dist.join("assets").join(asset).is_file())
    {
        Ok(())
    } else {
        Err("缺少完整的 frontend/dist。请先运行 npm --prefix frontend ci 和 npm --prefix frontend run build。\n\
             Missing built frontend/dist. Run npm --prefix frontend ci and npm --prefix frontend run build first.".into())
    }
}

fn fingerprint_dependencies(frontend: &Path) -> io::Result<String> {
    fingerprint(frontend, &["package.json", "package-lock.json"])
}

fn fingerprint(frontend: &Path, inputs: &[&str]) -> io::Result<String> {
    let mut hash = DefaultHasher::new();
    for input in inputs {
        hash_path(frontend, &frontend.join(input), &mut hash)?;
    }
    Ok(format!("{:016x}", hash.finish()))
}

fn hash_path(root: &Path, path: &Path, hash: &mut DefaultHasher) -> io::Result<()> {
    path.strip_prefix(root).unwrap_or(path).hash(hash);
    if path.is_dir() {
        let mut entries = fs::read_dir(path)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<io::Result<Vec<_>>>()?;
        entries.sort();
        for entry in entries {
            hash_path(root, &entry, hash)?;
        }
    } else if path.is_file() {
        fs::read(path)?.hash(hash);
    } else {
        "missing".hash(hash);
    }
    Ok(())
}
