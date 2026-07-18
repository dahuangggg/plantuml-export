# Changelog

All notable user-facing changes to PlantUML Export are recorded here. A version
is released only after its GitHub tag and assets are public; entries under
Unreleased describe candidate scope, not published availability.

## Unreleased

### Candidate: v0.1.0-rc.1

Status: source candidate only. The tag and six native assets are not assumed to
exist, and `release/native-helper-release.json` remains `unpublished` until the
release assets and checksums have been independently verified.

#### Added

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

#### Security and privacy

- Public remote includes use PlantUML `INTERNET` by default; allowlist and
  disabled modes monotonically restrict access.
- Trusted remote origins are user-only, machine-global authorizations. Project
  and Zed worktree settings cannot add them.
- Rendering remains local with no telemetry, server renderer, or source upload.

#### Known boundaries

- GitHub-only does not mean Zed Gallery availability. Source users install with
  Zed's **Install Dev Extension** flow.
- While helper metadata is `unpublished`, a source checkout requires a locally
  built `plantuml-export` on Zed's `PATH`. A later, separately reviewed metadata
  commit enables verified helper download without moving the RC tag.
- First managed use downloads approximately 66–78 MiB. Live preview, WebViews,
  export-on-save, file watching, Markdown-block export, and server rendering are
  outside v0.1.
- RC1 GNU/Linux binaries are verified on Ubuntu 24.04; broader distribution and
  older-glibc compatibility are not yet claimed.

## 中文

所有面向用户的重要变化都记录在这里。只有 GitHub tag 和资产真正公开后，
对应版本才算发布；“未发布”部分描述候选范围，不代表资产已经可用。

### 候选版本：v0.1.0-rc.1

状态：目前只是源码候选。这里不假定 tag 和六个平台原生资产已经存在；在
Release 资产和 checksum 完成独立验证前，`release/native-helper-release.json`
会保持为 `unpublished`。

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
- Helper metadata 为 `unpublished` 时，源码 checkout 要求 Zed 的 `PATH` 中存在
  本地构建的 `plantuml-export`。之后会用单独审核的 metadata 提交启用 helper
  自动下载，不移动 RC tag。
- 首次 managed 使用会下载约 66–78 MiB。实时预览、WebView、保存时导出、文件
  监视、Markdown 代码块导出和服务器渲染不在 v0.1 范围内。
- RC1 GNU/Linux 二进制只在 Ubuntu 24.04 上验证，暂不承诺其他发行版或旧版
  glibc 兼容性。
