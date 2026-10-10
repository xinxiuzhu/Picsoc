# ChatGPT MCP 与 PNG 设计

Picsoc 在同一个 Rust 进程中提供图库网页、Streamable HTTP MCP 和 PNG 渲染。MCP 默认关闭，需要显式启用；网页登录 Cookie 不授予 MCP 权限。渲染使用 `image`，文字使用 `ab_glyph`，没有 Python 服务或模型运行时依赖。

## 当前服务器的启动方法

已确认 `https://orionai.iepose.cn/api/auth/status` 返回 Picsoc 登录状态；局域网地址为 `http://192.168.2.101:3210`。服务器升级代码并重启后，MCP 地址为 `https://orionai.iepose.cn/mcp`。

先停止旧进程，在服务器仓库目录执行：

```sh
git pull --ff-only
cargo run --locked --release
```

首次运行会生成数据目录中的 `config.toml` 并打印路径。按 `Ctrl+C` 停止，编辑该文件。Debian root 用户的默认路径是 `/root/.local/share/picsoc/config.toml`；使用其他账号或自定义数据目录时以日志路径为准。保留生成的 `data_dir`，将下面字段修改为：

```toml
bind = "0.0.0.0:3210"
open_browser = false
password = "你的现有 Picsoc 登录密码"

[mcp]
enabled = true
public_url = "https://orionai.iepose.cn"
token = ""
redirect_uris = []
```

然后再次执行 `cargo run --locked --release`。这些字段应修改到已有文件对应位置，不要追加重复键或重复 `[mcp]` 表。已有文件不会覆盖，修改后须重启生效，配置非法则启动失败。原来的运行环境变量不再读取；将原密码与 MCP 配置手动移入 TOML。

沿用原来的 `--data-dir`，才能继续使用已有图库、收藏与标签，默认配置也会在该目录生成。需要自定配置位置可用 `--config /路径/config.toml`。显式 CLI 参数优先文件值，但不会重写已有文件。从源码构建需要 Node.js/npm，`cargo run` 会自动编译前端；使用发行程序时运行 `./picsoc`。

Docker 首次 `docker compose up -d --build` 生成宿主机 `picsoc-data/config.toml`（容器 `/data/config.toml`），先 `docker compose stop picsoc`，按上述内容配置 `password` 与 `[mcp]`，再 `docker compose start picsoc`。更新已有容器仍使用 `docker compose up -d --build`；之后配置修改可通过 `docker compose restart picsoc` 生效。素材继续只读挂载，作品保存到 `/data/generated`，授权记录保存到 `/data/mcp-oauth.json`；镜像带有中文与拉丁字体。直接运行的 Debian 如缺少中文字体，可安装：

```sh
sudo apt-get update
sudo apt-get install --yes fonts-wqy-zenhei fonts-dejavu-core
```

自选字体放进数据目录 `fonts/`（TTF/OTF/TTC，需有对应使用许可），重启加载。`get_fonts` 和网页设计页显示可用字体及中文覆盖；缺字会报错。字体总加载预算 64 MiB，默认优先中文字体和拉丁字体，自选字体数量有上限。

## HTTPS 反向代理

`mcp.public_url` 是精确的 HTTPS 根地址，不含子路径、查询或账号密码。局域网 HTTP 地址不能直接用于云端 ChatGPT 连接。授权页面也要浏览器可达，不能只代理 `/mcp`。

现有根代理需保留 Host，转发 `/mcp`、`/.well-known/oauth-protected-resource/mcp`、`/.well-known/oauth-authorization-server` 和 `/oauth/` 下的路径。Nginx HTTPS 虚拟主机示例：

```nginx
location / {
    proxy_pass http://192.168.2.101:3210;
    proxy_set_header Host $http_host;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_buffering off;
    proxy_read_timeout 75s;
    client_max_body_size 1m;
}
```

升级后检查公开的授权发现接口：

```sh
curl --fail https://orionai.iepose.cn/.well-known/oauth-protected-resource/mcp
curl --fail https://orionai.iepose.cn/.well-known/oauth-authorization-server
```

未授权的 `/mcp` 应返回 `401` 和 `WWW-Authenticate`，不能返回网页 HTML。授权后 `GET /mcp` 返回 `405` 是正常行为：本版通过 POST 返回 JSON，不提供独立 SSE 订阅。

## 在 ChatGPT 中连接

按当前 [官方连接文档](https://developers.openai.com/plugins/deploy/connect-chatgpt)，进入 Plugins，添加自定义 MCP 服务；账号和工作区需要允许自定义连接。

1. 名称填 `Picsoc`，地址填 `https://orionai.iepose.cn/mcp`。
2. 选择 OAuth。服务发布授权发现信息和动态客户端注册（DCR）；如需选择注册方式，选 DCR。
3. 打开 Picsoc 授权页面，输入现有 Picsoc 密码并确认申请的权限。
4. 查看发现的工具，在新对话中选中 Picsoc，开始搜索和设计。

默认允许稳定回调 `https://chatgpt.com/connector_platform_oauth_redirect`，发布 issuer 标识并在回调返回 `iss`。如果管理页面显示另一条回调地址，把其**完整精确值**加入 TOML 的 `mcp.redirect_uris` 字符串数组，例如 `redirect_uris = ["https://chatgpt.com/connector/oauth/完整回调标识"]`，然后重启；不支持通配回调。[官方鉴权文档](https://developers.openai.com/plugins/build/auth)

权限为 `picsoc:read`（搜索、看图、读布局）与 `picsoc:write`（保存、合成）。OAuth 使用 PKCE S256、一次性授权码、绑定 `/mcp` 的 resource、1 小时 access token 和 7 天可轮转 refresh token。客户端与令牌哈希持久保存，正常重启无需重复连接。密码改变后，已授权令牌在到期或撤销前仍有效；需要清除所有连接时，停机备份后移走 `mcp-oauth.json`，再启动并重新授权。不要在同一数据目录同时运行多个实例。

内网服务还可通过有相应账户、工作区权限的 [Secure MCP Tunnel](https://developers.openai.com/api/docs/guides/secure-mcp-tunnels) 连接。项目不自动运行 Tunnel 客户端；Picsoc 本身不调用 OpenAI API。当前已有 HTTPS 代理，优先直接连接。

## 工具与使用

| 工具 | 功能 |
| --- | --- |
| `list_libraries` / `get_library_folders` | 已导入的素材库与分页目录树 |
| `search_assets` | 名称、相对路径、标签、尺寸、比例、格式及排除词搜索，每次最多 50 条 |
| `preview_assets` | 1–24 张图片的 PNG 拼版，标注素材 ID；透明背景棋盘、缺失错误 |
| `get_fonts` | 可用字体 ID 与中文覆盖 |
| `list_designs` / `get_design` | 找作品，读取最新或指定版本布局 |
| `save_design` | 保存新作品或追加不可变布局版本 |
| `render_design` | 将已保存版本放入 PNG 队列，返回任务 ID |
| `get_render` | 查看进度；成功时返回实际预览图片、布局与下载路径 |

原图读取和合成都在服务器完成；候选拼版与结果会按调用发送给云端模型。不会把三万张原图一次性发送。当前搜索依据名称、路径与标签，不自动理解图片风格；先筛选，再让模型查看候选图片像素。

示例提示：

> 在 Picsoc 的 UIResources 文件夹里找蓝色边框和按钮，先看候选素材，再拼一个 1920×1080 的“星海”登录界面。保留布局，先出预览，确认后导出完整 PNG。

后续提示：

> 继续刚才那份设计，把标题上移 40 像素，按钮加宽到 420 像素，保存新版本。

## 布局与继续修改

```json
{
  "version": 1,
  "name": "星海登录",
  "canvas": { "width": 1920, "height": 1080, "background": "#101827" },
  "layers": [
    { "type": "image", "asset_id": 123, "x": 100, "y": 150, "width": 400, "height": 300, "fit": "contain", "opacity": 1 },
    { "type": "rect", "x": 760, "y": 780, "width": 400, "height": 80, "radius": 16, "color": "#2463EB" },
    { "type": "text", "text": "星海", "x": 660, "y": 100, "font_size": 64, "color": "#FFFFFF", "font_id": "default", "max_width": 600, "align": "center" }
  ]
}
```

示例素材 ID 要换成真实搜索结果；字体 `default` 必须有中文字形，或换成 `get_fonts` 返回的中文字体 ID。坐标从左上角开始，层数组从底到顶。颜色支持 `transparent`、`#RRGGBB`、`#RRGGBBAA`。图片支持 contain、cover、stretch，默认完整居中的 contain；矩形支持圆角，文字支持换行、行宽、对齐和行距。

新设计调用 `save_design` 传 `{ "scene": ... }`。修改传 `{ "design_id": "d_...", "expected_revision": 1, "scene": ... }`，防止覆盖别人刚保存的版本；冲突时先读取最新布局。渲染传 `{ "design_id": "d_...", "revision": 2, "quality": "preview" }`，随后间隔约 1 秒通过 `get_render` 查询任务。

网页“设计作品”能新建作品、编辑布局 JSON、保存、预览和下载 PNG/布局；素材详情可复制 ID。下载的 scene 可再次粘贴编辑。第一版尚未提供可拖拽画布编辑器。

## 范围与持久化

- 画布每边最多 4096 像素，最多 64 图层；preview 最长边 1280，final 使用原尺寸。
- 渲染默认单任务，等待队列容量 4。候选拼版与渲染共享资源闸门，素材逐张解码；图库缩略图独立运行。
- 文本单层最多 1024 字符、全布局最多 4096 字符，字号 4–512；缺字或不可用字体明确报错。基础中英文字排版已支持，复杂文字塑形、动画导出、任意旋转、PSD/SVG、高级混合模式尚未实现。
- GIF 取第一帧，导出静态 PNG。大图受到文件大小和解码缓冲上限限制。
- 布局保存在 `generated/designs/` 的版本文件，任务的 PNG、预览和布局在 `generated/jobs/`；备份整个数据目录一起保留。
- 保存记录素材版本指纹；来源修改或删除会使旧版本合成失败，需重新扫描并保存新布局。布局不是原图副本，复现需要保留素材与字体。
- 重启后未完成任务会标失败，可重新渲染；已完成作品和布局保留。

## 本地 MCP 客户端

支持固定 Bearer 凭据的本地客户端可以使用 TOML `[mcp]` 中的独立 `token`（32–1024 个可打印 ASCII 字符）；它不是网页密码，也不是 OpenAI API key，具有读取与生成作品权限。

在生成的 `config.toml` 中修改，然后执行 `cargo run --locked --release`：

```toml
[mcp]
enabled = true
public_url = ""
token = "replace-with-a-random-token-32-or-more-chars"
redirect_uris = []
```

将示例 `token` 换成自己的随机 ASCII 令牌。客户端使用 `http://127.0.0.1:3210/mcp`，请求头 `Authorization: Bearer ...` 和 `Accept: application/json, text/event-stream`。本版支持 MCP 2025-03-26、2025-06-18、2025-11-25 协商，没有 SSE 订阅、server push 或 session ID。生产 ChatGPT 连接使用前述 OAuth + HTTPS。
