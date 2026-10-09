# 贡献指南 / Contributing

感谢帮助改进 Picsoc。项目范围是可自托管的图片与 GIF 素材库：只读索引现有原图，提供浏览、搜索、收藏和标签。开始较大改动前，可先在 Issue 描述使用场景和预期行为。

## 本地开发

使用 Rust 1.96.1、Node.js 22、npm 和 Python 3.12，与 CI 保持一致。不需要另外安装 SQLite 或 libvips。

先构建前端，Rust 才能嵌入它：

```sh
cd frontend
npm ci
npm run build
cd ..
cargo run --locked -- --no-open --data-dir ./picsoc-data
```

界面开发时，另开终端运行 `cd frontend && npm run dev`，访问 Vite 输出的地址。开发代理将 API 转发到 `127.0.0.1:3210`；生产程序仍只需要 Rust 服务的一个端口。Windows 可使用同样的 npm/Cargo 命令；使用 Python 时将下文 `python3` 改为 `python`，可执行文件加上 `.exe`。

提交前运行：

```sh
cd frontend
npm run build
cd ..
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
python3 tests/api_smoke.py --binary target/debug/picsoc
```

API smoke 测试自行启动真实服务，使用临时端口和临时素材目录；它不需要已有服务或个人图片。新增 API 行为应补充对应测试和 [API 文档](docs/API.md)。提交前查看 `git diff --check`。

## 文案与界面

界面支持简体中文和英文。用户可见文案、无障碍标签、提示与错误需要使用翻译 key；同步修改两种语言资源，保留插值变量。运行 `npm run check:i18n`，并实际检查两种语言、空状态、文件夹选择和预览对话框。详见 [i18n 指南](docs/I18N.md)。

素材名称、目录路径和用户标签保持原文。新增图片功能应保留 GIF 原图动画、原图只读边界和低并发默认值。

## 提交与依赖

PR 说明应写清问题、最终行为和实际验证结果。仅在相关工具真正运行成功后声明通过；本机 macOS 检查不能代替 Windows、Debian 或 Docker 验证。CI 状态见 [Actions](https://github.com/xinxiuzhu/Picsoc/actions)。

提交 `Cargo.lock` 与 `frontend/package-lock.json`；依赖升级应说明必要性并运行相关检查。不要提交个人素材、数据库、缓存、`.env`、密码或私有日志。贡献代码适用仓库的 [Apache License 2.0](LICENSE)。

## English

Use Rust 1.96.1, Node.js 22, npm, and Python 3.12, matching CI. Build the frontend before compiling Rust, because its assets are embedded in the executable. The commands above run formatting, Clippy, Rust tests, a frontend production build, and the API smoke test against temporary files. On Windows, use `python` and `target/debug/picsoc.exe`.

For frontend development, run the Rust service on port 3210 and `npm run dev` in a second terminal. The development proxy forwards API requests to that service.

Keep changes within the image/GIF library scope and preserve original files. Add tests and API documentation for new behavior. Translate visible text and accessibility labels in both language dictionaries, keep interpolation variables unchanged, and check both layouts. See [the translation guide](docs/I18N.md).

Describe the problem, resulting behavior, and checks you actually ran in the PR. Commit dependency lockfiles, but never personal images, runtime databases, credentials, or `.env`. Contributions are licensed under Apache 2.0. Report security issues through the private process in [SECURITY.md](SECURITY.md).
