// 岗位与员工、今日花费（backlog F-15，和控制台的岗位页一样）：每个岗位的模型、今天花了多少（对比每日预算）、
// 提示词缓存命中率、员工（点了和他对话）、最近的报告。

import 'package:flutter/material.dart';

import '../protocol/models.dart';
import 'common.dart';
import 'report.dart';

/// 和控制台一样：不到一分钱的显示四位小数。
String usd(double n) => n > 0 && n < 0.01 ? '\$${n.toStringAsFixed(4)}' : '\$${n.toStringAsFixed(2)}';

String _cacheHit(Usage? u) {
  if (u == null || u.promptTokens == 0) return '还没有调用';
  final rate = (100 * u.cachedTokens / u.promptTokens).round();
  return '$rate%（输入 ${u.promptTokens} token，其中命中 ${u.cachedTokens}）';
}

class RolesPage extends StatelessWidget {
  const RolesPage({super.key, required this.onChat});

  /// 点“和 X 对话”：开一个发给这位员工的新对话。
  final void Function(String agentId) onChat;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final theme = Theme.of(context);
    final muted = TextStyle(color: theme.hintColor);
    final allRoles = state.allRoles;
    if (allRoles.isEmpty) {
      return EmptyHint(state.connected ? '还没有岗位。在主机的 ip.yaml 里定义 roles。' : '连上主机后，岗位和员工会显示在这里。');
    }
    final spend = state.spend;
    final hit = spend?.cacheHitRate;
    return ListView(
      padding: const EdgeInsets.fromLTRB(12, 12, 12, 24),
      children: [
        if (spend != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(4, 0, 4, 12),
            child: Text(
              '今天（UTC ${spend.date}）模型费用合计 ${usd(spend.totalUsd)}'
              '${spend.routingUsd > 0 ? '，其中自动分派 ${usd(spend.routingUsd)}' : ''}。'
              '没在端点 prices 里登记价格的模型不计费。'
              '${hit == null ? '' : '提示词缓存命中 ${(hit * 100).round()}%。'}',
              style: muted,
            ),
          ),
        for (final role in allRoles) _RoleCard(role: role, onChat: onChat),
      ],
    );
  }
}

class _RoleCard extends StatelessWidget {
  const _RoleCard({required this.role, required this.onChat});

  final Role role;
  final void Function(String agentId) onChat;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final theme = Theme.of(context);
    final muted = TextStyle(color: theme.hintColor);
    final agents = state.allAgents.where((a) => a.role == role.id).toList();
    final spent = state.spend?.roles[role.id] ?? 0;
    final limit = role.budgetPerDayUsd;
    final over = limit != null && spent >= limit;
    final ids = {for (final a in agents) a.id};
    final reports = state.reports.where((r) => ids.contains(r.agentId)).take(5).toList();

    Widget row(String label, Widget value) => Padding(
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(width: 92, child: Text(label, style: muted)),
          Expanded(child: value),
        ],
      ),
    );

    return Card(
      margin: const EdgeInsets.only(bottom: 12),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 14, 16, 10),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text.rich(
              TextSpan(
                children: [
                  TextSpan(text: role.displayName, style: theme.textTheme.titleMedium),
                  TextSpan(
                    text: '  ${role.id}',
                    style: muted.copyWith(fontFamily: 'monospace', fontSize: 12),
                  ),
                ],
              ),
            ),
            if (role.description.isNotEmpty) ...[const SizedBox(height: 6), _Description(role.description)],
            const SizedBox(height: 8),
            row(
              '模型',
              SelectableText(
                role.escalateModel == null ? role.model : '${role.model} → ${role.escalateModel}',
                style: const TextStyle(fontFamily: 'monospace', fontSize: 13),
              ),
            ),
            row(
              '今日费用',
              Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    limit == null ? usd(spent) : '${usd(spent)} / \$${limit.toStringAsFixed(limit % 1 == 0 ? 0 : 2)}',
                    style: over ? TextStyle(color: theme.colorScheme.error, fontWeight: FontWeight.w600) : null,
                  ),
                  if (limit != null && limit > 0) ...[
                    const SizedBox(height: 4),
                    LinearProgressIndicator(
                      value: (spent / limit).clamp(0, 1),
                      color: over ? theme.colorScheme.error : null,
                      minHeight: 4,
                      borderRadius: BorderRadius.circular(2),
                    ),
                  ],
                  if (over) Text('已用满，UTC 0 点后恢复', style: TextStyle(color: theme.colorScheme.error, fontSize: 12)),
                ],
              ),
            ),
            row('今日缓存命中', Text(_cacheHit(state.spend?.tokens[role.id]))),
            const SizedBox(height: 8),
            if (agents.isEmpty) Text('这个岗位还没有员工', style: muted),
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: [
                for (final a in agents)
                  ActionChip(
                    avatar: _AgentDot(
                      working: state.agentsWorking.contains(a.id),
                      failed: state.agentFailures.containsKey(a.id),
                    ),
                    label: Text('和 ${a.name} 对话'),
                    tooltip: state.agentFailures[a.id] == null ? a.id : '上一轮失败了：${state.agentFailures[a.id]}',
                    onPressed: () => onChat(a.id),
                  ),
              ],
            ),
            if (reports.isNotEmpty) ...[
              const SizedBox(height: 8),
              Text('最近的报告', style: theme.textTheme.labelMedium?.copyWith(color: theme.hintColor)),
              for (final r in reports)
                ListTile(
                  dense: true,
                  contentPadding: EdgeInsets.zero,
                  leading: const Icon(Icons.description_outlined, size: 20),
                  title: Text(r.fileName),
                  subtitle: Text(
                    '${agents.length > 1 ? '${state.agentName(r.agentId)} · ' : ''}${formatTime(r.modifiedMs)}',
                    style: muted,
                  ),
                  onTap: () => openReport(context, r.path, hint: r.agentId),
                ),
            ],
          ],
        ),
      ),
    );
  }
}

/// 员工状态的小圆点：正在处理是绿的，上一轮失败是红的，其余灰的。
class _AgentDot extends StatelessWidget {
  const _AgentDot({required this.working, required this.failed});

  final bool working;
  final bool failed;

  @override
  Widget build(BuildContext context) {
    final color = failed
        ? Theme.of(context).colorScheme.error
        : working
        ? const Color(0xFF22C55E)
        : Theme.of(context).disabledColor;
    return Container(
      width: 8,
      height: 8,
      decoration: BoxDecoration(color: color, shape: BoxShape.circle),
    );
  }
}

/// 岗位说明常常很长（工作流程写了好几条）：先显示三行，点了展开。
class _Description extends StatefulWidget {
  const _Description(this.text);

  final String text;

  @override
  State<_Description> createState() => _DescriptionState();
}

class _DescriptionState extends State<_Description> {
  bool _open = false;

  @override
  Widget build(BuildContext context) {
    final long = widget.text.split('\n').length > 3 || widget.text.length > 120;
    return InkWell(
      onTap: long ? () => setState(() => _open = !_open) : null,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(widget.text.trim(), maxLines: _open ? null : 3, overflow: _open ? null : TextOverflow.ellipsis),
          if (long)
            Text(_open ? '收起' : '展开', style: TextStyle(color: Theme.of(context).colorScheme.primary, fontSize: 12)),
        ],
      ),
    );
  }
}
