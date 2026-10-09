# 变更记录 / Changelog

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
