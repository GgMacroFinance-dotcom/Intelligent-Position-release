// 新建 Bot 的模型、校验和星球形态选择（方案 1，2026-11-27）。
//
// 主机的协议见 m2-spec D-42（2026-11-27 补齐）：
// - `create_role` / `create_agent` / `update_agent` / `delete_agent`，要“管理”权限；改动存在主机的数据库里，
//   重启后还在，不写回 ip.yaml。在线建的员工用主机默认的工作目录，不能指定别的目录。
// - `Agent.celestial_style`（`mercury`…`neptune`、`star`，不能是 `sun`）；没设时 App 按 id 自己挑。
// - 事件 `org_changed`：改完后的全部岗位和员工，各端直接换上。
//
// 连着主机、有“管理”权限时，新建 Bot 走主机（[AppState.createBot]）；日常权限、没连上或主机太旧时，
// 照旧只建在本机 `AppState._localBots` 里（能落轨、换轨、调重要度、移除，但不能和它对话）。
// =============================================================================

import 'package:flutter/foundation.dart';

import '../../protocol/models.dart';
import '../../star_map.dart';

/// 新建 Bot 的结果：建在主机上（[onHost]，m2-spec D-42），还是只在本机（日常权限、没连上或主机太旧）。
typedef CreatedBot = ({Agent agent, Celestial celestial, bool onHost});

/// 主机 `celestial_style` 的写法：行星用自己的 id，恒星一律是 `star`。
String celestialStyleOf(Celestial c) => c.kind == CelestialKind.star ? 'star' : c.id;

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
      'celestial_style': celestialStyleOf(celestial),
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
