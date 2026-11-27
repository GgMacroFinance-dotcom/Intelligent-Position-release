# M2 规格：Slack 渠道、MCP 服务端、Codex 引擎

> **这是活文档。** 接口、表结构、行为有变化时，先改这里，再改代码，两者放在同一个 PR 里。
> 当前版本：v0.26（2026-11-27）。第 3 到 6 节（D-01 到 D-25）和 D-26、D-27 都已实现；D-28（确认前持久化、处理成功后才记去重，#69）是提议，代码和这次修改在同一个 PR 里，等工程师审查。M2-6（真实 Codex 验收）2026-11-14 通过。D-20、D-23 加了会话累计用量（修 backlog E-10 的重复记账，v0.20），是提议，代码和这次修改在同一个 PR 里，等工程师审查。D-37（Socket Mode 经 HTTP 代理连接，backlog E-12，v0.22）是提议，代码和这次修改在同一个 PR 里，等工程师审查。D-35 的 iPhone 实现（F-21 之五，v0.23）是提议，代码和这次修改在同一个 PR 里，等工程师审查。D-38（回复里提到的报告作为文件发进线程，backlog E-13，v0.24）是提议，代码和这次修改放在同一个 PR 里，等工程师审查。D-39（浏览器走 Playwright MCP、`mcp_servers` 的 `tools` 白名单，backlog E-5，v0.25）是提议，代码和这次修改在同一个 PR 里，等工程师审查。D-40（App 桌面外壳的协议补充：对话的最后回复员工、协调岗位标记、星图轨道表，backlog F-23，v0.26）是提议，代码和这次修改在同一个 PR 里，等工程师审查。剩下的是 Slack 真实验收（M2-5，按 `docs/ip/m2-acceptance.md` 第 4 节做），以及第 7 节的待定问题。

## 0. 怎么改这份文档

分工：**Claude 写代码、改这份规格、推 PR；维护者审查、合并，并拍板待定问题。** 需要维护者提供的东西（API key、Slack 工作区等）写在各项的“依赖”里。

每条决定有编号（`D-xx`），每个待定问题有编号（`Q-xx`），编号一旦用过就不再复用。

| 状态 | 含义 |
| --- | --- |
| 提议 | 写下来了，还没人确认 |
| 已定 | 维护者确认，可以照着写代码 |
| 已实现 | 代码已合并，后面写上 PR 号 |
| 作废 | 不再采用。不删除，用 ~~删除线~~ 划掉，写明原因和替代它的条目 |

规则：

1. 改接口、改表结构、改对外行为：先在这里改对应条目，状态退回“提议”，评审通过后改为“已定”。
2. 实现某条决定的 PR，合并前把它改成“已实现（#PR 号）”。
3. 待定问题定下来以后，在问题下面写结论，再把结论写进对应的决定，问题标为“已关闭”。
4. 每次修改在文末“修订记录”加一行。

---

## 1. 范围与顺序

**本阶段要做的事：**

| 编号 | 内容 | 负责 | 依赖 | 状态 |
| --- | --- | --- | --- | --- |
| M2-0 | `ip-core` 新增两个字段：系统消息类型 `notice`、渠道来源 `via`（D-26、D-27） | Claude | — | 已实现（#20） |
| M2-1 | 通用渠道接口、四张表、投递器、假渠道、渠道测试 T1–T11（第 3 节） | Claude | M2-0 | 已实现（#24） |
| M2-2 | Slack 渠道，用 Socket Mode（第 4 节） | Claude | M2-1 | 已实现（#25），用假 Slack 服务器验证 |
| M2-3 | MCP 服务端：把组织工具开放给外部引擎（第 5 节） | Claude | — | 已实现（#21） |
| M2-4 | 引擎接口和 Codex 引擎，只给运维测试岗位用（第 6 节） | Claude | M2-3 | 已实现（#22），先用假 codex 验证；2026-11-14 用真实 Codex 跑通过一次示例项目（M2-6） |
| M2-5 | 真实验收：在 Slack 线程里提需求，经协调员派给宏观研究员，交回报告，结论回到 Slack 线程（同时完成 M0、M1 的真实验收） | Claude | M2-2、维护者提供 Slack 工作区（按 `examples/slack/manifest.yaml` 建好应用）、两个 Slack 令牌和模型 API key | 完成（2026-11-16，[记录](acceptance-2026-11-16-slack.md)）：真实工作区里手册第 4 节的 Slack 链路都符合。经本机转发器连接（Socket Mode 不走代理），回复里没有真实数据，私有频道和私信没验 |
| M2-6 | 真实验收：用真实 Codex 跑 `examples/ops-demo`，经历跑测试失败 → 修复 → 复跑通过 → 写报告 → 交回 | Claude | M2-4、维护者提供 Codex API key | 完成（2026-11-14，`codex-cli 0.169.0`，见 `docs/ip/acceptance-2026-11-14-codex.md`）。消息从控制台发，没经 Slack；还没在真实项目上跑过；续接会话时用量重复记账 2026-11-16 修了（backlog E-10，只用假 codex 测过） |

M2-1 和 M2-3 改的文件基本不重叠，可以同时做。

**本阶段不做：**

- 飞书、邮件：通用接口留出位置，等有真实用户再接。
- `ip-browser` 自研版：需要浏览器的岗位先在 `mcp_servers` 里配 Playwright MCP。注意 MCP 子进程不在沙箱里。2026-11-24 定了具体做法（D-39）：空壳 `ip-browser` 删掉，配法见 [browser.md](browser.md)。
- Claude Code 引擎：原来不在本阶段，2026-11-21 做了，套用第 6 节的同一个 `Engine` 接口（D-36，backlog E-6）。
- Slack 里逐字输出：第一版回复写完一次发出，见 Q-07。

---

## 2. 现有代码的接入点

| 用途 | 位置 | 说明 |
| --- | --- | --- |
| 收到消息后的处理 | `ip-server/src/work.rs` 的 `send` | 已包含路由、拿不准时发审批卡片 |
| 卡片被点击 | `ip-server/src/work.rs` 的 `resolve_approval` | 卡片只能被选定一次，这一点已经保证 |
| 往外发的事件 | `ip-store` 的 `events` 表，按 `seq` 递增 | 事件类型：`message.appended`、`task.put`、`approval.created`、`approval.resolved`、`setting.put` |
| 实时通知 | `Runtime::subscribe()` | 有损：订阅者跟不上会收到 `Lagged`，只能用来唤醒投递器 |
| 找派活对话 | `Task.origin_thread_id`，`Store::task_by_thread` | 执行对话可以顺着这条链找回最初提需求的对话 |
| 一轮工作 | `ip-runtime/src/agent.rs` 的 `steps()` | 引擎从这里替换 |
| 组织工具 | `ip-runtime/src/work_tools.rs` 的 `specs()` 和 `call(shared, me, thread, ..)` | MCP 服务端直接复用 |

---

## 3. 通用渠道接口（`ip-connectors`）

### 3.1 概念

- **渠道**：`ip.yaml` 里 `channels` 下的一项，比如 `slack`。名字在组织内唯一。
- **外部会话**：渠道里的一段对话，用一个字符串 key 表示，格式由各渠道自己决定。Slack 用 `<team>/<channel>/<thread_ts>`。
- **绑定**：一个外部会话对应一个 `ThreadId`。可以记住一个“粘住”的员工，同一会话里的后续消息都交给他，不再重新路由。

### D-01 依赖方向 · 已实现（#23、#24）

`ip-connectors` 只依赖 `ip-core`，再加 HTTP 和 WebSocket 客户端，不依赖 `ip-server`。`ip-server` 依赖 `ip-connectors`：启动时创建各渠道，并实现 `Hub`，让渠道把东西交进来。

### D-02 接口形状 · 已实现（#23）

实现在 `ip-connectors/src/lib.rs`。下面的签名为了好读写成 `async fn`；实际上 `Channel` 和 `Hub` 都要能当 `dyn` 用，所以返回 `BoxFuture`，和 `ModelClient` 一样不引入 `async-trait`。和提议相比多了三处：

- `HubError` 分两种：`Rejected(原因)` 表示按规则拒绝，原因要告诉操作的人（见 D-07）；`Failed(原因)` 表示内部错误。
- `Outbound::conversation()` 取出要发到的会话。
- `CancellationToken` 从 `ip_connectors` 重新导出，渠道实现不用自己依赖 `tokio-util`。

```rust
/// 外部会话 key，格式由渠道决定，ip 只当作不透明字符串。
pub struct ConversationKey(pub String);

pub struct Sender {
    pub external_id: String,   // Slack 的 U0123ABC
    pub display_name: String,  // 写进 Author::Human { name }
}

/// 渠道收到的一条消息。
pub struct Inbound {
    pub channel: String,              // ip.yaml 里的渠道名
    pub dedupe_key: String,           // 去重用，由渠道决定（见 D-12）
    pub conversation: ConversationKey,
    pub sender: Sender,
    pub text: String,                 // 已去掉 @机器人 之类的标记
    /// 这条消息能不能开启一段新会话。例如 Slack 频道里只有 @机器人 才算。
    pub may_start: bool,
}

/// 卡片上的一次点击。
pub struct CardClick {
    pub channel: String,
    pub dedupe_key: String,
    pub approval_id: ApprovalId,
    pub choice: u32,
    pub sender: Sender,
}

/// 要发出去的东西。
pub enum Outbound {
    Text { conversation: ConversationKey, text: String },
    Card { conversation: ConversationKey, approval: Approval },
    /// 卡片已被选定：更新原卡片，去掉按钮，写明谁选了什么。
    CardResolved { conversation: ConversationKey, approval: Approval, external_ref: String },
}

/// 发送成功后外部系统给的引用。Slack 是消息的 ts，更新卡片时要用。
pub struct Delivered { pub external_ref: String }

pub enum DeliverError {
    /// 可以重试：网络、限流（带建议等待时间）、5xx。
    Retry { after: Option<Duration>, reason: String },
    /// 重试也没用：会话不存在、机器人被移出频道等。记失败并跳过。
    Permanent(String),
}

/// 一个渠道的实现：Slack、假渠道……
pub trait Channel: Send + Sync + 'static {
    fn name(&self) -> &str;
    /// 连上外部系统，把收到的消息和点击交给 hub，直到 shutdown。断线自己重连。
    async fn run(self: Arc<Self>, hub: Arc<dyn Hub>, shutdown: CancellationToken);
    async fn deliver(&self, out: &Outbound) -> Result<Delivered, DeliverError>;
}

/// 由 ip-server 实现。
pub trait Hub: Send + Sync + 'static {
    async fn message(&self, inbound: Inbound) -> Result<(), HubError>;
    async fn click(&self, click: CardClick) -> Result<(), HubError>;
    // 以下三个是 D-28 加的：确认外部系统之前先存下原始内容，处理完再记结果。
    async fn stash(&self, id: &str, payload: &str) -> Result<bool, HubError>;  // 新存下时 true
    async fn stashed(&self) -> Result<Vec<Stashed>, HubError>;                 // 没处理完的，按收到的先后
    async fn settle(&self, id: &str, settle: Settle) -> Result<(), HubError>;  // Done | Failed(原因) | GaveUp(原因)
}
```

### D-03 收消息流程（`Hub::message`） · 已实现（#24）

实现在 `ip-server/src/channels.rs` 的 `ChannelHub`。用 `channels::start(app, channel, ChannelSettings)` 把一个渠道接到 `App` 上，返回的 `ChannelHandle` 用来停掉它。M2-2 读 `ip.yaml` 的 `channels` 后就调这个函数。实现时的几点说明：

- 写绑定用 `claim_binding`：同一个会话已经有绑定就返回已有的。所以同一会话几乎同时来两条消息时，只有第一条开启对话。
- 发消息走新加的 `work::send_as`，作者带着 `via`，路由按 `Via::key()` 匹配（D-27）。
- ~~去重记录在处理之前写入。所以如果一条消息处理到一半出了内部错误（`HubError::Failed`），外部系统重推时会被当成重复丢掉。这样换来的是不会重复开对话、重复发消息。~~ 作废，改为处理成功后才记，见 D-28（#69：渠道已经确认的消息一出内部错误就永久丢了）。
- `allow` 只按用户过滤（`ChannelSettings::allow_users`）。D-16 里 `allow.channels` 这类和外部会话格式有关的过滤，由各渠道自己做。

1. **去重**：`(channel, dedupe_key)` 在 `channel_inbox` 里已有就直接丢弃。外部系统会重复推送。处理成功（含按规则忽略、拒绝）后才写入，内部错误不写（D-28）。
2. **准入**：发送人或会话不在该渠道的 `allow` 名单里就忽略，不回复。
3. **查绑定** `(channel, conversation)`：
   - **有绑定，且记了员工**：调 `work::send(agent_id = 绑定的员工, thread_id = 绑定的对话)`，不重新路由。
   - **有绑定，但没记员工**（路由卡片还没人选）：只把消息写进对话（`Runtime::post`），等卡片选定。
   - **没有绑定，`may_start = false`**：忽略。
   - **没有绑定，`may_start = true`**：
     1. 先生成新的 `ThreadId`。
     2. 写绑定，`requester` 记为发送人。必须先写绑定再发消息，否则员工的回复可能先于绑定到达投递器，被当成无主对话丢掉。
     3. 调 `work::send(agent_id = 渠道配置的固定员工或 None, thread_id = 新 id)`。
     4. 路由结果有员工时，把员工补进绑定。路由出了审批卡片时，员工留空，卡片选定后补上（见 D-05）。
4. 消息作者写 `Author::Human { name: sender.display_name, via: Some(Via { channel, user_id: sender.external_id }) }`。路由时把 `Via::key()` 作为 `RouteInput.via` 传入（D-27）。

### D-04 往外发的流程（投递器） · 已实现（#24）

每个渠道一个投递器任务：

- **数据从事件日志读，不从广播收。** 从 `events` 表按 `seq` 往后读，读到的位置存在 `channel_cursors`。`Runtime::subscribe()` 只用来唤醒；漏了事件也没关系，另有 5 秒一次的兜底轮询。重启后从保存的位置接着读，不丢事件。
- **游标不能存在 `settings` 表里。** `put_setting` 会写一条 `setting.put` 事件，投递器读到它又会推进游标、再写一条，陷入死循环。所以游标用单独一张表，而且不写事件。
- **选哪些事件、发成什么：**

  | 事件 | 条件 | 发什么 |
  | --- | --- | --- |
  | `message.appended` | 作者是员工、内容是文字、对话有绑定 | `Outbound::Text`，提到的报告再各发一条 `Outbound::Report`（D-38） |
  | `message.appended` | 系统消息，`notice = turn_failed` | `Outbound::Text` |
  | `message.appended` | 其他系统消息（`task_brief`、`task_report`、`approval_answer`） | 不发（D-26） |
  | `message.appended` | 工具调用、工具结果、摘要 | 不发 |
  | `approval.created` | 能找到所属会话（D-06） | `Outbound::Card` |
  | `approval.resolved` | 这张卡片曾在该渠道发出 | `Outbound::CardResolved` |
  | `task.put` | — | 第一版不发 |

- **不重发**：发之前查 `channel_deliveries`，已发过就跳过。发成功后写入记录，再推进游标。如果进程恰好死在“已发出”和“写记录”之间，可能重发一次，这一点接受（至少送达一次）。
- **失败**：遇到 `Retry` 就退避重试（1 秒、2 秒、4 秒……最多 60 秒），游标不前进，后面的消息也不越过它，保证顺序。遇到 `Permanent` 就记一条失败记录，打警告日志，然后跳过。
- **控制台里选定的卡片**：同样会产生 `approval.resolved`，渠道里的卡片也随之更新。点击只有一条处理路径。
- **第一次接入的渠道**：从事件日志的末尾开始投递，不会把以前的历史补发出去。
- **游标**：每批最多读 256 条事件，读完一批记一次游标。中途停下的话，下次会从这批开头重读，靠 `channel_deliveries` 保证不重发。
- **重试等待**：`DeliverError::Retry` 带了建议等待时间时按建议等，没带时按 1、2、4……秒退避，最多 60 秒。
- 投递器另外每 24 小时清理一次 7 天前的去重记录。

### D-05 路由卡片选定后补上员工 · 已实现（#24）

`RouteMessage` 卡片被选定后，`resolve_approval` 会把消息交给某个员工。这时如果该对话有绑定、而绑定里没记员工，就把这个员工补进去。后续消息都交给他。

这一步做在 `work::resolve_approval` 里，所以在控制台里选定也一样会补上。

### D-06 卡片发到哪个会话 · 已实现（#24）

`ask_human` 多数发生在协调员派出的执行对话里。`AgentQuestion.thread_id` 是执行对话，没有绑定任何外部会话。查找规则：

1. 取卡片的对话：`RouteMessage.thread_id` 或 `AgentQuestion.thread_id`。`Question` 类卡片没有对话，只在控制台显示。
2. 这个对话有绑定，就发到这里。
3. 没有绑定，就用 `task_by_thread` 找到它所属的任务，换成 `origin_thread_id` 再查一次。最多往上找 8 层，比委派层数上限（默认 3）留了余量。
4. 一直找不到，就只在控制台显示。

### D-07 谁能点卡片 · 已实现（#24）

每个渠道配置 `approvers`：

- `requester`（默认）：只有开启这段会话的人能点。
- `anyone`：会话里的任何人都能点。
- 用户 id 列表：只有列出的人能点。

不符合的点击不生效：`Hub::click` 返回 `HubError::Rejected(原因)`，渠道把原因只给点击的人看（Slack 用只有本人可见的消息）。这种提示不经过投递器，所以 `Outbound` 里没有对应的变体。理由：运维测试岗位在 `ask_human` 里可能问“要不要重启线上服务”，不能让频道里的任何人都能拍板。

生效的点击调 `resolve_approval` 时，`by` 写成 `<显示名>（<渠道>:<用户 id>）`，审批记录里能查到是谁（D-27）。

被拒绝时的提示是“只有提需求的人能选这张卡片”或“只有指定的审批人能选这张卡片”。卡片已经被别人选过时，`resolve_approval` 的报错（“这张卡片已由 … 选了第 N 项”）也作为 `Rejected` 原样告诉点击的人。

### D-28 确认前持久化，处理成功后才记去重 · 提议（#69）

问题（#69）：Slack 信封先确认、再放进内存队列处理。处理时 `auth.test` 暂时出错，或者 Hub 返回内部错误，都只打日志；Slack 已经收到确认不会重推，进程在确认之后退出也一样，这条消息就永久丢了。而去重记录在处理之前写入（原 D-03），就算重推也会被当成重复丢掉。

- **确认之前先存**：渠道收到信封后先调 `Hub::stash(id, 原始内容)`，返回时已经提交到数据库，再回确认。存不下就不确认，让外部系统重推。存一次是一次本地写，不影响 Slack 3 秒内确认的要求。
- **按顺序处理，暂时失败就重试**：后台任务一个一个处理。`auth.test` 出错、Hub 返回 `Failed` 算暂时失败，`Hub::settle(id, Failed(原因))` 记一次，退避后重试（`RetryPolicy`：默认最多 6 次，间隔 1、2、4、8、16 秒）；成功、按规则忽略、`Rejected` 都算处理完（`Done`）；到次数上限 `GaveUp`，留档不再处理，不挡后面的。重试期间后面的信封照常确认、存下、排队。
- **重启接着处理**：渠道启动时先取 `Hub::stashed()`，再连 Slack；上次确认了没处理完的排在最前面。停止时正在退避的信封留在库里。失败次数跨重启累计。
- **同一个信封只处理一次**：处理完的信封标成 `done`、清掉内容，保留到去重记录的保留期（7 天）满再删。Slack 重推同一个 `envelope_id` 时 `stash` 返回 false，只确认不处理。
- **去重改到处理成功之后**（D-03 第 1 步）：Hub 先只读地查 `channel_inbox`，办完再写；`Failed` 不写，重试时还能办。同一个渠道的消息和点击用一把锁串行处理，查、办、记之间不会插进同一件事的另一次重推，所以重推撞在一起也只派一次活。
- **开会话做到一半的接着开**：绑定写好了、第一条消息没写进对话就出错时（对话是空的、绑定没记员工），重试时按新会话再开一次，不当成“等路由卡片”只记进对话。
- 仍然可能重复的窄窗口：消息已经写进对话、员工已经被叫醒，记去重之前存储出错或进程退出，重启后会再处理一次。要彻底消除得把“写消息”和“记去重”放进同一个事务，牵涉运行时的写入接口，这版没做。
- 新表 `channel_envelopes`（见 D-08），`ip-store` 新增 `stash_envelope`、`pending_envelopes`、`settle_envelope`、`inbound_seen`；`prune_inbound` 同时清理过了保留期的 `done`、`dropped` 信封。
- 测试：`ip-connectors` 的 Slack 测试（`auth.test` 暂时 500 后重试成功、Hub 失败重试到上限放弃、上次没处理完的先处理、退避中停止留到下次）；`ip-server` 的 T12–T14（内部错误后重投照常处理、开会话做到一半接着开、并发重投只派一次）和 Slack 端到端“确认后退出、重启后处理、重推不重复”。都用假 Slack、假模型。

### D-26 系统消息带类型 · 已实现（#20）

`Message` 新增可选字段 `notice`（`ip_core::Notice`），只有系统写的文字消息才带：

| `notice` | 什么时候写 | 写在哪个对话 | 渠道发不发 |
| --- | --- | --- | --- |
| `task_brief` | 派活时的任务说明 | 执行对话 | 不发 |
| `task_report` | 任务完成、受阻、取消时的结果 | 派活的对话 | 不发，协调员随后会汇总 |
| `approval_answer` | 卡片选定后的说明 | 卡片所在的对话 | 不发，卡片本身会更新（D-04） |
| `turn_failed` | 员工这一轮失败（费用用满、模型出错、步数超限） | 当前对话 | 发 |

- 旧消息没有这个字段，读出来是 `None`。值为 `None` 时序列化不写出这个字段，所以存储里的旧数据不变。
- 构造时用 `Message::notice(thread, kind, text)`，服务层用 `Runtime::post_notice`。
- 以后新增一种系统消息，必须带上类型，并在这张表里加一行，写明渠道发不发。

### D-27 渠道用户带来源 · 已实现（#20）

`Author::Human` 新增可选字段 `via: Option<Via>`，`Via { channel, user_id }`。控制台用户没有这一项，构造时用 `Author::human(name)`。

- 界面和提示词里照常显示 `name`。
- 路由规则 `from`：有 `via` 的人只按 `Via::key()`（`<渠道>:<用户 id>`，如 `slack:U012ABCDEF`）匹配，不按名字匹配。渠道里的显示名谁都能改，改成什么都冒用不了按名字写的规则。控制台用户仍按名字匹配。`RouteInput` 为此新增了 `via`。
- 审批记录里怎么写 `by`，见 D-07。

### D-08 表结构 · 已实现（#24）

绑定和任务、审批一样，走事件日志：新增事件类型 `binding.put`，再更新投影表。另外三张表只是运行状态，不写事件。

```sql
-- 外部会话 ↔ 对话。走事件日志（binding.put）。
CREATE TABLE IF NOT EXISTS channel_bindings (
    channel       TEXT NOT NULL,   -- ip.yaml 里的渠道名
    conversation  TEXT NOT NULL,   -- 外部会话 key
    thread_id     TEXT NOT NULL,
    agent_id      TEXT,            -- 粘住的员工；路由卡片未选定时为空
    requester     TEXT NOT NULL,   -- 开启会话的外部用户 id（D-07）
    created_at_ms INTEGER NOT NULL,
    seq           INTEGER NOT NULL,
    PRIMARY KEY (channel, conversation)
) WITHOUT ROWID;
-- 第一版一个对话只绑一个外部会话。
CREATE UNIQUE INDEX IF NOT EXISTS channel_bindings_by_thread ON channel_bindings(thread_id);

-- 收消息去重。只保留 7 天，启动时和每天清理一次。
CREATE TABLE IF NOT EXISTS channel_inbox (
    channel    TEXT NOT NULL,
    dedupe_key TEXT NOT NULL,
    at_ms      INTEGER NOT NULL,
    PRIMARY KEY (channel, dedupe_key)
) WITHOUT ROWID;

-- 确认前存下的原始信封（D-28）。处理完标 done 并清掉内容，放弃的标 dropped；
-- 两种都在去重记录的保留期满后由 prune_inbound 删掉。
CREATE TABLE IF NOT EXISTS channel_envelopes (
    rowid       INTEGER PRIMARY KEY,
    channel     TEXT NOT NULL,
    id          TEXT NOT NULL,               -- Slack 的 envelope_id
    payload     TEXT NOT NULL,
    received_ms INTEGER NOT NULL,
    attempts    INTEGER NOT NULL DEFAULT 0,  -- 失败过几次，跨重启累计
    state       TEXT NOT NULL DEFAULT 'pending',  -- pending | done | dropped
    error       TEXT,
    UNIQUE (channel, id)
);

-- 已发出的东西：防重发，以及更新卡片时要用的外部引用。
CREATE TABLE IF NOT EXISTS channel_deliveries (
    channel      TEXT NOT NULL,
    item         TEXT NOT NULL,   -- message:<id> | card:<approval id> | resolved:<approval id> | report:<会话>:<文件名>:<哈希>（D-38）
    conversation TEXT NOT NULL,
    external_ref TEXT,            -- Slack 消息的 ts；失败时为空
    state        TEXT NOT NULL,   -- sent | failed
    error        TEXT,
    at_ms        INTEGER NOT NULL,
    PRIMARY KEY (channel, item)
) WITHOUT ROWID;

-- 投递器读到事件日志的哪个位置。
CREATE TABLE IF NOT EXISTS channel_cursors (
    channel TEXT PRIMARY KEY,
    seq     INTEGER NOT NULL
) WITHOUT ROWID;
```

`ip-store` 新增的接口：

- `claim_binding(Binding)`：会话还没有绑定时写入，有了就返回已有的。
- `set_binding_agent(channel, conversation, agent)`：只在还没记员工时补上（D-05）。
- `binding(channel, conversation)`、`binding_by_thread(thread)`。
- `events_after(seq, limit)`：返回 `StoredEvent { seq, kind, payload }`。
- `remember_inbound(channel, key)`：返回是不是第一次见到。`prune_inbound(before_ms)` 清理旧记录。D-28 加了只读的 `inbound_seen(channel, key)`。
- D-28：`stash_envelope(channel, id, payload)`、`pending_envelopes(channel, limit)`、`settle_envelope(channel, id, Settle)`。
- `delivery(channel, item)`、`put_delivery(Delivery)`：`Delivery.sent` 为 false 表示永久失败、已跳过。
- `cursor(channel)`、`put_cursor(channel, seq)`。
- `approval(id)`：按 id 查审批卡片，点击时用。

绑定的写入和更新都会写一条 `binding.put` 事件；去重、投递记录和游标都不写事件。

### D-09 假渠道 · 已实现（#23）

`ip-connectors::fake::FakeChannel` 放在 `fake` feature 后面，`ip-server` 的测试打开这个 feature。用法和 `ip-models` 的脚本模型一样：

```rust
let (channel, handle) = FakeChannel::new("fake");
// 把 channel 接到 Hub 和投递器上之后：
let first = handle.say("会话-1", "u1", "点评一下本月 CPI").await?;   // may_start = true，等 Hub 处理完
handle.reply("会话-1", "u1", "再看看 PPI").await?;                   // may_start = false
handle.send(first).await?;                                            // 原样重推，模拟外部系统重复推送
handle.click(&approval_id, 0, "u1").await?;                           // 返回 Hub 的结果，比如 Rejected
let sent: Sent = handle.next(Duration::from_secs(5)).await.unwrap(); // Sent { out: Outbound, external_ref }
let two = handle.take(2, Duration::from_secs(5)).await;               // 按发出的顺序
handle.fail_next(2, DeliverError::Retry { after: None, reason: "限流".into() });
handle.attempts();                                                    // deliver 被调用的次数，含失败的
```

- 用户 `u1` 的显示名是 `用户u1`。
- 每次 `say`、`reply` 的去重 key 都不同。
- 发出去的东西依次拿到引用 `fake-1`、`fake-2`……，测试用它核对 `CardResolved` 是不是更新了原来那张卡片。
- 一个假渠道只能 `run` 一次，模拟重启时新建一个。
- 去重 key 的编号在整个进程里递增（#24 修正）。新建的假渠道不会和之前的撞号；否则模拟重启后的第一条消息会被当成重复推送丢掉。

### D-10 渠道测试 · 已实现（#23 写测试，#24 实现后全部通过）

测试放在 `ip-server/src/channel_tests.rs`，用假渠道加脚本模型，进 CI。

- 测试先于实现写好，共 13 个：T1 多了一个“渠道配了固定员工时不走路由”，T9 拆成名单和 `approvers` 两个。
- 测试只通过文件开头的 `attach(app, channel, settings)` 接触 M2-1 的代码。`Settings` 对应 D-16 的 `allow`、`approvers`、`agent`。其余用的都是已有接口：存储、运行时、`work::resolve_approval`。
- #24 把 `attach` 接上了 `channels::start`，去掉了 `#[ignore]`。13 个测试全部通过，连跑 5 遍都稳定，每遍约 2.6 秒。
- 测试不读 D-08 的表，只看渠道收到了什么、对话里存了什么，所以表结构和存储接口怎么定，不影响这些测试。

| 编号 | 场景 |
| --- | --- |
| T1 | 新会话按路由分派，回复回到同一会话 |
| T2 | 同一会话追问，交给同一个员工，不重新路由 |
| T3 | 路由拿不准 → 卡片发到会话 → 点击 → 交给选定岗位，绑定补上员工 |
| T4 | 协调员派活 → 执行员工 `ask_human` → 卡片出现在最初的会话 → 点击 → 执行员工继续 |
| T5 | 在控制台选定卡片后，渠道里的卡片同步更新 |
| T6 | 同一条消息推送两次，只处理一次 |
| T7 | 重新打开存储、重建投递器后，不重发也不漏发 |
| T8 | 发送失败后重试，顺序不乱；永久失败的跳过，后面的照常发 |
| T9 | 名单外的人发消息被忽略；不符合 `approvers` 的点击不生效 |
| T10 | 执行对话里的工具调用、执行员工的回复不发到渠道 |
| T11 | 绑定的对话里只有 `turn_failed` 系统消息发到渠道，任务回报和选定说明不发 |
| T12 | Hub 内部出错的消息不记去重，渠道重试时照常处理；成功后再推才算重复（D-28） |
| T13 | 开会话时绑定写好了、第一条消息没写进去：重试时接着开会话（D-28） |
| T14 | 同一条消息同时交三次，只派一次活（D-28） |

---

## 4. Slack 渠道

### D-11 接入方式：先做 Socket Mode · 已实现（#25）

- Socket Mode 用 WebSocket 接收事件，主机不需要公网 HTTPS 入口，适合客户在自己的云主机和自己的 Slack 工作区里各建一个应用。
- 限制：用 Socket Mode 的应用不能上架 Slack Marketplace。我们是每家客户自建应用，不受影响。
- 需要上架或多工作区分发时，再加 HTTP Events API（Q-06）。
- 流程：
  1. 用应用级令牌（`xapp-`，权限 `connections:write`）调 `apps.connections.open`，拿到 wss 地址并连上。
  2. 每个收到的信封都要在 3 秒内回 `{"envelope_id": ...}` 确认。先确认，再异步处理。D-28 改为先存下、再确认、再异步处理。
  3. 收到 `disconnect` 就重新调 `apps.connections.open` 换新地址；断线后退避重连。
- 第一版只支持一个 ip 进程连一个 Slack 应用（Q-08）。
- 不用第三方 Slack SDK。用到的接口只有七八个，自己写一层薄封装：`tokio-tungstenite`（rustls）加现有的 `reqwest`。
- 实现在 `ip-connectors/src/slack.rs`。每个信封先回 `envelope_id` 确认，再交给后台任务按顺序处理，处理慢的消息不会耽误确认。断线后按 1、2、4…… 秒退避重连，最多等 30 秒；收到 `disconnect` 立即重连。
- 主机只能经代理出网时，WebSocket 连接按环境变量走代理，见 D-37。

### D-37 Socket Mode 经 HTTP 代理连接 · 提议（backlog E-12）

问题（2026-11-16 真实验收发现）：Web API 调用（`reqwest`）读 `HTTPS_PROXY`，Socket Mode 的 WebSocket（`tokio_tungstenite::connect_async`）不读。只能经代理出网的主机上 `ip check` 能过，`ip serve` 连不上 Slack。

- **读哪些环境变量**：和 reqwest 用同一套规则，都由 hyper-util 的 `Matcher::from_env` 解析（reqwest 本来就依赖 hyper-util，没有新增 crate）。`wss://` 看 `HTTPS_PROXY`/`https_proxy`，`ws://` 看 `HTTP_PROXY`/`http_proxy`，都没设时用 `ALL_PROXY`/`all_proxy`；`NO_PROXY`/`no_proxy` 里的主机直连（规则同 curl：域名后缀、IP、CIDR、`*`）。进程启动、创建 Slack 渠道时读一次。`ip.yaml` 没有新配置项。
- **怎么连**：要走代理时先连代理，发 `CONNECT <主机>:<端口> HTTP/1.1`，代理回 2xx 后在这条隧道上做 TLS 和 WebSocket 握手。TLS 是端到端的，代理只看到主机名和端口。连代理加等它回应最多 30 秒。
- **认证**：代理地址里带用户名密码（`http://用户:密码@主机:端口`，特殊字符用百分号编码）时，用 `Proxy-Authorization: Basic …` 发给代理。日志和报错里只写代理的主机和端口，不写凭据。连上时的日志写明是直连还是经哪个代理。
- **TLS 信任**：tokio-tungstenite（`rustls-tls-native-roots`）和 reqwest（rustls-platform-verifier）在 Linux 上都用 rustls-native-certs 读根证书，`SSL_CERT_FILE`、`SSL_CERT_DIR` 两边都认。容器用自定义 CA 时，设好这两个变量，Web API 和 WebSocket 一起生效。
- **报错**：代理回 407 时，分“没给用户名密码”和“用户名密码不对”两种提示；回 403 时提示检查代理的放行名单；其他状态码原样带上；连不上代理、超时、代理地址是 `https://` 或 `socks5://`（暂不支持，只支持 `http://` 代理）都单独说明。这些都是连接失败，照常退避重连。
- **本机转发器**：2026-11-16 验收时用的本机转发器不再是必须的，只作为代理不允许 `CONNECT` 时的退路。
- 实现在 `ip-connectors/src/ws_proxy.rs`。
- 测试：`ws_proxy` 的单元测试用本地假代理（接受 `CONNECT`、转发到本地假 WebSocket 服务器）测经代理连通、Basic 认证、`NO_PROXY` 直连、按协议选代理、407 和 403 的报错（不含凭据）、不支持的代理协议、代理连不上；Slack 测试 `socket_mode_connects_through_http_proxy` 经假代理连假 Slack，事件照常收到和确认。都是假代理、假 Slack；真实 Slack 经真实代理还没验。

### D-38 回复里提到的报告作为文件发进线程 · 提议（backlog E-13）

问题（2026-11-16 真实验收发现）：员工回复里写的 `reports/<文件名>.md` 是它工作目录里的相对路径。在 Slack 里，写成 Markdown 链接的会变成点不开的链接，没写成链接的只是一段文字，看回复的人拿不到报告。控制台和 App 能点开（F-4、F-14），渠道没有对应的办法。

不采用的做法：回复里放控制台的链接。Slack 里的人多数没配对过设备，主机也不一定有对外的地址。

**分工**：认报告、读文件、改写文字放在投递器（`ip-server/src/channels.rs`），和渠道无关；渠道只负责把一份 Markdown 文件发进会话。以后接飞书、邮件，只要实现新的 `Outbound` 变体。

**1. 接口（`ip-connectors`，D-02 的补充）**

```rust
pub enum Outbound {
    // …原有三种不变…
    /// 一份报告：作为文件发进会话。渠道发不了文件时，可以把正文当文字发（见第 4 条）。
    Report {
        conversation: ConversationKey,
        filename: String,   // 不带目录，如 2026-11-16-cpi.md
        title: String,      // 报告第一个一级标题，没有就用文件名
        markdown: String,   // 正文，UTF-8，不超过 reports::MAX_READ_BYTES
    },
}
```

`Delivered.external_ref` 对报告来说是外部系统里的文件 id（Slack 的 `F…`）。

**2. 投递器认报告（D-04 的补充）**

只看会发进渠道的员工文字消息（D-04 表里第一行），系统消息（`turn_failed`）不看。

- **认哪些**：和控制台 `web/src/markdown.js` 的规则一致：正文、Markdown 链接的地址、行内代码里出现的 `reports/<文件名>.md`；前面紧挨着路径字符的（比如 `/tmp/x/reports/a.md`）不算。文件名的规则沿用 `reports::valid_name`。同一条消息里重复提到的只算一次，按第一次出现的先后排。
- **找谁的工作目录**：先找写这条消息的员工；他没有这份报告时，在其他员工里找，正好一位有就用他的，零位或多位都不附。读文件一律走 `reports::read_in`（挡子目录、`..`、隐藏文件、符号链接，核对真实路径），不另写一套。读文件在 `spawn_blocking` 里做。
- **附几份**：一条消息最多附 3 份，多出的只改写文字、不附。读出来被截断（超过 1 MB）的不附。
- **改写文字**：只改写地址是报告路径的 Markdown 链接，`[链接文字](reports/x.md)` 改成 ``链接文字（附件 `x.md`）``；没附上的（找不到、太大、超过 3 份）改成 ``链接文字（`reports/x.md`）``。不是链接的提法不动。改写只影响发进渠道的那份文字，对话里存的原文不变。
- **顺序和防重发**：一条消息先发文字（条目名照旧 `message:<消息 id>`），再按顺序发各份报告，条目名 `report:<会话>:<文件名>:<正文 SHA-256 的前 16 个十六进制字符>`。这样同一会话里同一份报告、同一个版本 7 天内只发一次（沿用 D-04 每日清理任务，只清理 `channel_deliveries` 中超过 7 天的报告条目）；报告改过以后再被提到会再发一次。不加新表，`channel_deliveries.item` 的注释补上 `report:…`。
- **失败**：和其他条目一样。`Retry` 退避重试、不越过；`Permanent` 记失败、打警告、跳过，不影响已经发出的文字。失败记录也占着这个条目名，所以同一会话里同一版本的报告 7 天内不会再试；报告改过以后照常再发。
- 重启后重读同一批事件时，报告按发送时磁盘上的内容读，可能比当初回复时新。接受。

**3. Slack 怎么发文件（D-14 的补充）**

`files.upload` 已停用，用三步上传：

1. `files.getUploadURLExternal`（表单参数 `filename`、`length`）拿 `upload_url` 和 `file_id`。
2. 把正文原样 `POST` 到 `upload_url`。这一步不带机器人令牌，不跟随重定向；`upload_url` 必须是 `https`，主机名是 `slack.com` 或其子域，不能带用户名密码。只有在测试和 `fake` feature 下，才另外放行和 `api_url` 同源的 `127.0.0.1` 地址（发布版的 `ip` 不开 `fake`）。走同一个 `reqwest` 客户端，所以代理规则照旧。
3. `files.completeUploadExternal`（JSON：`files: [{ id, title }]`、`channel_id`、`thread_ts`），文件出现在线程里。

错误按 D-14 分可重试和永久失败。第 2 步上传成功、第 3 步失败后重试时，从第 1 步重新来（旧的 `file_id` 没完成，Slack 会自己清掉）。

**4. 没有 `files:write` 权限时**

已经装好的应用没有这项权限，要在 Slack 后台加上并重装。没重装时，第 1 步会回 `missing_scope`。这时不算失败：在线程里把正文当文字发出去，开头一行 ``**<标题>**（`<文件名>`。应用还没有 files:write 权限，正文直接贴在这里）``，太长就按 D-14 的规则切成几条。每个渠道第一次遇到时打一条警告日志，写明要在 Slack 后台加 `files:write` 并重装应用。

**5. 应用配置和自检**

- D-15 的机器人权限加 `files:write`，`examples/slack/manifest.yaml` 同步。
- `ip check` 的 Slack 一项读 `auth.test` 回应头里的 `x-oauth-scopes`。缺 `files:write` 时这一项照常通过，另外附一句提醒，说明报告会贴成文字，要加权限并重装应用。回应里没有这个头时不提醒。
- 验收手册和 Slack 部署说明加一句：老应用要补 `files:write` 并重装。

**6. 测试和基准**

- 假 Slack（`fake::slack`）支持三步上传，记下上传的正文。
- `ip-connectors/src/slack/tests.rs`：三步上传的请求和字段、`missing_scope` 时退成文字、长报告切条、`upload_url` 不是 Slack 的地址时拒绝、错误分类。
- `ip-server` 的渠道测试：认报告的规则（正文、链接、行内代码、前面带路径的不算、去重、顺序）、链接改写、先找作者再找唯一的其他员工、零位或多位时不附、最多 3 份、超过 1 MB 不附、`..` 和符号链接不附、同一会话同一版本只发一次、改过以后再发、报告发失败不影响文字。
- `slack_tests.rs` 端到端：假模型写一份报告、回复里提到它，线程里先出现改写过的回复，再出现文件。
- 基准：认报告和改写文字是每条发往渠道的员工消息都要走的路径，在 `ip bench` 里加一项（没有合适的就新开 `ip bench deliver`），目标值实现时定，写进 `rust/README.md`。
- 只能用假 Slack 测；真实工作区里上传和 `missing_scope` 的提示要等下一次 Slack 真实验收（A-5 或 M2-5 重跑）再算验过。

**7. 敏感路径**

改的是 `ip-connectors/` 和 `ip-server/src/channels.rs`，属于“外部渠道”。报告正文会发到这个线程所在的 Slack 工作区，和回复本身去的是同一个地方；Slack 按工作区的保留策略存这些文件。PR 要一位工程师读过代码再合并。

### D-12 会话怎么划分 · 已实现（#25）

- 会话 key：`<team_id>/<channel_id>/<thread_ts>`。顶层消息的 `thread_ts` 就是它自己的 `ts`，机器人总在这个线程里回复。
- **频道**：只有 @机器人 的顶层消息能开启会话（`may_start = true`）。已绑定的线程里，名单内的人继续说话不用再 @。
- **私信**：规则相同，每条顶层私信开启一段新会话，回复在线程里（Q-01）。
- 去重 key 用 `msg:<channel_id>:<ts>`，不能只用 `event_id`。同一条 @机器人 的消息会同时触发 `app_mention` 和 `message.*` 两个事件，两者的 `event_id` 不同。卡片点击用 `click:<action_ts>:<user>`。
- 实现时补充的两条：
  - `message` 事件的文字里含 `<@机器人>` 时，也算能开启会话。否则 `message` 事件先到、按“不能开启会话”被忽略并记下去重 key，紧跟着的 `app_mention` 会被当成重复丢掉。现在两个事件解析出来完全一样，先到的生效。
  - 已有的线程里 @机器人，也能开启会话，会话就是这个线程。私信线程里的回复不能开启会话，只在已绑定时生效。

### D-13 收消息时过滤 · 已实现（#25）

- 忽略带 `bot_id` 的消息、`user` 是机器人自己（启动时用 `auth.test` 取得）的消息，以及 `message_changed`、`message_deleted`、`channel_join` 等子类型。这样机器人不会回复自己，也不会跟别的机器人互相回复。
- 去掉文字里的 `<@机器人id>`。
- 发送人的显示名用 `users.info` 查，缓存 24 小时，存在内存里即可。依次取 `profile.display_name`、`profile.real_name`、`real_name`、`name` 里第一个非空的；查不到时用用户 id。
- 只 @ 了机器人、没写别的字的消息，忽略。
- 子类型里只放行 `thread_broadcast` 和 `file_share`，其余（编辑、删除、进出频道等）都忽略。
- 配置里的 `allow.channels` 在这一层过滤；`allow.users` 由 Hub 过滤（D-03）。

### D-14 往外发 · 已实现（#25）

- **文字**：调 `chat.postMessage`，带 `thread_ts`。员工写的是标准 Markdown，Slack 的 mrkdwn 语法不同。优先用 Slack 的 `markdown` 块，太长就按段落切成几条发。具体长度上限开工时对照 Slack 文档核对。
- **卡片**：用 Block Kit 拼：
  - 一个 section 放问题和理由。
  - 一个 actions 块，每个选项一个按钮：`action_id = ip_choice_<序号>`，`value = <approval_id>:<序号>`，推荐项用 `style: primary`。
  - 一个 context 块写各选项的后果。
- **卡片选定后**：用 `chat.update`（借助记下的 ts）去掉按钮，改成“<名字> 选了 <选项>”。
- ~~**收到消息时**：给原消息加一个 :eyes: 表情（`reactions.add`），表示收到了。~~ 作废：渠道不知道 Hub 是受理了还是忽略了这条消息（名单外、未绑定的线程等），加了表情会让被忽略的消息也显示“已收到”。等以后 `Hub::message` 能返回是否受理时再加。第一版也不发“正在处理”之类的文字。
- 文字超过 11,000 字时按段落切成几条发（`markdown` 块一条消息总共 12,000 字的上限留了余量）。工作区不认 `markdown` 块（返回 `invalid_blocks`）时，退回纯文字重发一次，回复不会丢。
- mrkdwn 文字里的 `&`、`<`、`>` 都转义；按钮文字截到 75 字，section 截到 3,000 字以内。
- Web API 的错误：网络错误、HTTP 429（带 `Retry-After`）、5xx，以及 `ratelimited`、`internal_error`、`fatal_error`、`service_unavailable`、`request_timeout` 都算可重试；其余（比如 `channel_not_found`、`not_in_channel`）算永久失败。
- **限流**：同一个频道每秒大约只能发一条。投递器本来就是一个渠道顺序发，遇到 429 按 `Retry-After` 等待（对应 `DeliverError::Retry`）。

### D-15 Slack 应用配置 · 已实现（#25）

仓库里提供 `examples/slack/manifest.yaml`，客户在 Slack 后台用它一键建应用。开工时对照 Slack 文档核对下面的清单：

- 机器人权限：`app_mentions:read`、`chat:write`、`channels:history`、`groups:history`、`im:history`、`users:read`，D-38 提议再加 `files:write`（不加表情，所以不需要 `reactions:write`，见 D-14）
- 订阅事件：`app_mention`、`message.channels`、`message.groups`、`message.im`
- 打开交互（按钮点击会通过 Socket Mode 送达）
- 应用级令牌：`connections:write`

### D-16 `ip.yaml` 配置 · 已实现（#25）

```yaml
channels:
  slack:
    kind: slack
    bot_token_env: SLACK_BOT_TOKEN      # xoxb-…，密钥只放环境变量
    app_token_env: SLACK_APP_TOKEN      # xapp-…，Socket Mode 用
    allow:                              # 省略表示工作区里所有人、所有频道
      users: [U012ABCDEF]
      channels: [C012ABCDEF]
    approvers: requester                # requester | anyone | [U012ABCDEF, …]
    # agent: macro-1                    # 可选：这个渠道的消息都交给固定员工，不走路由
    # api_url: https://slack.com/api    # 一般不用改；测试里指向假服务器
```

实现时补充：

- 渠道名只能用字母、数字、`-`、`_`。它会出现在 `<渠道>:<用户 id>` 里，所以不能有冒号。
- `approvers` 写错、`agent` 指向不存在的员工，启动时拒绝。
- 令牌只能写环境变量名。环境变量没设置时拒绝启动。
- 渠道在 `App::from_config` 里接入，关闭服务时先停掉渠道，再停员工。

### Slack 测试和验收

已实现（#25），用假 Slack 服务器（`ip-connectors::fake::slack`，`fake` feature）。它模拟 Web API 的几个方法和 Socket Mode 的 WebSocket，CI 不依赖真实的 Slack。

- `ip-connectors/src/slack/tests.rs`：
  - 事件怎么变成交给 Hub 的消息：@机器人、同一条消息的两个事件、线程接话、别的机器人、自己、编辑、私信、名单外频道。每个信封都确认；显示名和机器人身份都有缓存。
  - 点击交给 Hub，被拒绝的点击只给点的人看原因，别的按钮不理。
  - 发文字（`markdown` 块、按段落切开、`invalid_blocks` 时退回纯文字）、发卡片（按钮、推荐项高亮、转义）、更新卡片。
  - 错误分成可重试和永久失败。
  - 收到 `disconnect` 后重连。
- `ip-server/src/slack_tests.rs`：
  - 端到端：从 `ip.yaml` 的配置建出渠道，在频道里 @机器人 → 宏观研究员回复到同一线程 → 规则不命中时发卡片 → 别人点不生效、只告诉他本人 → 提需求的人点了以后，原卡片更新、员工接手回复 → 名单外的频道不理会。
  - `channels` 配置的解析和各项校验。
- 真实验收（M2-5）：在真实工作区里人工走一遍 T1–T5，再完成真实模型的端到端流程。Slack 的接口和字段名原来是按资料写的；2026-11-16 在真实工作区的公开频道里走了一遍（[记录](acceptance-2026-11-16-slack.md)），用到的接口（`auth.test`、`apps.connections.open`、`users.info`、`chat.postMessage`、`chat.postEphemeral`、`chat.update`）和事件字段都对得上，没有改 `slack.rs`。私有频道和私信没验。

---

## 5. MCP 服务端（`ip-mcp` 的服务端部分）

目的：Codex 这类外部引擎有自己的工具循环，调不到 ip 的组织工具。ip 开一个 MCP 服务端，引擎连进来用。

### D-17 传输方式 · 已实现（#21）

在现有的 axum 服务上挂 `/mcp`，用 Streamable HTTP，即 rmcp 的 `transport-streamable-http-server` feature。Codex 在 `mcp_servers.<名字>` 里配 `url`，令牌通过 `bearer_token_env_var` 指定的环境变量传入。

- 服务是无状态的：不发会话 id，每个请求单独处理，回复用 JSON。
- 请求先在 HTTP 层检查令牌，没有令牌或令牌无效直接返回 401，不进 MCP 协议层。
- 只接受本机的 `Host`（`localhost`、`127.0.0.1`、`::1`），这是 rmcp 默认的 DNS rebinding 防护。引擎和 ip 跑在同一台机器上，用 `http://127.0.0.1:<端口>/mcp` 连接。
- 协议层在 `ip_mcp::server`（`http_service`、`TurnTools`），挂载和 401 检查在 `ip-server/src/mcp.rs`。

### D-18 每轮一个令牌 · 已实现（#21）

- 引擎每开始一轮，运行时生成一个随机令牌，绑定 `(agent_id, thread_id)`。这一轮结束后令牌作废，另留 60 秒宽限。
- 接口：`Runtime::grant_turn(agent, thread, ttl)` 签发，`Runtime::end_turn(token, TURN_TOKEN_GRACE)` 结束，`Runtime::turn_token_valid(token)` 检查。
- 签发时必须给最长期限 `ttl`（引擎传 `turn_timeout_secs` 再留点余量），到期自动作废。引擎异常退出、没来得及结束这一轮，也不会留下长期有效的令牌。结束一轮只会缩短期限，不会延长。
- 令牌是 `ipt_` 加 256 位随机数的十六进制。
- 令牌只存在内存里，进程重启后全部失效。
- `/mcp` 只认这种令牌，不认控制台的访问令牌。
- 工具调用时由令牌得到“我是谁、在哪个对话”，再调 `work_tools::call(shared, me, thread, name, args)`。员工不能冒充别人，也不能在别的对话里派活。

### D-19 开放哪些工具 · 已实现（#21）

- 组织工具：`create_task`、`update_task`、`list_tasks`、`ask_human`。
- 该岗位能用的数据工具：`data_catalog`、`data_series`。
- 岗位开了记忆时的三个记忆工具：只读的 `memory_search`、`memory_read`，和人明确要求时记一条更正的 `memory_note`。都只碰本岗位的（`context-memory-design.md` 5.5、5.6）。
- 该岗位能用的外部 MCP 工具，由 ip 转发。这样权限只在 `ip.yaml` 一个地方管，密钥也不交给引擎。
- 不开放 shell 和文件工具，Codex 有自己的。调这些名字会得到工具级错误“没有工具 …”。
- 实现上，员工自己的循环和 MCP 服务端用的是同一套工具（`ip-runtime` 的 `OrgTools`），两边开放的工具和权限不会分叉。
- MCP 服务端不往对话里写工具调用消息，由引擎那边统一记录（D-24），避免记两遍。
- `ask_human` 的返回语义和内置循环一样：“卡片已发出，人选定后结果会写进对话并叫回你；现在结束这一轮。”

### MCP 服务端测试

已实现（#21）：

- `ip-server/src/mcp_tests.rs`：用 rmcp 的 Streamable HTTP 客户端连本进程的 `/mcp`。
  - 列出的工具正好是四个组织工具。
  - `create_task` 建出的任务记在令牌对应的员工名下，派活对话是令牌对应的对话。
  - `ask_human` 的卡片记在令牌对应的员工和对话名下（D-06 靠它找回会话）。
  - 调 `shell` 得到工具级错误。
  - 一轮结束后，同一连接上的调用被拒绝。
  - 没有令牌、随便写的令牌、控制台的访问令牌，都在 HTTP 层得到 401；给不存在的员工签发令牌会报错；令牌到期作废，结束一轮不会延长期限。
- `ip-runtime` 的 `turn_tokens_expose_org_and_data_tools_but_not_workspace_tools`：员工有工作目录和数据源时，开放的工具包含数据工具、不含 shell 和文件工具；`write_file` 调不动，文件没有写出来。
- 员工身份由令牌决定，工具参数里没有可以指定员工的地方，所以拿 A 的令牌没法以 B 的身份操作。

---

## 6. 外部引擎：Codex、Claude Code（`ip-engines`）

### D-20 引擎接口 · 已实现（#22）

- `ip-runtime::engine` 定义 `Engine` 接口。员工的 `AgentSetup.engine` 有值时，每一轮交给引擎；没有时照旧走内置循环。
- 和提议的不同：内置循环没有包装成 `BuiltinEngine`。它和引擎走的是两条路径，硬套同一个接口只会多一层转发。
- `ip-engines` 依赖 `ip-runtime` 实现 Codex 引擎，由 `ip-server` 在启动时按配置连起来。
- 运行时负责引擎前后的事：查预算、挑出上一轮之后的新消息、签发本轮令牌、记账、保存会话。引擎只管跑这一轮，并通过 `sink` 把过程写回对话。

```rust
pub trait Engine: Send + Sync + 'static {
    fn kind(&self) -> &str;               // 如 "codex"，写进引擎会话表
    fn turn_timeout(&self) -> Duration;   // 本轮令牌的期限据此确定
    fn turn<'a>(&'a self, ctx: TurnContext<'a>) -> BoxFuture<'a, TurnReport>;
}

pub struct TurnContext<'a> {
    pub agent: &'a Agent,
    pub role: &'a Role,
    pub thread: &'a ThreadId,
    pub workspace: Option<&'a Path>,
    pub session: Option<&'a str>,     // 引擎在这个对话里已有的会话；None 表示第一轮
    pub session_usage: Usage,         // 这个会话到上一轮为止累计报告的用量；没有会话时全是 0（v0.20）
    pub system_prompt: &'a str,       // 组织规则、岗位说明、同事名单
    pub input: &'a str,               // 上一轮之后的新消息（不含员工自己写的），已整理成文字
    pub mcp: Option<McpGrant>,        // 本轮的 /mcp 地址和令牌（D-18）
    pub sink: &'a dyn TurnSink,       // 以员工的名义写消息
}

pub struct TurnReport { pub steps: u32, pub usage: Usage, pub cost_usd: f64,
                        pub session_usage: Option<Usage>,   // 会话到这一轮为止的累计用量；None 表示这一轮没拿到（v0.20）
                        pub session_id: Option<String>, pub error: Option<String> }
```

`TurnReport` 用 `error` 字段表示失败，而不是返回 `Result`：这样即使一轮失败，已经产生的用量也能如实记账。

### D-21 只给运维测试岗位用 · 已实现（#22）

和提议的不同：配置放在顶层的 `engines` 下，键是岗位 id，不写进岗位定义里。原因是岗位（`ip_core::Role`）是领域类型，前端也用；而引擎配置是部署细节（可执行文件路径、密钥变量名、价格），和 `endpoints` 是一类东西。这样岗位定义和前端类型都不用改。

```yaml
engines:
  ops-tester:
    kind: codex
    command: codex                 # 可执行文件，默认 codex
    model: <Codex 模型名>          # 传给 codex -m；省略时用 Codex 的默认模型
    api_key_env: CODEX_API_KEY     # 密钥只放环境变量，启动时读取，缺失时拒绝启动
    sandbox: workspace-write       # read-only | workspace-write
    network: false                 # Codex 里执行的命令能否联网
    turn_timeout_secs: 1800
    max_concurrency: 4             # 这个岗位同时最多几个 Codex 进程
    price: { input: 0, cached_input: 0, output: 0 }   # 美元 / 百万 token，用来记账和执行 budget_per_day_usd；也可以写 cache_write：codex-cli 0.169.0 的用量里有缓存写入（`cache_write_input_tokens`），按它计价
```

配置校验：

- `engines` 里的岗位必须存在。
- 用引擎的岗位设了 `budget_per_day_usd`，就必须写 `price`。
- 这个岗位的员工不能 `tools: false`，因为引擎需要工作目录。
- 用引擎的岗位不调 `model` 里的模型，所以不检查它的端点。字段本身仍要写，因为岗位定义要求有。

### D-22 每轮起一个进程 · 已实现（#22）

- 每轮跑一次 `codex exec --json`，跑完进程就退出。空闲员工不占进程，“每个空闲员工只占几 KB”这一点不变。
- 第一轮：从标准输入传入系统提示词、分隔线，再加对话里的新消息（Q-05）。
- 之后几轮：参数末尾加 `resume <Codex 会话 id>`，标准输入只传上一轮之后的新消息，比如人的追问、任务说明、任务结果、卡片选定结果。
- 每次调用的参数：
  - `exec --json --skip-git-repo-check`，工作目录不一定是 git 仓库
  - `--cd <workspace>`、`--sandbox <sandbox>`
  - `-c approval_policy="never"`
  - `-m <model>`（配置了模型时）
  - `-c mcp_servers.ip.url="<地址>"`、`-c mcp_servers.ip.bearer_token_env_var="IP_MCP_TOKEN"`（有 `/mcp` 时）
  - `-c sandbox_workspace_write.network_access=true`（`network` 为 true 时）
  - 最后是 `-`，表示从标准输入读提示词
- MCP 地址用本机回环地址加服务监听的端口：`http://127.0.0.1:<端口>/mcp`。由 `ip serve` 开始监听后设置（`Runtime::set_mcp_url`）。
- 本轮令牌的期限是 `turn_timeout_secs` 加 60 秒；这一轮结束时调 `end_turn`。
- 环境变量全部清空，只给 `PATH`、`HOME`、`CODEX_HOME`、`CODEX_API_KEY`、`IP_MCP_TOKEN`。
- 和提议的不同：`CODEX_HOME` 指向 `data_dir/engines/<员工 id>/codex`，`HOME` 指向 `data_dir/engines/<员工 id>`，都不在工作目录（`data_dir/workspaces/<员工 id>`）里。这样 Codex 在自己的沙箱里改不到自己的配置和会话记录。每个员工互相隔离，也不读宿主机的 `~/.codex`。
- 进程放在单独的进程组里，超时就整组终止，和 `ip-tools` 处理 shell 超时一样。
- `turn_timeout_secs` 从排队等空位就开始算（`max_concurrency` 用满时要排队），否则本轮令牌可能在 Codex 还没开始时就过期。
- 提示词在单独的任务里写进标准输入：提示词超过管道缓冲、而进程又不读时，不会卡在超时保护之外。
- 参数顺序（`exec` 的选项放在 `resume` 之前）和 JSONL 格式，原来是按 2026-11 的资料写的。2026-11-14 用真实 Codex（`codex-cli 0.169.0`）验收（M2-6）时核对过：参数顺序可用，输出解析改了四处（给组织工具的 MCP 服务器加 `default_tools_approval_mode="approve"`；中途的顶层 `error` 事件只当警告，成败看 `turn.failed` 和 `turn.completed`；空的 `"error": null` 不算出错；`cache_write_input_tokens` 计入用量）。见 `docs/ip/acceptance-2026-11-14-codex.md`。只在这一个版本上跑过。

### D-23 引擎会话表 · 已实现（#22）；累计用量四列是提议（v0.20，等工程师审查）

```sql
CREATE TABLE IF NOT EXISTS engine_sessions (
    agent_id      TEXT NOT NULL,
    thread_id     TEXT NOT NULL,
    engine        TEXT NOT NULL,   -- codex
    session_id    TEXT NOT NULL,   -- Codex 的会话 id（thread.started 事件里给出），resume 时用
    last_message  TEXT NOT NULL,   -- 已交给引擎的最后一条消息；下一轮从它之后取新消息
    updated_at_ms INTEGER NOT NULL,
    -- 这个会话到上一轮为止累计报告的用量（v0.20）
    used_prompt_tokens      INTEGER NOT NULL DEFAULT 0,
    used_cached_tokens      INTEGER NOT NULL DEFAULT 0,
    used_cache_write_tokens INTEGER NOT NULL DEFAULT 0,
    used_completion_tokens  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (agent_id, thread_id)
) WITHOUT ROWID;
```

- 这张表是运行状态，不写事件日志：每轮都会更新，写进日志只会让日志膨胀。
- `last_message` 只在一轮成功后前移，并且取这一轮开始时对话里的最后一条消息。所以这一轮进行中新来的消息不会被跳过，失败的一轮下次会重新交一遍。
- 岗位换了引擎种类时，旧会话不再续接，从头开始。
- 累计用量（v0.20，backlog E-10）：`codex exec resume` 的 `turn.completed` 报的是整个会话的累计用量，不是这一轮的。运行时把已存的累计值经 `TurnContext::session_usage` 交给引擎，引擎报回这一轮的差值（`usage`、`cost_usd`，记账用）和新的累计值（`session_usage`，存回这张表）。
  - 报的数每一项都不小于已存的累计值时，这一轮是两者之差。有一项更小，说明报的不是累计值，按这一轮的用量全额记，累计值往上加：宁可多记也不漏记。
  - Codex 另开了会话（`thread.started` 给的 id 和续接的不一样）时，基线是 0。
  - 失败的一轮不前移 `last_message`，但累计用量照存，否则下一轮会把这一轮已经记过的用量再记一次。
  - 旧库打开时自动补上这四列，默认 0；旧会话的下一轮会按全量记一次（多记这一次），之后正常。

### D-24 Codex 输出怎么记进对话 · 已实现（#22）

逐行解析 JSONL 输出，按下表处理：

| Codex 事件 | 怎么处理 |
| --- | --- |
| `thread.started` | 记下会话 id，这一轮成功后写入 `engine_sessions` |
| `item.completed`，类型 `agent_message` | 写成 `Body::Text` |
| `item.completed`，类型 `command_execution` | 写成工具调用 `codex.shell`（命令）和工具结果。退出码为 0 才算成功；输出截断到 16 KB，失败时附上退出码 |
| `item.completed`，类型 `file_change` | 写成工具调用 `codex.edit`，结果写改动清单（每行“类型 路径”） |
| `item.completed`，类型 `mcp_tool_call` | 写成工具调用 `mcp__<服务器>__<工具>` 和工具结果 |
| `item.completed`，类型 `reasoning` | 不记 |
| `turn.completed` | 按 `usage`（输入、缓存命中输入、缓存写入 `cache_write_input_tokens`、输出）和 `price` 折算费用，计入岗位预算 |
| `turn.failed`、`error` | 本轮失败，照旧走“这轮失败”和“任务受阻” |
| 进程超时 | 整组终止，本轮失败 |
| 退出码非 0、又没有 `turn.completed` | 本轮失败，原因里附上 stderr 末尾 4 KB |
| 不认识的事件、不是 JSON 的行 | 打调试日志后忽略 |

步数等于工具调用数加回复条数。第一版不发逐字的 Delta 事件。

### D-25 预算 · 已实现（#22）

- 每轮开始前检查预算，用满了就不启动这一轮。
- 一轮进行中拿不到用量，所以最多超出一轮的费用。用 `turn_timeout_secs` 兜底（Q-06）。
- 失败的一轮也按报告的用量记账。

### Codex 引擎测试

已实现（#22）：

- **单元测试**（`ip-engines/src/codex.rs`）：配置默认值，不允许的沙箱级别和拼错的字段被拒绝；首轮和续接时的命令行参数；各类 JSONL 事件和条目的解析。
- **端到端测试**（`ip-server/src/engine_tests.rs`）：用 bash 写的假 `codex`，记录每次调用的参数、标准输入、环境变量和工作目录，再按场景输出录好的 JSONL。
  - 第一轮带系统提示词新开会话，第二轮用 `resume sess-1`，标准输入只有新消息。
  - 输出按 D-24 记进对话，作者都是这位员工。
  - `CODEX_HOME`、`HOME` 在 `data_dir/engines/<员工 id>`，工作目录是员工的工作目录，环境里有本轮令牌。
  - 用量按价格计费，计入岗位当天的花费。
  - 超时 1 秒立即整组终止，不等脚本里的 `sleep 30`；派给它的任务标为受阻，派活对话收到任务回报，执行对话里有“这轮失败”。
  - 异常退出时，失败原因带上 stderr。
  - 假 codex 用 curl 调 `/mcp` 的 `update_task` 交回任务：任务完成，结果回到派活的对话。
  - `engines` 配置的解析和各项校验。


### D-36 Claude Code 引擎 · 已实现（backlog E-6）

和 Codex 一样套 D-20 的 `Engine` 接口，每轮起一个进程，跑完就退出。排队、超时整组终止、收 stderr 这些和 Codex 共用一份（`ip-engines/src/process.rs`），只是参数和输出格式不同。参数和输出格式按 Claude Code 2.1.330 写成，用它连假的模型接口核对过（见本节最后）；换版本时要重新核对。

```yaml
engines:
  ops-tester:
    kind: claude_code
    command: claude                # 可执行文件，默认 claude
    model: <模型名>                # 传给 --model；省略时用 Claude Code 的默认模型
    api_key_env: ANTHROPIC_API_KEY # 必填：--bare 只认 API key，不读登录状态和钥匙串
    # base_url: https://...        # 兼容 Anthropic 接口的网关，以 ANTHROPIC_BASE_URL 交给 Claude Code
    access: workspace-write        # read-only | workspace-write
    allowed_tools: ["Bash(cargo test:*)"]   # 预先放行的权限规则，原样交给 --allowedTools
    turn_timeout_secs: 1800
    max_concurrency: 4
    price: { input: 0, cached_input: 0, cache_write: 0, output: 0 }   # 美元 / 百万 token
```

配置校验：D-21 的几条照样适用。另外，`model` 和 `allowed_tools` 里的每一条不能为空、不能以 `-` 开头、不能含控制字符，否则会被当成别的参数；`base_url` 要以 `https://` 或 `http://` 开头。

**每轮的参数**

- `-p --bare --restricted --output-format stream-json --verbose`。
  - `--bare`：不读 CLAUDE.md、钩子、插件、自动记忆，认证只认 `ANTHROPIC_API_KEY`。
  - `--restricted`：不读用户、项目、本地的设置文件，文件工具只能动工作目录，Bash 要在 `--tools` 里点名才有。
- `--permission-mode acceptEdits --permission-prompts none`：工作目录里的改动自动接受；要人批准的一律拒绝，因为没有人能回答。
- `--tools Bash,Edit,Read`，`access: read-only` 时只有 `Read`。`--bare` 下内置工具只有这三个，`--tools` 写别的也不会有。
- `--strict-mcp-config`：不读工作目录里的 `.mcp.json`。有 `/mcp` 时再加：
  - `--mcp-config '{"mcpServers":{"ip":{"type":"http","url":"<地址>","headers":{"Authorization":"Bearer ${IP_MCP_TOKEN}"}}}}'`。令牌不写进参数（`ps` 看得到），由 Claude Code 从环境变量展开。
  - `--allowedTools mcp__ip`：组织工具已经由本轮令牌限定范围，一律放行。
- `allowed_tools` 里的每一条：`--allowedTools <规则>`。
- `--append-system-prompt-file <data_dir>/engines/<员工 id>/prompts/<对话 id>.md`：每轮把系统提示词写进这个文件，一轮结束后删掉。续接时也给，因为 Claude Code 记下第一轮的提示词后会沿用，压缩上下文后才重新生成。
- `--model <model>`（配置了时）。
- 之后几轮：`--resume <会话 id>`，会话 id 取自 `system`/`init` 事件。

和 Codex 不同，系统提示词不进标准输入，标准输入里只有上一轮之后的新消息，第一轮也一样。

环境变量全部清空，只给这几个：

- `PATH`。
- `HOME`：`data_dir/engines/<员工 id>`。
- `CLAUDE_CONFIG_DIR`：`HOME` 下面的 `claude`，会话记录也在里面。
- `ANTHROPIC_API_KEY`，以及配了时的 `ANTHROPIC_BASE_URL`。
- `IP_MCP_TOKEN`。
- `DISABLE_AUTOUPDATER=1`、`CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`：不自动升级，不发遥测和错误报告。

**输出怎么记进对话**

| stream-json 事件 | 怎么处理 |
| --- | --- |
| `system`，subtype `init` | 记下 `session_id`。`mcp_servers` 里没连上的记成警告，这一轮没正常结束时附在失败原因里 |
| `assistant` 的 `text` 块 | 写成 `Body::Text` |
| `assistant` 的 `tool_use` 块 | 写成工具调用。内置工具记成 `claude.<工具>`（如 `claude.Bash`），MCP 工具照原名 `mcp__ip__<工具>` |
| `user` 的 `tool_result` 块 | 写成工具结果，`is_error` 为真算失败，输出截断到 16 KB。被拒绝的调用也在这里，原因写在结果里 |
| `thinking` 块 | 不记 |
| 带 `parent_tool_use_id` 的消息（子智能体内部） | 不记 |
| `result` | 用量按 `usage` 和 `price` 折算，全部输入是 `input_tokens` 加上 `cache_read_input_tokens` 和 `cache_creation_input_tokens`。`is_error` 为真或 subtype 以 `error` 开头就是失败：原因取 `result`，没有就取 subtype；非零退出时附上 stderr 最后一行 |
| 没有 `result`，以及超时 | 和 Codex 一样（D-24） |

不用 `total_cost_usd` 记账：续接时它是整个会话的累计，每轮都记会重复。`usage` 只算这一次进程，直接计费；引擎会话表里的累计用量（D-23）在上一轮的基础上加上它。Codex 续接时报的用量本身就是累计值，那边按上一轮存下的累计值做差（backlog E-10）。

**能做什么**

Claude Code 没有 Codex 那样的系统沙箱，能做什么全靠它的权限规则：

- 工作目录外的读写、没放行的命令都会被拒绝。核对过 `touch /tmp/…`、`python3 -c …`、读 `/etc/hostname` 三种。
- Claude Code 认为安全的简单命令（如 `echo hi > a.txt && cat a.txt`）在 `acceptEdits` 下自动放行。
- `allowed_tools` 放行的命令不受工作目录限制，还能执行工作目录里的代码。比如 `Bash(cargo test:*)` 会跑项目里的测试代码，等于放行了这个项目能做的任何事。只在可信的项目上放行；要隔离，就把 ip 跑在容器里（`docs/ip/deploy-docker.md`）。
- Claude Code 自带的沙箱（Linux 上用 bubblewrap）第一版不用：要另外装，云主机上也没验过。
- 和 Codex 一样，不带 harness 的代理变量，只能经代理出网的主机上连不上 Anthropic。
- 续接时找不到会话（比如删了 `data_dir/engines`），这个对话之后每轮都会失败，原因写在“这轮失败”里。Codex 也一样。

**怎么核对的（2026-11-21）**

用本机的 Claude Code 2.1.330，连一个假的 Anthropic 消息接口（本机的 Python 脚本，按触发词回固定内容，不调真模型）：

- 单独跑 `claude`，核对了这些：
  - 上面的参数都能用，`${IP_MCP_TOKEN}` 会被展开，`--append-system-prompt-file` 生效；
  - 工作目录里 `.claude/settings.json` 写的钩子和放行规则都不生效；
  - 越界的命令和读取被拒绝；
  - 接口报错、续接时找不到会话时的输出；
  - 续接时 `usage` 和 `total_cost_usd` 的差别。
- 再用 `ip serve` 配 `claude_code` 引擎，从 `/ws` 操作：
  - 派任务后，员工经 `/mcp` 的 `update_task` 交回了任务。这一步是真的 Claude Code 作为 MCP 客户端连 ip 的 `/mcp`。
  - 新对话里跑了简单命令，也跑了放行的 `sh test.sh`。续接同一对话时，模型收到了前几轮的消息。
  - 没放行的命令被拒绝。

没用真的 Claude 模型跑过，也没在真实项目上跑过。

**测试**

- 单元测试（`ip-engines/src/claude_code.rs`）：配置默认值和校验；首轮、续接、只读时的参数；stream-json 各类事件的解析，样例取自 2.1.330 的真实输出。
- 端到端测试（`ip-server/src/engine_tests.rs`，和 Codex 共用假引擎脚本）：
  - 首轮和续接的参数、标准输入、提示词文件、环境变量、工作目录；
  - 输出记进对话；
  - 按价格计费，不按累计的 `total_cost_usd`；
  - 接口报错、缺少 `result`、找不到会话三种失败的原因；
  - 照 `--mcp-config` 用 curl 调 `/mcp` 的 `update_task` 交回任务。

---

## 7. 待定问题

| 编号 | 问题 | 建议 | 状态 |
| --- | --- | --- | --- |
| Q-01 | Slack 私信：每条顶层私信开一段新会话，还是一个人一段长会话？ | 结论：和频道统一，按线程划分，每条顶层私信开一段新会话，回复在线程里（D-12） | 已关闭 |
| Q-02 | 系统消息要不要发到渠道？比如“这轮工作失败”。可是任务结果也是系统消息，协调员对话里会很吵 | 结论：给系统消息加类型，渠道只发 `turn_failed`，见 D-26 | 已关闭 |
| Q-03 | Slack 用户名写进 `Author::Human` 时，要不要加前缀和控制台用户区分？路由规则的 `from` 条件按名字匹配 | 结论：名字不加前缀，另记来源渠道和用户 id；`from` 对渠道用户只按 id 匹配，见 D-27 | 已关闭 |
| Q-04 | Codex 进程要不要再套一层 bubblewrap？ | 结论：第一版不套，依靠 Codex 自带的沙箱、清空的环境变量和独立的 `CODEX_HOME`（D-22）。两层沙箱嵌套能否正常工作，等用真实 Codex 实测后再看 | 已关闭 |
| Q-05 | 系统提示词用标准输入传，还是用 Codex 的指令配置项传？ | 结论：用标准输入传，放在第一轮输入的最前面，续接时不再重复（D-22） | 已关闭 |
| Q-06 | 什么时候做 HTTP Events API？超出一轮的预算要不要中途终止？ | 都等第一个真实客户的需求再定 | 待定 |
| Q-07 | Slack 逐字输出：Slack 新出了流式发消息接口（`chat.startStream` / `appendStream` / `stopStream`） | 第二版再考虑，开工前核对接口和限流 | 待定 |
| Q-08 | 两个 ip 进程连同一个 Slack 应用（高可用） | 第一版不支持，文档里写明只能单实例 | 待定 |

---

## 8. 参考

- Slack Socket Mode：<https://docs.slack.dev/apis/events-api/using-socket-mode/>
- Slack 流式发消息：<https://docs.slack.dev/reference/methods/chat.startStream/>
- Codex 非交互模式（`codex exec --json`、`resume`）：<https://developers.openai.com/codex/noninteractive>
- Codex MCP 配置（`url`、`bearer_token_env_var`）：<https://developers.openai.com/codex/mcp>
- 本仓库：`rust/README.md`（M0/M1 清单），`docs/ip/nightly-lab-reference.md`（后续功能参考），`docs/ip/m2-acceptance.md`（真实验收手册）

上面 Slack 和 Codex 的细节写于 2026-11-10，没有逐条对照官方文档原文核对（当时的环境访问不了这两个文档站）。开工时请核对一遍，有出入就改这里。

---

## 9. 控制台和 App 用的 `/ws` 协议

`/ws` 的请求和回复格式见 `rust/web/src/types/`（ts-rs 生成）。这一节只记连接层面的约定：桌面端和手机端 App（`docs/ip/apps-plan.md`）要靠它们断线重连、发现版本不一致。

### D-29 协议版本号和事件补发 · 提议（backlog F-6）

背景：手机网络常断，断开期间的事件就丢了，控制台只能把列表整个重新拉一遍。桌面 App 自带一份控制台代码，连的主机版本可能和它不同，要能发现。

- 连上后服务先发一帧 `hello`：`{"type":"hello","protocol":1,"epoch":"<本次启动的标识>","seq":<从哪个序号接着推>,"resumed":<是否补发>}`。`protocol` 是协议版本号，只有不兼容的改动才加一；客户端发现和自己的不同时提示更新。客户端收到 `hello` 才算连上。
- 事件帧带序号：`{"type":"event","seq":123,"event":{...}}`。序号从 1 起，每次启动重新编号，所以要和 `epoch` 一起用。
- 服务端留最近 4096 个事件，合计不超过 4 MiB，已经编好号、序列化好。
- 重连时客户端在地址里带上 `since=<收到的最后一个序号>&epoch=<上次的 epoch>`。`epoch` 对得上、`since` 之后的事件都还留着，就回 `resumed: true`，先补发这些事件再接着推新的；否则回 `resumed: false`，客户端重新拉列表。
- 不带 `since` 的连接（第一次打开）不补发，`hello.seq` 是当前最新的序号。
- 连接中途跟不上（广播通道溢出）时，先从留着的事件里补；补不上才发 `lagged`，客户端重新拉。`lagged.skipped` 是跳过的事件数，不知道时为 0。
- 只有一个转发任务给事件编号、序列化，各连接直接发同一份文字，不再每个连接各序列化一遍。转发任务在第一个连接进来时启动，之前的事件没有连接要补，不留。
- 地址里的 `since`、`epoch` 不是秘密；来源检查和令牌检查照旧在升级之前做。
- 测试：重连补发、`epoch` 不对、补不上、中途溢出后从留着的事件补上、序号连续。基准 `ip bench events`：编号转发的吞吐、补发 4096 个事件的耗时。

### D-30 设备配对和设备令牌 · 提议（backlog F-7）

背景：现在所有人共用一个访问令牌，丢了只能换掉整个令牌，消息也分不清是谁发的。手机和电脑 App 要每台设备一个令牌（`docs/ip/apps-plan.md` 4.2、4.4）。

- 配对码：6 位数字，2 分钟内有效，只能用一次。`ip device pair --name <设备名> [--url <控制台地址>]`（直接写库，服务在运行时也能用）或控制台“设备”页生成；给了控制台地址就附上二维码，内容是 `<控制台地址>/#/pair/<配对码>`。库里只存 SHA-256(配对码 id + 配对码)。
- 换令牌：`POST /pair`，正文 `{"code":"123456"}`，来源检查和 `/ws` 一样。成功回 `{"token":"ipd_…","device":{...}}`，令牌只出现这一次；配对码不对、过期或用过回 400；一个有效期内猜错 10 次，作废所有待用的配对码，回 429。
- 设备令牌以 `ipd_` 开头，约 244 位随机数，库里只存 SHA-256。设备名由生成配对码的人定，设备自己改不了。
- `/ws` 认两种令牌：共用令牌，或某台设备的令牌。用设备令牌连上的连接，请求里的 `author` 一律换成设备名。
- 新请求：`who_am_i`（这个连接是哪台设备、什么权限）；`list_devices`、`create_pairing`、`rename_device`、`revoke_device` 要“管理”权限（D-31）。吊销后这台设备的连接立即断开，令牌作废。
- 存储：设备和待用的配对码放在设置表的 `devices`、`device_pairings` 两个键里，整份读写。设备列表只有服务进程写；配对码 `ip device pair` 也会写，所以“只能用一次”靠设备记录里的配对码 id 保证，不靠删掉配对码。
- 最后在线时间在连上时更新，至少隔 1 分钟写一次库。
- 测试：配对、只能用一次、过期、猜错太多次、旧的配对码列表被写回也换不了第二次、改名、吊销后断开、设备署名、设备不能管理设备、跨来源换令牌被挡。

### D-31 设备权限 · 提议（backlog F-8）

背景：配对过的设备不该都能改路由规则、清空记忆、管理别的设备（`docs/ip/apps-plan.md` 4.3）。

- 每台设备一档权限：`daily`（日常）或 `admin`（管理）。新配对的设备是 `daily`；D-30 之前存下、没有这一项的设备也按 `daily` 算。共用令牌（以及本机不设令牌时）等同 `admin`。
- `/ws` 每个请求先看权限：`admin` 什么都能做；`daily` 只能用 `ws.rs` 里 `daily_allows` 列出的请求类型，其余回错误“这台设备是‘日常’权限…”。没列出的都算 `admin`，以后新加的请求类型也一样，要给日常设备用得明确加进去（默认拒绝）。
- `daily` 能用的：`list_roles`、`list_agents`、`spend_today`、`list_threads`、`history`、`search`、`route`、`send`、`list_tasks`、`create_task`、`update_task`、`list_approvals`、`resolve_approval`、`list_reports`、`read_report`、`who_am_i`，外加两个只读的：`list_rules`（新对话默认要不要自动分派，看有没有路由规则）、`list_memory_off_threads`（对话页顶上显示这个对话进不进岗位记忆），以及星图上拖员工换轨道的 `list_orbit_pins`、`set_orbit_pins`（D-40）。改路由规则、规则草案、定时任务（看和立即运行）、记忆（看、改、抹掉、清空、开关对话记忆）、设备管理都只给 `admin`。
- 新请求 `set_device_scope {device_id, scope}`，要 `admin`，回改完后的 `DeviceInfo[]`。`DeviceInfo` 和 `who_am_i` 的回复多了 `scope`；`who_am_i` 原来的 `can_manage_devices` 去掉。
- 每个请求按设备现在的名字和权限算：改名、改权限后，连着的连接下一个请求就生效，不用重连；吊销到断开之间的请求一律拒绝。
- 管理设备可以改所有设备的权限，包括自己；共用令牌一直是 `admin`，不会把自己锁在外面。
- 控制台：`daily` 设备不显示“路由规则”“记忆”两页，任务页不显示定时任务，对话页的记忆开关不显示，不发只给 `admin` 的请求；“设备”页只显示这台设备自己。`admin` 的“设备”页每台设备一个权限下拉框，给“管理”前要确认。
- 测试：新设备是 `daily`、旧记录没有这一项也是 `daily`、日常能用和不能用的请求各列一遍、给了 `admin` 不重连就能用、收回后又不能用。

### D-32 通知规则 · 提议（backlog F-9）

背景：手机和电脑上要知道“有事等我”，但不能什么都推（`docs/ip/apps-plan.md` 第 5 节）。什么时候通知由服务端统一决定，网页、PWA、以后的桌面 App 和 Web Push（F-10）都照这一份。

- 规则（`notify.rs` 的 `Rules`）。一个任务从运行时收事件，挑出这几种：

  | 事件 | 级别 | 标题 | 标签 |
  | --- | --- | --- | --- |
  | 新审批卡（路由拿不准、一般问题） | `needs_me` | “这条消息该交给谁？”“等你决定” | `approval:<id>` |
  | 员工提问 | `needs_me` | “<员工> 在问你” | `approval:<id>` |
  | 一轮失败 | `needs_me` | “<员工> 这一轮失败了”，正文是错误的第一行 | `thread:<id>` |
  | 人派的任务受阻 | `needs_me` | “任务受阻：<标题>” | `task:<id>` |
  | 一轮结束且员工说了话 | `all` | “<员工> 回复了”，正文是这一轮最后一条回复 | `thread:<id>` |
  | 人派的任务完成 | `all` | “任务完成：<标题>”，正文是交回的结果 | `task:<id>` |

  定时任务触发的一轮失败也走“一轮失败”。员工派给员工的任务不通知人（结果交回派活的员工）。任务状态没变不重复通知。审批卡被选定、受阻的任务又动起来时，发撤销（`dismiss`），各端按标签撤掉那条通知。
- 预览：只看去掉代码块和表格后的开头 2 KB，先脱敏（和记忆用同一套 `redact`），再去掉 Markdown 记号，取头两句、最多 120 个字。开头一段里还有没配对上的私钥开头时，从那里起都不要。
- 每台设备一份设置 `NotifySettings`：`level`（`off`、`needs_me` 默认、`all`）、`muted_roles`（这些岗位的事不通知）、`mute_until_ms`（静音到）、`preview`（带不带正文）。共用令牌连上的控制台共用一份（存在 `shared` 名下）。存在设置表的 `notify_settings` 键里；设备吊销时删掉它那份。
- 新请求 `get_notify_settings`、`set_notify_settings {settings}`，日常权限就能用（改的是自己的）。
- 新的服务端帧：`{"type":"alert","alert":{kind, level, title, body, tag, thread_id?, role_id?, at_ms}}`，已按这个连接的设置筛过、关了预览时 `body` 为空；`{"type":"dismiss","tag":"..."}` 一律发。这两种帧不编号，断开期间的不补发。
- 控制台：页面开着但不在前台（隐藏或没有焦点）时弹系统通知，有 Service Worker 时用它弹（手机浏览器只认这种），点开去那个对话或“待办”；在前台时不弹。地址 `#/chat/<对话 id>` 打开那个对话。“设备与通知”页改设置。
- 页面关掉后的推送见 D-33（F-10）。
- 基准 `ip bench notify`：每个事件过一遍规则的吞吐（一轮 50 段文字加一条约 4 KB 的回复），一次预览的耗时。
- 测试：规则逐条、设置筛选（级别、岗位、静音、预览）、预览去掉代码块表格和密钥、按设备存取、两个连接按各自设置收到通知和撤销。

### D-33 Web Push · 提议（backlog F-10）

背景：D-32 的通知只在控制台开着时能弹，关掉页面、手机锁屏就收不到。第一版用 Web Push（`apps-plan.md` 5.3）：主机直接发给浏览器厂商的推送服务，内容端到端加密，项目方不运营任何服务。

- 密钥：主机第一次用到时生成一把 P-256 的 VAPID 密钥，存在设置表的 `webpush_vapid` 键里（PKCS#8，base64url）。换掉它，所有订阅都要重新订；控制台打开时发现公钥变了会自己重订。
- 加密和签名（`webpush.rs`）：负载按 RFC 8291 加密（`aes128gcm`，每条推送一把临时密钥和随机盐），请求头按 RFC 8292 带 VAPID（ES256 的 JWT：`aud` 是推送服务的源，`exp` 12 小时，配了 `push.contact` 时带 `sub`）。用的是 rustls 已经编进来的 `aws-lc-rs`，没有新的密码学依赖。加密逐字节对过 RFC 8291 附录 A 的例子。
- 负载：`{title, body, tag, thread_id?, silent?}` 的 JSON，最多 3000 字节；标题和正文和 `/ws` 上的 `alert` 一样，已按这台设备的设置筛过、关了预览时正文为空。请求头 `TTL` 一天；`Urgency` 要我处理的是 `high`，其余 `normal`；`Topic` 是标签的 SHA-256 取 base64url 的前 32 个字符，同一件事还没送达的旧推送被新的替换。
- 订阅：新请求 `push_key`（回 `{public_key}`）、`subscribe_push {endpoint, p256dh, auth}`、`unsubscribe_push {endpoint}`，日常权限就能用，订阅记在这台设备名下（共用令牌记在 `shared` 名下）。存在设置表的 `push_subscriptions` 键里；同一个地址再订一次替换旧的；一台设备最多 5 个、全部最多 500 个。设备吊销时删掉它的订阅。
- 只发给已知的推送服务，防止把主机当跳板去连内网：`https://`、没有端口和用户名，主机是 `fcm.googleapis.com`、`web.push.apple.com`，或以 `.push.services.mozilla.com`、`.notify.windows.com` 结尾。`p256dh` 要是 65 字节、能解析的 P-256 公钥，`auth` 要 16 字节。不跟随跳转。推送服务回 404 或 410 时删掉那个订阅。同时在发的最多 16 个，单个请求 10 秒超时；日志里不写订阅地址。
- 不重复打扰：新请求 `presence {active}`，页面在前台时每 20 秒报一次 `true`，切走、失焦时马上报 `false`；30 秒内报过 `true` 的设备不推（页面自己更新）。开了推送的页面不再自己弹系统通知，免得一件事弹两次。
- 撤销：`dismiss` 只发给之前推过这个标签的订阅（每个订阅记最近 64 个），发一条同标签、静音的“已在别处处理”，正文是原来的标题，把旧通知盖掉。浏览器要求每次推送都显示通知，所以做不到直接撤掉。
- 配置：`ip.yaml` 新增 `push: { contact: mailto:… }`（可选，`mailto:` 或 `https://` 开头）。按苹果的文档，发给 Safari 的推送要带联系方式。
- 控制台：Service Worker 收到推送就弹通知（同标签替换），点开和 D-32 一样去那个对话。“设备与通知”页多一行“推送”：浏览器不支持时说明原因（要 HTTPS；iPhone 要 iOS 16.4 以上并添加到主屏幕），支持时一个“开启推送”按钮，开了显示“关闭推送”。
- 和 `apps-plan.md` 5.2 不同：“同一个人在别的设备上 30 秒内有操作时不推”没做。现在没有“人”这一层，只有设备，一个人的手机和电脑是两台设备，没法知道是同一个人。做了账号以后再加。
- 基准：`ip bench notify` 加了一项，加密并签名一条推送的耗时。
- 测试：RFC 8291 例子逐字节、VAPID 签名能用公钥验过、只认已知推送服务、在前台的设备过期、`Topic` 长度；`/ws` 集成测试用假推送服务走一遍订阅、审批推送（解密后核对内容和请求头）、410 删订阅、在前台不推、处理后推“已在别处处理”。没有在真的 FCM、苹果、Mozilla 推送服务上跑过。

### D-34 桌面 App 连主机 · 提议（backlog F-11）

背景：桌面 App（`apps/desktop`，Tauri 2）自带一份控制台，不加载主机上的网页（`apps-plan.md` Q-4）。它的网页来源是 App 自己的，和主机地址不同源，原来的来源检查会把它挡在 `/ws` 外面。

- 来源检查（`origin.rs`）：主机配了访问令牌时，额外接受桌面 App 的三个来源 `tauri://localhost`（macOS、Linux）、`http://tauri.localhost`、`https://tauri.localhost`（Windows），连接照样要令牌。没配令牌时不接受：`tauri.localhost` 这种名字万一被解析到别处，别的网页也能带上这个来源，不能拿它当免令牌的凭据。`allowed_origins` 的写法不变，也不用写这几个。
- 配对：App 在壳里（Rust）发 `POST /pair`，不带 `Origin`，按非浏览器客户端处理，主机不用开跨域。配对码只能用一次，App 先确认系统钥匙串存得进去再发。
- 令牌：存在系统钥匙串，服务名 `intelligent-position`、账户名是主机地址；主机地址记在 App 配置目录的 `host.json`。换主机时删掉旧主机的令牌。网页每次打开时向壳要一次，用法和浏览器里一样（`/ws?token=`）。
- 壳只给网页 7 条命令（`get_host`、`pair`、`save_token`、`forget_host`、`notify`、`open_url`、`set_pending`），Tauri 的权限声明只有 `core:default`。`open_url` 只认 http、https、mailto。
- 通知：桌面 App 不订 Web Push（D-33），关窗口后常驻托盘、连接不断，照 D-32 的 `alert` 弹系统通知，窗口在前台时不弹。点通知现在只能把窗口叫出来。
- 测试：来源检查加了一条（配了令牌时接受这三个来源，没配时不接受，相近的写法不接受）；壳的主机地址规整有单元测试。真的 App 在 Linux（Xvfb、GNOME 钥匙串）上跑过配对、存令牌、连上、重开读回令牌；其余用 Playwright 加假的壳核对。macOS、Windows 没跑过。

### D-35 手机 App 的锁屏推送 · 提议（backlog F-21）

背景：Flutter 手机 App（`apps-plan.md` 第 9 节）开着、在后台还没被系统杀掉时，照 D-32 的 `alert` 弹本地通知（F-16）；锁屏久了、App 被杀以后，连接断了就收不到。原生 App 订不了浏览器的 Web Push（D-33），要走苹果的 APNs 和谷歌的 FCM。维护者 2026-11-21 定：项目运营一个推送中转，官方 App 装上就能收（Q-11）。

思路：**主机照旧发 Web Push**（RFC 8291 加密、RFC 8292 的 VAPID，D-33），中转在主机眼里就是又一个推送服务。手机 App 自己生成 Web Push 的密钥对，向中转登记拿到订阅地址，再用 D-33 已有的 `subscribe_push` 告诉主机。中转只见密文，解密在手机上做。协议不变：不加请求、不改帧。

- **中转（`ip-relay`，新的独立程序，代码在本仓库）**：
  - `POST /register {platform: "apns" | "fcm", token}`：`token` 是手机从 APNs 或 FCM 拿到的设备推送令牌。回 `{endpoint}`，地址是 `https://<中转>/w/<封条>`，封条是用中转自己的密钥（AES-256-GCM，从环境变量读）加密的 `{platform, token}`。中转不需要数据库，不存令牌。
  - `POST /w/<封条>`：照 Web Push 推送服务的样子收。请求体是 `aes128gcm` 的密文，原样转出去：APNs 用 `mutable-content` 的通知，由 iOS 的通知扩展（Notification Service Extension）解密后显示；FCM 用数据消息，由 Android App 的后台处理解密后弹通知。`Urgency: high` 的用高优先级，`Topic` 转成 APNs 的 `apns-collapse-id` 和 FCM 的 `collapse_key`，`TTL` 照转。成功回 201；APNs、FCM 说令牌失效时回 410，主机照 D-33 删掉这个订阅。
  - 只做转发：不解密、不记推送内容和令牌，日志只记次数和错误码。按来源地址和封条限流，单个请求体最多 3000 字节。
  - 凭据：APNs 用 `.p8` 签名密钥（Team ID、Key ID、App 的 Bundle ID），FCM 用 HTTP v1 接口和服务账号。全部经环境变量给中转（`IP_RELAY_*`，清单在 `crates/ip-relay/src/main.rs` 开头），不进仓库。封条密钥用 `ip-relay keygen` 生成；换了它，旧地址全部失效，App 下次打开时重新登记。
  - 基准 `ip bench relay`：打开封条、读请求头、拼好转给 APNs 的请求体，一条的中位数不超过 0.2 毫秒（不含网络往返）。
- **主机**：
  - `webpush.rs` 的白名单多认中转的主机名。官方中转的域名定下来后写进默认名单；自己搭中转的人在 `ip.yaml` 里加 `push: { hosts: [push.example.com] }`。只写主机名，照样只认 `https://`、不带端口和用户名、不跟随跳转；不让 App 登记任意地址，免得把主机当跳板连内网（和 D-33 同一个理由）。
  - 负载上限从 3000 字节降到 2800：APNs 和 FCM 的消息上限都是 4 KB，密文转成 base64 再加上外层字段要放得下。
- **App**：
  - 连上主机、要到系统通知权限后，拿设备推送令牌（iOS 向 APNs 要，Android 向 FCM 要），生成 P-256 密钥对和 16 字节的 `auth`，向中转登记，再 `subscribe_push`。私钥放系统安全存储；iOS 的通知扩展要读到它，放在 App 和扩展共用的钥匙串分组里。不上传。
  - 收到后按 F-16 的规则弹通知：同标签替换；`silent` 的“已在别处处理”直接按标签撤掉原来那条（原生 App 能做到，浏览器做不到）。点通知去那个对话，没有对话的去“待办”。
  - 推送令牌、中转地址变了就重新登记、重新 `subscribe_push`（同一台设备同一个地址替换）。在“我”页关掉锁屏推送、断开主机时 `unsubscribe_push`；设备被吊销时主机已经会删它的订阅（D-33）。
- **不重复打扰**：App 已经按 D-33 报 `presence`（F-16），在前台时主机不推。App 在后台但连接还在时，`alert` 和推送都会到：两边用同一个通知 id（标签的 FNV-1a，和 F-16 一样），后到的只替换、不再响铃（Android 的 `setOnlyAlertOnce`）。
- **Android 的实现**（F-21 之四）：收 FCM、解密、弹通知都在原生代码里（`IpPush.kt`、`WebPushCrypto.kt`），App 被杀掉后 FCM 叫起服务就能弹，不用启动 Flutter。中转在 FCM 的数据消息里另带 `urgency`（`high` 或 `normal`），App 按它选“要我处理的”或“员工回复和任务完成”通知渠道。私钥用 Android Keystore 里的一把 AES 密钥包起来，存在应用私有的 SharedPreferences。Firebase 的配置不进仓库（没有 `google-services.json`），构建时用 `-PIP_FCM_*` 给；中转地址用 `--dart-define=IP_PUSH_RELAY` 给。
- **iPhone 的实现**（F-21 之五）：中转发 `mutable-content` 的通知，标题“intelligent-position”、正文“有一条新通知”是解不开时的兜底，密文放在 `userInfo["ip"]`。通知扩展（`apps/flutter/ios/NotificationService/`）用钥匙串里 App 和扩展共用分组中的私钥解开，改写标题、正文，`threadIdentifier` 用 `thread_id`；解不开时照兜底显示。iOS 不让通知扩展什么都不显示，所以 `silent` 的“已在别处处理”是撤掉同一件事的旧通知（App 自己弹的按 FNV-1a 的 id，之前的推送按记下的标签），再显示一条不响铃、不亮屏（`passive`）的；同一件事已经显示过的（App 在后台还连着时按 F-16 弹过），新到的推送撤掉那条、自己只替换不响铃。私钥和 `auth` 的钥匙串属性是“首次解锁后、只在这台设备”：手机重启后还没解锁过时收到的解不开，显示兜底；不进备份。App 里（`ios/Runner/IpPush.swift`）向 APNs 要设备令牌（小写十六进制交给中转），每次启动重新要一次，变了告诉 Flutter 那边重新登记；点通知按 `userInfo` 里记的对话去那个对话或“待办”，不是锁屏推送的通知交给 flutter_local_notifications。方法通道 `ip/push` 和 Android 是同一套。APNs 的通知里没有 `urgency`，iPhone 不分两种通知。
- **要维护者准备的**：苹果开发者账号（APNs 密钥，并用它给 iOS App 签名，Q-12）；一个 Firebase 项目（FCM 服务账号，Android App 的 `google-services.json`）；跑中转的主机和域名。没有这些之前，中转和 App 只用假的 APNs、FCM 测。
- **Android 的依赖**：收 FCM 要谷歌 Play 服务（闭源）。官方 App 接受这一点；以后可以加 UnifiedPush 给没有 Play 服务的手机，主机这边不用再改。
- 测试：主机加白名单的单元测试（`push.hosts` 里的认、相近的写法不认）和配置检查；`/ws` 集成测试用假中转走一遍 App 式的订阅。中转用假的 APNs、FCM 测登记、封条解不开、转发的字段、令牌失效回 410、限流。App 用假的推送通道测登记、换令牌、关掉、收到后解密弹通知和撤销。没有在真的 APNs、FCM 和真手机上跑过之前，不写“可用”。

### D-40 App 桌面外壳的协议补充 · 提议（backlog F-23）

背景：Flutter App 的宽窗口外壳（2026-11-27 合并）列了一份后端适配清单（`docs/ip/flutter-desktop-backend-adaptation.md`）。这一条是其中第一批：协议小改动、能立即用上的三项（清单第 5、9、1 项）。

- **对话的最后回复员工**（清单第 5 项）：`ThreadSummary` 加 `last_agent_id`（`AgentId | null`）。员工发的消息写入时更新，人和系统的消息不动它；员工还没说过话时为空。`threads` 表加 `last_agent_id` 列（迁移 0002），已有的对话按 `messages` 的写入顺序补一次，从 SQLite 旧库导入后也补。App 用它把对话归到员工名下，重启后员工左栏也能看到没挂在任务、审批上的历史对话；App 本地知道的（刚发给另一位员工、他还没回）不被覆盖。
- **协调岗位标记**（清单第 9 项）：`Role` 加 `coordinator`，`ip.yaml` 里写 `coordinator: true`，最多一个岗位标，标了两个启动时报错。只是标记，不改这个岗位的工具、提示词和路由。是 false 时协议里不带这一项（TS 里是 `coordinator?: boolean`），旧主机也一样没有，客户端按 false 读。App 的星图：有岗位标了就让它的员工坐中心的太阳，标了的岗位没有员工时太阳空着；一个都没标（旧主机、ip.yaml 没写）才退回按 id、名字猜。示例配置给协调员标上。
- **星图轨道表**（清单第 1 项）：App 宽窗口的天穹有三条固定轨道（0 内环、1 中环、2 外环），人能把员工拖到某条轨道上固定住，满环时 App 按确定的规则把别人挤到外侧（规则在 `apps/flutter/lib/src/ui/shell/orbit.dart`）。主机只存结果、校验、广播，不重算挤人。
  - 新类型 `OrbitPin { agent_id, orbit_track: 0..2, orbit_pinned_at: Unix 毫秒 }`（`ip-core`）；没固定的员工不在表里，按紧急度自动排轨。
  - `list_orbit_pins`：回复 `OrbitPin[]`，按员工 id 排；已经不在组织里的员工不列（记录留在库里，同一个 id 加回来时还在原来的轨道上）。
  - `set_orbit_pins { changes: [{ agent_id, orbit_track?, orbit_pinned_at? }] }`：一次拖放改动的几位员工（拖进去的和被挤出去的）一起提交。`orbit_track` 为空是取消固定；`orbit_pinned_at` 省略时用主机收到的时间。同一位员工出现几次以最后一次为准。员工不存在、轨道不是 0 到 2、时间是负数时整批不改，回错误；一次最多 256 项。回复改完后的整张 `OrbitPin[]`。
  - 表真的变了才推事件 `orbit_pins_changed { pins: OrbitPin[] }`（整张表），各端直接换上；它照常编号、断线补发（D-29）。不发通知。
  - 存在设置表的 `star_map.orbit_pins` 键里，整份读写，读改写由一把锁串行化。
  - 权限：两个请求都给 `daily`（日常使用，各设备共用一张表）。
  - App：连上时拉一次表，主机上的员工以主机为准，本地新建的 Bot（主机上还没有）和演示星球照旧只留在本机；拖放落位后把主机上员工的改动交上去，主机拒了就重新拉表盖回去；旧主机没有这个请求时只留在本机。审批通过后解除固定（清单第 2 项）还在本机做，等第 2 项落到主机。
- 不加 `ip bench`：三项都不在热路径上（拖放由人触发，对话列表多读一列）。
- 测试：存储（写入更新、人的消息不改归属、迁移补旧对话、旧库导入后补）；组织（标两个报错、false 不出现在 JSON、旧 JSON 按 false 读、示例配置只标了协调员）；`/ws`（日常设备能改、整批校验、同一员工以最后一次为准、别的连接收到事件、重启后还在、删掉的员工不列、取消固定）；App（对话归属并进左栏、按标记找太阳并退回猜名字、轨道表随主机和事件换、只交主机上的员工、被拒后重拉、旧主机不发请求），都在假模型、假主机上。

---

## 10. 浏览器

### D-39 浏览器走 Playwright MCP，`mcp_servers` 加 `tools` 白名单 · 提议（backlog E-5）

- 不自研浏览器自动化，删掉一直是空壳的 `ip-browser` crate。需要浏览器的岗位在 `mcp_servers` 里配一项 Playwright MCP（stdio，`npx -y @playwright/mcp@<写死的版本>`），推荐参数和配法见 [browser.md](browser.md)：`--headless --isolated --no-webmcp --image-responses omit`，`--blocked-origins` 写上云主机元数据地址，`cwd` 指向单独建好的目录，一个岗位一项。
- `mcp_servers` 的每一项加可选字段 `tools`：服务器上工具的原名列表。写了的，只把单子里的工具交给员工（`specs_for` 里只有它们，`call` 调别的回“没有工具”）；省略表示服务器列出的全部，和原来一样。单子里写了、服务器上没有的名字，连上时记一条警告，不报错。名字为空或重复是配置错误。harness 自己按服务器调的 `call_json`（团队上下文取包）不受白名单影响。
- 原因：Playwright MCP 没有关掉某个工具的开关，它的 `browser_run_code_unsafe` 在它自己的 Node 进程里执行任意代码，而 stdio 子进程不在沙箱里，只能在 I.P. 这一侧挡住。白名单而不是黑名单，是为了服务器以后新加的工具不会自动交给员工。
- 推荐白名单 14 个工具（`browser_navigate`、`browser_navigate_back`、`browser_snapshot`、`browser_find`、`browser_click`、`browser_type`、`browser_fill_form`、`browser_select_option`、`browser_press_key`、`browser_hover`、`browser_wait_for`、`browser_tabs`、`browser_handle_dialog`、`browser_close`），不给执行代码、传本机文件、截图和看网络请求的工具，理由见 browser.md 第 3 节。
- 不在这一步做：Docker 部署里的浏览器（单独容器、网络和数据库、TCMS 隔开，backlog E-16）；登录状态；同一岗位几个员工各用各的浏览器上下文。
- 测试：`ip-mcp` 单元测试用假服务器测白名单（只交出单子里的、调别的被拒、写错的名字只警告、`call_json` 不受影响）和配置检查；`crates/ip-mcp/tests/playwright.rs` 用真的 Playwright MCP 跑推荐配置，设了 `IP_TEST_PLAYWRIGHT_MCP` 才跑，CI 不跑。2026-11-24 在 macOS 上用 Google Chrome 和 0.0.86 跑通过；没让模型用过，没在 Linux 和 Docker 里跑过。

---

## 修订记录

| 日期 | 版本 | 改动 |
| --- | --- | --- |
| 2026-11-10 | v0.1 | 初稿。范围：Slack（Socket Mode）、假渠道先行、MCP 服务端、Codex 引擎只给运维测试岗位 |
| 2026-11-10 | v0.2 | Q-02、Q-03 选定方案 C，关闭；新增 D-26（系统消息类型）、D-27（渠道用户来源），已定；新增 M2-0、测试 T11；D-03、D-04、D-07 随之更新 |
| 2026-11-10 | v0.3 | D-26、D-27 已实现（#20），M2-0 完成 |
| 2026-11-10 | v0.4 | D-17 到 D-19 已实现（#21），M2-3 完成。D-18 补充：签发令牌时必须给最长期限 `ttl` |
| 2026-11-10 | v0.5 | D-20 到 D-25 已实现（#22），M2-4 完成（用假 codex 验证）。Q-04、Q-05 关闭。和提议的不同：引擎配置放在顶层 `engines` 下（D-21）；`CODEX_HOME` 移到 `data_dir/engines/<员工 id>`，不在工作目录里（D-22）；内置循环不包装成 `BuiltinEngine`（D-20） |
| 2026-11-10 | v0.6 | D-02 接口类型、D-09 假渠道已实现（#23）；D-10 的 13 个测试已写好，标了 `#[ignore]`，接入点是 `channel_tests.rs` 的 `attach`。D-07 补充：不符合 `approvers` 的点击用 `HubError::Rejected` 返回，由渠道只给点击的人看 |
| 2026-11-10 | v0.7 | M2-1 改由 Claude 完成：D-01、D-03 到 D-08 已实现（#24），D-10 的 13 个测试全部通过。补充：接入用 `channels::start`；同一会话的并发首条消息用 `claim_binding` 处理；第一次接入的渠道不补发历史；D-05 做在 `resolve_approval` 里，控制台选定也会补上员工；D-06 最多找 8 层；假渠道的去重 key 改为进程内全局递增 |
| 2026-11-10 | v0.8 | M2-2 改由 Claude 完成：D-11 到 D-16 已实现（#25），用假 Slack 服务器验证。Q-01 按建议关闭。D-14 的 :eyes: 表情作废，D-15 不再需要 `reactions:write`。补充：含 `<@机器人>` 的 `message` 事件也能开启会话；线程里 @机器人 也能开启会话；`markdown` 块被拒时退回纯文字；`ip.yaml` 增加 `api_url` |
| 2026-11-10 | v0.9 | 分工改为：Claude 写代码、改规格、推 PR，维护者审查、合并并拍板待定问题。所有负责人都改为 Claude；M2-5、M2-6 的依赖写明需要维护者提供的账号和 key |
| 2026-11-10 | v0.10 | 为真实验收做准备（#26）：新增 `ip check` 自检命令和验收手册 `docs/ip/m2-acceptance.md`，M2-5、M2-6 按手册做 |
| 2026-11-15 | v0.11 | 新增 D-28（提议，#69）：确认前先存下信封，暂时失败重试，重启接着处理；去重改到处理成功之后，同一渠道串行处理。D-02 的 `Hub` 加 `stash`、`stashed`、`settle`；D-03 原来“去重在处理之前写入”的说明作废；D-08 加 `channel_envelopes` 表；D-10 加 T12–T14 |
| 2026-11-16 | v0.12 | 只改状态，接口没变：M2-6 完成（真实 Codex 2026-11-14 跑通 `examples/ops-demo`），M2-4 和 D-22 里“没有在真实的 Codex 上跑过”的说法按验收记录改了。PR 编号指私有开发仓库里的 PR |
| 2026-11-16 | v0.13 | 新增第 9 节和 D-29（提议，backlog F-6）：`/ws` 的协议版本号、事件序号和重连补发 |
| 2026-11-16 | v0.14 | 新增 D-30（提议，backlog F-7）：设备配对、设备令牌、设备署名和吊销 |
| 2026-11-16 | v0.15 | 新增 D-31（提议，backlog F-8）：设备权限“日常”“管理”，按请求类型放行、默认拒绝；`who_am_i` 的 `can_manage_devices` 换成 `scope` |
| 2026-11-16 | v0.16 | 新增 D-32（提议，backlog F-9）：通知规则、每台设备的通知设置、`alert` 和 `dismiss` 两种服务端帧 |
| 2026-11-16 | v0.17 | 新增 D-33（提议，backlog F-10）：Web Push、`push_key`、`subscribe_push`、`unsubscribe_push`、`presence` 四个请求，`ip.yaml` 的 `push.contact` |
| 2026-11-16 | v0.18 | 新增 D-34（提议，backlog F-11）：配了令牌时 `/ws` 和 `/pair` 接受桌面 App 的来源 |
| 2026-11-21 | v0.19 | 新增 D-35（提议，backlog F-21）：手机 App 的锁屏推送照 Web Push 发给项目运营的中转 `ip-relay`，中转转给 APNs、FCM；`ip.yaml` 加 `push.hosts`，推送负载上限降到 2800 字节。`/ws` 协议不变 |
| 2026-11-21 | v0.20 | 修 Codex 续接会话时用量重复记账（backlog E-10，提议，等工程师审查）：D-23 的引擎会话表加四列累计用量，D-20 的 `TurnContext` 加 `session_usage`、`TurnReport` 加 `session_usage`；每轮只记和上一轮累计值的差。只用假 codex 测过，累计值的数字取自 2026-11-14 真实 Codex 的核对 |
| 2026-11-21 | v0.21 | 新增 D-36（已实现，backlog E-6）：Claude Code 引擎（`kind: claude_code`），和 Codex 共用进程管理；第 6 节改名为“外部引擎” |
| 2026-11-21 | v0.22 | 新增 D-37（提议，backlog E-12）：Socket Mode 的 WebSocket 按 `HTTPS_PROXY`/`NO_PROXY` 等环境变量经 HTTP 代理的 CONNECT 隧道连接，规则和 Web API 一致；支持代理 Basic 认证；D-11 加一句指向 D-37。只用本地假代理和假 Slack 测过 |
| 2026-11-22 | v0.23 | D-35 加 iPhone 的实现（F-21 之五，提议，等工程师审查）：通知扩展解密、改写 APNs 送来的通知，“已在别处处理”撤掉旧的、换成不响铃不亮屏的；私钥放 App 和扩展共用的钥匙串分组。`/ws` 协议和中转都不变。没在真 iPhone、真 APNs 上收过 |
| 2026-11-24 | v0.24 | 新增 D-38（提议，backlog E-13）：员工回复里提到的 `reports/<文件名>.md` 由投递器读出、作为 `Outbound::Report` 发进同一会话，Slack 用三步上传发成文件，没有 `files:write` 时把正文贴成文字；链接改写成“附件 x.md”；同一会话同一版本 7 天内只发一次；D-15 加 `files:write`，`ip check` 提醒缺这项权限。现有清理仅涉及入站记录，此次补清理过期报告条目，保留其他投递记录 |
| 2026-11-24 | v0.25 | 新增第 10 节和 D-39（提议，backlog E-5）：浏览器走 Playwright MCP，删掉空壳 `ip-browser`；`mcp_servers` 每项加可选的 `tools` 白名单，只把单子里的工具交给员工。第 1 节“本阶段不做”里那一条指向 D-39。只用真的 Playwright MCP 在 macOS 上跑过集成测试，没让模型用过 |
| 2026-11-27 | v0.26 | 新增 D-40（提议，backlog F-23）：App 桌面外壳的第一批协议补充：`ThreadSummary.last_agent_id`（`threads` 表加列，迁移 0002）、`Role.coordinator`（最多一个）、星图轨道表 `list_orbit_pins`、`set_orbit_pins` 和事件 `orbit_pins_changed`；D-31 的日常权限加这两个请求。只在假模型、假主机上测过 |
