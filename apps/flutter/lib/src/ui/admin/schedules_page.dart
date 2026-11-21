// “定时任务”页（F-18，只给“管理”权限）：看定时任务、下次和上次运行，立即运行一次。
// 和网页控制台任务页底部的“定时任务”表一样（rust/web/src/app.js 的 scheduleTable）。定时任务在 ip.yaml 里配，这里不改。

import 'dart:async';

import 'package:flutter/material.dart';

import '../../protocol/client.dart';
import '../../protocol/models.dart';
import '../common.dart';

class SchedulesPage extends StatefulWidget {
  const SchedulesPage({super.key});

  @override
  State<SchedulesPage> createState() => _SchedulesPageState();
}

class _SchedulesPageState extends State<SchedulesPage> {
  List<Schedule>? _schedules;
  String? _error;
  final _running = <String>{};

  @override
  void initState() {
    super.initState();
    unawaited(_load());
  }

  Future<void> _load() async {
    try {
      final list = await AppScope.read(context).client.listSchedules();
      if (!mounted) return;
      setState(() {
        _schedules = list;
        _error = null;
      });
    } on RequestError catch (e) {
      if (mounted) setState(() => _error = '读不到定时任务：$e');
    }
  }

  Future<void> _runNow(Schedule s) async {
    final state = AppScope.read(context);
    final messenger = ScaffoldMessenger.of(context);
    setState(() => _running.add(s.id));
    try {
      final r = await state.client.runSchedule(s.id);
      await _load();
      messenger.showSnackBar(
        SnackBar(
          content: Text('「${s.description.isEmpty ? s.id : s.description}」已发出'),
          action: SnackBarAction(label: '打开对话', onPressed: () => state.openThreadFromAnywhere(r.message.threadId)),
        ),
      );
    } on RequestError catch (e) {
      if (mounted) showError(context, '运行失败：$e');
    } finally {
      if (mounted) setState(() => _running.remove(s.id));
    }
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final theme = Theme.of(context);
    final list = _schedules;
    if (_error != null) return EmptyHint(_error!);
    if (list == null) return const Center(child: CircularProgressIndicator());
    if (list.isEmpty) return const EmptyHint('还没有定时任务。在主机的 ip.yaml 里 schedules 下配置。');
    return RefreshIndicator(
      onRefresh: _load,
      child: ListView.separated(
        padding: const EdgeInsets.symmetric(vertical: 8),
        itemCount: list.length + 1,
        separatorBuilder: (_, _) => const Divider(height: 1),
        itemBuilder: (context, i) {
          if (i == list.length) {
            return Padding(
              padding: const EdgeInsets.all(16),
              child: Text('定时任务在主机的 ip.yaml 里配置，这里只能看和立即运行一次。立即运行不影响原定的下次运行。', style: theme.textTheme.bodySmall),
            );
          }
          final s = list[i];
          final to = s.to;
          final target = to == null
              ? '按路由规则'
              : [
                  state.roleName(to.role),
                  if (to.agent != null) state.agentName(to.agent!),
                  if (to.escalate) '升级模型',
                ].join(' · ');
          final last = s.lastRun;
          return Padding(
            key: ValueKey(s.id),
            padding: const EdgeInsets.fromLTRB(16, 12, 8, 12),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    spacing: 2,
                    children: [
                      Text(s.description.isEmpty ? s.id : s.description, style: theme.textTheme.titleSmall),
                      Text(s.text, maxLines: 3, overflow: TextOverflow.ellipsis, style: theme.textTheme.bodySmall),
                      Text('${s.cron}（UTC${s.utcOffset}）· 发给 $target', style: theme.textTheme.bodySmall),
                      Text(
                        !s.enabled
                            ? '已停用'
                            : s.nextRunMs != null
                            ? '下次 ${formatTime(s.nextRunMs!)}'
                            : '不会再运行',
                        style: theme.textTheme.bodySmall,
                      ),
                      if (last == null)
                        Text('还没运行过', style: theme.textTheme.bodySmall)
                      else
                        Wrap(
                          crossAxisAlignment: WrapCrossAlignment.center,
                          children: [
                            Text(
                              '上次 ${formatTime(last.atMs)} ${last.ok ? '成功' : '失败：${last.detail}'}',
                              style: theme.textTheme.bodySmall?.copyWith(
                                color: last.ok ? null : theme.colorScheme.error,
                              ),
                            ),
                            if (last.threadId != null)
                              TextButton(
                                onPressed: () => state.openThreadFromAnywhere(last.threadId!),
                                child: const Text('对话'),
                              ),
                          ],
                        ),
                    ],
                  ),
                ),
                const SizedBox(width: 8),
                OutlinedButton(
                  onPressed: _running.contains(s.id) || !state.connected ? null : () => _runNow(s),
                  child: Text(_running.contains(s.id) ? '运行中…' : '立即运行'),
                ),
              ],
            ),
          );
        },
      ),
    );
  }
}
