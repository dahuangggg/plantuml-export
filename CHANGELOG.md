# Changelog

All notable user-facing changes to PlantUML Export are recorded here. A version
is released only after its GitHub tag and assets are public; entries under
Unreleased describe default-branch changes not included in an immutable tag.

## Unreleased

### Changed

- The current default branch now contains the separately reviewed `published`
  metadata for all six verified `v0.1.0-rc.1` helpers. A GitHub/dev-installed Zed
  extension downloads and verifies its matching helper without `cargo install`
  or a standalone CLI on `PATH`. This follow-up does not move the immutable tag.

## v0.1.0-rc.1 - 2026-07-19

Status: published GitHub prerelease with six native assets, `SHA256SUMS`, and
artifact attestations. The immutable tag intentionally contains `unpublished`
helper metadata; the default-branch follow-up above enables automatic helper
download from those independently verified assets.

### Added

- Native `plantuml-export` commands for SVG, PNG, and PDF export, syntax checks,
  health reporting, version reporting, and the internal stdio LSP.
- A managed renderer that provisions checksum-pinned PlantUML 1.2026.6 and
  Eclipse Temurin JRE 21.0.11+10 without requiring user-installed Java or
  Graphviz for the default Smetana layout.
- Zed SVG/PNG/PDF Code Actions, structural diagnostics, and cancellable checks
  for saved PlantUML files.
- Transactional export with per-format ownership, rollback, private application
  state, and final-artifact-only output directories.
- A six-platform GitHub prerelease workflow with `SHA256SUMS` and GitHub artifact
  attestations.

### Security and privacy

- Public remote includes use PlantUML `INTERNET` by default; allowlist and
  disabled modes monotonically restrict access.
- Trusted remote origins are user-only, machine-global authorizations. Project
  and Zed worktree settings cannot add them.
- Rendering remains local with no telemetry, server renderer, or source upload.

### Known boundaries

- GitHub-only does not mean Zed Gallery availability. Source users install with
  Zed's **Install Dev Extension** flow.
- The immutable RC1 tag preserves the reviewed `unpublished` source candidate.
  Clone the current default branch for the follow-up metadata that installs the
  verified helper automatically; no standalone CLI is required by that flow.
- First managed use downloads approximately 66–78 MiB. Live preview, WebViews,
  export-on-save, file watching, Markdown-block export, and server rendering are
  outside v0.1.
- RC1 GNU/Linux binaries are verified on Ubuntu 24.04; broader distribution and
  older-glibc compatibility are not yet claimed.

## 中文

所有面向用户的重要变化都记录在这里。只有 GitHub tag 和资产真正公开后，
对应版本才算发布；“未发布”部分记录尚未包含在不可变 tag 中的默认分支变更。

### 未发布

#### 变更

- 当前默认分支已经包含经过单独审核的 `published` metadata，固定
  `v0.1.0-rc.1` 六个平台已验证 helper。通过 GitHub 安装的 Zed 开发扩展会自动
  下载并验证匹配 helper，不需要 `cargo install`，也不要求 `PATH` 中存在独立
  CLI。这次后续提交不会移动不可变 tag。

### v0.1.0-rc.1 - 2026-07-19

状态：GitHub 预发布已经发布，包含六个平台原生资产、`SHA256SUMS` 和 artifact
attestation。不可变 tag 按设计保留 `unpublished` helper metadata；上方的默认
分支后续变更使用这些经过独立验证的资产启用 helper 自动下载。

#### 新增

- 原生 `plantuml-export`：支持 SVG/PNG/PDF 导出、语法检查、健康检查、版本
  输出和内部 stdio LSP。
- Managed 渲染器：固定 PlantUML 1.2026.6 和 Eclipse Temurin JRE
  21.0.11+10；默认 Smetana 不要求用户安装 Java 或 Graphviz。
- Zed SVG/PNG/PDF Code Action、结构诊断，以及针对已保存 PlantUML 文件的可取消
  检查。
- 带每种格式独立所有权、回滚、私有应用状态和纯最终产物输出目录的事务式导出。
- 面向六个平台、生成 `SHA256SUMS` 和 GitHub artifact attestation 的 GitHub
  预发布工作流。

#### 安全与隐私

- 公网远程 include 默认使用 PlantUML `INTERNET`；allowlist 和 disabled 模式
  只能单调收紧访问。
- 受信任远程 origin 是用户级、机器全局授权；项目和 Zed 工作树设置不能新增。
- 渲染始终在本地完成，没有遥测、服务器渲染或源码上传。

#### 已知边界

- GitHub-only 不等于已上架 Zed Gallery；源码用户通过 Zed 的
  **Install Dev Extension** 流程安装。
- 不可变 RC1 tag 保留经过审核的 `unpublished` 源码候选。要使用自动安装已验证
  helper 的后续 metadata，请 clone 当前默认分支；该流程不要求独立 CLI。
- 首次 managed 使用会下载约 66–78 MiB。实时预览、WebView、保存时导出、文件
  监视、Markdown 代码块导出和服务器渲染不在 v0.1 范围内。
- RC1 GNU/Linux 二进制只在 Ubuntu 24.04 上验证，暂不承诺其他发行版或旧版
  glibc 兼容性。
