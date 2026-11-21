// “路由规则”页（F-18，只给“管理”权限）：试一试一句话会分给谁、规则草案（采纳前先编辑、忽略）、
// 看规则、编辑整套规则（关键词条件、交给谁、顺序、语义匹配、兜底）。和网页控制台“路由”页一样
// （rust/web/src/app.js 的 renderRouter、ruleEditor）。

import 'dart:async';

import 'package:flutter/material.dart';

import '../../app_state.dart';
import '../../protocol/client.dart';
import '../../protocol/router.dart';
import '../common.dart';

/// 关键词用逗号、顿号或换行分开。
List<String> splitList(String v) => [
  for (final x in v.split(RegExp(r'[,，、\n]'))) ?(x.trim().isEmpty ? null : x.trim()),
];

String describeTarget(AppState state, RouteTarget to) =>
    '→ ${state.roleName(to.role)}${to.agent != null ? '（${state.agentName(to.agent!)}）' : ''}${to.escalate ? '，用升级模型' : ''}';

class RulesPage extends StatefulWidget {
  const RulesPage({super.key});

  @override
  State<RulesPage> createState() => _RulesPageState();
}

class _RulesPageState extends State<RulesPage> {
  RulesView? _view;
  List<RuleDraft> _drafts = [];
  String? _error;

  /// 正在编辑的副本；为空时是看的模式。
  RouterSpec? _edit;
  String? _acceptedDraft;
  String? _saveError;
  bool _saving = false;

  final _try = TextEditingController();
  RouteDecision? _decision;

  @override
  void initState() {
    super.initState();
    unawaited(_load());
  }

  @override
  void dispose() {
    _try.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    final c = AppScope.read(context).client;
    try {
      final (view, drafts) = await (c.listRules(), c.listRuleDrafts()).wait;
      if (!mounted) return;
      setState(() {
        _view = view;
        _drafts = drafts;
        _error = null;
      });
    } on ParallelWaitError<(RulesView?, List<RuleDraft>?), (AsyncError?, AsyncError?)> catch (e) {
      if (mounted) setState(() => _error = '读不到路由规则：${e.errors.$1?.error ?? e.errors.$2?.error}');
    }
  }

  Future<void> _tryRoute() async {
    final text = _try.text.trim();
    if (text.isEmpty) return;
    try {
      final d = await AppScope.read(context).client.route(text);
      if (mounted) setState(() => _decision = d);
    } on RequestError catch (e) {
      if (mounted) showError(context, '路由失败：$e');
    }
  }

  Future<void> _dismiss(RuleDraft d) async {
    try {
      final drafts = await AppScope.read(context).client.dismissRuleDraft(d.id);
      if (mounted) setState(() => _drafts = drafts);
    } on RequestError catch (e) {
      if (mounted) showError(context, '忽略失败：$e');
    }
  }

  void _startEdit(RuleDraft? draft) {
    final view = _view;
    if (view == null) return;
    final spec = view.spec.copy();
    if (draft != null) spec.rules.add(draft.rule.copy());
    setState(() {
      _edit = spec;
      _acceptedDraft = draft?.id;
      _saveError = null;
    });
  }

  Future<void> _save() async {
    final spec = _edit;
    if (spec == null) return;
    final state = AppScope.read(context);
    setState(() => _saving = true);
    try {
      final saved = await state.client.saveRules(spec, acceptedDraft: _acceptedDraft);
      final drafts = await state.client.listRuleDrafts();
      if (!mounted) return;
      setState(() {
        _view = RulesView(spec: saved, fromConsole: true);
        _drafts = drafts;
        _edit = null;
        _acceptedDraft = null;
      });
      // 有没有规则决定新对话默认要不要自动分派。
      state.hasRules = saved.rules.isNotEmpty;
    } on RequestError catch (e) {
      if (mounted) setState(() => _saveError = '没有保存：$e');
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final theme = Theme.of(context);
    final view = _view;
    if (_error != null) return EmptyHint(_error!);
    if (view == null) return const Center(child: CircularProgressIndicator());
    final edit = _edit;
    if (edit != null) {
      return RuleEditor(
        spec: edit,
        title: _acceptedDraft != null ? '采纳草案：改好后保存' : '编辑规则',
        error: _saveError,
        saving: _saving,
        onSave: _save,
        onCancel: () => setState(() {
          _edit = null;
          _acceptedDraft = null;
        }),
      );
    }
    final spec = view.spec;
    final small = theme.textTheme.bodySmall;
    Widget heading(String text, {Widget? trailing}) => Padding(
      padding: const EdgeInsets.fromLTRB(16, 16, 8, 4),
      child: Row(
        children: [
          Expanded(child: Text(text, style: theme.textTheme.titleMedium)),
          ?trailing,
        ],
      ),
    );
    final d = _decision;
    return ListView(
      padding: const EdgeInsets.only(bottom: 32),
      children: [
        heading('试一试'),
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 16),
          child: Row(
            children: [
              Expanded(
                child: TextField(
                  controller: _try,
                  decoration: const InputDecoration(hintText: '输入一句话，看看会分给谁'),
                  onSubmitted: (_) => _tryRoute(),
                ),
              ),
              const SizedBox(width: 8),
              FilledButton(onPressed: _tryRoute, child: const Text('路由')),
            ],
          ),
        ),
        if (d != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
            child: Text(
              d.routed
                  ? '→ ${state.roleName(d.role ?? '')}${d.agent != null ? '（${state.agentName(d.agent!)}）' : ''}${d.escalate ? '，用升级模型' : ''}。${d.reason}'
                  : '→ 交给人决定${d.suggested != null ? '（推荐 ${state.roleName(d.suggested!)}）' : ''}。${d.reason}',
              style: TextStyle(color: d.routed ? null : theme.colorScheme.tertiary),
            ),
          ),
        if (_drafts.isNotEmpty) ...[
          heading('规则草案（${_drafts.length}）'),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 16),
            child: Text('你在审批卡片上给消息选岗位时，系统按消息里的关键词起草规则。草案不会自动生效，采纳前可以修改。', style: small),
          ),
          for (final dr in _drafts)
            Card(
              key: ValueKey('draft-${dr.id}'),
              margin: const EdgeInsets.fromLTRB(16, 8, 16, 0),
              child: Padding(
                padding: const EdgeInsets.all(12),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  spacing: 4,
                  children: [
                    Text(dr.rule.id, style: theme.textTheme.titleSmall),
                    Text(dr.source, style: small),
                    Text(dr.rule.when.describe()),
                    Text(describeTarget(state, dr.rule.to)),
                    Row(
                      mainAxisAlignment: MainAxisAlignment.end,
                      children: [
                        TextButton(onPressed: () => _dismiss(dr), child: const Text('忽略')),
                        FilledButton(onPressed: () => _startEdit(dr), child: const Text('采纳（先编辑）')),
                      ],
                    ),
                  ],
                ),
              ),
            ),
        ],
        heading(
          '规则（${spec.rules.length} 条，按顺序匹配，第一条命中的生效）',
          trailing: OutlinedButton(onPressed: () => _startEdit(null), child: const Text('编辑')),
        ),
        if (spec.rules.isEmpty) const ListTile(title: Text('还没有规则。')),
        for (final (i, r) in spec.rules.indexed)
          ListTile(
            key: ValueKey('rule-$i'),
            leading: CircleAvatar(radius: 14, child: Text('${i + 1}')),
            title: Text(r.description.isEmpty ? r.id : '${r.id}  ${r.description}'),
            subtitle: Text('${r.when.describe()}\n${describeTarget(state, r.to)}'),
            isThreeLine: true,
          ),
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            spacing: 6,
            children: [
              Text(
                spec.semantic != null
                    ? '规则都没命中时：先用 ${spec.semantic!.model} 按岗位职责做语义匹配，把握不低于 ${spec.semantic!.minConfidence} 就直接分派，否则把它的判断作为推荐交给兜底。'
                    : '没有开启语义匹配。',
              ),
              Text('兜底：${spec.fallbackRole == null ? '交给人决定' : '交给 ${state.roleName(spec.fallbackRole!)}'}'),
              Text(
                view.fromConsole ? '当前规则是在控制台或 App 保存的，重启后仍然生效（优先于 ip.yaml）。' : '当前规则来自 ip.yaml。在这里保存后，以这里保存的为准。',
                style: small,
              ),
            ],
          ),
        ),
      ],
    );
  }
}

/// 编辑整套规则。改的是传进来的副本，保存时整套交给主机校验。
class RuleEditor extends StatefulWidget {
  const RuleEditor({
    super.key,
    required this.spec,
    required this.title,
    required this.onSave,
    required this.onCancel,
    this.error,
    this.saving = false,
  });

  final RouterSpec spec;
  final String title;
  final VoidCallback onSave;
  final VoidCallback onCancel;
  final String? error;
  final bool saving;

  @override
  State<RuleEditor> createState() => _RuleEditorState();
}

class _RuleEditorState extends State<RuleEditor> {
  /// 每条规则一个本地的键，上移下移时输入框跟着规则走。
  final _keys = Expando<Key>();
  int _nextKey = 0;

  Key _keyOf(Rule r) => _keys[r] ??= ValueKey('edit-rule-${_nextKey++}');

  RouterSpec get spec => widget.spec;

  void _move(int i, int delta) {
    final j = i + delta;
    if (j < 0 || j >= spec.rules.length) return;
    setState(() {
      final r = spec.rules.removeAt(i);
      spec.rules.insert(j, r);
    });
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final theme = Theme.of(context);
    final roles = state.roles;
    final sem = spec.semantic;
    return ListView(
      padding: const EdgeInsets.only(bottom: 32),
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 16, 16, 4),
          child: Text(widget.title, style: theme.textTheme.titleMedium),
        ),
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 16),
          child: Text('关键词用逗号或顿号分开，英文不区分大小写。规则按顺序匹配，第一条命中的生效。', style: theme.textTheme.bodySmall),
        ),
        for (final (i, r) in spec.rules.indexed)
          _RuleForm(
            key: _keyOf(r),
            rule: r,
            index: i,
            count: spec.rules.length,
            onMove: (delta) => _move(i, delta),
            onDelete: () => setState(() => spec.rules.removeAt(i)),
            onChanged: () => setState(() {}),
          ),
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 12, 16, 0),
          child: Align(
            alignment: Alignment.centerLeft,
            child: OutlinedButton.icon(
              icon: const Icon(Icons.add),
              label: const Text('新规则'),
              onPressed: roles.isEmpty
                  ? null
                  : () => setState(
                      () => spec.rules.add(
                        Rule(
                          id: '规则${spec.rules.length + 1}',
                          to: RouteTarget(role: roles.first.id),
                        ),
                      ),
                    ),
            ),
          ),
        ),
        const Divider(height: 32),
        SwitchListTile(
          title: const Text('规则都没命中时做语义匹配'),
          value: sem != null,
          onChanged: (on) => setState(
            () => spec.semantic = on
                ? SemanticSpec(model: roles.isEmpty ? '' : roles.first.model, minConfidence: 0.7)
                : null,
          ),
        ),
        if (sem != null)
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 16),
            child: Row(
              children: [
                Expanded(
                  flex: 3,
                  child: TextFormField(
                    key: const ValueKey('semantic-model'),
                    initialValue: sem.model,
                    decoration: const InputDecoration(labelText: '模型', hintText: '端点/模型'),
                    onChanged: (v) => sem.model = v.trim(),
                  ),
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: TextFormField(
                    key: const ValueKey('semantic-confidence'),
                    initialValue: '${sem.minConfidence}',
                    keyboardType: const TextInputType.numberWithOptions(decimal: true),
                    decoration: const InputDecoration(labelText: '把握至少'),
                    onChanged: (v) {
                      final n = double.tryParse(v.trim());
                      if (n != null) sem.minConfidence = n;
                    },
                  ),
                ),
              ],
            ),
          ),
        ListTile(
          title: const Text('兜底'),
          trailing: DropdownButton<String?>(
            value: spec.fallbackRole,
            onChanged: (v) => setState(() => spec.fallbackRole = v),
            items: [
              const DropdownMenuItem<String?>(child: Text('交给人决定')),
              for (final r in roles) DropdownMenuItem<String?>(value: r.id, child: Text('交给 ${r.name ?? r.id}')),
            ],
          ),
        ),
        if (widget.error != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
            child: Text(widget.error!, style: TextStyle(color: theme.colorScheme.error)),
          ),
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 12, 16, 0),
          child: Row(
            mainAxisAlignment: MainAxisAlignment.end,
            spacing: 8,
            children: [
              TextButton(onPressed: widget.saving ? null : widget.onCancel, child: const Text('取消')),
              FilledButton(
                onPressed: widget.saving ? null : widget.onSave,
                child: Text(widget.saving ? '正在保存…' : '保存并生效'),
              ),
            ],
          ),
        ),
      ],
    );
  }
}

class _RuleForm extends StatelessWidget {
  const _RuleForm({
    super.key,
    required this.rule,
    required this.index,
    required this.count,
    required this.onMove,
    required this.onDelete,
    required this.onChanged,
  });

  final Rule rule;
  final int index;
  final int count;
  final void Function(int delta) onMove;
  final VoidCallback onDelete;
  final VoidCallback onChanged;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final r = rule;
    final w = r.when;
    Widget field(String label, String value, void Function(String) set, {String? hint}) => TextFormField(
      initialValue: value,
      decoration: InputDecoration(labelText: label, hintText: hint, isDense: true),
      onChanged: set,
    );
    final agents = state.agents.where((a) => a.role == r.to.role).toList();
    return Card(
      margin: const EdgeInsets.fromLTRB(16, 12, 16, 0),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(12, 4, 4, 12),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          spacing: 8,
          children: [
            Row(
              children: [
                Text('${index + 1}', style: Theme.of(context).textTheme.titleSmall),
                const Spacer(),
                IconButton(
                  tooltip: '上移',
                  onPressed: index == 0 ? null : () => onMove(-1),
                  icon: const Icon(Icons.arrow_upward),
                ),
                IconButton(
                  tooltip: '下移',
                  onPressed: index == count - 1 ? null : () => onMove(1),
                  icon: const Icon(Icons.arrow_downward),
                ),
                IconButton(tooltip: '删除规则', onPressed: onDelete, icon: const Icon(Icons.delete_outline)),
              ],
            ),
            Padding(
              padding: const EdgeInsets.only(right: 8),
              child: Column(
                spacing: 8,
                children: [
                  field('规则名', r.id, (v) => r.id = v.trim()),
                  field('说明（可选）', r.description, (v) => r.description = v),
                  field('含任一', w.any.join('、'), (v) => w.any = splitList(v)),
                  field('全部含', w.all.join('、'), (v) => w.all = splitList(v)),
                  field('不含', w.none.join('、'), (v) => w.none = splitList(v)),
                  field('正则', w.regex ?? '', (v) => w.regex = v.trim().isEmpty ? null : v),
                  field('发送人', w.from.join('、'), (v) => w.from = splitList(v), hint: '名字，或 slack:U012ABCDEF'),
                ],
              ),
            ),
            Wrap(
              crossAxisAlignment: WrapCrossAlignment.center,
              spacing: 12,
              children: [
                const Text('交给'),
                DropdownButton<String>(
                  value: state.roles.any((x) => x.id == r.to.role) ? r.to.role : null,
                  hint: Text(r.to.role),
                  onChanged: (v) {
                    if (v == null) return;
                    r.to
                      ..role = v
                      ..agent = null;
                    onChanged();
                  },
                  items: [for (final ro in state.roles) DropdownMenuItem(value: ro.id, child: Text(ro.name ?? ro.id))],
                ),
                DropdownButton<String?>(
                  value: agents.any((a) => a.id == r.to.agent) ? r.to.agent : null,
                  onChanged: (v) {
                    r.to.agent = v;
                    onChanged();
                  },
                  items: [
                    const DropdownMenuItem<String?>(child: Text('最空闲的员工')),
                    for (final a in agents) DropdownMenuItem<String?>(value: a.id, child: Text(a.name)),
                  ],
                ),
                Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Checkbox(
                      value: r.to.escalate,
                      onChanged: (v) {
                        r.to.escalate = v ?? false;
                        onChanged();
                      },
                    ),
                    const Text('用升级模型'),
                  ],
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
