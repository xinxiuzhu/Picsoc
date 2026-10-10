# MCP and PNG design workflow

[简体中文](MCP.md) · English

Picsoc can expose its indexed image library and Rust PNG renderer to an MCP client. A client can search and view candidate materials, save a structured scene, request a preview or final PNG, and later reopen the saved scene to continue editing. The existing Web interface and MCP endpoint share the same Rust process and port.

MCP is disabled by default. Deploying this code does not install a connection in a ChatGPT account or restart an existing Debian service.

## Connect through HTTPS and OAuth

The LAN Web address in this example is `http://192.168.2.101:3210/`. Cloud ChatGPT cannot reach that private address directly. Use the HTTPS reverse proxy at `https://orionai.iepose.cn`, or your own HTTPS domain. Configure the proxy to forward `/mcp`, `/.well-known/` and `/oauth/`, including the Authorization header, to the same Picsoc process. Do not strip those path prefixes. The [operations guide](OPERATIONS.md) includes an Nginx example.

Stop the old process, update the source, and run the service:

```sh
git pull --ff-only
cargo run --locked --release
```

Run Cargo from the Picsoc repository root. On first startup, Picsoc generates `./config.toml` in that project directory and prints the path. Press `Ctrl+C`, edit that file, keeping its generated `data_dir`, and change these fields:

```toml
bind = "0.0.0.0:3210"
open_browser = false
password = "replace-with-your-current-password"

[mcp]
enabled = true
public_url = "https://orionai.iepose.cn"
token = ""
redirect_uris = []
```

Edit the existing fields rather than appending duplicate keys/tables. Run `cargo run --locked --release` again. Existing configuration is retained, and invalid configuration prevents startup. Runtime environment variables are no longer read; manually copy previous settings into TOML.

Keep the original data directory. `--data-dir` selects data storage without relocating the default `./config.toml` in the launch working directory. Linux data defaults to `$XDG_DATA_HOME/picsoc` or `~/.local/share/picsoc`, including `/root/.local/share/picsoc` for root. Keep the data directory to reuse your indexed library, layouts, outputs and OAuth credentials. `--config PATH` selects a different configuration file. Without this explicit flag, an absent working-directory configuration can be generated from an older file in the initial data directory, retaining effective settings and the data path; the old file remains. Existing working-directory files are never overwritten, and explicit `--config` does not trigger migration. Explicit CLI arguments override file values without rewriting an existing file.

`mcp.public_url` must be an HTTPS origin without a path, query, fragment or credentials. HTTP is allowed only for loopback development, not a LAN OAuth issuer.

Docker generates host file `picsoc-data/config.toml` at `/data/config.toml`. On the first run, use `docker compose up -d --build`, then `docker compose stop picsoc`, edit `password` and `[mcp]`, and `docker compose start picsoc`. Upgrade an existing container with `docker compose up -d --build`; later file edits take effect after `docker compose restart picsoc`. Retain the `/data` and read-only `/library` mounts. Set the published port address in `compose.yaml` so Nginx can reach it; the domain terminates HTTPS at the proxy.

Check discovery after restarting:

```sh
curl -i https://orionai.iepose.cn/.well-known/oauth-protected-resource/mcp
curl -i https://orionai.iepose.cn/.well-known/oauth-authorization-server
curl -i https://orionai.iepose.cn/mcp
```

The discovery requests should return JSON. The unauthenticated `/mcp` request should return `401` with a `WWW-Authenticate` header pointing to resource metadata. An HTML Picsoc page at these paths usually means the old executable is still running or a proxy rewrote the path. `404` from `/mcp` can mean MCP is disabled. Successful Web login alone does not confirm MCP authorization.

In ChatGPT, use **Plugins → + → Add custom MCP server**, enter `https://orionai.iepose.cn/mcp`, and select OAuth. Install/test the connection, follow the Picsoc authorization page, and enter your Picsoc password there. The client discovers registration and token endpoints automatically; do not enter the shared password as a client secret. Availability and creation permissions depend on the account/workspace. Follow the current [OpenAI connection instructions](https://developers.openai.com/plugins/deploy/connect-chatgpt) if the entry point differs.

The authorization server supports public clients and `client_secret_post`/`client_secret_basic`, S256 PKCE, and the exact resource `https://orionai.iepose.cn/mcp`. It advertises issuer identification; the default allowed ChatGPT redirect is `https://chatgpt.com/connector_platform_oauth_redirect`. If the management page shows another production callback, copy its **complete** URL into the TOML string array `mcp.redirect_uris`, then restart. For example: `redirect_uris = ["https://chatgpt.com/connector/oauth/your-exact-callback-id"]`. Wildcards and arbitrary redirect hosts are not accepted. See [OpenAI's authentication requirements](https://developers.openai.com/plugins/build/auth).

`picsoc:read` permits discovery, search, asset previews and reading existing designs/results. `picsoc:write`, together with read, permits saving layouts and submitting renders. Access tokens last one hour; refresh families last at most seven days and rotate on every use. Replaying a consumed refresh token revokes that client's entire token family. Client registrations and token hashes persist in `mcp-oauth.json`; unexpired credentials survive service restarts. In-progress browser authorization and one-use codes do not survive a restart. No plaintext access/refresh token, client secret or shared password is saved in that file. Back up the whole data directory and protect it as account data.

## Search, view, compose and edit

The available tools are:

| Tool | Purpose |
| --- | --- |
| `list_libraries` | Discover imported libraries and material counts |
| `get_library_folders` | Browse indexed folders, 200 children per page |
| `search_assets` | Search filenames, relative paths and tags; combine folder/format/ratio/dimension/size/exclusion filters |
| `preview_assets` | View an actual PNG contact sheet with asset-ID labels for 1–24 distinct candidates |
| `get_fonts` | Find loaded server fonts and sample Chinese-glyph support |
| `list_designs` / `get_design` | Find and reopen saved layouts |
| `save_design` | Save a new scene or an immutable revision |
| `render_design` | Submit an asynchronous preview/final PNG job |
| `get_render` | Check status and view the actual resulting preview, layout and download paths |

Search returns 24 candidates by default, at most 50 per request. It is filename/path/tag search, not visual semantic search. The image preview tools return standard MCP image content with base64 PNG pixels, so the model can inspect the material rather than infer its appearance from a filename. Transparent candidates use a checkerboard; missing or unreadable assets appear in `missing` metadata. GIFs use the first frame for composition.

A useful first request is:

> In my game UI library, find blue borders, buttons and icons. Show the candidates, choose suitable asset IDs, and compose a 1920×1080 login screen with the title “星海”. Save the layout and render a small preview first.

The client should search, inspect contact sheets, check fonts, then call `save_design` with a scene before calling `render_design`. `render_design` returns a `job_id` immediately. Poll `get_render` with a short pause until its status is `succeeded` or `failed`; successful results include actual preview pixels. Request `quality: "final"` after inspecting the preview.

Scenes contain a named canvas and bottom-to-top image, rectangle and text layers. Images reference **indexed asset IDs**, never file paths. The renderer supports contain/cover/stretch sizing, alpha opacity, rounded rectangles, explicit text line breaks, width wrapping, alignment and line height. It does not execute generated Rust or shell commands. Detailed JSON examples and constraints are in the [API documentation](API.md).

To continue editing, call `get_design`, modify its `scene`, and call `save_design` with the returned `design_id` and current `revision` as `expected_revision`. Saving creates the next revision and preserves the previous one. A stale or missing expected revision causes a conflict; read the latest layout before retrying. A job's exported `layout.json` contains the full stored design, so use its `scene` when submitting an edit.

The Web **Designs** page lists saved work and lets you edit scene JSON, save revisions, render previews/finals, and download PNGs or scene JSON. Its editor JSON download contains the current scene, including unsaved edits; the job's layout download contains the saved design used for that render. The authenticated Web APIs serve PNG downloads; MCP OAuth/Bearer credentials apply only to `/mcp`. Original materials remain read-only.

## Fonts, resources and stored results

Use `get_fonts` before adding text. Native Debian can install `fonts-wqy-zenhei` and `fonts-dejavu-core`; the Docker runtime includes them. Alternatively, place licensed raw `.ttf`, `.otf` or `.ttc` files in the existing data directory's `fonts` folder and restart. Font selection uses an ID returned by Picsoc, not a tool-provided path. The loader checks the first eight sorted user-directory entries and keeps total loaded font data within 64 MiB. `supports_chinese` checks sample glyphs; saving text verifies every required character. Missing glyphs cause an error instead of silently drawing replacement boxes.

- Canvas edges: 1–4096 pixels, up to 16,777,216 pixels and 64 layers.
- Text: up to 1024 characters per layer and 4096 per layout; basic glyph layout, not a complex-script shaping engine.
- Preview canvas: longest edge at most 1280; final PNG uses the requested canvas size.
- Rendering: one separate worker with four waiting slots; a full queue returns a busy error. Asset contact sheets share the render lock and can also be busy.
- Composition source limits: 256 MiB file size, 64 MiB decoded/RGBA buffers and 32,768-pixel edges. These limits do not guarantee fixed total process memory.

Layouts live under `generated/designs/<design_id>/r<revision>.json`. Job states and `output.png`, `preview.png`, `layout.json` live under `generated/jobs/<job_id>/`. Completed files survive restarts; interrupted queued/running jobs become failed and must be resubmitted. There is no automatic output retention/cleanup policy yet. Keep generated outputs and font files in your data backup; back up originals separately.

Scenes store source-version fingerprints, not image copies. If a source changes, disappears or is renamed, an old scene may fail to render again. Rescan, inspect current IDs and save a new layout revision. Previously completed PNGs remain available.

## Other local MCP clients

A client that supports a custom Authorization header can use an independent static token. Edit the generated configuration, then restart with `cargo run --locked --release`:

```toml
[mcp]
enabled = true
public_url = ""
token = "replace-with-a-random-token-of-at-least-32-ASCII-characters"
redirect_uris = []
```

Connect that client to `http://127.0.0.1:3210/mcp` with `Authorization: Bearer <token>`. This credential has read and write access. It is separate from Web Cookie/Basic Auth and is not an OAuth client secret. Token-only mode rejects requests carrying an Origin header; this route is intended for server/CLI clients, not cross-origin browser calls.

The current transport uses stateless JSON-response Streamable HTTP for protocol versions `2025-03-26`, `2025-06-18` and `2025-11-25`, with initialize negotiation. It does not implement the changed `2026-07-28` lifecycle. Authenticated GET/DELETE `/mcp` return `405`; accepted notifications return empty `202`. POST clients must send `Content-Type: application/json` and `Accept: application/json, text/event-stream`. Maximum MCP body size is 1 MiB. Refer to the [MCP transport specification](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports) and [authorization specification](https://modelcontextprotocol.io/specification/2025-06-18/basic/authorization) for client integration.
