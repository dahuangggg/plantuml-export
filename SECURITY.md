# Security Policy

## Supported versions

Security fixes are provided on a best-effort basis for the latest published
PlantUML Export release or prerelease. Unpublished source candidates are not a
supported distribution channel.

## Report a vulnerability privately

Before the first public release, the repository owner must enable GitHub private
vulnerability reporting. Once the **Security** tab offers **Report a
vulnerability**, use it to open a private GitHub Security Advisory. Do not open a
public issue for a suspected vulnerability, leaked credential, private diagram,
or sensitive local path. Publication is blocked while that private channel is
unavailable.

Include the affected version/commit, operating system and architecture, the
smallest sanitized reproduction, expected impact, and whether managed downloads,
remote includes, path handling, or the Zed adapter are involved. Never attach
real secrets or confidential diagram source.

The renderer/network trust boundary and known residual risks are documented in
[docs/security.md](docs/security.md).

## 中文

最新已发布版本或候选版本的安全修复按最大努力提供；未发布源码候选不属于受支持
的分发渠道。

首次公开发布前，仓库所有者必须开启 GitHub private vulnerability reporting。
当 **Security** 页面出现 **Report a vulnerability** 后，请通过它创建私有
GitHub Security Advisory；该私有渠道不可用时禁止发布。疑似漏洞、泄露凭据、
私有图表或敏感本地路径不要提交到公开 Issue。请提供受影响版本/commit、系统与
架构、最小脱敏复现、预期影响，以及问题是否涉及 managed 下载、远程 include、
路径处理或 Zed 适配器；不要附带真实密钥或机密图表源码。
