# Picsoc 图片素材库

Picsoc 在本机运行 Rust 服务，通过浏览器提供中文图片素材库。源代码和使用说明：https://github.com/xinxiuzhu/Picsoc

## 启动

Windows：双击 `picsoc.exe`。也可以在 PowerShell 中运行：

```powershell
.\picsoc.exe --data-dir "$env:LOCALAPPDATA\Picsoc"
```

macOS / Debian：在终端切换到解压目录并运行：

```sh
./picsoc --data-dir ./picsoc-data
```

程序会打开 http://127.0.0.1:3210 。在界面添加本机图片目录的绝对路径，等待后台扫描生成缩略图。服务需要保持运行，按 `Ctrl+C` 可以停止。关闭浏览器后，可以重新打开上述地址。

Linux 包以 Debian 12 为构建基线，其他版本需要在目标机器验证。macOS 的 Intel 与 Apple Silicon 使用不同发行包；当前 macOS 包未签名或公证，如系统拦截，请在系统设置的“隐私与安全性”中允许可信来源的程序运行。

## 素材与数据

支持 JPEG、PNG、GIF、WebP、BMP、TIFF。原图只读索引，不会复制、移动或修改。数据库、标签和缩略图保存在 `--data-dir` 指定的目录；不指定时使用当前用户应用数据目录：Windows 为 `%LOCALAPPDATA%\Picsoc`，macOS 为 `~/Library/Application Support/Picsoc`，Linux 为 `$XDG_DATA_HOME/picsoc` 或 `~/.local/share/picsoc`。启动时会打印实际路径。保留数据目录即可在重启后继续使用已有素材库。

默认每 300 秒增量扫描，也可以在界面手动重扫。数据备份应在停止服务后复制整个数据目录；原图需要另外备份。

## 低配置运行

默认图片处理并发为 1，建议低配置机器保持此值：

```sh
./picsoc --workers 1 --scan-interval 600
```

`--workers` 可选 1–4，`--scan-interval 0` 关闭定时扫描。超过 256 MiB 文件大小、128 MiB 解码数据或 32,768 像素宽/高的图片会跳过缩略图。解码限制和并发数都不是总内存限制。

## 其他参数

- `--bind 127.0.0.1:3210`：监听地址，默认仅本机访问。
- `--data-dir 路径`：固定数据库与缩略图目录。
- `--no-open`：启动时不自动打开浏览器。
- `--version`：查看版本。
- `--help`：查看完整帮助。

局域网访问时使用 `--bind 0.0.0.0:3210`，并通过环境变量 `PICSOC_PASSWORD` 设置非空密码，登录用户名为 `picsoc`。HTTP Basic Auth 不提供传输加密，公网访问需通过 HTTPS 反向代理。完整的 Docker 与开发说明见仓库 README。

许可证：Apache License 2.0，见同目录 `LICENSE`。
