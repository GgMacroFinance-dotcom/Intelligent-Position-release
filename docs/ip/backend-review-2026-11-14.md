# 后端逻辑审阅与模型请求修复（2026-11-14）

审阅时的基线是旧仓库（现为只读存档）`main` 的 `cfd389a85`。下表的修复在本仓库由 PR #4 合入（backlog E-11），实现和测试的写法与审阅时的本地分支不同，测试在 [openai_cancel.rs](../../rust/crates/ip-models/tests/openai_cancel.rs)。“验证”一节记的是审阅当时在本地分支上的结果。

本轮阅读了运行时派活与预算、SQLite 写线程、记忆提炼与管理、WebSocket 请求处理、渠道去重、MCP 令牌和文件路径校验。落地改动集中在 `ip-models` 的请求生命周期和流解析，保持现有接口、数据结构与技术栈。C-3 要求的工程师审阅仍待完成。

## 已修复

| 问题 | 原来的后果 | 修复后的行为 |
| --- | --- | --- |
| 取消调用只在发送下一条流事件时才被察觉 | 已取消的请求可能继续调用上游，或一直占着连接、并发名额 | 后台任务同时监听接收端关闭；排队、退避、等响应头、读正文都可以取消 |
| 收到协议结束事件后仍等 HTTP EOF | 上游保持连接时，请求不能收尾，并发名额不能释放 | `Done` 发出后立即结束请求，保留结束事件里的用量；覆盖 Chat Completions 和 Responses |
| Chat Completions 在无完成信号时断流也补出成功 | 不完整的回答被当作成功，尚未确认完成的工具调用可能被交给运行时 | 没有 `[DONE]` 或 `finish_reason` 时 EOF 报错；保留已有 `finish_reason` 但没有 `[DONE]` 的兼容行为 |
| `[DONE]` 后继续解析数据 | 重复结束事件、迟到的文字或坏数据污染已完成结果 | Chat Completions 解码器进入终态后忽略后续数据 |
| `connect_timeout` 只交给 reqwest 的建连超时 | TCP 已连接、服务端一直不回响应头时可以无限等待 | 每次尝试增加响应头等待时限；使用已有 `connect_timeout`，默认 30 秒，不限制正常流式生成总时长 |
| 先读完整错误正文，再截成 2,000 字 | 错误响应过大时浪费内存；不结束的错误正文阻止返回或重试 | 最多保留 8 KiB，读取时间不超过 `connect_timeout`，最终显示最多 2,000 个字符；超时保留已知 HTTP 状态码和已读内容 |
| 指数退避先计算再封顶 | 极大的 `max_retries` 配置可能让整数计算溢出 | 先限制指数，再保留原有 30 秒上限和 `Retry-After` 行为 |

主要实现位于 [openai.rs](../../rust/crates/ip-models/src/openai.rs) 和 [sse.rs](../../rust/crates/ip-models/src/sse.rs)；[回归测试](../../rust/crates/ip-models/tests/openai_cancel.rs)用本地 TCP 假服务，不需要模型密钥，也不产生模型费用。

## 验证

- 修复前：新加的 9 个回归测试全部失败，失败原因与上表一致；原有模型测试通过。
- 修复后：`cargo test --locked -p ip-models --no-fail-fast` 通过，14 个单元测试、11 个 HTTP 测试，共 25 项。相对基线新增 11 项，另外两项覆盖退避溢出和正常慢流兼容。
- HTTP 测试刻意让连接保持打开，实际验证连接释放与并发名额恢复。
- `cargo fmt --all --check` 和 `git diff --check` 通过。
- `cargo clippy --all-targets --locked -p ip-models -- -D warnings` 通过。
- `cargo test --locked -p ip-core -p ip-store -p ip-router` 通过：核心类型 27 项、存储 19 项、路由 17 项，包含 TypeScript 类型导出检查。本轮合计 88 项测试通过。
- `cargo clippy --all-targets --locked -- -D warnings` 未通过：已有的 `ip-tools` 使用 `nix::sys`、`nix::unistd`、`process_group` 和 `std::os::unix`，不能在 Windows 编译。这些代码本轮未改；完整工作区需要在 Linux 上验证。

验证在 Windows 本机执行。已安装的 Ubuntu WSL 启动失败，错误为 `Wsl/Service/CreateInstance/CreateVm/E_UNEXPECTED`。Linux 专用的 shell / bubblewrap、外部引擎和整机性能基准尚未在本轮运行；未连接真实模型、Slack 或 Codex，未部署。

## 仍需专项处理的审阅发现

以下来自源码检查，尚未加入并发故障注入测试，本轮列为待验证风险。

1. **任务更新的读改写不在同一个事务。** `ip-runtime/src/work.rs::update_task` 先读任务，修改克隆，再用 `put_task` 整行覆盖。两次并发更新分别修改负责人和结果时，后写入可能覆盖另一方的字段；两次同时关单也可能重复回报。应在写线程中做条件更新，并把“是否首次结束”的判定和更新放进同一事务，之后再通知运行时。
2. **记忆更正未使用管理操作的同一把锁。** `ip-runtime/src/memory_tools.rs::note_tool` 检查对话开关、待处理数量后再插入；`memory_admin` 清空或关闭记忆时持有 `memory_pass`，但工具写入未持有它。并发时可能在关闭 / 清空后又落一条更正，或超过 50 条上限。需要统一串行化或将条件插入下沉到存储事务，并测试关闭与写入的交错。
3. **渠道去重早于业务成功。** `ip-server/src/channels.rs::on_message` / `on_click` 先通过 `fresh` 持久化去重记录，再执行业务。如果后续发生暂时故障，同一个事件重投时会直接被忽略。需要区分处理中、成功和可重试失败；不能简单把去重挪到末尾，否则并发重投又会重复执行。

模型传输另有一个明确的边界：正常成功响应在发出结束事件前若永久静默，本轮没有新增流空闲超时。该时限应与等待响应头区分，并结合推理模型的实际停顿时间配置，避免误杀正常生成。

这些后续改动涉及任务状态、记忆政策或外部渠道，需要独立的并发及故障恢复测试。
