# Picsoc Image Library

Picsoc runs a local Rust service and opens a Chinese/English image library in your system browser. Source and full documentation: https://github.com/xinxiuzhu/Picsoc

## Start

Windows: double-click `picsoc.exe`, or run it in PowerShell:

```powershell
.\picsoc.exe --data-dir "$env:LOCALAPPDATA\Picsoc"
```

macOS / Debian: open a terminal in the extracted directory:

```sh
./picsoc
```

The first startup generates `./config.toml` in the launch working directory and prints its path. Run the executable from its extracted directory. `--data-dir` changes data storage, not the default configuration location. Press `Ctrl+C`, edit it, and start again. Existing files are retained; invalid configuration prevents startup. Without explicit `--config`, an absent working-directory file can be generated from an older configuration in the initial data directory, preserving effective settings, the data path and the old file. Existing working-directory files are never overwritten; explicit `--config` does not trigger migration. Open http://127.0.0.1:3210 if the browser does not open automatically. Keep the service running; `Ctrl+C` stops it. Closing the browser leaves the service available. Choose a language in the page header, add a folder on the machine running Picsoc, and wait for background scanning.

Linux uses Debian 12 as its build baseline. Other systems need target-machine verification. Intel and Apple Silicon Macs use separate packages. macOS packages are currently unsigned and not notarized.

Windows x64 builds configure a static C runtime, with direct DLL imports checked during release. Target-machine verification is still required.

## Files and data

JPEG, PNG, GIF, WebP, BMP, and TIFF are supported. Originals stay in place and are not moved or modified. GIF originals retain animation; TIFF uses a thumbnail preview.

Indexes, favorites, tags, and thumbnails persist in the data directory. Default paths are `%LOCALAPPDATA%\Picsoc` on Windows, `~/Library/Application Support/Picsoc` on macOS, and `$XDG_DATA_HOME/picsoc` or `~/.local/share/picsoc` on Linux. The service prints the actual path; `--data-dir PATH` overrides it.

Stop the service before backing up the entire data directory and the separate working-directory `config.toml`. Back up originals separately. Keep library paths unchanged when restoring. Removing a library deletes its metadata and cached thumbnails, while leaving originals intact.

## Options and remote access

- `--config PATH`: select a TOML file; generate it if absent.
- `--bind 127.0.0.1:3210`: listen locally by default.
- `--data-dir PATH`: choose a persistent data folder.
- `--workers 1`: one image worker by default; range 1–4.
- `--scan-interval 300`: scan periodically; `0` disables periodic scans.
- `--no-open`: do not open a browser automatically.
- `--version` / `--help`: show version or CLI help (CLI text is currently Chinese).

Keep one worker for lower-spec computers. Thumbnail processing skips files over 256 MiB, decoded buffers over 128 MiB, or dimensions over 32,768 pixels. These decoder limits are not total process memory limits.

Edit settings in `config.toml`; explicit CLI options override them without rewriting an existing file. Runtime environment variables are no longer read. For trusted LAN access, set `bind = "0.0.0.0:3210"`, `open_browser = false` and a nonempty `password`, then restart. The configuration holds credentials in plaintext; new Unix files use mode `0600`. Protect the file and its backups. Enter that password on the browser login page; sign out when finished. Without a password, the library opens directly. Basic Auth scripts use the username `picsoc`. Use HTTPS for untrusted networks. Authentication is a shared password, with no individual accounts or library permissions. See the repository's security and operations guides.

License: Apache License 2.0, in the included `LICENSE`.
