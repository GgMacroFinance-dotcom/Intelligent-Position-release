<!-- 草稿（发布前清单 R-10）。标了“待维护者定”的地方定下来以后，删掉这一行。 -->
# 参与贡献

谢谢你愿意花时间。intelligent-position 还在早期，接口和结构都可能变。动手之前先看这一页，可以少走弯路。

## 先看哪里

- 这是什么、现在真实做到哪一步：[`README.md`](README.md)。
- 还没做完的事，按建议的先后排好了：[`docs/ip/backlog.md`](docs/ip/backlog.md)。
- 已经敲定的决定：[`docs/ip/project-spec.md`](docs/ip/project-spec.md)。
- harness 的结构、本地检查、编码约定、敏感路径清单：[`rust/README.md`](rust/README.md)、[`rust/CONTRIBUTING.md`](rust/CONTRIBUTING.md)。
- 宏观数据工具（Node.js，MIT）：[`macro-mcp/README.md`](macro-mcp/README.md)。

## 提问题和建议

- 发现 bug、文档写错了、照 README 跑不起来：开 issue，用“问题报告”模板。请写清楚版本或提交号、系统、配置里相关的部分（**去掉密钥**）、你做了什么、看到了什么、本来以为会怎样。
- 想加功能或改设计：先开 issue 说想法，用“功能建议”模板，不要直接提一个大 PR。项目有既定的先后顺序（见 backlog），和它冲突的改动可能暂时不收。
- 安全问题不要开公开的 issue，见 [`SECURITY.md`](SECURITY.md)。

## 提 PR

1. 先看 backlog 和开放的 PR，同一件事不要做两遍。要接 backlog 里的某一条，先在对应的 issue 里说一声；没有 issue 就开一个。
2. 从 `main` 拉分支。一个 PR 只做一件事，尽量小。
3. 改 `rust/` 下的代码，提交前在 `rust/` 目录跑：

   ```sh
   cargo fmt --all --check
   cargo clippy --all-targets --locked
   cargo test --locked
   ```

   改 `macro-mcp/` 的，在那个目录跑 `npm test`。
4. PR 描述按模板写四段：做了什么、有没有动到敏感路径、怎么验证的、哪些还没验。
5. CI 通过后由维护者合并。动到敏感路径（工具和沙箱、令牌与鉴权、MCP、外部渠道、外部引擎，清单在 [`rust/CONTRIBUTING.md`](rust/CONTRIBUTING.md)）的 PR，要有一位维护者读过代码才合并，会慢一些。

## 几条硬规矩

- **照实写验证情况。** 没在真实环境跑过的东西，不要写“已上线”“已验证”。用假模型、假 Slack、假 Codex 测的，写明是假的。
- **密钥不进仓库。** 不要把 API key、令牌、密码写进代码、配置示例、PR、issue 或日志片段。密钥只放环境变量。
- **依赖的许可证要和 Apache-2.0 兼容**（MIT、Apache-2.0、BSD 等）。允许的清单在 `rust/deny.toml`，CI 会检查。
- **不要贴来源不明或许可证不兼容的代码。** 参考了别的开源项目的设计可以，在 PR 里写明参考了什么；移植了代码要保留原许可声明。
- **性能是功能的一部分。** 新增的热路径要在 `ip bench` 里有基准。
- 注释和文档用中文，标识符用英文。

## 许可和签名

- `rust/`、`homepage/` 和文档以 Apache-2.0 发布，`macro-mcp/` 以 MIT 发布。你提交的内容按所在目录的许可证发布。
- 用 DCO（[Developer Certificate of Origin](https://developercertificate.org/)）确认你有权提交这些内容：每个提交用 `git commit -s` 加上 `Signed-off-by` 一行。不需要签 CLA。CI 的 `dco` 工作流会检查 PR 里每个非合并提交都有这一行，缺了就不通过；补签用 `git rebase --signoff <目标分支>`。

## 行为准则

参与这个项目要遵守[行为准则](CODE_OF_CONDUCT.md)。
