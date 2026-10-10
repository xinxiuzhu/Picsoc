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

失败响应通常是 `{error: string, code: string}`，配合 `400`、`401`、`403`、`404` 或 `500` 状态码。`code` 是稳定的错误标识；说明语言通过请求的 `Accept-Language` 选择（`zh-CN` 或 `en`，默认中文）。素材、目录与标签的原名称不参与翻译。请求体限制为 64 KiB。不存在的 API 返回 `404`。

## 浏览器登录

未设置密码时，认证状态为 `{password_required: false, authenticated: true}`，直接进入素材库。设置密码后，匿名状态为 `{password_required: true, authenticated: false}`。成功登录后返回 `authenticated: true`，会话有效期为 24 小时；退出、会话过期或服务重启后需要重新登录。

会话 Cookie `picsoc_session` 使用 `HttpOnly`、`SameSite=Strict` 和 `Path=/`，经同 Host 的 HTTPS Origin 登录时额外设置 `Secure`。会话保存在服务内存中，密码不写入浏览器持久存储。错误密码返回 `401`，不会建立会话。未认证的数据请求返回 `401` JSON，不触发浏览器原生 Basic 登录弹窗；脚本可以主动发送 Basic Authorization。认证接口也遵守 Host 和 Origin 检查。

## 目录接口

目录选择接口返回 `{path, parent, roots, directories, truncated}`，`roots` 和 `directories` 内的项目是 `{name, path}`。不传 `path` 时返回用户目录及系统根目录/Windows 盘符；传入绝对目录路径时返回该目录下的子文件夹。列表隐藏点开头的目录，仍可直接填写它们的路径；不列出文件及目录符号链接。单次最多列出 1000 个子目录，更多时 `truncated` 为 `true`。

目录树接口返回 `{folders, parent, direct_asset_count, separator, truncated, next_cursor}`。省略 `parent` 或传入空字符串表示库根目录，`folders` 是当前层级的直属子目录，每项为 `{path, name, parent, asset_count, direct_asset_count, has_children}`。`asset_count` 包含后代目录，`direct_asset_count` 仅计本层素材；响应顶层的 `direct_asset_count` 是请求的 parent 目录本层数量。`has_children` 表示存在真实子目录，与有没有图片无关。扫描记录空目录，旧库迁移推导素材路径的祖先目录，重新扫描补齐空目录。

目录按相对路径升序分页，`limit` 默认为 1000，必须为 1–1000；前端使用每页 200 项。`truncated=true` 表示有后续页，将 `next_cursor` 原样作为同一 parent 请求的 `cursor` 加载更多，末页 `next_cursor=null`。cursor 必须是 parent 的直接子目录路径，但不要求该目录仍存在，因此扫描删掉 cursor 目录后仍可向后继续；非法 cursor 或 limit 返回 400。各页的顶层直接素材计数都描述整个 parent，与分页范围无关。

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

## 验证

先构建前端及 Rust 可执行程序，然后运行：

```sh
python3 tests/api_smoke.py --binary target/debug/picsoc
```

Windows 使用 `python` 和 `target/debug/picsoc.exe`。测试启动真实服务，只操作临时目录；覆盖目录添加、中文搜索、GIF、缩略图、收藏与标签、批量事务回滚、空目录与多层目录树、当前层/递归范围、方向比例与像素/字节范围的组合分页、非法筛选、文件变更、重启持久化、字节范围请求、路径边界以及移除素材库保留原图。Windows 环境跳过需要系统权限的符号链接测试。
