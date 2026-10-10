# 变更记录 / Changelog

## 未发布 / Unreleased

- 首次启动在数据目录自动生成 `config.toml`，停止后编辑、重启生效；支持 `--config` 和显式 CLI 覆盖，已有配置不重写，非法配置直接拒绝启动。运行配置不再读取环境变量；Docker 同样使用持久化 TOML。
- Generate persistent TOML configuration on first startup, with explicit CLI overrides, validation and Unix `0600` creation. Runtime settings move from environment variables to `config.toml`, including Docker.

- 新增可选 MCP Streamable HTTP 服务，提供素材库与目录查询、搜索、带编号的实际图片拼版、字体查询和设计布局/合成工具；默认关闭，网页登录与 MCP 授权独立。
- 支持用于 ChatGPT 的 OAuth 发现、精确回调白名单、S256 PKCE、单次授权码、短期访问令牌和 refresh 轮转；凭据只持久化哈希，支持重启后继续授权。独立 Bearer token 用于支持该方式的本地/API 客户端。
- 新增 Rust PNG 合成：素材图层、矩形/圆角矩形和文字图层，透明通道、缩放模式、文字换行与对齐；单渲染线程与有界队列，先预览再导出原画布尺寸。
- 布局保存为不可变 JSON 版本，更新需匹配 `expected_revision`，避免覆盖他人修改；已保存布局和合成成品可继续查看、下载和修改，原素材保持只读。
- Add an opt-in MCP server with independent OAuth/Bearer authentication, actual image previews, bounded Rust PNG composition, and persistent editable layout revisions. Server deployment and authorization in a ChatGPT account remain explicit setup steps.

- 左侧目录树与扫描跳过 `.git` 等隐藏点目录，保留普通空目录；旧隐藏素材在成功重扫后清理索引，原文件保留。
- 支持桌面侧栏收起/展开和独立的手机抽屉；新增每行缩略图数量调节，图片保持完整显示，浏览器记住偏好。
- Skip hidden dot-directories and add persistent sidebar and thumbnail display controls, while preserving virtualized browsing.
- 新增简洁共享密码登录页和退出登录；会话过期自动返回登录，保留脚本 Basic Auth 兼容。
- 语言选择改为统一菜单样式，支持键盘；修复非法网格密度缓存和目录刷新丢失已加载页数。
- 缩略图只缩小、不放大小尺寸素材，避免小图导入产生额外处理和缓存；继续复用已生成的缓存。
- Add a shared-password login page with expiring browser sessions and logout, and replace the native language select with a styled menu.
- 新增左侧素材库目录树，扫描并持久化真实层级与空文件夹；支持仅当前目录或包含子目录，旧索引自动迁移。
- 新增方向、常用/自定义宽高比、像素宽高、文件大小范围和更多排序，可与已有搜索、标签、收藏、格式组合。
- 支持文件名排除关键词，例如 `map` 隐藏名称中包含该词的素材，可逐词移除并与其他条件组合，原文件保留。
- 启动时明确显示实际监听地址，并在 IPv4 全地址监听时自动列出网卡 IPv4 访问链接。
- Add persistent folder trees (including empty directories), direct/recursive browsing, composable orientation/ratio/dimension/size filters, and detected IPv4 access URLs.

- 修复新克隆仓库直接 `cargo run` 时缺少 `frontend/dist` 的编译失败；Cargo 自动构建并嵌入网页，源文件变化自动重建，并保留 Docker/发行的预构建模式。
- Add automatic frontend builds to Cargo, including fresh-checkout regression checks and explicit prebuilt frontend support for Docker and release jobs.

- 重设计 Web 界面：macOS 风格浅色侧栏、蓝色操作按钮、集中搜索、完整图片网格、文件夹导入面板与大图预览，适配窄屏并保留中英文。
- 支持 `⌘K` / `Ctrl+K` / `/` 聚焦搜索，改善弹窗键盘焦点与移动端预览关闭入口。
- Redesign the Web UI with a macOS-inspired sidebar, streamlined search and import, image-first browsing, responsive previews, and keyboard shortcuts in Chinese and English.
- 扫描普通图片目录时跳过 Apple Photos `.photoslibrary` 图库，避免其读取权限错误使整个素材库显示扫描异常；保留旧图库索引、收藏和标签。
- 直接选择图库包或其内部目录时提供中英文导出提示，文件夹选择器不再列出图库包；普通目录权限错误仍保留索引并报告异常。
- Skip Apple Photos library packages when scanning image folders, retain existing package annotations, and provide localized export guidance when selecting unsupported Photos library paths.

## 0.1.0

下列记录描述 0.1.0 代码中的功能。各平台验证见 [Actions](https://github.com/xinxiuzhu/Picsoc/actions)，发行包和容器镜像的可用状态以实际发布结果为准；版本附件见 [Releases](https://github.com/xinxiuzhu/Picsoc/releases)。

- Rust 本地 HTTP 服务与嵌入式 React Web 界面，一个端口即可使用。
- 从服务机器上的已有文件夹添加素材库，支持文件夹选择与手填绝对路径。
- 递归索引 JPEG、PNG、GIF、WebP、BMP、TIFF，原图保持原位置和内容。
- SQLite 持久化索引、收藏与标签；缩略图写入独立数据目录。
- 图片网格按需加载与虚拟滚动，支持关键词、格式、收藏及标签筛选和排序。
- 多选素材后批量收藏、添加/移除标签；每批最多 500 项，元数据修改在事务中全量成功或回滚。
- 按素材库的已索引子文件夹浏览，递归筛选目录及后代图片，目录为空时不显示。
- 图片详情、标签编辑、相对路径复制及原图下载；GIF 原图保留动画，TIFF 使用缩略图预览。
- 手动重扫、扫描取消界面/API 与周期增量扫描，默认图片解码并发为 1。
- 简体中文与英文界面切换、浏览器语言偏好保存、按请求语言返回 API 系统消息。
- 可选共享密码认证、Host/Origin 检查、只读原图目录及媒体路径边界检查。
- Windows x64、macOS Intel/Apple Silicon、Debian 12 原生构建检查与 Docker 构建工作流。
- 版本 tag 触发的发行包、SHA-256 校验和及 GHCR 镜像发布工作流。
- 开发、API、翻译、发布、安全和备份恢复文档。

### English

Version 0.1.0 provides an embedded Web interface, read-only image/GIF indexing, folder browsing, search and recursive folder filters, favorites and tags, atomic batch metadata changes, persistent SQLite metadata, cached thumbnails, original downloads, periodic scans, Chinese/English localization, and optional shared-password authentication.

Cross-platform build and tag-release workflows are configured. Their status must be checked in Actions; source availability is not proof of a successful release. See the README and operations guide for current limits.
