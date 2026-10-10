# HTTP API

前端与 API 使用同一地址，默认是 `http://127.0.0.1:3210`。请求和响应使用 UTF-8 JSON；媒体接口返回图片字节。设置 `PICSOC_PASSWORD` 后，浏览器通过登录页获得会话 Cookie，数据 API 与媒体接口需要认证。脚本仍可使用 HTTP Basic Auth，用户名固定为 `picsoc`。静态前端与以下认证接口允许匿名访问，不返回个人素材或路径。

## 接口

| 方法 | 路径 | 行为 |
| --- | --- | --- |
| GET | `/api/auth/status` | 返回 `{password_required, authenticated}` |
| POST | `/api/auth/login` | 传入 `{password}`，验证共享密码并建立浏览器会话，返回认证状态 |
| POST | `/api/auth/logout` | 撤销当前浏览器会话并清除 Cookie，返回认证状态 |
| GET | `/api/health` | `{ok: true, version: "0.1.0"}` |
| GET | `/api/stats` | 图片数量、总字节数、收藏数量、素材库数量 |
| GET | `/api/directories?path=...` | 浏览服务机器的现有子目录，用于文件夹选择 |
| GET | `/api/libraries` | `{libraries: Library[]}` |
| POST | `/api/libraries` | 传入 `{name, path}`，添加服务所在机器的绝对目录，返回 `201` 和 Library，后台开始扫描 |
| DELETE | `/api/libraries/{id}` | 删除索引及缩略图缓存，保留原文件，返回 `{ok: true}` |
| POST | `/api/libraries/{id}/scan` | 开始增量扫描，返回 `{ok: true}` |
| POST | `/api/libraries/{id}/scan/cancel` | 请求停止扫描，已经建立的索引保留 |
| GET | `/api/libraries/{id}/folders?parent=...` | 懒加载已扫描的直属子目录（包含空目录）、直接与递归素材数量 |
| GET | `/api/assets` | 分页搜索，返回 `{assets: Asset[], total, offset, limit}` |
| POST | `/api/assets/batch` | 批量收藏、添加或移除标签，返回 `{updated: number}` |
| GET | `/api/assets/{id}` | 获取 Asset |
| PATCH | `/api/assets/{id}` | 传入 `{favorite?: boolean, tags?: string[]}`，返回更新后的 Asset |
| GET | `/api/assets/{id}/thumbnail` | 返回磁盘缓存的 PNG；首次访问可能等待生成，无法解码时返回 SVG 占位图 |
| GET | `/api/assets/{id}/original` | 流式返回原文件，GIF 保留动画，支持单段 `Range` 请求 |
| GET | `/api/tags` | `{tags: [{name, count}]}` |
| GET | `/api/design-fonts` | `{fonts: [{id, name, supports_chinese}]}`，查询服务启动时加载的字体 |
| GET | `/api/designs?limit=50&offset=0` | `{designs: DesignSummary[], limit, offset}`，最新修改优先 |
| POST | `/api/designs` | 传入 `SaveDesign`，新建布局或保存新版本，返回 `StoredDesign` 和 `200` |
| GET | `/api/designs/{design_id}?revision=1` | 获取指定布局版本；省略 revision 获取最新版本 |
| POST | `/api/designs/{design_id}/render` | 传入 `{revision?: number, quality: "preview" \| "final"}`，返回已入队的 `RenderJob` 和 `200` |
| GET | `/api/design-jobs/{job_id}` | 获取合成状态与成功后的下载地址 |
| GET | `/api/design-jobs/{job_id}/output.png` | 下载该任务的 PNG，预览任务输出较小画布，final 任务输出原画布尺寸 |
| GET | `/api/design-jobs/{job_id}/preview.png` | 查看该任务的预览 PNG，最长边不超过 1280 |
| GET | `/api/design-jobs/{job_id}/layout.json` | 下载该任务使用的 `StoredDesign`，含 scene 和素材版本记录 |

失败响应通常是 `{error: string, code: string}`，配合 `400`、`401`、`403`、`404`、`409`、`429` 或 `500` 状态码。`code` 是稳定的错误标识；说明语言通过请求的 `Accept-Language` 选择（`zh-CN` 或 `en`，默认中文）。素材、目录与标签的原名称不参与翻译。通常请求体限制为 64 KiB；`POST /api/designs` 限制为 256 KiB。不存在的 API 返回 `404`。

## 浏览器登录

未设置密码时，认证状态为 `{password_required: false, authenticated: true}`，直接进入素材库。设置密码后，匿名状态为 `{password_required: true, authenticated: false}`。成功登录后返回 `authenticated: true`，会话有效期为 24 小时；退出、会话过期或服务重启后需要重新登录。

会话 Cookie `picsoc_session` 使用 `HttpOnly`、`SameSite=Strict` 和 `Path=/`，经同 Host 的 HTTPS Origin 登录时额外设置 `Secure`。会话保存在服务内存中，密码不写入浏览器持久存储。错误密码返回 `401`，不会建立会话。未认证的数据请求返回 `401` JSON，不触发浏览器原生 Basic 登录弹窗；脚本可以主动发送 Basic Authorization。认证接口也遵守 Host 和 Origin 检查。

## 目录接口

目录选择接口返回 `{path, parent, roots, directories, truncated}`，`roots` 和 `directories` 内的项目是 `{name, path}`。不传 `path` 时返回用户目录及系统根目录/Windows 盘符；传入绝对目录路径时返回该目录下的子文件夹。列表隐藏点开头的目录，仍可直接填写它们的路径；不列出文件及目录符号链接。单次最多列出 1000 个子目录，更多时 `truncated` 为 `true`。

目录树接口返回 `{folders, parent, direct_asset_count, separator, truncated, next_cursor}`。省略 `parent` 或传入空字符串表示库根目录，`folders` 是当前层级的直属子目录，每项为 `{path, name, parent, asset_count, direct_asset_count, has_children}`。`asset_count` 包含后代目录，`direct_asset_count` 仅计本层素材；响应顶层的 `direct_asset_count` 是请求的 parent 目录本层数量。`has_children` 表示存在真实子目录，与有没有图片无关。扫描记录空目录，旧库迁移推导素材路径的祖先目录，重新扫描补齐空目录。

目录按相对路径升序分页，`limit` 默认为 1000，必须为 1–1000；前端使用每页 200 项。`truncated=true` 表示有后续页，将 `next_cursor` 原样作为同一 parent 请求的 `cursor` 加载更多，末页 `next_cursor=null`。cursor 必须是 parent 的直接子目录路径，但不要求该目录仍存在，因此扫描删掉 cursor 目录后仍可向后继续；非法 cursor 或 limit 返回 400。各页的顶层直接素材计数都描述整个 parent，与分页范围无关。

库内隐藏点目录（名称以 `.` 开头，如 `.git`）不返回，过滤在分页 LIMIT 之前执行，`has_children` 也只计可见目录。含隐藏目录组件的 parent 返回空树。成功重扫会移除旧版本已索引的内部隐藏素材；重扫完成前，素材统计仍使用原有索引。这条规则不排除手动添加的隐藏库根目录本身，原文件始终保留。

库根目录下的项目 `parent` 为 `null`，其他项目的 `parent` 为上级相对路径。直接把返回的 `path` 编码为下次请求的参数；`separator` 明确服务平台主分隔符（Unix 为 `/`，Windows 为 `\\`），不能盲目同时按两种符号拆分，因为 Unix 文件名允许反斜杠。绝对路径或含 `..` 的路径返回 `400`。

## 批量整理

`POST /api/assets/batch` 接收：

```json
{
  "ids": [42, 43],
  "favorite": true,
  "add_tags": ["设计参考"],
  "remove_tags": ["待整理"]
}
```

`ids` 去重后必须有 1–500 个正整数，至少提供一项有效变更。`favorite`、`add_tags` 和 `remove_tags` 均可省略；`favorite: false` 取消收藏。标签会去除首尾空白、空值和重复项，每个最多 50 个 Unicode 字符，不能包含换行或空字符；每张图片最终最多 50 个标签。移除在添加之前执行，同一标签同时出现时最终保留。

全部变更在一个事务内提交。任一 ID 不存在返回 `404`，任一素材标签超限返回 `400`，整批回滚；成功返回去重后处理的素材数。此接口只修改索引元数据，不修改原文件。

## 数据结构

`Library` 包含：

```json
{
  "id": 1,
  "name": "设计素材",
  "path": "/library",
  "asset_count": 120,
  "scan": {"state": "idle", "processed": 120, "total": 120, "error": null}
}
```

`scan.state` 为 `idle`、`scanning` 或 `error`。`total` 在遍历过程中为 `null`。扫描状态描述目录遍历进度，缩略图任务可能稍后完成。取消扫描后回到 `idle`。

扫描自动跳过嵌套的 Apple Photos `.photoslibrary` 图库包。`POST /api/libraries` 和目录浏览接口不能直接使用图库包及其内部目录，会返回 `400`、错误码 `photos_library_unsupported` 及按请求语言提供的导出说明；请先从 Photos 导出到普通图片文件夹。旧版本已索引的包内记录和注释会保留，普通目录读取失败仍使扫描报错且不清理旧索引。

`Asset` 包含：

```json
{
  "id": 42,
  "library_id": 1,
  "name": "风景.png",
  "relative_path": "参考/风景.png",
  "format": "png",
  "size": 182400,
  "width": 1920,
  "height": 1080,
  "modified_at": 1791504000,
  "favorite": false,
  "tags": ["参考"],
  "thumbnail_url": "/api/assets/42/thumbnail",
  "original_url": "/api/assets/42/original"
}
```

`size` 单位为字节，`modified_at` 是 Unix 秒数。尺寸在缩略图生成前或解码失败时可能为 `null`。文件内容变化会更新索引及缓存，收藏和标签仍保留；文件改名暂视为删除旧素材并添加新素材。

`Stats` 字段为 `total_assets`、`total_size`、`total_favorites`、`total_libraries`。

## 搜索与分页

`GET /api/assets` 支持以下查询参数：

| 参数 | 用途 |
| --- | --- |
| `q` | 文件名、相对路径、标签的子串搜索，支持中文 |
| `exclude_names` | 换行分隔的文件名排除关键词；包含任意关键词的文件名不返回，ASCII 大小写不敏感，`%`/`_`/反斜杠按字面匹配，仅作用于文件名 |
| `library_id` | 指定素材库 |
| `folder` | 库内相对目录，必须同时指定 `library_id`；省略或空字符串表示库根目录 |
| `folder_recursive` | `true`（默认）包含子目录，`false` 仅当前目录的直接素材；必须指定 `library_id`，库根目录也可仅查看本层 |
| `favorite` | `true` 仅收藏，`false` 仅未收藏 |
| `format` | `jpg`、`png`、`gif`、`webp`、`bmp`、`tiff` |
| `tag` | 完整匹配标签 |
| `orientation` | `landscape` 宽大于高、`portrait` 高大于宽、`square` 宽高相等 |
| `aspect_ratio` | 正数宽:高，例如 `16:9`、`1:1` 或 `2.35:1`，允许 ±2% 相对误差 |
| `min_width` / `max_width` | 最小/最大像素宽度，非负整数，包含边界 |
| `min_height` / `max_height` | 最小/最大像素高度，非负整数，包含边界 |
| `min_size` / `max_size` | 最小/最大文件字节数，非负整数，包含边界 |
| `sort` | `modified`（默认），`name`/`name_desc`，`size`（大到小）/`size_asc`，`width`、`height`、`pixels`（大到小） |
| `offset` | 从 0 开始的偏移 |
| `limit` | 默认 100，限制在 1–200 |

例如：`/api/assets?library_id=1&folder=%E5%8F%82%E8%80%83&folder_recursive=true&aspect_ratio=16%3A9&min_width=1920&sort=pixels&limit=100`。参数可以组合使用，分页 `total` 与返回素材应用相同条件。相同排序值通过素材 ID 保持稳定顺序；未知尺寸不匹配方向、比例或像素范围。非法枚举、比例、负数、反转范围或未指定库的目录范围返回 `400`。

例如 `exclude_names=map%0Anormal` 同时排除文件名包含 `map` 或 `normal` 的图片。排除词去除首尾空白、空项和重复项；总 UTF-8 长度不超过 4096 字节，最多 50 项，每项最多 100 个 Unicode 字符，超限返回 400。空字符串不筛选。排除条件与其他条件共同参与计数和分页，不删除素材或修改原文件。

## 设计布局与 PNG 合成

设计 HTTP API 沿用网页 Cookie/Basic Auth；MCP OAuth/Bearer 凭据只用于 `/mcp`，不能代替网页登录下载成品。`design_id` 为 `d_` 加 32 个小写十六进制字符，`job_id` 为 `j_` 加同样长度的后缀；接口不接受任意输出路径。

新建布局传入以下 `SaveDesign`。素材编号应从 `/api/assets` 或 MCP 搜索取得，示例中的 `42` 需要替换为实际编号：

```json
{
  "scene": {
    "version": 1,
    "name": "登录页面",
    "canvas": {"width": 1920, "height": 1080, "background": "#101B30"},
    "layers": [
      {"type": "image", "asset_id": 42, "x": 120, "y": 180, "width": 1680, "height": 700, "fit": "contain", "opacity": 1},
      {"type": "rect", "x": 760, "y": 850, "width": 400, "height": 80, "color": "#007AFF", "radius": 20},
      {"type": "text", "text": "星海", "x": 560, "y": 100, "font_size": 72, "color": "#FFFFFF", "font_id": "default", "max_width": 800, "align": "center"}
    ]
  }
}
```

图层按数组顺序从底到顶绘制，坐标以原始画布左上角为原点，超出画布的部分被裁去。画布宽高均为 1–4096，总像素最多 16,777,216；最多 64 个图层。背景和颜色支持 `transparent`、`#RRGGBB`、`#RRGGBBAA`；`opacity` 为 0–1，默认 1。布局 `version` 默认且仅支持 1，名称须有内容且最多 120 个 Unicode 字符，未知字段会被拒绝。

图像与矩形图层宽高为 1–4096，所有图层坐标为 -8192–8192。图像 `fit` 默认 `contain`，等比完整放入目标框并居中；`cover` 等比填满目标框并裁去超出部分，`stretch` 按目标框宽高缩放。GIF 在合成中使用首帧。矩形 `radius` 默认为 0，不能超过短边一半。

文字 `font_size` 为 4–512，`font_id` 为字体接口返回的编号或 `default`。默认字体自动选择能覆盖全部所需字符的已加载字体；找不到完整字形会返回 `400`，不会静默输出方框。每层文字最多 1024 字符，布局文字总计最多 4096；支持显式换行、按 `max_width` 换行、`left`/`center`/`right` 对齐和 `line_height` 倍数（0.5–4，默认 1.2）。`max_width` 默认使用画布宽度。当前使用字形绘制与基础字距处理，没有复杂文字塑形或完整排版引擎。

`StoredDesign` 包含 `{design_id, revision, created_at, name, scene, asset_sources, latest_job}`；`asset_sources` 是 `{asset_id, cache_key}` 数组，用于检测素材版本变化。新设计 revision 为 1，修改设计时必须同时提交 `design_id`、等于当前版本的 `expected_revision` 和完整 `scene`。匹配成功会写入新版本，旧版本保留；缺失或过期的 `expected_revision` 返回 `409`。每个设计最多 10,000 个版本。下载的 layout.json 是完整 StoredDesign，继续修改时使用其中的 scene，并按最新版本提交 expected_revision。

渲染必须引用已保存设计。HTTP 渲染请求必须显式传入 `quality`；`revision` 可省略以选择最新版本：

```json
{"revision": 1, "quality": "preview"}
```

`preview` 将最长边缩小至不超过 1280，`final` 使用完整画布；两者均生成 PNG 和可下载布局。服务使用一个独立渲染线程，最多等待 4 个任务；队列满返回 `429`，不创建有效任务。入队响应的 `status` 为 `queued`，之后可能成为 `running`、`succeeded` 或 `failed`，应稍作间隔再请求任务状态。重启后保留已完成成品，未完成任务标记为 failed，需要重新提交。

任务成功后 `output_url`、`preview_url`、`layout_url` 为相对路径，例如 `/api/design-jobs/j_0123456789abcdef0123456789abcdef/output.png`；成功前它们为 null。`RenderJob` 还包含 `design_id`、`revision`、`quality`、`created_at`、`finished_at`、`error`、输出 `width` 和 `height`。成功前访问成品文件返回 `409`，未知文件名返回 `400`。输出 PNG 保留透明通道，原素材不会被改写。

素材在保存后发生变化、删除或改名，旧布局可能无法再次渲染；服务通过已索引版本与实际文件 metadata 检查，避免静默替换内容。重新读取素材并保存新布局版本后再渲染。已经完成的 PNG 独立于原素材保留。

原始 TTF/OTF/TTC 字体可以放入数据目录的 `fonts` 下，重启后加载；工具不能传入任意字体路径。使用 `/api/design-fonts` 确认实际可用字体，`supports_chinese` 是样例字形检测，保存布局还会逐字确认所需字符。原生 Debian 可安装 `fonts-wqy-zenhei` 和 `fonts-dejavu-core`；Docker 运行镜像包含这两种字体。系统和用户字体总加载预算为 64 MiB，用户目录按路径排序后只检查前 8 项。

## MCP 接口

MCP 使用独立路由和鉴权，默认关闭，通过 `--mcp` 或 `PICSOC_MCP_ENABLED=true` 明确启用。`POST /mcp` 接收单个 JSON-RPC 2.0 消息，支持 `initialize`、`ping`、`tools/list`、`tools/call`；通知成功返回无内容的 `202`。请求需声明 `Content-Type: application/json` 和 `Accept: application/json, text/event-stream`，有认证的 `GET`/`DELETE /mcp` 返回 `405`（本实现不提供 SSE 或 session）。单次 MCP 请求限制为 1 MiB。

当前支持 `2025-03-26`、`2025-06-18`、`2025-11-25`，初始化会协商版本；后续 `MCP-Protocol-Version` 不能指定不支持的版本。不会通过 MCP 暴露任意文件系统、命令执行或 Rust 编译。

| 工具 | 主要输入和行为 |
| --- | --- |
| `list_libraries` | 无输入，返回已索引库的编号、名称与数量 |
| `get_library_folders` | `library_id`，可选 `parent`/`cursor`；每页 200 个直属目录 |
| `search_assets` | 使用素材搜索参数；默认 24、最多 50 项，返回候选元数据而非视觉语义搜索结果 |
| `preview_assets` | `asset_ids`，1–24 个不重复正整数；返回有编号的实际 PNG 拼版及 missing 列表 |
| `get_fonts` | 无输入，返回实际加载的字体 |
| `list_designs` | 可选 `limit`/`offset`，默认 50、最多 100 项 |
| `get_design` | `design_id`，可选 `revision`；返回可继续编辑的布局 |
| `save_design` | `SaveDesign`；保存新设计或匹配 expected_revision 的新版本 |
| `render_design` | `design_id`，可选 `revision`/`quality`，MCP 的 quality 默认 preview；返回入队任务 |
| `get_render` | `job_id`；返回任务状态，成功时附实际预览 PNG、布局及相对下载路径 |

工具成功结果带 `structuredContent` 与文本 `content`，预览工具还带标准 `type: "image"`、`mimeType: "image/png"` 和 base64 像素。合成业务失败通过工具结果 `isError: true` 报告；JSON-RPC 参数/方法错误通过 error 报告。只读授权可以查询、预览，保存和渲染要求写权限。

OAuth 发现位于 `/.well-known/oauth-protected-resource/mcp`（及无路径兼容地址）与 `/.well-known/oauth-authorization-server`，注册/授权/token/revoke 分别为 `/oauth/register`、`/oauth/authorize`、`/oauth/token`、`/oauth/revoke`。未认证的 MCP 401 携带发现地址。OAuth 输入限制为 16 KiB。部署、精确回调、S256 PKCE、作用域和令牌重启行为见 [MCP 部署说明](MCP.md) / [English guide](MCP.en.md)。

## 验证

先构建前端及 Rust 可执行程序，然后运行：

```sh
python3 tests/api_smoke.py --binary target/debug/picsoc
```

Windows 使用 `python` 和 `target/debug/picsoc.exe`。测试启动真实服务，只操作临时目录；覆盖目录添加、中文搜索、GIF、缩略图、收藏与标签、批量事务回滚、空目录与多层目录树、当前层/递归范围、方向比例与像素/字节范围的组合分页、非法筛选、文件变更、重启持久化、字节范围请求、路径边界以及移除素材库保留原图。Windows 环境跳过需要系统权限的符号链接测试。
