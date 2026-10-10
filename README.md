# Picsoc 图片素材库

简体中文 · [English](README.en.md)

Picsoc 是一个在自己电脑或服务器上运行的图片素材库。启动 Rust 服务后，在浏览器打开界面；前端静态文件随程序提供，使用时只需要一个端口。第一版为 0.1.0。

原图保留在现有目录，Picsoc 只读取图片并建立索引。数据库、标签和缩略图存放在独立的数据目录，重启后继续使用。

## 第一版

- 添加本机目录，递归索引 JPEG、PNG、GIF、WebP、BMP、TIFF 图片。
- 图片网格按需加载缩略图，浏览、搜索、筛选和查看图片详情。
- 收藏与标签保存在 SQLite 数据库中；支持多选后批量收藏、添加或移除标签。
- 按素材库中的子文件夹浏览与递归筛选，只显示已索引且包含图片的目录。
- 支持中文和英文界面，可在页面中切换语言。
- 后台扫描生成缩略图；支持取消扫描、手动重扫以及默认每 300 秒的增量扫描。
- 配置 Windows 和 macOS 原生程序、Debian 原生程序与 Docker 的构建及发布流程。

技术栈：Rust、Axum、Tokio、SQLite、React、TypeScript、Vite。SQLite 随程序构建，图片处理使用 Rust `image`，第一版不需要另外安装 Node.js、SQLite 或 libvips。Node.js 只在编译前端时使用。

## Windows / macOS / Debian 快速开始

可按下文“从源码开发”构建运行，也可以从 [GitHub Releases](https://github.com/xinxiuzhu/Picsoc/releases) 下载对应系统及 CPU 的版本附件；先确认附件已生成。容器镜像状态独立于附件，以发布 workflow 的 GHCR job 为准。发行 workflow 的构建目标如下：

| 系统 | 文件名后缀 | 启动 |
| --- | --- | --- |
| Windows x64 | `windows-x86_64.zip` | 双击 `picsoc.exe` |
| macOS Apple Silicon | `macos-aarch64.tar.gz` | 终端运行 `./picsoc` |
| macOS Intel | `macos-x86_64.tar.gz` | 终端运行 `./picsoc` |
| Debian x64 | `linux-x86_64.tar.gz` | 终端运行 `./picsoc` |

启动后默认打开 [http://127.0.0.1:3210](http://127.0.0.1:3210)。点击“添加素材库”，通过文件夹选择器浏览并选择已有素材文件夹，也可以手动填写绝对路径，例如 Windows 的 `D:\素材`，macOS 的 `/Users/你的用户名/Pictures`，Debian 的 `/home/你的用户名/Pictures`。

文件夹选择器浏览的是运行 Picsoc 服务的机器。原生运行时可选择本机文件夹；通过局域网访问时，选择服务机器上的文件夹。确认添加后，后台扫描读取原图并创建索引和缩略图。

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

1. 创建数据目录，并配置素材位置：

   ```sh
   mkdir -p picsoc-data
   cp docs/compose.env.example .env
   id -u
   id -g
   ```

   编辑 `.env`：将 `PICSOC_UID`、`PICSOC_GID` 设置为普通运行用户的上述编号，将 `PICSOC_LIBRARY_PATH` 设置为已有素材目录的绝对路径。如果使用 root 管理 Docker，保持默认 `10001:10001`，不要填写 `0:0`。`picsoc-data` 需要能被该用户写入，素材目录需要能被该用户读取和遍历。

   例如：

   ```dotenv
   PICSOC_UID=1000
   PICSOC_GID=1000
   PICSOC_DATA_PATH=./picsoc-data
   PICSOC_LIBRARY_PATH=/home/你的用户名/Pictures
   ```

   默认 UID/GID 为 `10001:10001`。如果继续使用默认编号，在 Linux 上为数据目录设置所有者：`sudo chown 10001:10001 picsoc-data`。不要对原图目录执行这条命令。Compose 不会自动创建不存在的挂载目录，以免出现由 root 创建而导致无法写入的数据目录。

2. 构建并启动：

   ```sh
   docker compose up -d --build
   ```

3. 打开 [http://127.0.0.1:3210](http://127.0.0.1:3210)，点击“添加素材库”，通过文件夹选择器选择 **`/library`** 或其子文件夹，也可以手动填写 `/library`。

   容器看到的是 `/library`，不能直接使用宿主机的 `/home/...` 路径。该目录通过只读挂载提供，数据库和缩略图通过 `/data` 持久化。后续重启仍能打开已有索引：

   ```sh
   docker compose restart
   docker compose logs --tail 100 picsoc
   ```

停止服务使用 `docker compose down`。数据在宿主机的 `picsoc-data` 中保留。第一次添加目录后，图片会随后台任务逐步出现，导入大量素材时可以继续浏览已完成的部分。

确认所选版本镜像已上传 GHCR 后，可以使用该镜像而无需在部署机编译。在 `.env` 添加 `PICSOC_IMAGE=ghcr.io/xinxiuzhu/picsoc:0.1.0`，然后：

```sh
docker compose pull
docker compose up -d --no-build
```

如果镜像尚未公开，需等待仓库维护者将 GHCR 包的访问权限设置为公开，或使用已授权的 GitHub 凭据拉取。提交代码本身不会部署服务。

## 低配置机器

Docker 默认限制为 1 个 CPU、512 MiB 内存，图片处理并发为 1。`.env` 可以调整：

```dotenv
PICSOC_WORKERS=1
PICSOC_MEMORY_LIMIT=512m
PICSOC_CPU_LIMIT=1.0
PICSOC_SCAN_INTERVAL=300
```

这些是资源配置，不是所有素材都能在 512 MiB 内存下处理的保证。单张图片超过 256 MiB 文件大小、128 MiB 解码数据或 32,768 像素宽/高时，程序跳过缩略图并显示预览失败。解码限制不等于进程总内存限制，多个任务并行仍可能超过容器硬内存上限并使其退出。遇到导入期间反复重启，先保持 `PICSOC_WORKERS=1`，查看日志，再提高内存上限。原生运行时 `--workers 1` 只控制处理并发，不限制进程总内存。

如果素材主要存放在机械硬盘或网络盘，可以将扫描间隔调大，例如 `PICSOC_SCAN_INTERVAL=1800`；设置为 `0` 关闭定时扫描，之后通过界面手动重扫。第一版使用周期增量扫描，并非即时文件监控。

## 参数与局域网访问

```text
picsoc --bind 127.0.0.1:3210 --data-dir ./picsoc-data --workers 1
```

| 参数 | 环境变量 | 默认值 |
| --- | --- | --- |
| `--bind` | `PICSOC_BIND` | `127.0.0.1:3210` |
| `--data-dir` | `PICSOC_DATA_DIR` | 当前用户应用数据目录，见上表 |
| `--workers` | `PICSOC_WORKERS` | `1`，可选 `1`–`4` |
| `--scan-interval` | `PICSOC_SCAN_INTERVAL` | `300` 秒，`0` 关闭周期扫描 |
| `--no-open` | — | 不自动打开浏览器 |
| `--version` | — | 显示版本后退出 |

默认只允许本机访问。如需其他设备浏览，原生服务设置 `--bind 0.0.0.0:3210`，Docker 将 `.env` 的 `PICSOC_HOST_BIND` 改为 `0.0.0.0`，并配置非空 `PICSOC_PASSWORD`。登录用户名固定为 `picsoc`。浏览器访问服务机器的局域网 IP，例如 `http://192.168.1.20:3210`。

密码认证使用 HTTP Basic Auth。HTTP 连接没有传输加密；跨公网或不可信网络使用时，应通过具有 HTTPS 的反向代理提供访问。不要直接将无密码的素材服务开放到公网。

## 从源码开发

CI 使用 Rust 1.96.1、Node.js 22 和 npm。在仓库根目录执行下面的命令，先构建前端，再通过 Cargo 编译并启动 Rust 服务：

```sh
npm --prefix frontend ci
npm --prefix frontend run build
cargo run --locked --release
```

Windows PowerShell 可以使用同样的命令。前端文件会嵌入 Rust 程序，不需要单独启动前端服务。首次运行或前端更新后需要重新构建前端；之后直接运行 `cargo run --locked --release` 即可。

`cargo run` 使用调试构建，日常使用和部署建议加上 `--release`。默认打开 [http://127.0.0.1:3210](http://127.0.0.1:3210)，按 `Ctrl+C` 停止服务。

服务参数放在 `--` 后面，例如指定数据目录：

```sh
cargo run --locked --release -- --data-dir ./picsoc-data --workers 1
```

Debian 服务器需要局域网访问时：

```sh
PICSOC_PASSWORD='换成你自己的强密码' cargo run --locked --release -- --bind 0.0.0.0:3210 --no-open
```

登录用户名为 `picsoc`。也可以使用 `scripts/build.sh` / `scripts/start.sh`，Windows 使用对应的 `.ps1` 脚本。

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
