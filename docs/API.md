# HTTP API

前端与 API 使用同一地址，默认是 `http://127.0.0.1:3210`。请求和响应使用 UTF-8 JSON；媒体接口返回图片字节。设置 `PICSOC_PASSWORD` 后所有路由都需要 HTTP Basic Auth，用户名固定为 `picsoc`。

## 接口

| 方法 | 路径 | 行为 |
| --- | --- | --- |
| GET | `/api/health` | `{ok: true, version: "0.1.0"}` |
| GET | `/api/stats` | 图片数量、总字节数、收藏数量、素材库数量 |
| GET | `/api/directories?path=...` | 浏览服务机器的现有子目录，用于文件夹选择 |
| GET | `/api/libraries` | `{libraries: Library[]}` |
| POST | `/api/libraries` | 传入 `{name, path}`，添加服务所在机器的绝对目录，返回 `201` 和 Library，后台开始扫描 |
| DELETE | `/api/libraries/{id}` | 删除索引及缩略图缓存，保留原文件，返回 `{ok: true}` |
| POST | `/api/libraries/{id}/scan` | 开始增量扫描，返回 `{ok: true}` |
| POST | `/api/libraries/{id}/scan/cancel` | 请求停止扫描，已经建立的索引保留 |
| GET | `/api/libraries/{id}/folders?parent=...` | 浏览已索引素材的直属子目录及递归图片数量 |
| GET | `/api/assets` | 分页搜索，返回 `{assets: Asset[], total, offset, limit}` |
| POST | `/api/assets/batch` | 批量收藏、添加或移除标签，返回 `{updated: number}` |
| GET | `/api/assets/{id}` | 获取 Asset |
| PATCH | `/api/assets/{id}` | 传入 `{favorite?: boolean, tags?: string[]}`，返回更新后的 Asset |
| GET | `/api/assets/{id}/thumbnail` | 返回磁盘缓存的 PNG；首次访问可能等待生成，无法解码时返回 SVG 占位图 |
| GET | `/api/assets/{id}/original` | 流式返回原文件，GIF 保留动画，支持单段 `Range` 请求 |
| GET | `/api/tags` | `{tags: [{name, count}]}` |

失败响应通常是 `{error: string, code: string}`，配合 `400`、`401`、`403`、`404` 或 `500` 状态码。`code` 是稳定的错误标识；说明语言通过请求的 `Accept-Language` 选择（`zh-CN` 或 `en`，默认中文）。素材、目录与标签的原名称不参与翻译。请求体限制为 64 KiB。不存在的 API 返回 `404`。

目录选择接口返回 `{path, parent, roots, directories, truncated}`，`roots` 和 `directories` 内的项目是 `{name, path}`。不传 `path` 时返回用户目录及系统根目录/Windows 盘符；传入绝对目录路径时返回该目录下的子文件夹。列表隐藏点开头的目录，仍可直接填写它们的路径；不列出文件及目录符号链接。单次最多列出 1000 个子目录，更多时 `truncated` 为 `true`。

已索引子目录接口返回 `{folders, parent, truncated}`。省略 `parent` 或传入空字符串表示库根目录，`folders` 是当前层级的直属子目录，每项为 `{path, name, parent, asset_count}`。`asset_count` 包含后代目录；没有已索引图片的空目录不显示。列表最多 1000 项，更多时 `truncated` 为 `true`。库根目录下的项目 `parent` 为 `null`，其他项目的 `parent` 为上级相对路径。直接把返回的 `path` 编码为下次请求的参数；路径使用服务平台的分隔符，客户端不要自行按 `/` 拆分。绝对路径或含 `..` 的路径返回 `400`。

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
| `library_id` | 指定素材库 |
| `folder` | 库内相对目录，包含后代目录，必须同时指定 `library_id`；空字符串表示整个库 |
| `favorite` | `true` 仅收藏，`false` 仅未收藏 |
| `format` | `jpg`、`png`、`gif`、`webp`、`bmp`、`tiff` |
| `tag` | 完整匹配标签 |
| `sort` | `modified`（默认）、`name`、`size` |
| `offset` | 从 0 开始的偏移 |
| `limit` | 默认 100，限制在 1–200 |

例如：`/api/assets?q=%E9%A3%8E%E6%99%AF&favorite=true&limit=100`。参数可以组合使用。相同排序值通过素材 ID 保持稳定顺序。

## 验证

先构建前端及 Rust 可执行程序，然后运行：

```sh
python3 tests/api_smoke.py --binary target/debug/picsoc
```

Windows 使用 `python` 和 `target/debug/picsoc.exe`。测试启动真实服务，只操作临时目录；覆盖目录添加、中文搜索、GIF、缩略图、收藏与标签、批量事务回滚、分层目录及递归筛选、文件变更、重启持久化、字节范围请求、路径边界以及移除素材库保留原图。Windows 环境跳过需要系统权限的符号链接测试。
