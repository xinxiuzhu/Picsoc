# 安全说明 / Security

## 部署边界

Picsoc 当前面向个人使用或可信设备间共享，使用一个服务、一个 SQLite 数据库及可选的共享密码。它没有独立用户账号、角色权限或租户隔离。

默认原生服务只监听 `127.0.0.1:3210`。Docker 示例仅将端口发布到宿主机回环地址，容器使用非特权用户，原图目录只读挂载。服务会检查请求的 Host 与 Origin；这些检查不能代替身份认证、系统权限和网络访问控制。

设置非空 `PICSOC_PASSWORD` 后，所有路由使用 HTTP Basic Auth，用户名为 `picsoc`。密码为空时禁用认证。HTTP 没有传输加密；通过其他设备或公网使用时应配置 HTTPS，并保持后端端口仅对受控代理或可信网络可达。部署示例见 [运维指南](docs/OPERATIONS.md)。

能够访问并通过认证的用户可以浏览服务机器的目录、添加该进程有权限读取的素材库、查看和下载原图、修改收藏与标签、移除索引。没有每用户的目录白名单；不要将密码分享给不受信任的人。容器只挂载需要索引的素材目录，有助于缩小服务可见的文件范围。

原图只读索引；移除素材库不会删除原图。递归扫描跳过目录内的符号链接，并检查媒体路径是否仍处于素材目录中。图片解码有大小和像素限制，但它们不是进程总内存限额，也不保证任意损坏文件都能安全处理。使用低权限系统用户、保持依赖更新，并避免导入不可信来源的大量文件。

数据库、标签、路径和缩略图以本机普通文件存储，没有应用层加密。原图下载保留文件内容及其中的元数据。备份、`.env`、日志和数据目录应使用合适的文件权限；不要将真实素材或凭据提交到仓库。

## 私密漏洞报告

请不要在公开 Issue、PR 或讨论中发布可利用的细节、个人素材或密码。

优先打开仓库的 [Security 页面](https://github.com/xinxiuzhu/Picsoc/security)，若显示 **Report a vulnerability**，使用 GitHub 的私密漏洞报告提交复现步骤。该入口是否可用取决于仓库维护者是否启用了私密报告；当前文档不声称已启用。

若私密入口不可用，可以创建只写“需要私密安全联系方式”的 Issue，请维护者提供私密渠道，暂时不披露漏洞详情。本项目目前没有公布专用安全邮箱，也没有承诺固定响应时限。

报告宜包含：受影响的 commit/版本、系统与运行方式、精简复现步骤、影响和必要日志。去掉真实路径、素材、认证头与密码。

## 维护状态

0.1.0 目前处于未发布开发阶段，没有已发布版本的支持周期承诺。修复记录会写入 [CHANGELOG.md](CHANGELOG.md)；正式版本发布状态以 GitHub Releases 和 Actions 为准。

## English

Picsoc is a personal or trusted-network application with an optional shared Basic Auth password (`picsoc` is the username). It has no per-user roles, library isolation, or application-level encryption. Anyone with access can add image directories readable by the service, download originals, and edit library metadata. Use a low-privilege account, mount only the required folders, protect backups and credentials, and use HTTPS for access across devices or untrusted networks.

The default native listener and the Docker host port are loopback-only. Host/Origin checks, read-only indexing, symlink exclusions, and decoder limits reduce specific risks; they do not replace authentication, operating-system permissions, or total process memory limits.

Do not disclose exploitable details in public issues. Use **Report a vulnerability** on the [repository Security page](https://github.com/xinxiuzhu/Picsoc/security) if private reporting is enabled. If it is unavailable, open an issue asking only for a private contact channel and withhold the details. No dedicated security email or response deadline is currently published. Include a minimal reproduction and affected version, without personal files or secrets. Version 0.1.0 has not been released.
