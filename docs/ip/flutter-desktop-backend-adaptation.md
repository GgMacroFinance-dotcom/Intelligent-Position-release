# Flutter 桌面宽屏外壳新功能与后端适配清单

> 范围：本轮 `design/dusk-shell` 宽屏桌面外壳重构（全部前端改动位于 `apps/flutter/`，不涉及 `rust/` 与 `apps/desktop/`）。
> 目的：汇总新加入的桌面端交互功能、代码里预留的 `TODO(backend-*)` / `BACKEND / WIRE GAP` 接口位置，以及全库扫描发现的隐性协议缺口，便于后端按需排期适配。
>
> **进展（2026-11-27）**：第一批（第 5、9、1 项）主机端已做，App 也接上了，规格见 `docs/ip/m2-spec.md` D-40；第 2 项也已做，规格见 D-41（任务和审批卡片的字段和这里建议的一致：`pending_approval`、`task_completion`，choice 0 通过、1 打回修改）。排期跟踪在 `docs/ip/backlog.md` F-23。第 3、7 项定了改动存数据库、不写回 ip.yaml。下面各项的“当前前端状态”是 PR #74 合并时的样子，已做的在总览表里标了。
>
> **联调共通提示**：凡修改 `rust/crates/ip-core` 或 `rust/crates/ip-server/src/protocol.rs` 中的结构体或枚举，请同步重新生成 `apps/flutter/lib/src/protocol/wire.dart`；`apps/flutter/lib/src/protocol/models.dart` 会在编译期自动校验字段对齐。

---

## 一、总览速查表

| 编号 | 类别 | 功能 / 问题点 | 当前前端状态 | 建议后端落点 |
|---|---|---|---|---|
| **1**（已做，D-40） | 本轮新功能（显式预留） | 三条固定轨道手动拖拽落位 & 满环级联挤出同步 | 内存闭环（跨 980px 不丢），预留批量同步钩子 | `ip-core/src/org.rs`、`ip-server/src/protocol.rs` & `ws.rs` |
| **2**（已做，D-41） | 本轮新功能（显式预留） | 任务「完成 · 待审批」离轨停泊 & 双向审批回流 | 已兼容真实协议枚举；当前本地演示兜底 | `ip-core/src/task.rs`、`approval.rs`、`ip-runtime`、`ip-server` + 重生成 `wire.dart` |
| **3** | 本轮新功能（显式预留） | 右下角 `+ 新建 Bot` 动态增删岗位/员工 & 星球形态持久化 | 本地 `AppState._localBots` 闭环，已备 Wire 载荷 | `ip-core/src/org.rs`、`ip-runtime/src/lib.rs`、`ip-server/src/config.rs` & `protocol.rs` & `ws.rs` |
| **4** | 本轮新功能（显式预留） | 星球滚轮/左栏调重要度 → AI 资源分配传导 & 冲突先后反馈 | 仅开放缩放手感 + 顺位计算 + 空端口，后端暂不执行 | `ip-core/src/org.rs`、`ip-runtime/src/agent.rs` & `budget.rs`、`ip-server/src/protocol.rs` |
| **5**（已做，D-40） | 全库扫出（隐性协议缺口） | 对话归属员工字段缺失（重启后左栏漏历史闲聊） | 仅靠内存 `threadAgent` + 任务/审批表拼凑 | `ip-core` / `ip-server` `ThreadSummary` & `list_threads` |
| **6** | 全库扫出（隐性协议缺口） | 聊天输入框左下角 `+` 附件按钮禁用占位 | 按钮已摆位并禁用（提示主机暂不支持） | `ip-server` 上传端点 & `Send` / `MessageBody` 多模态协议 |
| **7** | 全库扫出（隐性协议缺口） | 三栏右栏「高级」页与岗位页全只读（需改 `ip.yaml` 重启） | 只读展示，无法在线改模型/预算/职责/工具 | 与第 3 项合并：`update_role` / `update_agent` |
| **8** | 全库扫出（隐性协议缺口） | 定时任务页仅支持查看与「立即运行一次」 | 无法新建/编辑 cron、启停或删除 | `ip-server/src/protocol.rs` & 定时任务持久化 |
| **9**（已做，D-40） | 全库扫出（隐性协议缺口） | 中心「太阳（幕僚长）」身份靠名称/ID 字符串启发式猜测 | 依赖匹配 `coordinator`/`chief`/“协调”/“幕僚” | `ip-core/src/org.rs` `Role` 或 `Agent` 显式标记 |
| **10** | 全库扫出（体验优化） | 缺少部分实时推送事件（报告/花费/组织/星图偏好） | 每轮结束防抖重拉全量列表；旧主机降级兼容 | `ip-runtime` `RuntimeEvent` |

---

## 二、本轮桌面新功能明确预留的 4 项后端适配

### 1. 三条通用轨道手动拖拽落位 & 满环级联挤出同步（`TODO(backend-orbit-sync)`）

- **产品行为**：
  - 宽屏天穹设有 3 条解耦的通用固定轨道：内环（`0`，上限 3）、中环（`1`，上限 4）、外环（`2`，上限 7；总常规容量 14，超额时外环软兜底）。
  - 用户可手动拖拽任意非太阳星球吸附到目标环；满环时按确定性 1A 规则将最旧/最不急的星球级联挤向外侧轨道。
  - 任务「完成 · 待审批」通过时自动解除该员工的手动轨道 pin，回自动排轨。
- **需后端适配**：
  1. 在 `Agent`（或星图布局偏好表）持久化两个可选字段：
     - `orbit_track: Option<u8>`（`0 = 内环`、`1 = 中环`、`2 = 外环`、`null = 恢复自动`）
     - `orbit_pinned_at: Option<i64>`（Unix ms，用于满环挤出破平）
  2. 提供批量同步接口（WS RPC 或 `PATCH /star-map/orbit-pins`），接收 `changes: [{ agent_id, orbit_track, orbit_pinned_at }]`（包含用户拖入的员工与被级联挤出的员工），并同步多端。
- **前端代码与注释位置**：
  - 规则与协议约定：`apps/flutter/lib/src/ui/shell/orbit.dart`（L3–L36）
  - 序列化结构 `OrbitPin.toJson()` / `OrbitPinChange`：`apps/flutter/lib/src/ui/shell/orbit.dart`（L45–L98）
  - 全局状态：`apps/flutter/lib/src/app_state.dart`（L128–L131，`manualOrbitPins`）
  - 批量同步预留钩子 `_syncOrbitChangesToBackend`：`apps/flutter/lib/src/ui/shell/desktop_shell.dart`（L475–L499）

---

### 2. 任务「完成 · 待审批」离轨停泊 & 双向审批回流（`TODO(backend-completion-approval)`）

- **产品行为**：
  - 当员工完成任务进入「已完成 · 待审批」时，其星球缩小脱离三条轨道并停泊到左中下底栏（释放 3/4/7 槽位，锁定不可拖拽），与左上角红绿灯待办、三栏工作大窗三处数据同源。
  - **严格语义隔离**：现有的 `agent_question` 表示员工干活中途提问（未完成），继续留在内环，**不可**与「任务完成待审批」混用。
  - 审批通过后结项、清除手动轨道 pin 并放大飞回自动排轨；审批打回后退回进行中并飞回活跃轨道。
- **需后端适配（以下两路至少补齐其一，两路皆补时前端自动按 `agentId` 去重）**：
  1. **任务状态枚举**：`TaskStatus` 增加 `pending_approval`（或 `awaiting_approval`），交付时将验收摘要写入 `Task.result`；审批通过改 `done`，打回改 `in_progress`。
  2. **审批主体类型**：`ApprovalSubject` 增加 `task_completion`（或 `task_approval`），携带 `agent_id`、`task_id`、`thread_id?`、`deliverable?`；固定选项顺序 `choice 0 = 审批通过`、`choice 1 = 打回修改`。
  3. 重新生成 `apps/flutter/lib/src/protocol/wire.dart`（当前未知 `TaskStatus` 落为空串，未知 `ApprovalSubject` 不解析 `agent_id`）。
- **前端代码与注释位置**：
  - 完整空缺文档与聚合提取 `CompletionApproval.extractFrom`：`apps/flutter/lib/src/ui/shell/completion_approval.dart`（L7–L176）
  - 协议模型兼容 getter `Approval.isTaskCompletion` / `Task.isPendingApproval`：`apps/flutter/lib/src/protocol/models.dart`（L427–L444、L502–L532）
  - 状态聚合、演示触发与审批动作 `decideCompletionApproval`：`apps/flutter/lib/src/app_state.dart`（L110–L127、L371–L430）

---

### 3. 右下角 `+ 新建 Bot` 动态增删岗位/员工 & 星球形态持久化（`TODO(backend-create-bot)`）

- **产品行为**：
  - 用户从 9 种非太阳天体（水星、金星、地球、火星、木星、土星、天王星、海王星、恒星；严格排除太阳，允许重复选用）中挑选形态、输入名称，并可折叠配置绑定已有岗位或新建岗位（`model.default`、`model.escalate`、`budget_per_day_usd`、`tools`、`workspace`）。
  - 当前在前端 `AppState._localBots` 闭环演示（可落轨、换轨、缩放重要度、查看配置与移除，但不向主机发送 `send`，避免触发 `UnknownAgent`）。
- **需后端适配**：
  1. **暴露管理权限 WS RPC**（注：`rust/crates/ip-runtime/src/lib.rs` 底层已有 `Runtime::add_role` 与 `Runtime::spawn_agent`，但 `ip-server` 目前仅开放只读 `list_roles` / `list_agents`）：
     - `create_role`：`{ id, name?, description, model: { default, escalate? }, budget_per_day_usd? } -> Role`
     - `create_agent`：`{ id, role, name, tools?, workspace?, celestial_style? } -> Agent`
     - `update_agent`：`{ agent_id, name?, role?, tools?, workspace?, celestial_style? } -> Agent`
     - `delete_agent`：`{ agent_id } -> null`
  2. **持久化策略**：写回 `ip.yaml` 或迁入 PostgreSQL `ip` schema 并在重启 `ip serve` 时优先合并（类似 `rules::load_saved`）。
  3. **星球形态字段**：在 `Agent` / `AgentConfig` 增加可选字段 `celestial_style: Option<String>`，合法值为 `mercury | venus | earth | mars | jupiter | saturn | uranus | neptune | star`（严禁 `sun`）。
  4. **实时广播事件**：`RuntimeEvent` 新增 `RoleAdded` / `AgentSpawned` / `AgentRemoved`。
- **前端代码与注释位置**：
  - 完整 5 点空缺文档与预留 Wire 载荷 `CreateBotDraft.toPlannedWireJson`：`apps/flutter/lib/src/ui/shell/local_bot.dart`（L3–L43、L132–L154）
  - 本地闭环与待替换 RPC 入口 `createLocalBot` / `removeLocalBot` / `celestialOverrides`：`apps/flutter/lib/src/app_state.dart`（L189–L303）
  - 星图覆盖注入口 `buildStarMap(celestialOverrides: ...)`：`apps/flutter/lib/src/star_map.dart`（L210–L236）
  - 前端说明与占位区：`apps/flutter/lib/src/ui/shell/create_bot_dialog.dart`（L1155–L1165）、`apps/flutter/lib/src/ui/shell/agent_panel.dart`（L495–L560）

---

### 4. 星球滚轮/左栏调重要度 → AI 资源分配传导 & 冲突先后顺序反馈（预留接口，后端暂未执行）

- **产品行为**：
  - 与「轨道 = 紧急度」正交，引入「星球大小 = 重要度（`0.0..1.0`，对应球径 `18px..48px`）」：
    - 天穹上悬停任意在轨非太阳星球滚轮即可平滑放大/缩小，滚动当下短暂浮现 `重要度 XX% · 顺位 #N`（停滚 1.1s 淡出，太阳不参与）；
    - 三栏大窗左侧底部支持滑块 / `-` / `+` 调节与一键 `复位`。
  - 当前前端已实时算出全队确定性顺位 `#1..#N` 与归一化资源配额占比 `resourceShare`，所有后端传导与冲突反馈端口目前为 `Noop` 空实现（不发请求、不弹模拟告警）。
- **需后端适配**：
  1. **重要度持久化与 WS RPC**：
     - `Agent` 新增 `importance: Option<f64>`（`0.0..1.0`，`null` 回自动）与 `importance_updated_at: Option<i64>`；
     - 开放 `set_agent_importance { agent_id, importance, updated_at_ms }` 与 `clear_agent_importance { agent_id }`，由前端注入真实 `ImportanceSyncPort` 实现（滚轮已自带 300ms 防抖）。
  2. **AI 分配与调度传导**（`rust/crates/ip-runtime/src/agent.rs` & `budget.rs`）：
     - 并发槽上限、模型 API 限流（429/RPM/TPM）、共享工具（浏览器/MCP）争用时按「有效重要度 → 轨道 → 调权时间」排队放行；
     - 团队预算紧张时按 `resourceShare` 加权倾斜，高顺位优先保留 `model.escalate` 资格，低顺位收紧至轻量默认模型；
     - 将全队重要度顺位注入中心太阳（幕僚长）提示词上下文，影响拆活派单优先级。
  3. **资源冲突反馈事件**（`RuntimeEvent::ResourceContention`）：
     - 发生资源争用时推送冲突快照（`kind`、`occurred_at_ms`、`summary`、`queue[{ agent_id, rank, effective_importance, disposition, waiting_behind_agent_id?, reason }]`），接入 `ContentionFeedbackPort` 即可实时显形。
- **前端代码与注释位置**：
  - 完整契约、载荷 `toPlannedWireJson`、顺位计算器 `PriorityAllocator` 及两大预留端口 `ImportanceSyncPort` / `ContentionFeedbackPort`：`apps/flutter/lib/src/ui/shell/planet_importance.dart`（L3–L32、L80–L85、L194–L375）
  - 全局状态与端口挂载：`apps/flutter/lib/src/app_state.dart`（L133–L188）
  - 滚轮防抖钩子：`apps/flutter/lib/src/ui/shell/desktop_shell.dart`（L104–L106、L238–L267）
  - 星图重要度覆盖注入口：`apps/flutter/lib/src/star_map.dart`（L212–L214、L285–L295）

---

## 三、全库扫描发现的 6 项桌面端隐性协议缺口

### 5. 对话归属员工字段缺失（重启后左栏「XX 的对话」漏历史闲聊）
- **问题**：`list_threads` 返回的 `ThreadSummary` 不含归属员工字段，前端目前仅靠当次运行内存缓存 `threadAgent` + 任务表 + 审批表拼凑；App 重启后，未绑定任务/审批的普通历史对话不会出现在对应员工左栏列表里，必须点开过一次历史才恢复识别。
- **建议适配**：`ThreadSummary` 增加 `last_agent_id: Option<AgentId>`（或 `agent_ids: Vec<AgentId>`），可选支持 `list_threads { agent_id?: ... }`。
- **代码位置**：`apps/flutter/lib/src/ui/shell/agent_panel.dart`（L32–L73）、`apps/flutter/lib/src/app_state.dart`（L97–L98、L752–L766）。

### 6. 聊天输入框 `+` 附件按钮占位但后端暂不支持
- **问题**：聊天输入框左下角已摆好 `+` 附件按钮，当前 `onPressed: null` 置灰禁用，提示 `附件（主机还不支持）`。
- **建议适配**：补充文件/图片上传端点，并在 `Send` 请求及 `MessageBody` 中支持附件字段与多模态透传。
- **代码位置**：`apps/flutter/lib/src/ui/chat_page.dart`（L559–L566）。

### 7. 三栏大窗右栏「高级」页 & 岗位页全只读
- **问题**：目前只能查看岗位模型、预算与说明，修改仍需手改主机 `ip.yaml` 并重启。
- **建议适配**：与第 3 项合并补齐 `update_role` / `update_agent`。
- **代码位置**：`apps/flutter/lib/src/ui/shell/agent_panel.dart`（L1607–L1663）、`apps/flutter/lib/src/ui/roles_page.dart`（L32）。

### 8. 定时任务仅支持查看与「立即运行一次」
- **问题**：设置 → 定时任务页与三栏大窗「概况」里的定时任务卡片仅支持 `list_schedules` 和 `run_schedule`，无法在客户端增删改或启停。
- **建议适配**：如需客户端闭环管理，新增 `save_schedule` / `toggle_schedule` / `delete_schedule` 并支持持久化覆盖 `ip.yaml`。
- **代码位置**：`apps/flutter/lib/src/ui/admin/schedules_page.dart`（L2、L70–L82）、`apps/flutter/lib/src/ui/shell/agent_panel.dart`（L1245–L1363）。

### 9. 中心「太阳（幕僚长）」身份靠名称/ID 字符串启发式猜测
- **问题**：协议层无显式总协调员标记，前端靠匹配岗位 `id` 含 `coordinator` / `chief` 或中文含“协调”/“幕僚”判断太阳归属；一旦修改岗位名或 ID，中心太阳会变为空位。
- **建议适配**：在 `Role` 或 `Agent` 上提供显式字段 `is_coordinator: bool`（或 `kind: coordinator | worker`）。
- **代码位置**：`apps/flutter/lib/src/star_map.dart`（L1–L3、L166–L172）、`apps/flutter/lib/src/ui/shell/agent_panel.dart`（L616–L618）、`apps/flutter/lib/src/ui/shell/agent_sheet.dart`（L1147）。

### 10. 部分状态缺少实时推送事件（前端靠防抖重拉全量列表兜底）
- **问题**：`RuntimeEvent` 尚无报告新增、费用变动、定时任务变更、组织/星图布局变更事件，前端在每轮结束用 300ms/500ms/800ms 定时器重拉列表；旧版主机上 `list_memory_off_threads` 与 `get_notify_settings` 需 `try/catch` 降级。
- **建议适配（优化项）**：按多端强一致需求逐步补齐 `ReportChanged` / `SpendChanged` / `OrgChanged` / `StarMapPrefsChanged`。
- **代码位置**：`apps/flutter/lib/src/app_state.dart`（L683–L694、L843–L888）、`apps/flutter/lib/src/protocol/models.dart`（L82–L104）。

---

## 四、非后端本地/平台备忘（无需后端改动）

- **外观深浅色仅存内存**：`apps/flutter/lib/src/ui/shell/settings.dart`（L18–L19，`themeChoice`），重启 App 会复位，后续只需前端落本机存储。
- **桌面端离线推送**：`apps/flutter/lib/src/push/lock_push.dart`（L1–L20）与 `apps/flutter/lib/src/ui/me_page.dart`（L180–L202）目前仅移动端接入 `ip-relay` 锁屏推送，桌面端走前台常驻连接 + 系统通知。
- **纯前端视觉/交互层**：文艺复兴字符画天穹与天体着色器（`apps/flutter/shaders/*`，含旧版恒星平滑圆切与新建弹窗暗金液态流光）、窗顶切换栏等大等距对齐、跨 `980px` 缩放状态保留、本地演示环境变量（`IP_DEMO_PLANETS` / `IP_DEMO_APPROVALS`）均无需后端适配。

---

## 五、推荐后端排期顺序

1. **第一批（协议小改动、收益立竿见影）**：第 **5 项**（`ThreadSummary.last_agent_id`）、第 **9 项**（`is_coordinator`）、第 **1 项**（`orbit_track` / `orbit_pinned_at`）。
2. **第二批（核心桌面工作流闭环）**：第 **2 项**（`pending_approval` / `task_completion` + 重生成 `wire.dart`）、第 **3 + 7 项**（组织/岗位动态增删改 + `celestial_style` + 广播事件）。
3. **第三批（调度传导与功能增强）**：第 **4 项**（重要度持久化 → `ip-runtime` 优先级并发/预算调度 & `ResourceContention` 事件）、第 **6 项**（聊天附件）、第 **8 项**（定时任务在线编辑）。
