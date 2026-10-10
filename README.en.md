# Picsoc Image Library

[简体中文](README.md) · English

Picsoc is an image library that runs on your computer or server. A Rust service serves an embedded Web interface through one port. The interface supports Simplified Chinese and English; its language choice is saved in the current browser.

Your original images stay in their existing folders. Picsoc reads and indexes them, while the database, favorites, tags, and thumbnails are stored in a separate data directory.

## Version 0.1.0

Build from source using the instructions below, or download a matching archive from [GitHub Releases](https://github.com/xinxiuzhu/Picsoc/releases) when its assets are available. Native archives and GHCR images are published by separate jobs; check [Actions](https://github.com/xinxiuzhu/Picsoc/actions) for the outcome of each.

- Add an existing folder using the service's folder picker or an absolute path.
- Recursively index JPEG, PNG, GIF, WebP, BMP, and TIFF images.
- Browse a virtualized image grid with thumbnails loaded on demand.
- Expand a sidebar folder tree, including empty directories; browse only the selected folder or include its descendants.
- Search names, relative paths, and tags; combine library, folder, orientation, aspect ratio, pixel dimensions, file size, format, favorites, tags, and filename exclusions.
- Store favorites and tags in SQLite; apply favorites and add/remove tags to a selection of images.
- Preview images, navigate with arrow keys, copy relative paths, and download originals.
- Keep GIF animation in the original preview. TIFF uses a thumbnail preview, with the original available for download.
- Scan in the background, cancel an active scan, rescan manually, or use periodic incremental scans (every 300 seconds by default).
- Use one worker by default and an optional shared password for remote access.
- Sign in through a simple shared-password page and sign out when finished; without a password, open the library directly.

The stack is Rust, Axum, Tokio, bundled SQLite, React, TypeScript, and Vite. Image processing uses the Rust `image` crate. Native runtime packages need no separate Node.js, SQLite, or libvips installation; Node.js is used to build the frontend.

## Start from source

Install Rust (minimum 1.88), Node.js (18, 20, or 22+) and npm; Rust 1.96.1 and Node.js 22 match CI. Cargo automatically installs locked frontend dependencies and builds the embedded Web UI, including from a fresh clone without `frontend/dist`. From the repository root:

```sh
cargo run --locked --release
```

The same command works in Windows PowerShell. The first build downloads npm dependencies; source, configuration and lockfile changes trigger a rebuild. Unchanged builds reuse the frontend. The resulting executable needs neither Node.js nor a separate frontend service.

On Debian, install missing tools with `sudo apt update` and `sudo apt install nodejs npm` (omit `sudo` as root), then check `node --version`. Use `-- --no-open` on a headless server.

Docker and release builders can set `PICSOC_FRONTEND_PREBUILT=1` after running `npm --prefix frontend ci` and `npm --prefix frontend run build`. This mode still requires a complete `frontend/dist`; it fails clearly when the Web UI is missing.

`cargo run` uses a debug build; use `--release` for everyday use and deployment. Pass service arguments after `--`:

```sh
cargo run --locked --release -- --data-dir ./picsoc-data --workers 1
```

For access from other devices on a trusted LAN:

```sh
PICSOC_PASSWORD='replace-with-your-password' cargo run --locked --release -- --bind 0.0.0.0:3210 --no-open
```

The browser shows a simple password login page; sessions last 24 hours and end on logout or service restart. Without a password, the library opens directly. Basic Auth scripts use the username `picsoc`. Startup logs show the actual listening address separately from the local browser URL. When a native service listens on `0.0.0.0`, it lists active IPv4 interfaces and their browser links, placing physical interfaces before virtual bridges and VPNs. Address detection does not need an external service and failures do not stop Picsoc. In Docker, detected addresses belong to the container; use the host IP and published port instead. The `scripts/build.sh` / `scripts/start.sh` helpers are also available, with `.ps1` equivalents on Windows.

The default address is [http://127.0.0.1:3210](http://127.0.0.1:3210). The native service opens your system browser automatically. Keep the service running; closing the browser does not stop it. Press `Ctrl+C` in its terminal to stop the service.

Click **Add library** and select a folder containing your images, or enter an absolute path such as `D:\Pictures`, `/Users/your-name/Pictures`, or `/home/your-name/Pictures`. The picker shows folders on the computer running the service. If you connect remotely, these are server folders, not folders on the device running your browser.

Apple Photos `.photoslibrary` packages are skipped, so you can add a `Pictures` folder containing one. To manage images from Photos, export them as JPEG, PNG, or TIFF into an ordinary folder and add that folder. The package itself and its internal folders cannot be added as libraries. See [Apple's export guide](https://support.apple.com/guide/photos/pht6e157c5f/mac).

After scanning, expand the arrow next to a library in the sidebar. Folders expand and select independently; counts include descendants. Use the breadcrumb and the **Include subfolders** toggle to change scope. Existing indexes migrate their image paths into folder records; a rescan adds previously unrecorded empty directories.

Hidden directories beginning with a dot, such as `.git` and `.cache`, are omitted from the folder tree and skipped during scanning. A successful rescan removes previously indexed images inside these directories while keeping the original files. Explicitly adding a hidden directory as a library root still indexes its ordinary contents.

Use the top sidebar button to collapse or expand navigation. Mobile navigation remains a separate drawer. **Display settings** adjusts the number of thumbnails per row; narrow screens reduce the column count to fit. Images remain fully visible, and this browser remembers the settings. Originals and cached thumbnail files are unchanged.

Open **Filters** for landscape, portrait, square, preset/custom aspect ratios, pixel width/height ranges, and file size ranges. Aspect ratios allow a ±2% relative tolerance. Assets with unknown dimensions do not match orientation, ratio, or pixel filters until dimensions are available. Combine these conditions with search, format, tags, favorites, and folder scope, and remove individual active filters.

Use **Exclude filenames** to hide unwanted assets. For example, `map` hides image names containing `map` or `MAP`. Separate terms with newlines or English/Chinese commas; this browser remembers the exclusions. Matching applies to the filename, with literal `%` and `_`, and does not match folder names or tags. Original files are retained.

Batch actions add/remove tags and change favorites without changing original files. One batch supports up to 500 selected images; it succeeds fully or makes no metadata changes.

## Native packages

The configured package targets are:

| System | Archive suffix | Start |
| --- | --- | --- |
| Windows x64 | `windows-x86_64.zip` | Double-click `picsoc.exe` |
| macOS Apple Silicon | `macos-aarch64.tar.gz` | Run `./picsoc` |
| macOS Intel | `macos-x86_64.tar.gz` | Run `./picsoc` |
| Debian x64 | `linux-x86_64.tar.gz` | Run `./picsoc` |

Linux binaries use Debian 12 as the build baseline. Compatibility on other distributions or versions needs verification on the target machine. Windows, both macOS architectures, Debian, and Docker have CI jobs; consult the results for the commit you use. macOS packages are currently unsigned and not notarized. Windows packages are executables in ZIP archives, without an installer or code signing.

Windows x64 builds configure a static C runtime and check the executable's direct DLL imports in CI and the release workflow. This is intended to avoid a separate Visual C++ runtime installation; actual build results and target-machine verification still apply.

The default data directory is:

| System | Directory |
| --- | --- |
| Windows | `%LOCALAPPDATA%\Picsoc` |
| macOS | `~/Library/Application Support/Picsoc` |
| Linux | `$XDG_DATA_HOME/picsoc`, or `~/.local/share/picsoc` |

The service prints the actual directory at startup. Override it with `--data-dir /your/fixed/data/path`. Keep this directory to reuse your library, favorites, and tags after restarting.

## Docker on Debian

Install Docker Engine and its Compose plugin. Run these commands from the repository root:

```sh
mkdir -p picsoc-data
cp docs/compose.env.example .env
id -u
id -g
```

Edit `.env` to use an existing image folder and the numeric UID/GID of the unprivileged account that can read it:

```dotenv
PICSOC_UID=1000
PICSOC_GID=1000
PICSOC_DATA_PATH=./picsoc-data
PICSOC_LIBRARY_PATH=/home/your-name/Pictures
```

The data folder must be writable and the image folder readable and traversable by that account. The default is `10001:10001`; if you keep it, set the data folder's owner on Linux with `sudo chown 10001:10001 picsoc-data`. Do not apply this to the original image folder or use `0:0`. Compose does not automatically create missing bind-mount folders.

```sh
docker compose up -d --build
```

Open [http://127.0.0.1:3210](http://127.0.0.1:3210). Choose **/library** or a subfolder in the folder picker, or enter `/library`. This is the container path; the host's `/home/...` path is not available inside the container.

Images are mounted read-only at `/library`; metadata and thumbnails persist at `/data`. The container uses an unprivileged UID/GID and a read-only root filesystem. Stop it with `docker compose down`; the host data folder is retained. Logs and restarting:

```sh
docker compose logs --tail 100 picsoc
docker compose restart
```

Once a version has been published to GHCR, select its fixed tag in `.env` (`PICSOC_IMAGE=ghcr.io/xinxiuzhu/picsoc:published-version`) and use `docker compose pull`, then `docker compose up -d --no-build`. A package may require the maintainer to make it public before unauthenticated pulls work. A source push does not deploy a server.

## Resources and limits

Docker defaults to one CPU, 512 MiB of memory, and one image worker:

```dotenv
PICSOC_WORKERS=1
PICSOC_MEMORY_LIMIT=512m
PICSOC_CPU_LIMIT=1.0
PICSOC_SCAN_INTERVAL=300
```

These settings are not a guarantee that every collection fits in 512 MiB. Files over 256 MiB, decoded image buffers over 128 MiB, or dimensions over 32,768 pixels are skipped for thumbnails, while the original remains accessible. Decoder budgets do not bound total process memory; additional workers increase peak usage. Keep one worker on lower-spec machines and inspect logs before raising concurrency.

For slow or network storage, increase the scan interval. `PICSOC_SCAN_INTERVAL=0` disables periodic scans; startup and manual scans still run. The first version uses periodic incremental scans, rather than immediate file-system monitoring. Manual rescanning retries previously failed thumbnails.

See [performance and the reproducible benchmark](docs/PERFORMANCE.md). Its recorded result uses synthetic solid-color PNGs; it does not predict import speed for real photo collections or other machines.

## Configuration and network access

| CLI option | Environment variable | Default |
| --- | --- | --- |
| `--bind` | `PICSOC_BIND` | `127.0.0.1:3210` |
| `--data-dir` | `PICSOC_DATA_DIR` | User data directory above |
| `--workers` | `PICSOC_WORKERS` | `1`, allowed range `1–4` |
| `--scan-interval` | `PICSOC_SCAN_INTERVAL` | `300` seconds; `0` disables periodic scans |
| `--no-open` | — | Disable automatic browser opening |
| `--version` | — | Print version and exit |

For trusted LAN access, set the native bind address to `0.0.0.0:3210`, or set Docker's `PICSOC_HOST_BIND=0.0.0.0`. Configure a nonempty `PICSOC_PASSWORD` and enter it on the login page; Basic Auth scripts use `picsoc` as the username. Use the server's LAN address in the browser.

Basic Auth over plain HTTP has no transport encryption. Use an HTTPS reverse proxy for access over untrusted networks, with the backend port restricted. There are no independent user accounts or per-library access controls. See [security](SECURITY.md) and the [operations guide](docs/OPERATIONS.md) before sharing access.

## Backup and maintenance

Stop the service and back up the entire data directory, not just the SQLite file: current changes may still be in its WAL. Back up originals separately. Restore with the same library paths and verify them before scanning. Removing a library keeps originals but removes its favorites and tags; adding it again does not restore that metadata.

For upgrades, keep the previous program and a full pre-upgrade data backup. Restore both when rolling back; database downgrade compatibility is not guaranteed. The [operations guide](docs/OPERATIONS.md) covers backup, restore, upgrade, rollback, and an Nginx HTTPS example.

## Documentation and contributing

- [User guide (Chinese)](docs/USERGUIDE.md)
- [HTTP API](docs/API.md)
- [Architecture (Chinese)](docs/ARCHITECTURE.md)
- [Localization](docs/I18N.md)
- [Performance](docs/PERFORMANCE.md)
- [Build and release process](docs/RELEASING.md)
- [Operations](docs/OPERATIONS.md)
- [Contribution guide](CONTRIBUTING.md)
- [Security policy](SECURITY.md)
- [Changelog](CHANGELOG.md)

Cargo builds the frontend before embedding it in Rust. `npm run build` includes the locale-key checker and TypeScript checks. Rust validation uses `cargo fmt --all --check`, `cargo clippy --locked --all-targets -- -D warnings`, and `cargo test --locked`. The API smoke test starts an isolated service: `python3 tests/api_smoke.py --binary target/debug/picsoc` (Windows: `python` and `picsoc.exe`).

Dependency versions are locked in `Cargo.lock` and `frontend/package-lock.json`.

## License

[Apache License 2.0](LICENSE).
