# M2 规格：Slack 渠道、MCP 服务端、Codex 引擎

> **这是活文档。** 接口、表结构、行为有变化时，先改这里，再改代码，两者放在同一个 PR 里。
> 当前版本：v0.12（2026-11-16）。第 3 到 6 节（D-01 到 D-25）和 D-26、D-27 都已实现；D-28（确认前持久化、处理成功后才记去重，#69）是提议，代码和这次修改在同一个 PR 里，等工程师审查。M2-6（真实 Codex 验收）2026-11-14 通过。剩下的是 Slack 真实验收（M2-5，按 `docs/ip/m2-acceptance.md` 第 4 节做），以及第 7 节的待定问题。

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
| M2-5 | 真实验收：在 Slack 线程里提需求，经协调员派给宏观研究员，交回报告，结论回到 Slack 线程（同时完成 M0、M1 的真实验收） | Claude | M2-2、维护者提供 Slack 工作区（按 `examples/slack/manifest.yaml` 建好应用）、两个 Slack 令牌和模型 API key | 验收手册和 `ip check` 已就绪（#26），等维护者提供账号和 key 后按 `docs/ip/m2-acceptance.md` 第 4 节做 |
| M2-6 | 真实验收：用真实 Codex 跑 `examples/ops-demo`，经历跑测试失败 → 修复 → 复跑通过 → 写报告 → 交回 | Claude | M2-4、维护者提供 Codex API key | 完成（2026-11-14，`codex-cli 0.169.0`，见 `docs/ip/acceptance-2026-11-14-codex.md`）。消息从控制台发，没经 Slack；还没在真实项目上跑过；续接会话时用量重复记账没修（backlog E-10） |

M2-1 和 M2-3 改的文件基本不重叠，可以同时做。

**本阶段不做：**

- 飞书、邮件：通用接口留出位置，等有真实用户再接。
- `ip-browser` 自研版：需要浏览器的岗位先在 `mcp_servers` 里配 Playwright MCP。注意 MCP 子进程不在沙箱里。
- Claude Code 引擎：以后套用第 6 节的同一个 `Engine` 接口。
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
  | `message.appended` | 作者是员工、内容是文字、对话有绑定 | `Outbound::Text` |
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

- **确认之前先存**：渠道收到信封后先调 `Hub::stash(id, 原始内容)`，返回时已经提交到 SQLite，再回确认。存不下就不确认，让外部系统重推。存一次是一次本地写，不影响 Slack 3 秒内确认的要求。
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
    item         TEXT NOT NULL,   -- message:<id> | card:<approval id> | resolved:<approval id>
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

- 机器人权限：`app_mentions:read`、`chat:write`、`channels:history`、`groups:history`、`im:history`、`users:read`（不加表情，所以不需要 `reactions:write`，见 D-14）
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
- 真实验收（M2-5）：在真实工作区里人工走一遍 T1–T5，再完成真实模型的端到端流程。Slack 的接口和字段名都是按资料写的，没有对照真实工作区核对过；验收时要过一遍，有出入就改 `slack.rs` 和这一节。

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

## 6. Codex 引擎（`ip-engines`）

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
    pub system_prompt: &'a str,       // 组织规则、岗位说明、同事名单
    pub input: &'a str,               // 上一轮之后的新消息（不含员工自己写的），已整理成文字
    pub mcp: Option<McpGrant>,        // 本轮的 /mcp 地址和令牌（D-18）
    pub sink: &'a dyn TurnSink,       // 以员工的名义写消息
}

pub struct TurnReport { pub steps: u32, pub usage: Usage, pub cost_usd: f64,
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

### D-23 引擎会话表 · 已实现（#22）

```sql
CREATE TABLE IF NOT EXISTS engine_sessions (
    agent_id      TEXT NOT NULL,
    thread_id     TEXT NOT NULL,
    engine        TEXT NOT NULL,   -- codex
    session_id    TEXT NOT NULL,   -- Codex 的会话 id（thread.started 事件里给出），resume 时用
    last_message  TEXT NOT NULL,   -- 已交给引擎的最后一条消息；下一轮从它之后取新消息
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (agent_id, thread_id)
) WITHOUT ROWID;
```

- 这张表是运行状态，不写事件日志：每轮都会更新，写进日志只会让日志膨胀。
- `last_message` 只在一轮成功后前移，并且取这一轮开始时对话里的最后一条消息。所以这一轮进行中新来的消息不会被跳过，失败的一轮下次会重新交一遍。
- 岗位换了引擎种类时，旧会话不再续接，从头开始。

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
