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
- Generate `config.toml` on first startup; stop the service, edit it, and restart to apply settings without runtime environment variables.
- Use one worker by default and an optional shared password for remote access.
- Sign in through a simple shared-password page and sign out when finished; without a password, open the library directly.
- Connect an MCP client to search indexed assets, inspect PNG contact sheets, compose PNG designs with Rust, and save editable layout revisions. Optional OAuth supports ChatGPT through an HTTPS reverse proxy; see the [MCP setup guide](docs/MCP.en.md).

The stack is Rust, Axum, Tokio, bundled SQLite, React, TypeScript, and Vite. Image processing uses the Rust `image` crate. Native runtime packages need no separate Node.js, SQLite, or libvips installation; Node.js is used to build the frontend.

## Start from source

Install Rust (minimum 1.88), Node.js (18, 20, or 22+) and npm; Rust 1.96.1 and Node.js 22 match CI. Cargo automatically installs locked frontend dependencies and builds the embedded Web UI, including from a fresh clone without `frontend/dist`. From the repository root:

```sh
cargo run --locked --release
```

The same command works in Windows PowerShell. The first build downloads npm dependencies; source, configuration and lockfile changes trigger a rebuild. Unchanged builds reuse the frontend. The resulting executable needs neither Node.js nor a separate frontend service.

On Debian, install missing tools with `sudo apt update` and `sudo apt install nodejs npm` (omit `sudo` as root), then check `node --version`. Set `open_browser = false` in the generated `config.toml` on a headless server.

Docker and release builders can set `PICSOC_FRONTEND_PREBUILT=1` after running `npm --prefix frontend ci` and `npm --prefix frontend run build`. This mode still requires a complete `frontend/dist`; it fails clearly when the Web UI is missing.

`cargo run` uses a debug build; use `--release` for everyday use and deployment. Pass service arguments after `--`:

```sh
cargo run --locked --release -- --data-dir ./picsoc-data --workers 1
```

The first startup generates `./config.toml` in the current working directory and prints its path. Running Cargo from the Picsoc repository root puts the file in the project root. Press `Ctrl+C`, edit the file, and run `cargo run --locked --release` again. For LAN access, set `bind = "0.0.0.0:3210"`, `open_browser = false`, and a nonempty `password`. Existing configuration files are retained; invalid settings stop startup with an error.

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

Install Docker Engine and its Compose plugin. In `compose.yaml`, point the image mount's `source` at an existing folder. The default container account is `10001:10001`; edit `user` if a different unprivileged UID/GID is required. Keep the data folder writable and the image folder readable and traversable by that account.

```sh
mkdir -p picsoc-data
sudo chown 10001:10001 picsoc-data
docker compose up -d --build
docker compose logs --tail 100 picsoc
docker compose stop picsoc
```

Do not change the originals' ownership. If you keep the default image mount `./library`, create that folder before starting; Compose does not create missing mount sources.

Edit the generated host file `picsoc-data/config.toml` (container path `/data/config.toml`) to set the password, workers, scan interval or MCP settings. The image's fixed startup arguments select `/data`, bind to `0.0.0.0:3210` inside the container, and disable browser opening; they override those three file fields. Host port publishing, UID/GID, mounts, resource limits and image selection belong in `compose.yaml`.

```sh
docker compose start picsoc
```

Open [http://127.0.0.1:3210](http://127.0.0.1:3210). Choose **/library** or a subfolder in the picker; the host path is not available inside the container. Originals are read-only. Configuration, metadata, thumbnails and designs persist at `/data`. Later TOML edits take effect after `docker compose restart picsoc`, or stop/edit/start. `docker compose down` removes the container while retaining host data.

For a published GHCR version, edit `image` in `compose.yaml` to its fixed tag, such as `ghcr.io/xinxiuzhu/picsoc:published-version`, then use `docker compose pull` and `docker compose up -d --no-build`. The package must be public or accessible to your GitHub credentials. A source push does not deploy a server.

## Resources and limits

Docker defaults to one CPU and 512 MiB of memory; edit `cpus` and `mem_limit` in `compose.yaml`. Edit application settings in `config.toml`:

```toml
workers = 1
scan_interval = 300
```

These settings are not a guarantee that every collection fits in 512 MiB. Files over 256 MiB, decoded image buffers over 128 MiB, or dimensions over 32,768 pixels are skipped for thumbnails, while the original remains accessible. Decoder budgets do not bound total process memory; additional workers increase peak usage. Keep one worker on lower-spec machines and inspect logs before raising concurrency.

For slow or network storage, increase `scan_interval`; `0` disables periodic scans while startup and manual scans still run. The first version uses periodic incremental scans. Manual rescanning retries previously failed thumbnails.

See [performance and the reproducible benchmark](docs/PERFORMANCE.md). Its synthetic solid-color PNG result does not predict real collection import speed.

## TOML configuration and network access

Run `cargo run --release` from the Picsoc repository root to generate `config.toml` in that project directory. Native executables also default to `./config.toml` in the launch working directory; run release packages from their extracted directory. Startup logs print the selected file. Data storage remains in the platform-specific directory listed above, including `/root/.local/share/picsoc` for Debian root. Keep the generated `data_dir` value when editing other settings:

```toml
bind = "0.0.0.0:3210"
data_dir = "/root/.local/share/picsoc"
open_browser = false
workers = 1
scan_interval = 300
password = "replace-with-your-password"

[mcp]
enabled = false
public_url = ""
token = ""
redirect_uris = []
```

Stop with `Ctrl+C`, edit, then run the same command again. Existing files are never overwritten. Syntax errors, unknown fields or invalid values stop startup instead of falling back to defaults. Unix creates the file with mode `0600`; it holds the password and any static MCP token in plaintext. Preserve restrictive access. Move previous runtime environment values into TOML manually: Picsoc no longer reads `PICSOC_*` runtime configuration. The separate frontend build flag documented above remains a build-time setting.

| CLI option | Purpose |
| --- | --- |
| `--config PATH` | Select the configuration file; generate it if absent |
| `--data-dir PATH` | Select data storage without changing the default configuration location |
| `--bind ADDRESS` | Explicitly override the listener |
| `--workers COUNT` | Explicitly override workers, range `1–4` |
| `--scan-interval SECONDS` | Explicitly override periodic scans; `0` disables them |
| `--no-open` | Explicitly disable automatic browser opening |
| `--version` / `--help` | Print version or help and exit |

Explicit CLI settings override file values. They are recorded when generating a new file but do not rewrite an existing file. Editing `data_dir` selects another storage location; it does not migrate existing data.

When no explicit `--config` is given, the working-directory file is absent, and the initial data directory contains an older `config.toml`, Picsoc reads that file and creates the working-directory configuration with its effective settings and data path. The old file is retained. An existing working-directory file is never overwritten, and explicit `--config` does not trigger this migration. See the [MCP guide](docs/MCP.en.md) for `[mcp]` settings.

For trusted LAN access, use `bind = "0.0.0.0:3210"` and a nonempty `password`. Docker additionally needs the published port changed to `"0.0.0.0:3210:3210"` in Compose. Enter the password on the login page; Basic Auth scripts use `picsoc` as the username. Use the server's LAN IP in the browser.

Basic Auth over plain HTTP has no transport encryption. Use an HTTPS reverse proxy for untrusted networks with the backend port restricted. There are no independent user accounts or per-library access controls. See [security](SECURITY.md) and the [operations guide](docs/OPERATIONS.md).

## Backup and maintenance

Stop the service and back up the entire data directory and the native working-directory `config.toml` separately. Docker keeps its configuration inside `/data`. Do not copy just the SQLite file: current changes may still be in its WAL. Back up originals separately. Restore with the same library paths and verify them before scanning. Removing a library keeps originals but removes its favorites and tags; adding it again does not restore that metadata.

For upgrades, keep the previous program and a full pre-upgrade data backup. Restore both when rolling back; database downgrade compatibility is not guaranteed. The [operations guide](docs/OPERATIONS.md) covers backup, restore, upgrade, rollback, and an Nginx HTTPS example.

## Documentation and contributing

- [User guide (Chinese)](docs/USERGUIDE.md)
- [HTTP API](docs/API.md)
- [MCP and PNG design workflow](docs/MCP.en.md)
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
