# 运维：备份、恢复、升级与 HTTPS

本文面向运行 Picsoc 服务的人。使用发行包或 GHCR 镜像前，确认对应附件或版本 tag 已实际生成；源码部署可按选定 commit 构建。首次部署和目录权限设置见 [README](../README.md)。

## 数据与路径

原图放在你自己的素材目录；Picsoc 数据目录保存以下文件。Docker 示例中原图与数据分别是 `/library` 和 `/data`，对应宿主机 `.env` 中的 `PICSOC_LIBRARY_PATH` 和 `PICSOC_DATA_PATH`。

| 路径 | 内容与备份用途 |
| --- | --- |
| `picsoc.sqlite3` 及可能存在的 `-wal` / `-shm` | 索引、素材编号、收藏和标签；须作为同一份停机数据备份 |
| `thumbnails/` | 已生成的缩略图缓存；保留可避免重算 |
| `generated/designs/` | 保存的设计布局及不可变版本 |
| `generated/jobs/` | 任务状态、PNG 成品、预览和任务使用的布局；尚未完成的任务在重启后标记为失败 |
| `fonts/` | 用户提供的原始字体文件；复现文字样式需要相同字体，系统字体另行安装 |
| `mcp-oauth.json` | OAuth 客户端与 access/refresh 令牌哈希；保留后未过期连接可继续使用 |

原生默认路径：Windows `%LOCALAPPDATA%\Picsoc`；macOS `~/Library/Application Support/Picsoc`；Linux `$XDG_DATA_HOME/picsoc` 或 `~/.local/share/picsoc`。启动日志会显示实际目录，`--data-dir` 可以覆盖它。

备份整个数据目录能保留收藏、标签、已有索引、设计布局与成品；不要把 `generated`、`fonts` 或授权文件排除在备份之外。原图需要单独备份；数据库和布局不能恢复已丢失的原图。记录原图的绝对路径或容器挂载位置，恢复时保持相同路径。改名或移动文件当前会被识别为删除旧素材并添加新素材，原来的收藏和标签不会自动迁移，引用旧素材编号的布局也可能无法再次合成。

`mcp-oauth.json` 不包含明文密码或令牌，但包含授权身份与令牌哈希，须与配置备份一起限制访问。Unix 上新写入授权文件权限为 `0600`；恢复后保留权限并确保运行账号能读取和写入数据目录。只有缩略图可以根据原图重建，已导出的 PNG、历史布局和用户字体没有同等的自动重建保证。当前没有自动清理成品的保留策略，应监测 `generated` 占用。

## 停机备份

不要只复制运行中的 `picsoc.sqlite3`：未 checkpoint 的内容可能仍在 WAL 中。先停止服务并确认进程退出，再复制整个数据目录。

Docker 示例在仓库根目录执行。替换两个变量为实际宿主机目录，并使用有权读取数据目录的账号；先确保备份目录有足够空间：

```sh
set -eu
picsoc_data_dir=/srv/picsoc/data
picsoc_backup_dir=/srv/picsoc/backups
mkdir -p "$picsoc_backup_dir"
chmod 700 "$picsoc_backup_dir"
umask 077
docker compose stop picsoc
picsoc_backup_file="$picsoc_backup_dir/data-$(date -u +%Y%m%dT%H%M%SZ).tar.gz"
tar -czf "$picsoc_backup_file" -C "$picsoc_data_dir" .
tar -tzf "$picsoc_backup_file" > /dev/null
install -m 600 .env "$picsoc_backup_dir/config.env"
docker compose start picsoc
```

只在归档成功后重新启动。目录所有者若为 `10001`，备份账号需要对应读取权限；不要为了备份修改原图目录的权限。配置备份含密码，不能公开。保留多个时间点和一份独立设备上的副本，并定期实际恢复到隔离目录验证。

原生 macOS/Linux：在运行窗口按 `Ctrl+C`，确认服务退出后，使用上述 `tar` 命令备份实际数据目录。Windows 可在停止服务后用 PowerShell 复制：

```powershell
$picsocData = Join-Path $env:LOCALAPPDATA 'Picsoc'
$picsocBackup = Join-Path $env:USERPROFILE ('Picsoc-backup-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
Copy-Item -LiteralPath $picsocData -Destination $picsocBackup -Recurse
```

自定义了 `--data-dir` 时，相应更改变量。备份文件继承备份位置的访问权限，选择仅自己能访问的位置。

## 恢复

1. 停止服务，保留现有数据目录的副本，避免覆盖唯一可用的数据。
2. 将可信备份解压或复制到一个新的数据目录，不要与正在使用的数据混合。
3. 检查目录中有 `picsoc.sqlite3`，按备份内容核对 `thumbnails`、`generated`、`fonts` 和 `mcp-oauth.json`；确认原图目录已恢复，路径及内容正确。
4. Docker 修改 `PICSOC_DATA_PATH` 指向新目录，确认运行 UID/GID 对数据目录有写权限、对素材目录有读取及遍历权限。原生程序用 `--data-dir 新目录` 启动。
5. 启动同版本程序，核对素材库、图片数量、收藏和标签，打开原图与 GIF；如有设计，检查历史布局、PNG 下载和字体，再恢复常规访问。

在素材目录仍为空或指向错误位置时不要启动扫描；一次成功扫描会移除索引中已不存在的文件。容器仍使用同样的 `/library` 挂载路径时，即使宿主目录位置调整，已有索引也可以继续使用。跨系统迁移导致服务内路径变化时，目前没有自动路径迁移工具；可在原系统完成备份，或先让新环境提供相同路径。

设计使用 SQLite 索引中的素材编号与保存时的版本指纹，不保存原图副本。迁移时同时保留数据库、原素材、字体和 generated，尽量保留原图修改时间与内容；不要通过删除库、重新添加来替代索引迁移。若来源在迁移中发生变化，先重扫，再读取旧 scene、确认素材编号并另存新布局版本。不同系统的默认字体可能不同，需稳定样式时使用同一份用户字体文件和 font_id。

继续使用原公开域名时，沿用 `PICSOC_PUBLIC_URL` 与 `mcp-oauth.json`，未过期 OAuth 凭据可继续验证；服务端短期授权码和网页会话仍需重新建立。变更公开域名会改变 `/mcp` 的 resource/issuer，旧 OAuth token 不能用于新地址，应在 ChatGPT 更新连接并重新授权。恢复旧备份会恢复该备份中尚未过期的授权状态；需要撤销全部 OAuth 连接时，停止服务、备份后移走 `mcp-oauth.json`，再启动并重新授权。静态 `PICSOC_MCP_TOKEN` 来自环境配置，需另行更换；只改网页密码不会立即撤销已签发的 OAuth token。

不要同时用两个 Picsoc 实例写同一数据目录：文件布局与 OAuth 持久状态不支持跨进程协作。备份验证应使用隔离目录，并避免让测试实例扫描缺失的真实素材路径。

## 升级与回滚

升级前记录正在运行的 commit、二进制版本或完整镜像 tag/digest，并完成停机备份。先看新版本变更记录和实际 CI 结果。当前没有通用的数据库降级或迁移回滚保证。

原生运行：保留旧二进制，将新二进制放到单独目录，沿用原数据目录启动，检查版本、图片、收藏和标签，以及设计布局、下载和 MCP 授权。源码可使用 `cargo build --locked --release` 自动构建并嵌入前端，或使用 [构建脚本](../scripts/build.sh)。

Docker 运行：选择已经发布的固定版本，在 `.env` 设置 `PICSOC_IMAGE=ghcr.io/xinxiuzhu/picsoc:具体版本`，保持 UID/GID、数据和素材路径一致，然后：

```sh
docker compose pull picsoc
docker compose up -d --no-build picsoc
docker compose logs --tail 100 picsoc
```

使用 `--no-build` 是为了运行选定的已发布镜像。不要将 `latest` 当作固定版本。本地源码构建没有发布镜像时，更新到选定 commit 后使用 `docker compose up -d --build picsoc`。

失败时先停止新服务，保留出错后的数据副本，恢复升级前的整个数据目录，再运行对应旧版本；不要只更换旧二进制继续读取已被新版修改的数据。重新核对素材、收藏、标签和原图。备份之后发生的编辑需要另行处理。

## HTTPS 反向代理示例

以下使用同一台 Debian 机器上的 Nginx 代理到本仓库的 Docker Compose 服务。Compose 保持 `PICSOC_HOST_BIND=127.0.0.1`；容器内部仍使用 `PICSOC_BIND=0.0.0.0:3210`。在 `.env` 设置非空 `PICSOC_PASSWORD`，登录用户名为 `picsoc`。只开放代理需要的 HTTPS 端口，后端 3210 不公开。

准备你自己的域名、DNS 和可信 TLS 证书，将示例域名与证书路径换为实际值。Nginx 配置片段放在它的 `http` 配置上下文中；本示例使用域名根路径，不支持把 Picsoc 放到 `/picsoc/` 子路径。

```nginx
server {
    listen 80;
    server_name picsoc.example.com;
    return 308 https://picsoc.example.com$request_uri;
}

server {
    listen 443 ssl;
    server_name picsoc.example.com;

    ssl_certificate     /etc/ssl/picsoc/fullchain.pem;
    ssl_certificate_key /etc/ssl/picsoc/privkey.pem;
    ssl_protocols TLSv1.2 TLSv1.3;
    client_max_body_size 1m;

    location / {
        proxy_pass http://127.0.0.1:3210;
        proxy_http_version 1.1;
        proxy_set_header Host $http_host;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header Connection "";
        proxy_buffering off;
        proxy_cache off;
        proxy_read_timeout 120s;
    }
}
```

保留浏览器的 Origin、Cookie 和脚本的 Authorization；不要为了绕过 403 去清空 Origin、关闭后端认证或添加宽泛的 CORS。使用 `$http_host` 保留 Host 中的端口，保证后端的同源检查能与浏览器 Origin 对齐。以上指令参考 [Nginx 官方代理文档](https://nginx.org/en/docs/http/ngx_http_proxy_module.html)；生产前应在自己的目标机器执行配置检查与实际验证，本文不代表该部署已在你的环境运行：

```sh
sudo nginx -t
sudo systemctl reload nginx
curl --fail --user picsoc https://picsoc.example.com/api/health
```

`curl --user picsoc` 会交互询问密码。浏览器访问域名后，核对登录、目录选择、添加素材库、收藏/标签保存、预览、下载与 GIF 动画。确认未认证的数据 API 和媒体请求返回 `401`，并从另一台设备确认无法直接连接宿主机 3210。

原生服务若仍绑定 `127.0.0.1`，会拒绝上述公开域名 Host。此示例因此使用 Compose：宿主机端口仅回环可达，容器内监听地址允许代理保留真实 Host。若改为原生广域绑定，需要额外防火墙仅允许代理连接；不要照抄配置后将后端直接开放。

启用 MCP 时设置 `PICSOC_MCP_ENABLED=true`、`PICSOC_PUBLIC_URL=https://你的域名` 与非空 `PICSOC_PASSWORD`。根代理应原样转发 `/mcp`、`/.well-known/` 与 `/oauth/`，不能只开放 MCP 工具端点；代理请求体上限 1 MiB 覆盖 MCP 布局请求，服务端仍分别限制普通 API 64 KiB、设计保存 256 KiB 和 OAuth 16 KiB。详细步骤见 [ChatGPT MCP 说明](MCP.md)。

当前示例服务器的 `https://orionai.iepose.cn/api/auth/status` 已确认能到达 Picsoc，网页局域网入口为 `http://192.168.2.101:3210/`。这项检查不表示新 MCP 代码已部署，或 ChatGPT 账户已完成连接；需在服务器更新并重启后检查公开发现接口和 `/mcp` 401，再由使用者完成 OAuth 授权。

## 诊断

| 现象 | 首先检查 |
| --- | --- |
| 网页打不开 | 服务是否运行、端口是否占用、实际绑定地址、容器日志 |
| 添加目录失败 | 路径是否属于服务机器；Docker 内填写 `/library`；UID/GID 是否能遍历目录 |
| 图片数量没更新 | 扫描状态与错误、扫描间隔；手动重扫可重试之前失败的缩略图 |
| 缩略图失败 | 原图权限、格式、文件/像素限制、数据目录剩余空间；TIFF 使用缩略图而非浏览器原图 |
| 容器反复退出 | 查看退出状态与日志，保持 `PICSOC_WORKERS=1`，检查内存上限和磁盘空间 |
| 代理请求 403 | Host 与 Origin 是否相同、原生是否仅回环绑定，不要删除 Origin 绕过检查 |
| 修改数据时 401 | 浏览器会话是否过期、代理是否保留 Cookie，重新登录；脚本检查密码、用户名和 Authorization |
| ChatGPT 发现 MCP 时收到 HTML | 是否仍运行旧二进制、MCP 是否明确启用、代理是否原样转发 `/mcp` 和发现路径 |
| 合成失败或队列忙 | 任务 error、素材版本/权限、字体字形、解码限制与数据目录空间；稍后重试忙队列 |
| 更新布局返回 409 | 重新读取最新布局版本，用当前 revision 作为 expected_revision 保存 |
| 迁移后文字无法合成 | 用户字体是否恢复、系统字体是否安装，查询 get_fonts 并确认 scene 中的 font_id |

删除素材库只删除索引和缓存，不删除原图；但对应收藏和标签也会被移除。误移除后应从停机备份恢复元数据，重新添加目录不能恢复旧标签。
