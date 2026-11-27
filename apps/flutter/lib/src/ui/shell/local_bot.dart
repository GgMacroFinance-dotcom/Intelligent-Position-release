// 本地新建 Bot 的模型、校验、星球形态选择与后端协议空缺清单（方案 1，2026-11-27）。
//
// =============================================================================
// 【后端协议与持久化空缺说明（BACKEND / WIRE GAP）—— 后续联调必读】
// =============================================================================
// 1. WebSocket 协议空缺（`rust/crates/ip-server/src/protocol.rs` & `ws.rs`）：
//    - 当前主机 `/ws` 协议对组织定义仅开放只读查询：`Request::ListRoles`、`Request::ListAgents`。
//    - 尚未暴露任何动态创建/修改/删除岗位与员工的 RPC 请求，建议后端补充以下管理权限（`Scope::Admin`）请求：
//        * `create_role`：{ id, name?, description, model: { default, escalate? }, budget_per_day_usd? } -> Role
//        * `create_agent`：{ id, role, name, tools?, workspace?, celestial_style? } -> Agent
//        * `update_agent`：{ agent_id, name?, role?, tools?, workspace?, celestial_style? } -> Agent
//        * `delete_agent`：{ agent_id } -> null
//    - 注意：Rust 运行时底层（`rust/crates/ip-runtime/src/lib.rs`）其实已经具备动态注册能力：
//        * `Runtime::add_role(&self, role: Role)`（lib.rs:493）
//        * `Runtime::spawn_agent(&self, setup: AgentSetup)`（lib.rs:370）
//      但 `ip-server` 层的 `App.org` 目前仍是启动期一次性装配（`app.rs:289-325`），尚未支持运行时热更新与落盘。
//
// 2. 持久化空缺（`ip.yaml` / PostgreSQL）：
//    - 目前岗位与员工完全来自主机启动时加载的静态 `ip.yaml`（`Config::load`）；
//    - 后端后续需决定是写回 `ip.yaml`（类似 `examples/ops-gateway/model_prices.py`）还是迁入 PostgreSQL `ip` schema 独立表，
//      并在重启 `ip serve` 时优先合并持久化数据（类似 `rules::load_saved` 的“控制台保存优先于 ip.yaml”模式）。
//
// 3. 星球形态持久化字段空缺（`rust/crates/ip-core/src/org.rs` & `ip-server/src/config.rs`）：
//    - 当前 `Agent` 结构体只有 `{ id, role, name }`，`AgentConfig` 只有 `{ id, role, name, tools, workspace }`，
//      没有记录用户手选星球形态的字段。
//    - 建议后端在 `Agent` / `AgentConfig` 上增加可选字段：
//        * `celestial_style: Option<String>`
//          合法值对应非太阳的 9 种天体：`mercury` | `venus` | `earth` | `mars` | `jupiter` |
//          `saturn` | `uranus` | `neptune` | `star`（严格禁止 `sun`，太阳仅保留给中心幕僚长）；
//          允许多个员工重复选用同一星球形态；字段为空时前端仍按 `assignCelestials(agent.id)` 自动哈希分配。
//
// 4. 实时广播事件空缺（`ip-runtime` `RuntimeEvent`）：
//    - 当前 `RuntimeEvent` 尚无 `RoleAdded` / `AgentSpawned` / `AgentRemoved` 事件；
//    - 后端补齐后，多端在线客户端收到事件即可直接把新星球飞入天穹轨道、无需全量重拉。
//
// 5. 对话与任务联调边界（当前前端本地闭环行为）：
//    - 在上述接口补齐前，通过本弹窗新建的 Bot 仅保存在全局 `AppState._localBots` 中：
//        * 立刻以用户手选的星球形态进入天穹外环轨道，可拖拽换轨、出现在开窗顶栏、跨 980px 缩放不丢失；
//        * 可右键触发「完成 · 待审批」演示闭环；
//        * 点开三栏员工大窗时展示本地配置档案与「移除此本地 Bot」按钮，不向主机发送 `send` 请求（否则主机必报 `UnknownAgent`）。
//    - 后续后端接通 `create_role` / `create_agent` 后，只需把 `AppState.createLocalBot` 替换为真正的 RPC 调用，
//      前端表单（[CreateBotDraft.toPlannedWireJson]）与星图管线（`buildStarMap(celestialOverrides: ...)`）可直接复用。
// =============================================================================

import 'package:flutter/foundation.dart';

import '../../protocol/models.dart';
import '../../star_map.dart';

/// 新建 Bot 时全部可选的 9 种天体形态：八大行星 + 恒星（天狼星）。
///
/// 规则（2026-11-27 维护者定）：
/// - **太阳严格禁止选择**：太阳专属于中心幕僚长，绝不进入普通 Bot 选星列表；
/// - **允许重复选择**：多位员工可以选用同一颗行星/恒星形态（已使用的仅做轻提示）。
final List<Celestial> selectableBotCelestials = List<Celestial>.unmodifiable([...planets, stars.first]);

/// 按天体 ID 查找对应的可选非太阳天体（用于未来从后端 `Agent.celestial_style` 反序列化）。
Celestial? selectableCelestialById(String? id) {
  if (id == null || id.isEmpty || id == sun.id) return null;
  if (id == 'star') return stars.first;
  for (final c in selectableBotCelestials) {
    if (c.id == id) return c;
  }
  return null;
}

/// 生成满足后端 `ConfigError::BadAgentId`（仅含 ASCII 字母、数字、`-`、`_`）的唯一 ID。
String generateUniqueSlug(String raw, {required String prefix, required Set<String> existingIds}) {
  final ascii = raw
      .trim()
      .toLowerCase()
      .replaceAll(RegExp(r'[^a-z0-9_-]+'), '-')
      .replaceAll(RegExp(r'-+'), '-')
      .replaceAll(RegExp(r'^[-_]+|[-_]+$'), '');
  final base = ascii.isNotEmpty ? ascii : prefix;
  if (ascii.isNotEmpty && !existingIds.contains(base)) return base;
  var n = 1;
  while (existingIds.contains('$base-$n')) {
    n++;
  }
  return '$base-$n';
}

/// 新建 Bot 表单提交草稿（字段严格对齐 `ip.yaml` 的 `Role` + `AgentConfig` + 新增 `celestial_style` 提案）。
@immutable
class CreateBotDraft {
  const CreateBotDraft({
    required this.name,
    required this.celestial,
    this.existingRoleId,
    this.newRoleName,
    this.newRoleDescription,
    this.modelDefault = 'openai/gpt-6-luna',
    this.modelEscalate,
    this.budgetPerDayUsd,
    this.tools = true,
    this.workspace,
  });

  /// 员工显示名（对应 `AgentConfig.name`）。
  final String name;

  /// 用户手选的非太阳天体形态（对应提案字段 `AgentConfig.celestial_style`）。
  final Celestial celestial;

  /// 绑定到已有岗位时的岗位 ID；为 null 表示随同本 Bot 一并新建一个岗位。
  final String? existingRoleId;

  /// 新建岗位时的显示名（对应 `Role.name`）。
  final String? newRoleName;

  /// 新建岗位时的职责说明与工作流提示词（对应 `Role.description`，后端语义路由也依赖该文本）。
  final String? newRoleDescription;

  /// 新建岗位时的默认模型 `<端点>/<模型>`（对应 `Role.model.default`）。
  final String modelDefault;

  /// 新建岗位时的升级模型（可选，对应 `Role.model.escalate`）。
  final String? modelEscalate;

  /// 新建岗位时的每日预算上限（美元，可选且必须 > 0，对应 `Role.budget_per_day_usd`）。
  final double? budgetPerDayUsd;

  /// 是否给该员工开放沙箱 shell 与文件工具（对应 `AgentConfig.tools`，默认 true）。
  final bool tools;

  /// 可选工作目录路径（对应 `AgentConfig.workspace`，为空时由主机默认 `<data_dir>/workspaces/<id>`）。
  final String? workspace;

  bool get createsNewRole => existingRoleId == null;

  /// 预留的后端协议请求体示意（供后续 `create_role` / `create_agent` 对接时直接核对字段）。
  Map<String, Object?> toPlannedWireJson({required String agentId, required String roleId}) => {
    if (createsNewRole)
      'create_role': {
        'id': roleId,
        'name': (newRoleName ?? '').trim().isEmpty ? roleId : newRoleName!.trim(),
        'description': (newRoleDescription ?? '').trim().isEmpty ? '${name.trim()}的岗位职责说明' : newRoleDescription!.trim(),
        'model': {
          'default': modelDefault.trim().isEmpty ? 'openai/gpt-6-luna' : modelDefault.trim(),
          if ((modelEscalate ?? '').trim().isNotEmpty) 'escalate': modelEscalate!.trim(),
        },
        if (budgetPerDayUsd != null && budgetPerDayUsd! > 0) 'budget_per_day_usd': budgetPerDayUsd,
      },
    'create_agent': {
      'id': agentId,
      'role': roleId,
      'name': name.trim(),
      'tools': tools,
      if ((workspace ?? '').trim().isNotEmpty) 'workspace': workspace!.trim(),
      // TODO(backend-create-bot)：后端 `Agent` / `AgentConfig` 补齐此可选字段即可持久化手选星球。
      'celestial_style': celestial.kind == CelestialKind.star ? 'star' : celestial.id,
    },
  };
}

/// 一条在前端本地创建（待后端接入动态创建接口后同步持久化）的 Bot 档案。
@immutable
class LocalBotRecord {
  const LocalBotRecord({
    required this.agent,
    required this.celestial,
    required this.tools,
    required this.createdAtMs,
    this.createdRole,
    this.workspace,
  });

  final Agent agent;

  /// 若本次新建顺带创建了新岗位，则随记录保存，供星图/岗位页/右栏 Inspector 同源读取。
  final Role? createdRole;

  /// 用户手选的星球形态（永不为太阳）。
  final Celestial celestial;

  /// 对应 `AgentConfig.tools`。
  final bool tools;

  /// 对应 `AgentConfig.workspace`。
  final String? workspace;

  final int createdAtMs;
}
