// 在线改主机的组织（m2-spec D-42、D-44，backlog F-23 ⑦）：改岗位、删岗位，改员工、删员工。
// 要这台设备有“管理”权限（[AppState.canEditOrg]）。改动存在主机的数据库里，不写回 ip.yaml；
// 改岗位、改员工的名字和岗位会重启相关的员工，他们有人在干活时主机拒绝，提示等这一轮做完再改。

import 'package:flutter/material.dart';

import '../app_state.dart';
import '../protocol/client.dart' show RequestError;
import '../protocol/models.dart';
import 'common.dart';
import 'shell/local_bot.dart' show celestialStyleOf, selectableBotCelestials;

/// 编辑一个岗位。保存成功返回 true；取消或失败（失败时框里显示原因）不关框，关了返回 null。
Future<bool?> showRoleEditor(BuildContext context, Role role) => showDialog<bool>(
  context: context,
  builder: (_) => _RoleEditor(role: role),
);

/// 编辑一位主机上的员工（名字、岗位、星图形态），也能删掉他。改了或删了返回 true。
Future<bool?> showAgentEditor(BuildContext context, Agent agent) => showDialog<bool>(
  context: context,
  builder: (_) => _AgentEditor(agent: agent),
);

/// 删掉一个岗位前先确认。删了返回 true。
Future<bool> confirmDeleteRole(BuildContext context, Role role) async {
  final ok = await showDialog<bool>(
    context: context,
    builder: (context) => AlertDialog(
      title: Text('删掉岗位「${role.displayName}」？'),
      content: const Text('岗位的记忆和花费记录留在主机上。路由规则、定时任务还指着它时主机会拒绝。'),
      actions: [
        TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('取消')),
        FilledButton(
          key: const ValueKey('confirm-delete-role'),
          onPressed: () => Navigator.pop(context, true),
          child: const Text('删掉'),
        ),
      ],
    ),
  );
  if (ok != true || !context.mounted) return false;
  try {
    await AppScope.read(context).deleteRole(role.id);
    return true;
  } on RequestError catch (e) {
    if (context.mounted) showError(context, '删岗位失败：$e');
    return false;
  }
}

class _RoleEditor extends StatefulWidget {
  const _RoleEditor({required this.role});

  final Role role;

  @override
  State<_RoleEditor> createState() => _RoleEditorState();
}

class _RoleEditorState extends State<_RoleEditor> {
  late final _name = TextEditingController(text: widget.role.name ?? '');
  late final _description = TextEditingController(text: widget.role.description);
  late final _model = TextEditingController(text: widget.role.model);
  late final _escalate = TextEditingController(text: widget.role.escalateModel ?? '');
  late final _budget = TextEditingController(
    text: widget.role.budgetPerDayUsd == null ? '' : '${widget.role.budgetPerDayUsd}',
  );
  late bool _approve = widget.role.approveCompletion;
  late bool _coordinator = widget.role.coordinator;
  bool _saving = false;
  String? _error;

  @override
  void dispose() {
    for (final c in [_name, _description, _model, _escalate, _budget]) {
      c.dispose();
    }
    super.dispose();
  }

  Future<void> _save() async {
    final budgetText = _budget.text.trim();
    final budget = budgetText.isEmpty ? null : double.tryParse(budgetText);
    if (budgetText.isNotEmpty && (budget == null || budget <= 0)) {
      setState(() => _error = '每日预算要写大于 0 的数字（美元），不设就留空');
      return;
    }
    if (_description.text.trim().isEmpty || _model.text.trim().isEmpty) {
      setState(() => _error = '岗位职责和默认模型都要写');
      return;
    }
    final name = _name.text.trim();
    final escalate = _escalate.text.trim();
    final next = widget.role.copyWith(
      name: name.isEmpty ? widget.role.id : name,
      description: _description.text.trim(),
      model: _model.text.trim(),
      escalateModel: () => escalate.isEmpty ? null : escalate,
      budgetPerDayUsd: () => budget,
      approveCompletion: _approve,
      coordinator: _coordinator,
    );
    setState(() {
      _saving = true;
      _error = null;
    });
    try {
      await AppScope.read(context).updateRole(next);
      if (mounted) Navigator.pop(context, true);
    } on RequestError catch (e) {
      if (mounted) setState(() => _error = '主机没有保存：$e');
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    InputDecoration field(String label, {String? hint}) =>
        InputDecoration(labelText: label, hintText: hint, border: const OutlineInputBorder(), isDense: true);
    return AlertDialog(
      title: Text('编辑岗位 ${widget.role.id}'),
      content: SizedBox(
        width: 460,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              TextField(key: const ValueKey('role-name'), controller: _name, decoration: field('显示名')),
              const SizedBox(height: 10),
              TextField(
                key: const ValueKey('role-description'),
                controller: _description,
                minLines: 3,
                maxLines: 10,
                decoration: field('岗位职责', hint: '也是自动分派比对的文字'),
              ),
              const SizedBox(height: 10),
              TextField(
                key: const ValueKey('role-model'),
                controller: _model,
                decoration: field('默认模型', hint: '<端点>/<模型>'),
              ),
              const SizedBox(height: 10),
              TextField(
                key: const ValueKey('role-escalate'),
                controller: _escalate,
                decoration: field('难题升级到（可选）', hint: '<端点>/<模型>'),
              ),
              const SizedBox(height: 10),
              TextField(
                key: const ValueKey('role-budget'),
                controller: _budget,
                keyboardType: const TextInputType.numberWithOptions(decimal: true),
                decoration: field('每日预算，美元（可选）', hint: '要在主机的端点 prices 里登记了价格'),
              ),
              CheckboxListTile(
                key: const ValueKey('role-approve'),
                contentPadding: EdgeInsets.zero,
                controlAffinity: ListTileControlAffinity.leading,
                value: _approve,
                onChanged: (v) => setState(() => _approve = v ?? false),
                title: const Text('派给这个岗位的任务做完要我验收'),
              ),
              CheckboxListTile(
                key: const ValueKey('role-coordinator'),
                contentPadding: EdgeInsets.zero,
                controlAffinity: ListTileControlAffinity.leading,
                value: _coordinator,
                onChanged: (v) => setState(() => _coordinator = v ?? false),
                title: const Text('协调岗位（星图中心的太阳，最多一个）'),
              ),
              Text(
                '保存后这个岗位的员工会重启才用上新设置；他们有人在干活时主机会拒绝，等这一轮做完再改。'
                '改动存在主机上，不写回 ip.yaml。',
                style: theme.textTheme.bodySmall?.copyWith(color: theme.hintColor),
              ),
              if (_error != null) ...[
                const SizedBox(height: 8),
                Text(_error!, style: TextStyle(color: theme.colorScheme.error)),
              ],
            ],
          ),
        ),
      ),
      actions: [
        TextButton(onPressed: _saving ? null : () => Navigator.pop(context), child: const Text('取消')),
        FilledButton(
          key: const ValueKey('role-save'),
          onPressed: _saving ? null : _save,
          child: _saving
              ? const SizedBox.square(dimension: 16, child: CircularProgressIndicator(strokeWidth: 2))
              : const Text('保存'),
        ),
      ],
    );
  }
}

class _AgentEditor extends StatefulWidget {
  const _AgentEditor({required this.agent});

  final Agent agent;

  @override
  State<_AgentEditor> createState() => _AgentEditorState();
}

class _AgentEditorState extends State<_AgentEditor> {
  late final _name = TextEditingController(text: widget.agent.name);
  late String _role = widget.agent.role;

  /// 空字符串是“自动”（按 id 挑）。
  late String _style = widget.agent.celestialStyle ?? '';
  bool _saving = false;
  String? _error;

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  Future<void> _run(Future<void> Function(AppState state) work) async {
    setState(() {
      _saving = true;
      _error = null;
    });
    try {
      await work(AppScope.read(context));
      if (mounted) Navigator.pop(context, true);
    } on RequestError catch (e) {
      if (mounted) setState(() => _error = '主机没有改：$e');
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  Future<void> _save() async {
    final a = widget.agent;
    final name = _name.text.trim();
    if (name.isEmpty) {
      setState(() => _error = '名字不能为空');
      return;
    }
    final styleChanged = _style != (a.celestialStyle ?? '');
    if (name == a.name && _role == a.role && !styleChanged) {
      Navigator.pop(context, false);
      return;
    }
    await _run(
      (state) => state.updateHostAgent(
        a.id,
        name: name == a.name ? null : name,
        role: _role == a.role ? null : _role,
        celestialStyle: styleChanged ? _style : null,
      ),
    );
  }

  Future<void> _delete() async {
    final ok = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('删掉员工「${widget.agent.name}」？'),
        content: const Text('他的对话、任务、记忆和工作目录都留在主机上。他在干活、或定时任务还指着他时主机会拒绝。'),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('取消')),
          FilledButton(
            key: const ValueKey('confirm-delete-agent'),
            onPressed: () => Navigator.pop(context, true),
            child: const Text('删掉'),
          ),
        ],
      ),
    );
    if (ok != true || !mounted) return;
    await _run((state) => state.deleteHostAgent(widget.agent.id));
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final theme = Theme.of(context);
    return AlertDialog(
      title: Text('编辑员工 ${widget.agent.id}'),
      content: SizedBox(
        width: 420,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            TextField(
              key: const ValueKey('agent-name'),
              controller: _name,
              decoration: const InputDecoration(labelText: '名字', border: OutlineInputBorder(), isDense: true),
            ),
            const SizedBox(height: 10),
            DropdownButtonFormField<String>(
              key: const ValueKey('agent-role'),
              initialValue: state.roles.any((r) => r.id == _role) ? _role : null,
              isExpanded: true,
              decoration: const InputDecoration(labelText: '岗位', border: OutlineInputBorder(), isDense: true),
              items: [
                for (final r in state.roles) DropdownMenuItem(value: r.id, child: Text('${r.displayName}（${r.id}）')),
              ],
              onChanged: (v) => setState(() => _role = v ?? _role),
            ),
            const SizedBox(height: 10),
            DropdownButtonFormField<String>(
              key: const ValueKey('agent-style'),
              initialValue: _style,
              isExpanded: true,
              decoration: const InputDecoration(labelText: '星图形态', border: OutlineInputBorder(), isDense: true),
              items: [
                const DropdownMenuItem(value: '', child: Text('自动（按 id 挑）')),
                for (final c in selectableBotCelestials)
                  DropdownMenuItem(value: celestialStyleOf(c), child: Text(c.name)),
              ],
              onChanged: (v) => setState(() => _style = v ?? ''),
            ),
            const SizedBox(height: 10),
            Text(
              '改名字或岗位会重启他，他在干活时主机会拒绝；只改形态不用。改动存在主机上，不写回 ip.yaml。',
              style: theme.textTheme.bodySmall?.copyWith(color: theme.hintColor),
            ),
            if (_error != null) ...[
              const SizedBox(height: 8),
              Text(_error!, style: TextStyle(color: theme.colorScheme.error)),
            ],
          ],
        ),
      ),
      actions: [
        TextButton(
          key: const ValueKey('agent-delete'),
          onPressed: _saving ? null : _delete,
          style: TextButton.styleFrom(foregroundColor: theme.colorScheme.error),
          child: const Text('删掉这位员工'),
        ),
        TextButton(onPressed: _saving ? null : () => Navigator.pop(context), child: const Text('取消')),
        FilledButton(
          key: const ValueKey('agent-save'),
          onPressed: _saving ? null : _save,
          child: _saving
              ? const SizedBox.square(dimension: 16, child: CircularProgressIndicator(strokeWidth: 2))
              : const Text('保存'),
        ),
      ],
    );
  }
}
