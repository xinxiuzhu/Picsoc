# Picsoc 图片素材库

简体中文 · [English](README.en.md)

Picsoc 是一个在自己电脑或服务器上运行的图片素材库。启动 Rust 服务后，在浏览器打开界面；前端静态文件随程序提供，使用时只需要一个端口。第一版为 0.1.0。

原图保留在现有目录，Picsoc 只读取图片并建立索引。数据库、标签和缩略图存放在独立的数据目录，重启后继续使用。

## 第一版

- 添加本机目录，递归索引 JPEG、PNG、GIF、WebP、BMP、TIFF 图片。
- 图片网格按需加载缩略图，浏览、搜索、筛选和查看图片详情。
- 收起侧栏扩大浏览空间；调整每行缩略图显示数量，当前浏览器记住设置。
- 收藏与标签保存在 SQLite 数据库中；支持多选后批量收藏、添加或移除标签。
- 左侧展开素材库目录树，保留真实文件夹层级和空目录；选择目录后可查看当前层或包含子文件夹的素材。
- 按方向、宽高比、像素宽高、文件大小、格式、标签和收藏组合筛选；支持自定义比例、排除噪音文件名与多种排序。
- 支持中文和英文界面，可在页面中切换语言。
- 首次启动自动生成 `config.toml`，停止后编辑配置，再启动即可生效；运行配置无需环境变量。
- 可选共享密码登录页，支持退出登录；未设置密码直接进入。
- 可选 Rust MCP 服务：让 ChatGPT 搜索、查看素材拼版，保存可继续修改的布局并合成 PNG；网页“设计作品”可编辑布局与下载成品。配置见 [MCP 与 PNG 设计](docs/MCP.md)。
- 后台扫描生成缩略图；支持取消扫描、手动重扫以及默认每 300 秒的增量扫描。
- 配置 Windows 和 macOS 原生程序、Debian 原生程序与 Docker 的构建及发布流程。

技术栈：Rust、Axum、Tokio、SQLite、React、TypeScript、Vite。SQLite 随程序构建，图片处理使用 Rust `image`，发行程序运行时不需要另外安装 Node.js、SQLite 或 libvips。从源码构建需要 Node.js 和 npm，用于编译前端。

## Windows / macOS / Debian 快速开始

可按下文“从源码开发”构建运行，也可以从 [GitHub Releases](https://github.com/xinxiuzhu/Picsoc/releases) 下载对应系统及 CPU 的版本附件；先确认附件已生成。容器镜像状态独立于附件，以发布 workflow 的 GHCR job 为准。发行 workflow 的构建目标如下：

| 系统 | 文件名后缀 | 启动 |
| --- | --- | --- |
| Windows x64 | `windows-x86_64.zip` | 双击 `picsoc.exe` |
| macOS Apple Silicon | `macos-aarch64.tar.gz` | 终端运行 `./picsoc` |
| macOS Intel | `macos-x86_64.tar.gz` | 终端运行 `./picsoc` |
| Debian x64 | `linux-x86_64.tar.gz` | 终端运行 `./picsoc` |

首次启动会在数据目录生成 `config.toml` 并打印路径；已有文件会保留。需要修改配置时，按 `Ctrl+C` 停止，编辑该文件，再运行同一条启动命令。启动后默认打开 [http://127.0.0.1:3210](http://127.0.0.1:3210)。点击“添加素材库”，通过文件夹选择器浏览并选择已有素材文件夹，也可以手动填写绝对路径，例如 Windows 的 `D:\素材`，macOS 的 `/Users/你的用户名/Pictures`，Debian 的 `/home/你的用户名/Pictures`。

文件夹选择器浏览的是运行 Picsoc 服务的机器。原生运行时可选择本机文件夹；通过局域网访问时，选择服务机器上的文件夹。确认添加后，后台扫描读取原图并创建索引和缩略图。

Apple Photos 的 `.photoslibrary` 图库包会自动跳过，可以正常添加包含它的 `Pictures` 文件夹。要管理「照片」App 中的图片，请先将其导出为 JPEG、PNG 或 TIFF 到普通文件夹，再添加该文件夹；图库包及其内部目录不作为素材库导入。参见 [Apple 导出说明](https://support.apple.com/zh-cn/guide/photos/pht6e157c5f/mac)。

扫描后点击左侧素材库前的箭头展开目录树；目录可独立展开或选中，数字表示包含后代的素材数量。选择目录后用上方路径导航返回父目录，并通过“包含子文件夹”切换递归范围。升级已有素材库后，原素材路径会自动迁移为目录索引；重新扫描会补齐之前未记录的空目录。

素材库内部的 `.git`、`.cache` 等以点开头的隐藏目录不显示在目录树中，扫描会跳过其内容。旧版本已索引的隐藏目录图片会在成功重扫后清理索引，原文件保留。手动填写隐藏目录的绝对路径并将其作为独立素材库时，仍可索引该根目录内的普通内容。

顶部侧栏按钮可以收起或展开左侧导航；手机使用独立的抽屉导航。工具栏“显示设置”可调整每行缩略图数量，图片随列数自动缩放，保持完整显示。窄屏根据可用宽度减少列数。这些设置只影响浏览器显示，不修改原图或重新生成缩略图缓存。

点击“筛选”设置横图、竖图、方图，常用或自定义宽高比，以及像素宽高和文件大小范围。比例允许 ±2% 的相对误差；尺寸尚未识别的素材不会匹配方向、比例或像素范围，缩略图处理完成后会自动出现。条件可与目录、关键词、格式、标签及收藏组合，结果列表中的筛选项可以逐个移除。

“排除文件名”用于隐藏噪音素材：例如输入 `map`，文件名包含 `map` 或 `MAP` 的图片就不显示。多个关键词用换行或中英文逗号分隔，当前浏览器会记住这些排除词。只有图片文件名参与匹配，目录名称和标签不受影响；`%`、`_` 等按字面处理，原文件不会被删除。

服务需要保持运行。关闭浏览器后服务仍然可以运行，重新打开上述地址即可；关闭运行程序的终端或按 `Ctrl+C` 可以停止服务。

默认使用当前用户的应用数据目录，启动时会打印实际路径：

| 系统 | 默认数据目录 |
| --- | --- |
| Windows | `%LOCALAPPDATA%\Picsoc` |
| macOS | `~/Library/Application Support/Picsoc` |
| Debian / Linux | `$XDG_DATA_HOME/picsoc`，未设置时为 `~/.local/share/picsoc` |

可以指定其他固定的数据目录：

```sh
./picsoc --data-dir /home/你的用户名/.local/share/picsoc
```

Windows PowerShell 示例：

```powershell
.\picsoc.exe --data-dir "$env:LOCALAPPDATA\Picsoc"
```

Linux 发行包以 Debian 12 为构建基线；其他 Debian 版本与 Linux 发行版需要在目标机器验证兼容性。Windows、Mac Intel 和 Mac Apple Silicon 分别配置 CI 构建测试，跨平台检查结果以当前 commit 的 [Actions](https://github.com/xinxiuzhu/Picsoc/actions) 为准。macOS 发行包当前不包含 Apple 签名和公证；如系统拦截，请通过系统设置的“隐私与安全性”确认允许可信来源的程序运行。

Windows x64 构建配置静态 C 运行时，并在 CI 与发行流程检查最终程序的直接 DLL 依赖，以减少额外安装 Visual C++ 运行时的需求。是否通过仍以实际构建检查和目标电脑验证为准。

## Debian Docker 部署

需要 Docker Engine 与 Docker Compose 插件。下面的命令在仓库根目录执行。

1. 创建数据目录，编辑 `compose.yaml` 的素材挂载 `source`，指向已有素材目录。默认运行用户为 `10001:10001`；数据目录须可写，素材目录须可读且可遍历。如需其他普通用户编号，在 `compose.yaml` 修改 `user`。使用 root 管理 Docker 时仍保留普通容器用户。

   ```sh
   mkdir -p picsoc-data
   sudo chown 10001:10001 picsoc-data
   ```

   只修改数据目录的所有者。默认素材挂载是 `./library`；使用默认路径时须先创建它，使用已有路径时在 Compose 中填写对应路径。Compose 不自动创建不存在的挂载目录。

2. 构建并启动，查看生成的配置路径，然后停止服务编辑：

   ```sh
   docker compose up -d --build
   docker compose logs --tail 100 picsoc
   docker compose stop picsoc
   ```

   编辑宿主机的 `picsoc-data/config.toml`；容器内对应 `/data/config.toml`。例如设置 `password`、`workers`、`scan_interval` 与 `[mcp]`。镜像启动参数固定数据目录 `/data`、监听 `0.0.0.0:3210` 和不打开浏览器，这三项优先于配置文件。端口发布地址、运行用户、挂载路径、镜像及资源上限在 `compose.yaml` 修改。

3. 启动后打开 [http://127.0.0.1:3210](http://127.0.0.1:3210)，添加素材库时选择 **`/library`** 或其子目录：

   ```sh
   docker compose start picsoc
   ```

   容器使用 `/library`，不能直接使用宿主机 `/home/...` 路径。原图只读挂载，配置、数据库、缩略图及作品通过 `/data` 持久化。之后修改 TOML，可停止、编辑再启动，也可编辑后执行 `docker compose restart picsoc`。

停止并移除容器使用 `docker compose down`，宿主机数据保留。第一次添加目录后，图片会随后台任务逐步出现，导入大量素材时可以继续浏览已完成的部分。

确认固定版本镜像已上传 GHCR 后，可以直接修改 `compose.yaml` 的 `image`，例如 `ghcr.io/xinxiuzhu/picsoc:0.1.0`，然后无需在部署机编译：

```sh
docker compose pull
docker compose up -d --no-build
```

镜像未公开时，需要维护者将 GHCR 包设为公开，或使用已授权的 GitHub 凭据拉取。提交代码本身不会部署服务。

## 低配置机器

Docker 默认限制为 1 个 CPU、512 MiB 内存，在 `compose.yaml` 修改 `cpus` 与 `mem_limit`。应用的图片处理并发和扫描间隔在 `config.toml` 修改：

```toml
workers = 1
scan_interval = 300
```

这些是资源配置，不是所有素材都能在 512 MiB 内存下处理的保证。单张图片超过 256 MiB 文件大小、128 MiB 解码数据或 32,768 像素宽/高时，程序跳过缩略图并显示预览失败。解码限制不等于进程总内存限制，多个任务并行仍可能超过容器硬内存上限并使其退出。遇到导入期间反复重启，先保持 `workers = 1`，查看日志，再提高内存上限。原生运行的并发设置不限制进程总内存。

机械硬盘或网络盘可以设置 `scan_interval = 1800`；`0` 关闭定时扫描，启动和手动扫描仍会运行。第一版使用周期增量扫描，并非即时文件监控。

## TOML 配置与局域网访问

首次启动后，默认配置位于上述数据目录的 `config.toml`。例如 Debian 的 root 用户是 `/root/.local/share/picsoc/config.toml`。按 `Ctrl+C` 停止，编辑配置，再启动；程序不会覆盖已有文件。配置语法或字段值错误时会直接报错，不会回落到默认设置启动。Unix 新生成文件权限为 `0600`，其中密码和静态 MCP token 为明文，请保留访问权限。

Debian 服务器配置示例，`data_dir` 保持生成文件中的实际目录：

```toml
bind = "0.0.0.0:3210"
data_dir = "/root/.local/share/picsoc"
open_browser = false
workers = 1
scan_interval = 300
password = "换成你自己的密码"

[mcp]
enabled = false
public_url = ""
token = ""
redirect_uris = []
```

`[mcp]` 配置方法见 [ChatGPT MCP 说明](docs/MCP.md)。已有部署请将原环境变量中的密码、公开地址和 MCP 设置手动移入 TOML；应用不再读取 `PICSOC_*` 运行环境变量。

| 参数 | 用途 |
| --- | --- |
| `--config 路径` | 指定配置文件；不存在时生成 |
| `--data-dir 路径` | 指定数据目录；未提供 `--config` 时也在该目录定位配置 |
| `--bind 地址` | 显式覆盖监听地址 |
| `--workers 数量` | 显式覆盖处理并发，允许 `1`–`4` |
| `--scan-interval 秒` | 显式覆盖扫描间隔，`0` 关闭周期扫描 |
| `--no-open` | 显式禁止自动打开浏览器 |
| `--version` / `--help` | 显示版本或帮助后退出 |

日常只需编辑 TOML。显式 CLI 参数优先于文件；首次生成会记录这些参数，后续运行不会把临时覆盖写回已有文件。修改配置文件中的 `data_dir` 会改变数据存放位置，不会迁移既有数据库或原图。

默认只允许本机访问。局域网使用 `bind = "0.0.0.0:3210"` 并设置非空 `password`；Docker 还需在 `compose.yaml` 将发布端口改为 `"0.0.0.0:3210:3210"`。浏览器访问服务机器的 IPv4 地址，例如 `http://192.168.2.101:3210`，登录页输入配置的密码；脚本 Basic Auth 用户名固定为 `picsoc`。

启动日志分别显示实际监听地址和本机网页地址。原生服务监听 `0.0.0.0` 时会列出活跃网卡 IPv4 访问链接，优先普通网卡，再显示虚拟网桥或 VPN；读取地址失败不会中断服务。容器内检测到的是容器网卡地址，Docker 仍使用宿主机 IP 与已发布端口。

浏览器通过登录页建立会话，脚本支持 HTTP Basic Auth。HTTP 没有传输加密；公网或不可信网络使用 HTTPS 反向代理。不要直接将无密码的素材服务开放到公网。

## 从源码开发

需要 Rust（最低 1.88）、Node.js（18、20 或 22+）和 npm；推荐与 CI 一样使用 Rust 1.96.1、Node.js 22。新克隆的仓库不包含 `frontend/dist`，Cargo 会自动安装锁定的前端依赖、构建网页，再把网页嵌入 Rust 程序。在仓库根目录执行：

```sh
cargo run --locked --release
```

Windows PowerShell 可以使用同样的命令。首次构建会访问 npm 仓库；网页源码、配置或 lockfile 变化后自动重建，未变化时复用构建结果。最终程序不需要 Node.js 或单独的前端服务。

Debian 尚未安装 Node.js/npm 时，可以使用 `sudo apt update` 和 `sudo apt install nodejs npm`（root 用户省略 `sudo`），再用 `node --version` 确认版本满足上述要求。无桌面环境的服务器在生成的 `config.toml` 中设置 `open_browser = false`；局域网配置见上一节。

Docker 和发行构建可以显式使用已构建网页：先运行 `npm --prefix frontend ci` 与 `npm --prefix frontend run build`，再设置构建环境变量 `PICSOC_FRONTEND_PREBUILT=1`。此模式仍检查 `frontend/dist`，缺失时明确报错，不会生成没有界面的程序。

`cargo run` 使用调试构建，日常使用和部署建议加上 `--release`。默认打开 [http://127.0.0.1:3210](http://127.0.0.1:3210)，按 `Ctrl+C` 停止服务。

服务参数放在 `--` 后面，例如指定数据目录：

```sh
cargo run --locked --release -- --data-dir ./picsoc-data --workers 1
```

Debian 服务器首次运行后，按 `Ctrl+C`，照上一节修改生成的 TOML，再执行 `cargo run --locked --release`。

浏览器显示简洁登录页，输入 TOML 中的密码进入；未设密码直接进入。会话有效期为 24 小时，支持退出登录，服务重启后需重新登录。脚本 Basic Auth 用户名为 `picsoc`。也可以使用 `scripts/build.sh` / `scripts/start.sh`，Windows 使用对应的 `.ps1` 脚本。

验证使用 `npm run build`、`cargo fmt --all --check`、`cargo clippy --locked --all-targets -- -D warnings` 和 `cargo test --locked`。`Cargo.lock` 与 `frontend/package-lock.json` 固定依赖版本。API 定义见 [docs/API.md](docs/API.md)，发布流程见 [docs/RELEASING.md](docs/RELEASING.md)。

## 数据备份

停止服务后备份整个数据目录，确保 SQLite 数据库与缩略图一并保留。原图目录需要另外备份。不要在数据库运行中只复制单个 SQLite 文件，以免遗漏 WAL 中尚未合并的内容。恢复时使用备份的数据目录，并保持原图挂载路径与已有索引一致。

## 文档导航

- [使用手册](docs/USERGUIDE.md)：扫描、搜索、子目录、收藏、标签、批量整理和预览。
- [HTTP API](docs/API.md) 与 [界面语言](docs/I18N.md)。
- [架构说明](docs/ARCHITECTURE.md)：数据模型、扫描队列、缓存和扩展边界。
- [性能与基准](docs/PERFORMANCE.md)：默认资源策略和限定输入的可复现测试。
- [运维指南](docs/OPERATIONS.md)：备份、恢复、升级、回滚和 HTTPS 反向代理。
- [构建与发布](docs/RELEASING.md)、[变更记录](CHANGELOG.md)。
- [贡献指南](CONTRIBUTING.md) 与 [安全说明](SECURITY.md)。

## 许可证

[Apache License 2.0](LICENSE)。
