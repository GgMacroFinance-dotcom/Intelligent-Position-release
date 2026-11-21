// “我”页：连的哪台主机、这台设备叫什么、什么权限，这台设备的通知设置，断开并忘掉主机。

import 'package:flutter/material.dart';

import '../app_state.dart';
import '../protocol/client.dart';
import '../protocol/models.dart';
import 'admin/devices_page.dart';
import 'admin/memory_page.dart';
import 'admin/rules_page.dart';
import 'admin/schedules_page.dart';
import 'common.dart';

class MePage extends StatelessWidget {
  const MePage({super.key});

  Future<void> _forget(BuildContext context) async {
    final ok = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('断开并忘掉这台主机？'),
        content: const Text('这台设备的令牌会从本机删掉，要再连得重新配对。主机那边的设备记录还在，要在控制台“设备”页吊销。'),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('取消')),
          FilledButton(onPressed: () => Navigator.pop(context, true), child: const Text('断开并忘掉')),
        ],
      ),
    );
    if (ok == true && context.mounted) await AppScope.read(context).forgetHost();
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final host = state.host;
    final me = state.me;
    final status = switch (state.status) {
      ConnStatus.open => '已连接',
      ConnStatus.connecting => '正在连接…',
      ConnStatus.closed => state.rejected != null ? '被主机拒绝' : '断开了，正在重连',
    };
    return ListView(
      children: [
        ListTile(title: const Text('主机'), subtitle: SelectableText(host?.address.text ?? '—')),
        ListTile(title: const Text('连接'), subtitle: Text(status)),
        ListTile(title: const Text('设备名'), subtitle: Text(me?.device?.name ?? host?.deviceName ?? '—')),
        ListTile(
          title: const Text('权限'),
          subtitle: Text(
            me == null
                ? '—'
                : '${me.scope.label}${me.device == null ? '（共用令牌）' : ''}。路由规则、定时任务、记忆和设备管理要“管理”权限，在控制台“设备”页给',
          ),
        ),
        ListTile(
          title: const Text('协议版本'),
          subtitle: Text('App $kProtocol，主机 ${state.serverProtocol ?? '—'}${state.outdated ? '（不一致，请升级）' : ''}'),
        ),
        if (state.isAdmin) ...[const Divider(), const AdminSection()],
        const Divider(),
        const NotifySection(),
        const Divider(),
        Padding(
          padding: const EdgeInsets.all(16),
          child: OutlinedButton.icon(
            onPressed: host == null ? null : () => _forget(context),
            icon: const Icon(Icons.link_off),
            label: const Text('断开并忘掉这台主机'),
          ),
        ),
      ],
    );
  }
}

/// 明早 8 点（本地时间）。
int nextMorning(DateTime now) {
  var d = DateTime(now.year, now.month, now.day, 8);
  if (!d.isAfter(now)) d = d.add(const Duration(days: 1));
  return d.millisecondsSinceEpoch;
}

/// 这台设备的通知设置（F-16），和网页控制台“设备与通知”页的一样，存在主机上（m2-spec D-32）。
class NotifySection extends StatelessWidget {
  const NotifySection({super.key});

  Future<void> _save(BuildContext context, AppState state, NotifySettings next) async {
    try {
      await state.saveNotify(next);
    } on RequestError catch (e) {
      if (context.mounted) showError(context, '保存通知设置失败：$e');
    }
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = state.notify;
    final theme = Theme.of(context);
    final header = Padding(
      padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
      child: Text(state.me?.device != null ? '这台设备的通知' : '通知', style: theme.textTheme.titleMedium),
    );
    if (s == null) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          header,
          ListTile(subtitle: Text(state.connected ? '主机没有通知设置（版本太旧），请升级主机。' : '连上主机后才能改。')),
        ],
      );
    }
    void save(NotifySettings next) => _save(context, state, next);
    final now = DateTime.now();
    final until = s.muteUntilMs;
    final muted = until != null && until > now.millisecondsSinceEpoch;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        header,
        if (state.notificationsAllowed == false)
          ListTile(
            leading: Icon(Icons.notifications_off_outlined, color: theme.colorScheme.error),
            title: const Text('系统没允许这个 App 发通知'),
            subtitle: const Text('在系统设置里给 intelligent-position 打开通知，或者点右边再问一次。'),
            trailing: TextButton(onPressed: state.askNotificationPermission, child: const Text('再问一次')),
          ),
        ListTile(
          title: const Text('通知哪些事'),
          trailing: DropdownButton<NotifyLevel>(
            value: s.level,
            onChanged: (v) => v == null ? null : save(s.copyWith(level: v)),
            items: [for (final l in NotifyLevel.values) DropdownMenuItem(value: l, child: Text(l.label))],
          ),
        ),
        SwitchListTile(
          title: const Text('通知里显示内容预览'),
          subtitle: const Text('密钥会打码'),
          value: s.preview,
          onChanged: (v) => save(s.copyWith(preview: v)),
        ),
        ListTile(
          title: Text(muted ? '静音到 ${formatTime(until)}' : '静音'),
          subtitle: Wrap(
            spacing: 8,
            children: muted
                ? [TextButton(onPressed: () => save(s.copyWith(clearMute: true)), child: const Text('取消静音'))]
                : [
                    TextButton(
                      onPressed: () =>
                          save(s.copyWith(muteUntilMs: now.add(const Duration(hours: 1)).millisecondsSinceEpoch)),
                      child: const Text('静音 1 小时'),
                    ),
                    TextButton(
                      onPressed: () => save(s.copyWith(muteUntilMs: nextMorning(now))),
                      child: const Text('静音到明早 8 点'),
                    ),
                  ],
          ),
        ),
        if (state.roles.isNotEmpty) ...[
          const Padding(padding: EdgeInsets.fromLTRB(16, 8, 16, 0), child: Text('这些岗位的事要通知：')),
          for (final r in state.roles)
            CheckboxListTile(
              title: Text(r.name ?? r.id),
              value: !s.mutedRoles.contains(r.id),
              onChanged: (on) => save(
                s.copyWith(
                  mutedRoles: on == true ? s.mutedRoles.where((id) => id != r.id).toList() : [...s.mutedRoles, r.id],
                ),
              ),
            ),
        ],
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 4, 16, 8),
          child: Text(
            '要我处理的：审批卡、员工提问、一轮失败、任务受阻。“全部”另外包括员工回复和任务完成。'
            'App 在前台时不弹；手机上 App 被系统关掉、锁屏以后暂时收不到（要等推送中转，见 F-21）。'
            '${state.me?.device == null ? '用共用令牌连上的共用这一份设置。' : ''}',
            style: theme.textTheme.bodySmall,
          ),
        ),
      ],
    );
  }
}

/// “管理”权限才有的几页（F-18），从“我”页进去。日常权限的设备看不到这一节，主机那边也会拒绝这些请求。
class AdminSection extends StatelessWidget {
  const AdminSection({super.key});

  void _push(BuildContext context, String title, Widget page) => Navigator.of(context).push(
    MaterialPageRoute<void>(
      builder: (_) => Scaffold(
        appBar: AppBar(title: Text(title)),
        body: page,
      ),
    ),
  );

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
          child: Text('管理', style: theme.textTheme.titleMedium),
        ),
        ListTile(
          leading: const Icon(Icons.devices_outlined),
          title: const Text('设备'),
          subtitle: const Text('添加设备、改权限、吊销'),
          trailing: const Icon(Icons.chevron_right),
          onTap: () => _push(context, '设备', const DevicesPage()),
        ),
        ListTile(
          leading: const Icon(Icons.schedule_outlined),
          title: const Text('定时任务'),
          subtitle: const Text('看下次和上次运行，立即运行一次'),
          trailing: const Icon(Icons.chevron_right),
          onTap: () => _push(context, '定时任务', const SchedulesPage()),
        ),
        ListTile(
          leading: const Icon(Icons.psychology_outlined),
          title: const Text('记忆'),
          subtitle: const Text('决策者笔记、岗位记忆摘要、更正、各对话的记忆'),
          trailing: const Icon(Icons.chevron_right),
          onTap: () => _push(context, '记忆', const MemoryPage()),
        ),
        ListTile(
          leading: const Icon(Icons.alt_route),
          title: const Text('路由规则'),
          subtitle: const Text('试一试、规则草案、编辑规则'),
          trailing: const Icon(Icons.chevron_right),
          onTap: () => _push(context, '路由规则', const RulesPage()),
        ),
      ],
    );
  }
}
