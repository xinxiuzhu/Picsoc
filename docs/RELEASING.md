# 构建与发布

## 自动检查

`.github/workflows/ci.yml` 配置在 `main` push、pull request 或手动触发时运行，构建目标为 Windows x64、macOS Intel、macOS Apple Silicon 和 Debian 12。前端先执行 `npm ci` 与 `npm run build`，随后进行 Rust 格式、Clippy、测试、release 构建、版本命令以及 `tests/api_smoke.py` 的实际服务测试。端到端脚本自行启动临时端口上的服务，不需要已有实例。

Debian 的 Rust 检查和编译在 `rust:1.96.1-bookworm` 容器中完成。Docker job 还构建运行镜像，确认默认用户为 `10001:10001`，启动容器检查 `/api/health` 与嵌入的中文 Web 首页。普通 CI 不上传发行包、发布镜像或部署服务。

## 版本发布

先让 CI 通过，将 `Cargo.toml` 和 `frontend/package.json` 的版本更新为同一版本，同步 lockfile，再创建对应的 Git tag。例如第一版为 `v0.1.0`。

`release.yml` 仅在推送形如 `v0.1.0` 的 tag 时触发，并检查 tag 与 Cargo package version 一致。它会：

1. 在 Windows、macOS Intel 和 macOS Apple Silicon runner 上分别构建、测试；Linux 二进制通过 Dockerfile 的 Debian 12 builder 导出，将 glibc 构建基线固定在 Debian 12，不使用 Ubuntu runner 自身环境构建。
2. 将可执行程序、Apache 2.0 许可证和中文/英文运行说明打包，生成四个系统/CPU 对应的压缩包及 `SHA256SUMS.txt`。
3. 通过 GitHub Release 发布附件。
4. 将 `linux/amd64`、`linux/arm64` Docker 镜像上传至 `ghcr.io/仓库所有者/picsoc`。正式版本获得 `0.1.0` 与 `latest` 标签；带连字符的预发布版本仅获得自身版本标签，不覆盖 `latest`。

Windows 发行包无需 Docker。Debian 可以直接运行二进制或使用镜像；镜像不包含素材文件。Mac Intel 与 Apple Silicon 目前是分别发布的二进制，并非 universal bundle。

Windows x64 MSVC 目标在 `.cargo/config.toml` 中配置 `target-feature=+crt-static`，静态链接 C 运行时。CI 与发行 job 通过 `scripts/verify-windows-runtime.ps1` 列举包含预发布版本的已注册 Visual Studio 安装，按实际文件寻找 x64 `dumpbin.exe`，不依赖固定组件 ID；日志包含安装数量、工具候选及查询退出码。随后检查最终二进制不直接依赖 VCRUNTIME、MSVCP、CONCRT、ucrtbase 或 api-ms-win-crt DLL；工具缺失、检查失败或发现依赖都会使 job 失败。该设置参考 [Rust 官方链接说明](https://doc.rust-lang.org/reference/linkage.html#static-and-dynamic-c-runtimes)。仍以实际构建检查及目标电脑运行为准；Windows 自身的系统 DLL 仍是运行要求。

GitHub Release 与 GHCR 上传由独立 job 执行，可能出现其中一个发布成功而另一个失败。检查 Actions 结果后，对失败的 job 重跑；Release 上传支持覆盖同名附件。不要将成功创建 Release 等同于镜像已经上传。

## 权限与首发设置

默认 workflow 权限为 `contents: read`。只有 GitHub Release job 使用 `contents: write`，只有镜像发布 job 使用 `packages: write`；发布使用该次 workflow 的 `GITHUB_TOKEN`，不需要在仓库添加个人访问令牌。第三方 Actions 固定为已核对的完整 commit SHA，旁边注释保留对应发行版本。

首发后检查 GHCR package 的可见性；GitHub package 不一定自动公开，如需朋友无登录拉取，将该包设置为 public。macOS 发行包当前未进行 Apple 签名和公证。Windows 当前为可执行程序压缩包，不包含代码签名或安装器。

## 本地构建

```sh
./scripts/build.sh
docker build --target runtime --tag picsoc:local .
docker build --target binary --output type=local,dest=release-bin .
```

最后一条命令导出与运行镜像相同 builder 构建的 Linux 可执行文件。需在对应平台运行 Docker；Mac 上默认得到与其 Docker 架构一致的 Linux 二进制，要构建 x64 可额外指定 `--platform linux/amd64`。

Docker 镜像使用 Debian bookworm，Rust 编译器固定为 1.96.1。`RUST_VERSION` build argument 可以覆盖构建版本，但改动时应同时检查 CI 和发行 workflow 的版本。Node 构建阶段固定在 22 系列，npm 与 Cargo 的依赖使用已提交的 lockfile。

## 验证边界

YAML/Compose 静态校验只能验证配置结构。跨平台构建、镜像构建、自动上传和在干净系统上的实际运行需要 Actions 与目标系统验证。不要将本机 macOS 构建成功视为 Windows/Debian 的验证结果。
