// 任务完成待审批机制（2026-11-27 设计定稿：1C + 2 + 3B + 4 + 5A）：
// 当某位员工完成任务、进入「已完成 · 待审批」状态时，其星球缩小并平滑脱离三条公转轨道，
// 停泊到页面左中下底栏（紧邻左下角连接状态胶囊右侧），与左上角红绿灯待办保持数据同源；
// 点击左中下缩小星球直接滑出该员工的三栏工作大窗进行审批；审批通过后解除手动固定并飞回轨道，
// 审批驳回后飞回活跃轨道继续修改。
//
// =============================================================================
// 【后端协议（m2-spec D-41，2026-11-27 主机已补齐）】
// =============================================================================
// 1. 哪些任务要验收：建任务时勾“做完要我验收”（`create_task.needs_approval`），或负责人的岗位在 ip.yaml
//    配了 `approve_completion: true`；任务上是 `Task.needs_approval`。
// 2. 员工交回 done 时，主机把任务改成 `pending_approval`（交回的结论在 `Task.result`），
//    并发一张 `ApprovalSubject.task_completion` 验收卡片（`agent_id`、`task_id`、`thread_id?`，结论在 `Approval.reason`）。
//    选项固定：choice 0 = 通过（任务置 done、结果回报派活人、主机解除这位员工的轨道固定），
//    choice 1 = 打回修改（任务置回 in_progress，`resolve_approval.note` 写进负责人的对话，叫他接着改）。
// 3. 任务页把 `pending_approval` 的任务改成 done / in_progress，主机当作在它的卡片上选通过 / 打回；
//    其他改法主机会拒绝。两路都在时下面的 [CompletionApproval.collect] 按员工和对话去重，优先用卡片。
// 3. 语义隔离（方案 5A，严禁与中途提问混用）：
//    - 现有的 `agent_question` 表示员工干活中途卡住等你答复（任务未完成），继续留在内环（紧急度 1.0）。
//    - 只有满足本模块提取规则的「任务完成待审批」才会触发星球缩小离轨并停泊到左中下底栏。
//
// 4. 审批动作与轨道回流契约（方案 2）：
//    - 离轨期间：该员工不参与 3/4/7 三条公转轨道容量计数，原槽位立即释放；星球锁定不可拖拽。
//    - 审批通过（CompletionDecision.approve）：
//        * 调用 `resolveApproval(approval, 0)`（或将 `pending_approval` 任务 `updateTask(..., 'done')`）；
//        * 主机解除该员工的轨道固定并推 `orbit_pins_changed`；前端也同步清除本地的 `OrbitPin`，
//          让星球平滑放大飞回自动排轨（空闲时落外环，有其他活跃任务回中/内环）。
//    - 审批驳回（CompletionDecision.reject）：
//        * 调用 `resolveApproval(approval, 1, note: 要改的)`（或将任务 `updateTask(..., 'in_progress')`）；
//        * 星球平滑放大飞回活跃轨道（中环/内环）继续处理。
//
// 5. 本地演示兜底（没连主机时预览用）：
//    - 通过 `AppState.triggerDemoCompletionApproval`（演示模式下右键任意轨道星球，或启动参数
//      `IP_DEMO_APPROVALS=N`）在前端构造 [CompletionSource.localDemo] 记录，完整走通
//      「在轨 → 缩小离轨停泊左中下(1C) → 左上角待办同源同步(4) → 点击开三栏窗审批(3B) → 放大飞回轨道(2)」闭环。
// =============================================================================

import 'package:flutter/foundation.dart';

import '../../protocol/models.dart';

/// 人工对「已完成 · 待审批」任务做出的审批动作。
enum CompletionDecision {
  /// 审批通过：任务结项，解除手动固定（orbit pin），星球飞回自动排轨（通常落空闲外环）。
  approve,

  /// 审批驳回 / 打回修改：任务退回进行中，星球飞回活跃轨道（中环/内环）继续处理。
  reject,
}

/// 待审批记录的数据来源（方便区分真实后端协议数据与前端演示兜底数据）。
enum CompletionSource {
  /// 主机下发的验收卡片 `Approval(subjectKind: 'task_completion')`。
  backendApproval,

  /// 主机下发的 `Task(status: 'pending_approval')`。
  backendTask,

  /// 没连主机时用于预览的本地模拟项。
  localDemo,
}

/// 一条「任务已完成 · 待审批」的统一视图模型（左中下泊位、左上角待办、三栏工作大窗三处共用同一份数据源）。
@immutable
class CompletionApproval {
  const CompletionApproval({
    required this.id,
    required this.agentId,
    required this.agentName,
    required this.taskTitle,
    required this.summary,
    required this.createdAtMs,
    required this.source,
    this.approval,
    this.task,
    this.threadId,
  });

  /// 唯一标识（用于列表 key 与去重）。
  final String id;

  /// 提交交付的员工 ID（与星图里的 [BotBody.agent.id] 一一对应）。
  final String agentId;

  /// 员工显示名（用于左中下泊位与左上角待办卡片）。
  final String agentName;

  /// 已完成等待审批的任务标题。
  final String taskTitle;

  /// 员工提交的交付摘要 / 验收说明。
  final String summary;

  /// 提交待审批的毫秒时间戳（Unix ms）。
  final int createdAtMs;

  /// 数据来源：后端审批卡 / 后端待审任务 / 本地演示项。
  final CompletionSource source;

  /// 若来自后端 Approval，保留原始对象供 `resolveApproval` 调用。
  final Approval? approval;

  /// 若来自后端 Task，保留原始对象供 `updateTask` 调用。
  final Task? task;

  /// 关联的执行对话 ID（有值时点开三栏大窗可自动定位到该对话）。
  final String? threadId;

  /// 后端 Approval.subjectKind 是否属于「任务完成待审批」（严格区别于 `agent_question` 员工中途提问）。
  static bool isCompletionApprovalKind(String subjectKind) =>
      subjectKind == 'task_completion' || subjectKind == 'task_approval';

  /// 后端 Task.status 是否属于「已完成待审批」（主机用 `pending_approval`）。
  static bool isPendingApprovalTaskStatus(String status) =>
      status == 'pending_approval' || status == 'awaiting_approval';

  /// 从后端 `approvals`、`tasks` 以及前端演示池中提取统一的「完成待审批」列表（按最新提交时间倒序）。
  ///
  /// 去重原则：主机同时下发了 `task_completion` 类型的 Approval 与 `pending_approval` 状态的 Task
  /// 指向同一员工的同一对话/任务，优先保留信息更完整的 `backendApproval`。
  static List<CompletionApproval> collect({
    required List<Approval> approvals,
    required List<Task> tasks,
    required String Function(String agentId) agentName,
    List<CompletionApproval> localPending = const [],
  }) {
    final out = <CompletionApproval>[];
    final seenIds = <String>{};
    final approvalAgentsThreads = <(String, String?)>{};

    // 1. 优先识别主机下发的 `task_completion` 验收卡片。
    for (final a in approvals) {
      if (!a.pending || !isCompletionApprovalKind(a.subjectKind)) continue;
      final aid = a.subjectAgentId;
      if (aid == null || aid.isEmpty) continue;
      final id = 'approval:${a.id}';
      if (!seenIds.add(id)) continue;
      approvalAgentsThreads.add((aid, a.subjectThreadId));
      out.add(
        CompletionApproval(
          id: id,
          agentId: aid,
          agentName: agentName(aid),
          taskTitle: a.question,
          summary: a.reason.isNotEmpty ? a.reason : '任务已完成，等待你的审批验收。',
          createdAtMs: a.createdAtMs,
          source: CompletionSource.backendApproval,
          approval: a,
          threadId: a.subjectThreadId,
        ),
      );
    }

    // 2. 再识别 `pending_approval` 状态的任务（没有卡片时兜底）。
    for (final t in tasks) {
      if (!isPendingApprovalTaskStatus(t.status)) continue;
      final aid = t.assignee;
      if (aid == null || aid.isEmpty) continue;
      if (t.threadId != null && approvalAgentsThreads.contains((aid, t.threadId))) continue;
      final id = 'task:${t.id}';
      if (!seenIds.add(id)) continue;
      out.add(
        CompletionApproval(
          id: id,
          agentId: aid,
          agentName: agentName(aid),
          taskTitle: t.title,
          summary: (t.result != null && t.result!.trim().isNotEmpty) ? t.result!.trim() : '任务已完成，等待你的审批验收。',
          createdAtMs: t.updatedAtMs,
          source: CompletionSource.backendTask,
          task: t,
          threadId: t.threadId ?? t.originThreadId,
        ),
      );
    }

    // 3. 合并本地演示/模拟待审批项（协议补齐前用于实机预览）。
    for (final item in localPending) {
      if (!seenIds.add(item.id)) continue;
      out.add(item);
    }

    out.sort((a, b) => b.createdAtMs.compareTo(a.createdAtMs));
    return out;
  }

  /// 按员工归并出「每颗星球主待审批记录」（同一员工多条待审时保留最新一条用于泊位展示，并可通过列表查全量）。
  static Map<String, CompletionApproval> primaryByAgent(List<CompletionApproval> items) {
    final map = <String, CompletionApproval>{};
    for (final item in items) {
      map.putIfAbsent(item.agentId, () => item);
    }
    return map;
  }
}
