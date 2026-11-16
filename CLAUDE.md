# 给在这个仓库里工作的 Claude

intelligent-position（I.P.）是一个开源优先的“AI 原生组织架构”项目：一个 Rust 进程跑在云 Linux 主机上，每个数字员工是其中一个异步任务。协调层用云端最强的模型，执行层用便宜的云端模型，以后换成岗位专家小模型。现在只用宏观研报一个岗位做展示。

## 先读这些

- `docs/ip/project-spec.md`：项目总览。已敲定的决定、当前真实状态、先后顺序、开源发布门槛、待定事项。
- `docs/ip/backlog.md`：还没做完的事，按建议的先后排好了。从这里挑活。
- `rust/README.md`：harness 的结构、怎么跑、各阶段做了什么。
- `rust/CONTRIBUTING.md`：流程、敏感路径清单、本地检查、编码约定。
- `docs/ip/m2-spec.md`：渠道、MCP 服务端、外部引擎的规格。改这些接口先改它，和代码放在同一个 PR。

## 工作规则

1. 开工前看 `docs/ip/backlog.md` 和开放的 PR。可能有别人（或别人的 Claude）在同时推进，同一条不要做两遍；拿不准先在 issue 里问。
2. 一个 PR 只做一件事，尽量小。PR 描述按 `.github/pull_request_template.md` 写。
3. 动到敏感路径（清单在 `rust/CONTRIBUTING.md`）的 PR，在描述里写明，等一位维护者读过代码再合并。其他 PR 本地检查通过后可以直接合并。
4. CI 已恢复（2026-11-13）：改到 `rust/**` 的 PR 会自动跑 `rust` 工作流（fmt、clippy、test、前端类型检查、基准）。合并前看 CI 结果，在 PR 描述里写 CI 是否通过；CI 没覆盖或没跑到的，在 `rust/` 下本地跑 `cargo fmt --all --check`、`cargo clippy --all-targets --locked`、`cargo test --locked`，跑了什么写什么。提交信息不要加 `[skip ci]`。不要为了重跑而重跑。
5. 没在真实环境跑过的东西，不要写“已上线”“已验证”。用假模型、假 Slack、假 Codex 测的，写明是假的。主页和 README 里的说法以真实跑通为准。
6. 密钥只放环境变量或云端环境的密钥设置，不写进仓库、PR、issue 和聊天。
7. nightly-lab 是非商用许可，只能参考设计，不得复制代码。依赖只用与 Apache-2.0 兼容的许可。
8. 性能是功能的一部分：新增热路径要有 `ip bench` 基准。
9. 注释和文档用中文，标识符用英文。
10. 做完一条，在同一个 PR 里更新 `docs/ip/backlog.md` 和 `rust/README.md` 里对应的勾选项。

## 小提示

- `rust/target` 会涨到几十 GB，磁盘满时 `cargo clean`。
- 国家统计局的数据走 `macro-mcp/src/china.js`（和 which-api 的 `packages/macro-mcp/src/china.js` 是同一份，两边一起改）。统计局没有开放 API，用的是 2026 年改版后网站自己的接口；境外能否访问没测。`examples/public-data` 里的 CSV 不进仓库。
- 示例配置连的 Finance Macro Data API 是 Render 免费套餐上的测试部署，闲置会休眠，休眠后第一次连接约等 1 分钟。
