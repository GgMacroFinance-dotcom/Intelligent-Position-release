# intelligent-position harness（Rust）

AI 原生组织的运行底座：一个 Rust 进程跑在云 Linux 主机上，每个数字员工是其中一个异步任务。
项目是什么、现在到哪一步，见仓库根目录的 [`README.md`](../README.md) 和 [`docs/ip/project-spec.md`](../docs/ip/project-spec.md)。

## 快速开始

```sh
cd rust
cp ip.example.yaml ip.yaml                         # 改岗位、员工、模型端点
export OPENAI_API_KEY=...                          # 模型；密钥只放环境变量
export WHICHAPI_API_KEY=...                        # 可选：FRED 数据经 WhichAPI 取，见 ../macro-mcp/README.md
cargo run --release -p ip-cli -- serve --config ip.yaml   # 默认监听 127.0.0.1:7878
curl http://127.0.0.1:7878/healthz
```

浏览器打开 `http://127.0.0.1:7878/` 是 Web 控制台：和员工对话（流式回复、工具调用过程可展开）、搜索历史、查看岗位与员工、任务、审批、路由规则、岗位记忆。
配置了访问令牌时，第一次用 `http://<主机>:7878/?token=<令牌>` 打开，令牌会记在浏览器里。
控制台是不需要构建的原生 JS（`web/src`），编译进 `ip` 可执行文件；前端类型由 ts-rs 从 Rust 类型生成到 `web/src/types`，
改了 Rust 里的接口类型后跑一次 `cargo test` 并提交生成的文件，CI 会检查两边一致并用 `tsc` 做类型检查。

WebSocket 接口在 `/ws`。发一条消息：

```json
{"id": 1, "type": "send", "agent_id": "macro-1", "text": "点评一下本月 CPI", "author": "张三"}
```

服务先回 `{"type":"reply","id":1,"ok":true,"data":<消息>}`，随后推送 `{"type":"event","event":{...}}`：
`turn_started`、`delta`（流式文字）、`message`（写入存储的每条消息，含工具调用和结果）、`turn_finished` 或 `turn_failed`。
其他请求：`list_roles`、`list_agents`、`list_threads`、`history`（`thread_id`、`before_ms`、`limit`）、`search`（`query`、`thread_id`）、
`route`、`list_rules`、`list_tasks`、`create_task`、`update_task`、`list_approvals`、`resolve_approval`，以及“记忆”页用的 `get_memory`、`save_memory_text`、`forget_thread_memory`、`delete_memory_note`、`clear_memory`、`list_memory_off_threads`、`set_thread_memory`，看报告用的 `list_reports`、`read_report`。完整格式见 `web/src/types/Request.ts`。
不指定 `agent_id` 的 `send` 按路由规则分派。
监听非本机地址时必须设置 `auth_token_env`，客户端用 `Authorization: Bearer <令牌>` 或 `/ws?token=<令牌>` 连接。
握手时还检查来源，挡住别的网页借浏览器连控制台：没配令牌时 `Host` 必须是本机地址（`localhost`、`127.0.0.1`、`[::1]`），
带 `Origin` 的请求（浏览器）必须和 `Host` 同源或写在 `allowed_origins` 里，否则 403。不带 `Origin` 的非浏览器客户端只看令牌。
放在反向代理后面且代理改写了 `Host` 时，把对外地址写进 `allowed_origins`，如 `[https://ip.example.com]`。

`/mcp` 是给 Codex 这类外部引擎用的 MCP 服务端（Streamable HTTP），开放的是组织工具、本岗位的数据工具和外部 MCP 工具，不含 shell 和文件工具。
它只认运行时为每一轮签发的令牌（`Runtime::grant_turn`），不认上面的访问令牌，并且只接受本机地址的请求。详见[《M2 规格》](../docs/ip/m2-spec.md) D-17 到 D-19。

启动前可以先检查配置：`ip check --config ip.yaml` 逐项检查模型端点的 key、工具沙箱、数据源、MCP 服务器、Codex 和 Slack 令牌能不能用，有失败时退出码为 1。它不发消息、不调模型。用真实账号验收的步骤见[《M2 真实验收手册》](../docs/ip/m2-acceptance.md)。

质量评测：`ip eval --config ip.yaml --set evals/macro-smoke.yaml` 用真实模型跑评测集里的固定问题，每次用一个新的临时数据目录。
检查项都用程序判定：本轮正常结束、调了取数工具、写了报告、报告里的数字能在工具结果里找到（允许四舍五入、单位换算和两数之差）、
表格里的数字带观测日期、取不到的写明“未取得”、报告小节齐全。打印每题的结果、花费、token 用量和提示词缓存命中率，`--out` 写 JSON，`--model` 换掉被测岗位的模型。
全部通过时退出码为 0。宏观评测的第二题要用 FRED，跑之前要设 `WHICHAPI_API_KEY`（见 `../macro-mcp/README.md`）。现在的评测集只有两个宏观问题，用来发现明显退步，不足以比较模型；结果存在 `docs/ip/evals/`。

性能基准：

```sh
cargo run --release -p ip-cli -- bench startup --check   # 启动时间、空闲内存
cargo run --release -p ip-cli -- bench store --check     # 5 万条消息写入与搜索，每轮读岗位上下文，找待提炼的对话
cargo run --release -p ip-cli -- bench agents --check    # 500 个数字员工常驻与并发
```

`--check` 时未达标即失败，CI 每个 PR 都跑这三项。

4 核云主机上的实测：

| 指标 | 实测 | 目标 | OpenMausBot 对照 |
| --- | --- | --- | --- |
| 服务启动（中位） | 3.2–4.4 ms | 100 ms | 0.75–1.46 s |
| 服务空闲内存 | 12.2–12.7 MB | 40 MB | 134–160 MB |
| 5 万条消息搜索 p95 | 5.4 ms（p50 3.6 ms） | 10 ms | 113–136 ms，中文搜不到 |
| 写入吞吐（64 路并发） | 约 1 万条/秒 | — | — |
| 每个空闲数字员工的内存 | 4–12 KB | 64 KB | 每个 agent 一个 CLI 进程 |
| 500 个员工同时工作，每轮额外耗时 p95 | 23–25 ms | 80 ms | — |

## 用 Docker 部署

镜像里带工具沙箱 bubblewrap，构建时加 `INSTALL_CODEX=true` 还会装上 Codex。步骤、路径和安全上的取舍见[《用 Docker 部署》](../docs/ip/deploy-docker.md)，最短的用法是：

```sh
cd rust
cp ip.example.yaml ip.yaml          # 取消 auth_token_env: IP_TOKEN 的注释，按需改岗位、渠道、引擎
cp deploy/env.example .env          # 填密钥
docker compose run --rm ip check --config /srv/ip/ip.yaml
docker compose up -d --build
```

## 目录

| crate | 职责 | 阶段 |
| --- | --- | --- |
| `ip-core` | 领域模型：组织、岗位、数字员工、任务、消息、授权 | M0 |
| `ip-store` | SQLite、只追加事件日志、专用写线程、中文全文搜索 | M0 |
| `ip-runtime` | 数字员工循环（M0）；上下文压缩、定时任务（M1）；决策者笔记、岗位记忆（提炼、合并、查记忆、更正） | M0 |
| `ip-models` | 云端模型与可配置推理端点（本地 / 外接 GPU） | M0 |
| `ip-tools` | shell、文件工具（含 `edit_file`）与 bubblewrap 沙箱 | M0 |
| `ip-server` | `/healthz`、`/ws` 接口，`ip.yaml` 配置 | M0 |
| `ip-cli` | 命令行 `ip`：`serve`、`bench` | M0 |
| `ip-router` | 消息 → 岗位 → 员工 / 模型的自动路由（规则 v1） | M1 |
| `ip-mcp` | MCP 客户端（rmcp，stdio 子进程或远程 Streamable HTTP，按岗位授权）；服务端 `/mcp` 把组织工具开放给外部引擎（M2） | M1 |
| `ip-data` | 数据源与只读取数工具（`data_catalog`、`data_series`，同比 / 环比），按岗位授权 | M1 |
| `ip-browser` | 浏览器自动化、截图预处理 | M2 |
| `ip-connectors` | 外部渠道的通用接口（`Channel`、`Hub`）和测试用的假渠道、假 Slack 服务器；Slack 渠道（Socket Mode）已接入，飞书、邮件以后接在这里 | M2 |
| `ip-engines` | 外部引擎：把岗位的每一轮交给外部编程智能体；目前有 Codex（`codex exec --json`，组织工具经 `/mcp` 回调），Claude Code 待做 | M2 |

## M0 任务清单

不设固定分工：谁看到没做的就接，开工前在对应的 GitHub issue 或 PR 里说一声，避免重复。Claude 会按清单顺序往下推进。

- [x] 仓库与 CI：workspace、12 个 crate 骨架、GitHub Actions、基准框架、许可与贡献指南（#3）
- [x] `ip-core`：组织、岗位、数字员工、对话、消息；岗位用 YAML 定义（#4）
- [x] `ip-store`：建表、事件日志、批量写、中文搜索、`ip bench store`（#4）
- [x] `ip-models`：OpenAI 兼容端点（DeepSeek、vLLM、SGLang、llama.cpp 通用）、流式输出、工具调用、限流和重试、测试用脚本模型（#5）；调用方丢掉流（排队、重试退避、等响应头、读正文时都算）或收到协议结束事件时立即断开连接、交还并发名额（#68，用假 HTTP 服务测）
- [x] 模型调用故障处理（E-11）：每次尝试等待响应头默认最多 30 秒，正常流式生成不受该时限限制。错误正文限时、限量读取；Chat Completions 意外断流报错，结束后的数据忽略。用假 HTTP 服务和流解析测试覆盖。
- [x] `ip-tools`：shell 和文件工具；shell 跑在 bubblewrap 沙箱里（只读系统目录、断网、清空环境变量），超时整组终止，输出限长；文件工具从工作目录的文件描述符出发打开（Linux 用 `openat2(RESOLVE_BENEATH)`，其他系统逐级 `O_NOFOLLOW`），检查和打开是同一步，不怕检查后路径被换成指向外部的符号链接（#66，本机测试含并发换目录的压力测试）
- [x] `ip-runtime`：收消息 → 调模型 → 执行工具 → 写回；每个数字员工一个 tokio 任务；步数上限、失败写回对话、事件广播
- [x] `ip-server`：WebSocket 发消息、收流式回复、列出岗位/员工/对话、历史和搜索；`ip.yaml` 配置；访问令牌；握手时检查来源（`Host`、`Origin`，#65），只用测试客户端和本机浏览器验过
- [x] 基准：`ip bench agents`，500 个常驻员工的内存和并发开销，接入 CI
- [x] 在云主机上用真实模型端点走一遍 M0 验收（2026-11-11，OpenAI `gpt-6-luna`）：对话逐字流出、每轮显示费用；shell 工具能跑命令；中文搜索能搜到历史消息；`ip bench startup/store/agents --check` 三项都是 `"pass":true`。见[验收记录](../docs/ip/acceptance-2026-11-11.md)
- [ ] 沙箱断网：验收用的云主机没有 bubblewrap（`sandbox.mode: none`），“沙箱里 curl 应该失败”这一项没验

## M1 任务清单

- [x] ts-rs 生成前端类型（含 WebSocket 协议），CI 检查生成结果已提交
- [x] 最小 Web 控制台：对话、岗位与员工、任务、审批、路由规则五页（后来加了第六页“记忆”，见下面“上下文与记忆”）
- [x] 控制台手机布局（backlog F-1）：窄屏（720px 以下）导航移到底部，对话页列表和对话分两屏、带返回栏，审批选项整行可点，输入框 16px 免得 iOS 放大，留出刘海和底部横条的安全区。只在 Playwright 的 390×844 手机视口里截图核对过（假模型），没在真手机上试过
- [x] 控制台可安装成应用（PWA，backlog F-2）：`/manifest.webmanifest`、图标和根路径的 `/sw.js`；Service Worker 只缓存控制台自己的页面、脚本、样式和图标，先走网络、断网才用缓存，不碰 `/ws` 和数据。在 Playwright 的 Chromium 里核对过：没有可安装性错误，断网后页面外壳能从缓存打开。没在真手机和 Safari 上装过；装法见[《用 Docker 部署》](../docs/ip/deploy-docker.md)
- [x] 员工回复渲染 Markdown（backlog F-3）：`web/src/markdown.js` 先解析成块和行内元素再生成 DOM，不执行 HTML，链接只认 http、https、mailto，表格在气泡里横向滚动；人写的消息照原样显示。渲染过的消息按 id 缓存，流式输出时不重复解析整段对话。解析器的单元测试在 `web/test/`（`node --test test/*.test.mjs`），CI 的 rust 工作流会跑。只在 Playwright 里用假模型核对过
- [x] 看员工写的报告（backlog F-4）：`list_reports`、`read_report` 两个请求（`crates/ip-server/src/reports.rs`），只读员工工作目录 `reports/` 这一层的 `.md`，挡住子目录、`..`、隐藏文件和符号链接，再核对一次真实路径，单份最多读 1 MB。控制台岗位卡片列出最近的报告，对话、任务结果和报告里提到的 `reports/<文件名>.md` 都能点开，在浮层里按 Markdown 显示，手机上占满整屏、按返回键关掉。只在 Playwright 里用假模型和手写的示例报告核对过
- [x] `ip-router` 路由 v1：`ip.yaml` 里的规则（关键词、排除词、正则、发送人）按顺序匹配，把没指定员工的消息分派给岗位里最空闲的员工，可要求用升级模型；控制台有“自动分派”和路由规则页
- [x] 路由 v2（一）：规则在控制台编辑并立即生效（存进数据库，重启后仍在，优先于 `ip.yaml`）；人工在审批卡片上改派时生成规则草案，人工编辑后才生效；拿不准时出审批卡片
- [x] 路由 v2（二）：语义匹配（可选）：规则都没命中时请模型按岗位职责判断，把握够就分派，不够时把判断作为审批卡片的推荐项
- [x] 任务与审批卡片：任务（状态、负责人、截止时间）；审批卡片只能被选定一次，路由拿不准时自动发卡片，选定岗位后交给该岗位最空闲的员工
- [x] 员工的组织工具：`create_task`、`update_task`、`list_tasks`、`ask_human`（发选择卡片，人选定后结果写回对话并叫回该员工）；系统提示词带同事名单
- [x] 改文件工具 `edit_file`（精确替换）、上下文压缩（超出 `context_budget_chars` 时把较早对话写成摘要）
- [x] 员工知道今天是哪天（backlog E-9）：发给模型的每条人、同事、系统消息开头标上它自己的发送时间（`[2026-11-14 周六 13:22 UTC]`），交给 Codex 的记录和压缩摘要的记录也带；系统提示词只加一句固定说明，不放日期。时间只由消息自己决定，已缓存的前缀不会因为换了一天而失效。时区用 `ip.yaml` 顶层的 `utc_offset` 设置（如 `+08:00`，显示成 `UTC+08:00`），省略时是 UTC；每日预算仍按 UTC 0 点重置2026-11-14 用真实模型验过（[记录](../docs/ip/evals/2026-11-14-e9/README.md)）
- [x] 协调层 v1：`coordinator` 岗位接住规则没命中的需求（`fallback: { role: coordinator }`），用 `create_task` 派给执行员工；派活即另开执行对话通知负责人，负责人 `update_task` 交回结论后，结果写回派活对话并叫回协调员汇总。不能派给自己，委派层数有上限（`max_delegation_depth`，默认 3）。一次派出多个任务时，等最后一个有结果才叫回协调员汇总（中途只把结果记进对话），任务受阻则立即叫回
- [x] MCP 客户端（rmcp）：`ip.yaml` 的 `mcp_servers` 启动 stdio 服务器，工具名 `mcp__<服务器>__<工具>`，可按岗位限定。stdio 子进程不继承 harness 的环境变量，只带 `PATH`、`HOME` 等几个基本变量和配置里写明的（`env`、`env_inherit`、`env_from`），模型和渠道的密钥不会交给 MCP 服务器。启动时某个服务器连不上，只是它的工具暂时用不了，服务和其他岗位照常启动（backlog E-4，本机起服务验过，没有用真实模型对话）。已补握手超时、握手成功后工具列表报错或超时的假 HTTP 服务测试，验证前后正常服务器仍可调用工具。没连上的在后台重连 5 次（30 秒起，间隔翻倍），连上后员工下一步就能用它的工具；密钥没设的不重连。重连只用假服务器测过
- [x] 数据源接口 `ip-data`：`ip.yaml` 的 `data_sources`（`csv_dir`：`catalog.yaml` + 每个序列一个 CSV），员工用 `data_catalog` 查目录、`data_series` 取数（可截取时间段、算同比 / 环比 / 差值），结果带来源和数据截至日期；可按岗位限定
- [x] 宏观研报岗位流程骨架：岗位说明写明取数 → 写报告（`reports/<日期>-<主题>.md`，固定结构）→ 注明来源 → `update_task` 交回；`examples/macro-data` 是虚构的示例数据，用假模型跑通了协调员派活 → 取数 → 写报告 → 交回 → 汇总
- [x] 公开数据导入 `ip data pull`：`catalog.yaml` 的序列写 `fetch`（`fred`：FRED 公开 CSV，无需 key），拉取后并入 CSV；`examples/public-data` 现在没有序列：FRED 的序列 2026-11-11 删掉（美国数据只走 Finance Macro Data API），国家统计局的序列 2026-11-15 删掉（中国数据改走 macro-mcp 的统计局工具，`nbs` 拉取方式也去掉了）；`fred` 这种拉取方式代码里还在。建议每天定时跑：`0 9 * * * ip data pull --config /path/to/ip.yaml`
- [x] 岗位每日费用上限：端点配 `prices`（美元 / 百万 token，`cached_input` 是命中缓存的输入、`cache_write` 是写进缓存的输入，按端点报告的用量分开计价），岗位设 `budget_per_day_usd` 后用满当天不再调用模型（按 UTC 自然日，账本存数据库，重启不清零）；控制台在每轮结束时显示费用
- [x] 费用补全：语义分派调用模型的费用单列记账（不占岗位预算）；岗位页显示各岗位今日已用 / 上限；执行员工这一轮失败（费用用满、模型出错、步数超限）时，派给它的任务标为受阻并写回派活的对话，不自动叫回协调员重派，避免一圈圈重试花钱
- [x] 提示词缓存命中率能量出来（backlog D-2 的第一步）：账本按岗位累计当天的 token 用量（输入、其中命中前缀缓存的、输出），每轮结束时记一次，成功和失败的轮次都记，没登记价格的模型也记，语义分派单列，重启不清零；`spend_today` 返回 `tokens` 和 `routing_tokens`，控制台岗位页显示各岗位今日缓存命中率，`ip eval` 每题和合计都打印。命中率 = 命中缓存的输入 token / 全部输入 token。2026-11-14 用真实模型量过（[实测记录](../docs/ip/prompt-cache-2026-11-14.md)）：研究员 84%–97%，协调员 34%（请求短，钱主要花在输出上）
- [x] 运维测试岗位流程骨架：岗位说明写明复现 → 定位 → 最小修改 → 复跑验证 → 报告（`reports/<日期>-<主题>.md`）→ 交回，动线上先 `ask_human`；员工可用 `workspace` 指定工作目录（指向要维护的项目）；`examples/ops-demo` 是故意留了 bug 的小项目，用假模型跑通了跑测试失败 → 修复 → 复跑通过 → 写报告
- [x] 定时任务：`ip.yaml` 的 `schedules`（五段 cron + `utc_offset` 时区），到点给岗位或员工发一条消息、开一段新对话，不写 `to` 时按路由规则分派；停机期间错过的不补跑；上次运行结果存数据库；控制台任务页显示下次运行时间，可立即运行
- [x] MCP 远程接入和 Finance Macro Data API：`mcp_servers` 可以写 `url` 连远程服务器（Streamable HTTP，令牌用 `bearer_token_env`；必须是 https，只有本机地址可以用 http）。宏观研报岗位的美国和全球数据改为直接调 Finance Macro Data API（which-api 仓库的 `packages/macro-mcp`，12 个工具），中国数据仍用 `ip data pull` 拉的国家统计局序列。用本机启动的同一版服务（0.2.0）联调过：握手、列工具、取 FRED 实时数据都通。2026-11-11 的真实验收里，真实模型经 Render 上的测试部署取到了数（见下面宏观研报岗位的真实验收一条）
- [x] Finance Macro Data API 开源（backlog C-2）：9 个美国宏观数据工具放进仓库根目录的 `macro-mcp/`（MIT，Node.js 18.17 以上，零依赖），示例配置改成本机 stdio 启动。和 WhichAPI 托管版的差别：不带 API 目录工具；FRED 不直连，经 WhichAPI 托管服务取，要 `WHICHAPI_API_KEY`；快照不带 ICE BofA 的三个面板（2026-11-16 起这些序列可以用 `get_fred_series` 取，只用模拟的回复测过）。包自己的测试进 CI（`macro-mcp` 工作流）；经 WhichAPI 取 FRED 这条路包自己的测试用的是模拟；2026-11-14 的评测里配真实模型真实跑过（[记录](../docs/ip/evals/2026-11-14-e8/README.md)）
- [x] 中国数据进 Finance Macro Data API（backlog E-2）：`macro-mcp` 有国家统计局 4 个工具：`search_china_indicators` 按中文名搜指标，`browse_china_catalog` 浏览目录，`get_china_indicator` 取任一指标（月度、季度、年度），`get_china_series` 是 CPI、PPI、PMI、M2 的快捷入口，存的 id 失效时按名字重新找到。代码和 which-api 托管版是同一份（which-api PR #14）。2026-11-15 在境内网络上真实跑通；境外网络能否访问没测
- [x] FRED 走官方 API（backlog C-2）：WhichAPI 托管服务改调 FRED 官方 API（which-api PR #13，key 配在 Render 上）；`macro-mcp` 的 `get_fred_series` 把 `units` 转发过去，算同比用 `pc1`，岗位说明写明不要自己算。托管服务这条路在本机用真实 key 跑通过；Render 上部署后的调用 2026-11-15 直接请求过一次，取到了数（见[发布前清单](../docs/ip/release-checklist.md)）
- [x] 示例配置的模型：协调员用 `openai/gpt-6-astra`，执行岗位用 `openai/gpt-6-luna`，配了价格，协调员和宏观研究员设了每日费用上限（价格 2026-11-14 按 OpenAI 的模型文档核对过，补了 `cached_input` 和 `cache_write`，用之前再核对）
- [x] 模型端点支持 OpenAI Responses 接口：端点写 `api: responses`（可选 `reasoning_effort`），其余端点默认仍走 Chat Completions。gpt-6-astra、gpt-6-luna 在 Chat Completions 里不能同时带工具和推理，只能走 Responses
- [x] 宏观研报岗位用真实模型和真实数据验收（美国数据部分，2026-11-11）：gpt-6-luna 研究员调 Finance Macro Data API（Render 测试部署）取 FRED 等实时数据写报告；gpt-6-astra 协调员派活 → 研究员交回 → 选择卡片 → 汇总，全程跑通，当天总花费约 0.57 美元。记录和发现的问题见[《宏观研报岗位真实验收记录》](../docs/ip/acceptance-2026-11-11.md)
- [x] 国家统计局数据：网站改版后旧接口停用，岗位改用 macro-mcp 的统计局工具，2026-11-15 在境内网络上真实取到数

## M2 任务清单

接口、表结构和分工见 [《M2 规格》](../docs/ip/m2-spec.md)（活文档，改接口先改它）。

- [x] `ip-core` 新增两个字段：系统消息带类型 `notice`，外部渠道只转发“这轮失败”；人的消息可带来源渠道 `via`，路由规则 `from` 对渠道用户只按用户 id 匹配，改显示名冒用不了
- [x] 通用渠道接口、绑定表、投递器、假渠道和渠道测试：`ip_server::channels::start` 把一个渠道接到服务上；收消息时去重、按名单准入、找或建会话绑定、分派，同一会话的追问粘住原来的员工；投递器从事件日志读，只发员工回复、“这轮失败”、卡片和卡片选定结果，失败重试、顺序不乱、重启不重发；卡片按 `approvers` 决定谁能点。用假渠道跑通了 T1–T11
- [x] Slack 渠道（Socket Mode）：`ip.yaml` 的 `channels` 配置，主机不需要公网入口；在频道里 @机器人 或私信开启会话，回复和审批卡片都在同一个线程里，卡片用按钮选，选定后原卡片更新；应用清单在 `examples/slack/manifest.yaml`。用假 Slack 服务器跑通了，真实工作区待验收。收到的事件先存进数据库再向 Slack 确认，处理暂时失败时退避重试，进程在确认后退出也会在重启后接着处理；渠道去重改在处理成功之后（D-28，#69，只用假 Slack 测过）
- [x] MCP 服务端：组织工具开放给外部引擎，每轮一个令牌
- [x] 引擎接口和 Codex 引擎（先只给运维测试岗位）：`ip.yaml` 的 `engines` 按岗位配置；每轮起一个 `codex exec --json` 进程，同一对话用 `resume` 续接、只交新消息；超时整组终止；用量计入岗位预算。用假 codex 跑通了；真实 Codex 的验收见下面“真实验收：Codex”一条
- [x] Docker 部署：`rust/Dockerfile`、`docker-compose.yml`、`deploy/env.example`；镜像带 bubblewrap，可选带 Codex；以 uid 10001 运行，数据放命名卷，端口只绑本机。在容器里用假模型跑通了沙箱里的 shell，`ip check`、健康检查、令牌校验都测过；真实模型、Slack、Codex 和其他验收一起做。见[《用 Docker 部署》](../docs/ip/deploy-docker.md)
- [x] 质量评测 `ip eval`（backlog B-1）：评测集 `evals/macro-smoke.yaml` 两个固定问题（有数据的、一半取不到的），检查项全由程序判定；2026-11-12 用 `gpt-6-luna` 真实跑两遍，都是 2/2 通过，每遍不到 0.01 美元，结果在 `docs/ip/evals/2026-11-12-luna/`
- [x] 真实验收：Slack 线程 → 协调员 → 宏观研究员 → 报告回到 Slack（backlog A-3，2026-11-16，[记录](../docs/ip/acceptance-2026-11-16-slack.md)）：真实工作区里线程回复、续问、派活和汇总、选择卡片、别人点被拒、名单外频道不理会、重启后续接都符合。Socket Mode 是经本机转发器连的（容器只能经代理出网，backlog E-12）；取数工具连不上数据源，回复里没有真实数据；私有频道和私信没验。步骤见[《M2 真实验收手册》](../docs/ip/m2-acceptance.md)
- [x] 真实验收：Codex 跑通 `examples/ops-demo`（backlog A-4，2026-11-14，[记录](../docs/ip/acceptance-2026-11-14-codex.md)）：`codex-cli 0.169.0` 修好 bug、写报告、经 `/mcp` 的 `update_task` 交回任务，追问时续接原会话。没经 Slack，从控制台发的消息。验收中修了 Codex 引擎的 MCP 工具审批、中途 `error` 和 `"error": null` 被当成失败、缓存写入没记账；续接会话时用量重复记账还没修（E-10）

**M0 验收**：在云主机上用一个可执行文件启动；通过 WebSocket 和一个岗位对话，它能在沙箱里跑 shell 命令；中文能搜到历史消息；启动、内存、并发三项达到方案里的性能目标。

## 上下文与记忆

设计见 [`docs/ip/context-memory-design.md`](../docs/ip/context-memory-design.md)，分 7 步做。

- [x] 第 1 步：系统提示词多两层，决策者笔记和岗位记忆摘要，按岗位存在设置表里，排在稳定内容之后；`memory` 配置（默认关，可限定岗位，两层各有字数上限）；`ip context show / set / clear` 查看和修改，服务运行时也能改，下一轮生效；`ip bench store` 加了每轮读这两层的耗时。只用假模型测过
- [x] 第 2 步：后台任务从凉下来的对话里按岗位提炼摘要（默认闲置 6 小时后，每轮最多 2 份）。交给模型前和存库前都做密钥脱敏；费用记在岗位当天的账上，预算剩余不到 25% 时不提炼；失败退避重试；外部渠道的对话默认不进记忆，从渠道对话派出去的执行对话（顺着任务的派活来源往上查）也一样（#67）。`ip context show` 能看到提炼结果。只用假模型测过
- [x] 第 3 步：同一个后台任务里，每轮提炼之后按岗位把对话摘要合并成岗位记忆摘要，员工下一轮开工就带上。选中的对话摘要（默认 30 天内用过或生成的，最多 256 份）和上次合并时一样就不调模型；没变的只给标题，新增和改过的给全文；失败或没有输出时保留旧的一版。默认用岗位自己的模型，可用 `memory.consolidation_model` 另指定。分批合并时，暂缓更新的摘要保留已合并版本的来源记录，后续过期或移除仍能清理。只用假模型测过
- [x] 第 4 步：开了记忆的岗位多两个只读工具。`memory_search` 在本岗位的对话记忆里按词查找，`memory_read` 读一份全文；岗位取自调用者本人，别的岗位的搜不到也读不到。读成功记一次使用，合并时用得多的优先、长期没人读的淘汰。`ip bench store` 加了搜索耗时（2000 份时 p95 约 8 毫秒）。只用假模型测过
- [x] 第 5 步：人在对话里说“记住……”“以后别……”“忘掉……”时，员工用 `memory_note` 记一条更正，下次合并时生效，保留期内每次合并都带上，忘掉的内容不会被带回来。程序只允许人的消息引起的一轮里记，外部渠道的对话和从它派出去的执行对话不能记。只用假模型测过
- [x] 第 6 步：控制台“记忆”页。按岗位看决策者笔记、岗位记忆摘要、各对话的记忆和被读过的次数、人要求的更正；笔记和摘要可以直接改，对话记忆可以抹掉，更正可以删，整个岗位可以清空（决策者笔记不动）。对话页顶部可以按对话关掉记忆，关掉的对话不提炼、也不能记更正。只用假模型测过，页面在本机浏览器里点过
- [x] 第 7 步：用真实模型跑一遍（2026-11-15，gpt-6-luna，合并另用 gpt-6-astra 对比）。提炼、合并、`memory_search` / `memory_read`、`memory_note`、抹掉、按对话关记忆、清空都按设计工作；合并提示词有两处会被抄进摘要，已改并重跑确认；员工是否照摘要做还没验证。见 [`docs/ip/memory-2026-11-15-real-run.md`](../docs/ip/memory-2026-11-15-real-run.md)

## 许可

Apache-2.0，见根目录的 [LICENSE](../LICENSE)。参与方式见 [CONTRIBUTING.md](CONTRIBUTING.md)。
