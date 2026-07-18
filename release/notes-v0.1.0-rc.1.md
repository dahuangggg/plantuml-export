# PlantUML Export v0.1.0-rc.1

<!-- Prepared release body; this file alone does not imply that a tag or assets exist. -->

This is the first GitHub-only release candidate for the native PlantUML Export
toolchain and its thin Zed language extension. It is intended for release
validation and early feedback, not as a final v0.1.0 stability promise.

## Highlights

- Export saved PlantUML files to SVG, PNG, or PDF from the CLI or Zed Code
  Actions.
- Use managed PlantUML 1.2026.6, Temurin JRE 21.0.11+10, and Smetana without
  installing Java or Graphviz separately.
- Keep SVG, PNG, and PDF for the same source without one format overwriting
  another.
- Keep manifests, locks, staging, journals, and backups out of the user-visible
  output directory.
- Run local diagnostics and cancellable saved-file checks through the same
  native helper used for export.

## Distribution and installation

This release line is GitHub-only and is not listed in the Zed Gallery. Source
extension users must clone the repository, install the pinned Rust toolchain and
`wasm32-wasip2` target with `rustup`, then choose **Zed: Install Dev Extension**.
See the [README source-checkout instructions](https://github.com/dahuangggg/plantuml-export/blob/v0.1.0-rc.1/README.md#install-this-source-checkout-as-a-zed-dev-extension).

The source candidate intentionally contains `unpublished` helper metadata.
Therefore, its dev extension needs a locally built `plantuml-export` on Zed's
`PATH`. Only after the Release assets pass consumer verification will a separate
follow-up commit add their exact URLs and checksums. A checkout containing that
`published` metadata ignores `PATH` and downloads the verified helper itself;
the immutable RC tag is not moved.

If the RC workflow succeeds, the Release is expected to contain exactly these
native CLI/helper assets plus `SHA256SUMS`:

- `plantuml-export-aarch64-apple-darwin`
- `plantuml-export-x86_64-apple-darwin`
- `plantuml-export-aarch64-unknown-linux-gnu`
- `plantuml-export-x86_64-unknown-linux-gnu`
- `plantuml-export-aarch64-pc-windows-msvc.exe`
- `plantuml-export-x86_64-pc-windows-msvc.exe`

Do not install an asset until its release, attestation, and matching
`SHA256SUMS` entry have been verified. The exact conditional commands are in the
[README CLI instructions](https://github.com/dahuangggg/plantuml-export/blob/v0.1.0-rc.1/README.md#install-the-optional-cli-after-rc1-is-published).

## First use and known boundaries

- The first managed export, check, or LSP startup downloads approximately
  66–78 MiB of pinned PlantUML and Temurin assets. Later uses reuse the installed
  user cache under the documented structural checks; offline first use cannot
  provision missing assets.
- Files must be saved before Zed export. The v0.1 scope excludes live preview,
  WebViews, export-on-save, file watching, Markdown-block export, and server
  rendering.
- Public remote includes work without configuration through PlantUML's upstream
  `INTERNET` checks. They reduce SSRF exposure but are not a strong network
  sandbox or an absolute defense against DNS rebinding.
- Rendering is local. There is no telemetry, remote renderer, or diagram-source
  upload; remote include hosts still receive ordinary HTTP requests.
- Only the six listed OS/architecture targets are release contracts for RC1.
- The Linux assets are built and smoke-tested on Ubuntu 24.04 with glibc. Other
  GNU/Linux distributions may work but are not verified RC1 compatibility
  targets.

## Suggested RC validation

- Install only the Zed dev extension in a clean profile and, after published
  metadata exists, verify SVG/PNG/PDF Code Actions with no standalone CLI, Java,
  or Graphviz installed.
- Run the downloaded host CLI's `version`, `health`, `check`, and SVG/PNG/PDF
  export commands in a clean directory.
- Report the platform, selected asset name, command, and sanitized error through
  GitHub Issues. Do not publish private diagrams, local paths, or user config.

## 中文

这是原生 PlantUML Export 工具链及其轻量 Zed 语言扩展的第一个 GitHub-only
候选版本，用于发布验证和早期反馈，不代表最终 v0.1.0 稳定性承诺。

### 亮点

- 从 CLI 或 Zed Code Action 把已保存的 PlantUML 文件导出为 SVG、PNG 或 PDF。
- 使用受管 PlantUML 1.2026.6、Temurin JRE 21.0.11+10 和 Smetana，无需单独
  安装 Java 或 Graphviz。
- 同一源文件的 SVG、PNG 和 PDF 可以同时保留，不会互相覆盖。
- Manifest、锁、staging、journal 和备份不会出现在用户可见输出目录中。
- 诊断、保存检查和导出使用同一个原生 helper。

### 分发与安装

本版本只通过 GitHub 分发，没有上架 Zed Gallery。源码扩展用户必须 clone
仓库，通过 `rustup` 安装固定 Rust 工具链和 `wasm32-wasip2` target，再选择
**Zed: Install Dev Extension**。参阅
[README 源码安装说明](https://github.com/dahuangggg/plantuml-export/blob/v0.1.0-rc.1/README.zh-CN.md)。

源码候选刻意保留 `unpublished` helper metadata，因此开发扩展需要 Zed 的
`PATH` 中存在本地构建的 `plantuml-export`。只有 Release 资产通过消费者验证后，
才会由后续独立提交写入准确 URL 和 checksum。包含 `published` metadata 的
checkout 会忽略 `PATH` 并自动下载、验证 helper；不可变 RC tag 不会移动。

如果 RC 工作流成功，Release 应当恰好包含上方列出的六个原生 CLI/helper 资产
和 `SHA256SUMS`。验证 Release、attestation 和对应 checksum 之前不要安装；条件
式命令见 [README CLI 说明](https://github.com/dahuangggg/plantuml-export/blob/v0.1.0-rc.1/README.zh-CN.md)。

### 首次使用与已知边界

- 首次 managed 导出、检查或 LSP 启动会下载约 66–78 MiB 的固定 PlantUML 和
  Temurin 资产；之后按照文档中的结构检查复用已安装用户缓存，离线首次使用
  无法补齐缺失资产。
- Zed 导出前必须保存文件。v0.1 不包含实时预览、WebView、保存时导出、文件
  监视、Markdown 代码块导出或服务器渲染。
- 公网远程 include 通过 PlantUML 上游 `INTERNET` 检查实现零配置使用；它会
  降低 SSRF 风险，但不是强网络沙箱，也不能绝对防御 DNS rebinding。
- 渲染在本地完成，没有遥测、远程渲染器或图表源码上传；远程 include 主机
  仍会收到普通 HTTP 请求。
- RC1 只承诺上方六种操作系统/架构组合。
- Linux 资产在 Ubuntu 24.04 + glibc 上构建并完成真实冒烟；其他 GNU/Linux
  发行版可能可用，但不是经过验证的 RC1 兼容目标。

### 建议的 RC 验证

- 在干净 Zed profile 中只安装开发扩展；当 published metadata 存在后，在没有
  独立 CLI、Java 或 Graphviz 的环境中验证 SVG/PNG/PDF Code Action。
- 在干净目录中运行下载的当前平台 CLI 的 `version`、`health`、`check` 和
  SVG/PNG/PDF 导出。
- 通过 GitHub Issues 报告平台、资产名、命令和已脱敏错误；不要公开私有图表、
  本地路径或用户配置。
