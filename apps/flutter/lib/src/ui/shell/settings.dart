// 设置弹窗（设计重构第一步，照 dusk bot 的设置）：左边分类，右边内容。
// 内容沿用原来“我”页和“管理”各页的控件，只是换了地方（features.txt Me-*、N-*、A-* 都还在）。
// “管理”下的四页只给“管理”权限的设备看。

import 'package:flutter/material.dart';

import '../../protocol/client.dart';
import '../admin/devices_page.dart';
import '../admin/memory_page.dart';
import '../admin/rules_page.dart';
import '../admin/schedules_page.dart';
import '../common.dart';
import '../me_page.dart';
import 'tokens.dart';

enum SettingsTab { general, notify, appearance, devices, schedules, memory, rules, about }

/// 设置里的外观选择（浅色、深色、跟随系统），存在内存里；重启后跟随系统。
final themeChoice = ValueNotifier<ThemeMode>(ThemeMode.system);

Future<void> showSettings(BuildContext context, {SettingsTab tab = SettingsTab.general}) {
  return showGeneralDialog<void>(
    context: context,
    barrierDismissible: true,
    barrierLabel: '关闭设置',
    barrierColor: Shell.of(context).scrim,
    transitionDuration: Motion.dialog,
    pageBuilder: (_, _, _) => _SettingsDialog(initial: tab),
    // 统一的弹窗动效：上浮 6 像素、从 0.985 放大，同时淡入。
    transitionBuilder: (context, anim, _, child) {
      final t = CurvedAnimation(parent: anim, curve: Motion.curve);
      return FadeTransition(
        opacity: t,
        child: AnimatedBuilder(
          animation: t,
          builder: (_, c) => Transform.translate(
            offset: Offset(0, 6 * (1 - t.value)),
            child: Transform.scale(scale: 0.985 + 0.015 * t.value, child: c),
          ),
          child: child,
        ),
      );
    },
  );
}

class _SettingsDialog extends StatefulWidget {
  const _SettingsDialog({required this.initial});

  final SettingsTab initial;

  @override
  State<_SettingsDialog> createState() => _SettingsDialogState();
}

class _SettingsDialogState extends State<_SettingsDialog> {
  late SettingsTab _tab = widget.initial;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final size = MediaQuery.sizeOf(context);
    final admin = state.isAdmin;
    var tab = _tab;
    // 权限降成“日常”后，管理的几页不再显示。
    if (!admin && const {SettingsTab.devices, SettingsTab.schedules, SettingsTab.memory, SettingsTab.rules}.contains(tab)) {
      tab = SettingsTab.general;
    }
    Widget item(SettingsTab t, IconData icon, String label) => _NavItem(
      icon: icon,
      label: label,
      selected: tab == t,
      onTap: () => setState(() => _tab = t),
    );
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: ConstrainedBox(
          constraints: BoxConstraints(maxWidth: 1012, maxHeight: (size.height - 48).clamp(400, double.infinity)),
          child: Material(
            color: s.menu,
            clipBehavior: Clip.antiAlias,
            shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(Radii.x3)),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                SizedBox(
                  width: 220,
                  child: Material(
                    color: s.panel,
                    child: ListView(
                      padding: const EdgeInsets.fromLTRB(10, 18, 10, 18),
                      children: [
                        item(SettingsTab.general, Icons.tune, '通用'),
                        item(SettingsTab.notify, Icons.notifications_none, '通知'),
                        item(SettingsTab.appearance, Icons.palette_outlined, '外观'),
                        if (admin) ...[
                          Padding(
                            padding: const EdgeInsets.fromLTRB(10, 16, 10, 6),
                            child: Text('管理', style: TextStyle(color: s.inkTertiary, fontSize: 12.5)),
                          ),
                          item(SettingsTab.devices, Icons.devices_outlined, '设备'),
                          item(SettingsTab.schedules, Icons.schedule_outlined, '定时任务'),
                          item(SettingsTab.memory, Icons.psychology_outlined, '记忆'),
                          item(SettingsTab.rules, Icons.alt_route, '路由规则'),
                        ],
                        const SizedBox(height: 16),
                        item(SettingsTab.about, Icons.info_outline, '关于'),
                      ],
                    ),
                  ),
                ),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Padding(
                        padding: const EdgeInsets.fromLTRB(28, 18, 14, 6),
                        child: Row(
                          children: [
                            Expanded(
                              child: Text(
                                _title(tab),
                                style: TextStyle(color: s.ink, fontSize: 17, fontWeight: FontWeight.w600),
                              ),
                            ),
                            IconButton(
                              tooltip: '关闭',
                              onPressed: () => Navigator.of(context).pop(),
                              icon: Icon(Icons.close, size: 18, color: s.inkSecondary),
                            ),
                          ],
                        ),
                      ),
                      Expanded(
                        child: AnimatedSwitcher(
                          duration: Motion.fast,
                          child: KeyedSubtree(key: ValueKey(tab), child: _body(tab)),
                        ),
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  String _title(SettingsTab t) => switch (t) {
    SettingsTab.general => '通用',
    SettingsTab.notify => '通知',
    SettingsTab.appearance => '外观',
    SettingsTab.devices => '设备',
    SettingsTab.schedules => '定时任务',
    SettingsTab.memory => '记忆',
    SettingsTab.rules => '路由规则',
    SettingsTab.about => '关于',
  };

  Widget _body(SettingsTab t) => switch (t) {
    SettingsTab.general => const _GeneralPane(),
    SettingsTab.notify => ListView(padding: const EdgeInsets.symmetric(horizontal: 12), children: const [NotifySection()]),
    SettingsTab.appearance => const _AppearancePane(),
    SettingsTab.devices => const DevicesPage(),
    SettingsTab.schedules => const SchedulesPage(),
    SettingsTab.memory => const MemoryPage(),
    SettingsTab.rules => const RulesPage(),
    SettingsTab.about => const _AboutPane(),
  };
}

class _NavItem extends StatelessWidget {
  const _NavItem({required this.icon, required this.label, required this.selected, required this.onTap});

  final IconData icon;
  final String label;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    return Padding(
      padding: const EdgeInsets.only(bottom: 2),
      child: Material(
        color: selected ? s.raised : Colors.transparent,
        borderRadius: BorderRadius.circular(10),
        child: InkWell(
          borderRadius: BorderRadius.circular(10),
          onTap: onTap,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
            child: Row(
              children: [
                Icon(icon, size: 18, color: selected ? s.ink : s.inkSecondary),
                const SizedBox(width: 10),
                Text(
                  label,
                  style: TextStyle(
                    color: selected ? s.ink : s.inkSecondary,
                    fontSize: 14,
                    fontWeight: selected ? FontWeight.w600 : FontWeight.w500,
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// 设置里一组：灰色小标题，下面几行。
class SettingsGroup extends StatelessWidget {
  const SettingsGroup({super.key, required this.title, required this.children});

  final String title;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(28, 14, 28, 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.only(bottom: 4),
            child: Text(title, style: TextStyle(color: s.inkTertiary, fontSize: 12.5)),
          ),
          ...children,
        ],
      ),
    );
  }
}

/// 设置里的一行：左边名字（和说明），右边值或控件。
class SettingsRow extends StatelessWidget {
  const SettingsRow({super.key, required this.label, this.detail, this.trailing, this.onTap});

  final String label;
  final String? detail;
  final Widget? trailing;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    final row = Padding(
      padding: const EdgeInsets.symmetric(vertical: 11),
      child: Row(
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(label, style: TextStyle(color: s.ink, fontSize: 14)),
                if (detail != null) ...[
                  const SizedBox(height: 2),
                  Text(detail!, style: TextStyle(color: s.inkSecondary, fontSize: 12.5)),
                ],
              ],
            ),
          ),
          if (trailing != null) ...[const SizedBox(width: 16), trailing!],
        ],
      ),
    );
    return onTap == null ? row : InkWell(onTap: onTap, borderRadius: BorderRadius.circular(8), child: row);
  }
}

/// 右边一个灰底的值（像 dusk 的下拉按钮）。
class ValuePill extends StatelessWidget {
  const ValuePill(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
      decoration: BoxDecoration(color: s.card, borderRadius: BorderRadius.circular(999)),
      child: SelectableText(text, style: TextStyle(color: s.ink, fontSize: 13)),
    );
  }
}

class _GeneralPane extends StatelessWidget {
  const _GeneralPane();

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
    if (ok == true && context.mounted) {
      Navigator.of(context).pop();
      await AppScope.read(context).forgetHost();
    }
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final host = state.host;
    final me = state.me;
    final status = switch (state.status) {
      ConnStatus.open => '已连接',
      ConnStatus.connecting => '正在连接…',
      ConnStatus.closed => state.rejected != null ? '被主机拒绝' : '断开了，正在重连',
    };
    return ListView(
      children: [
        SettingsGroup(
          title: '主机',
          children: [
            SettingsRow(label: '地址', trailing: ValuePill(host?.address.text ?? '—')),
            SettingsRow(label: '连接', trailing: ValuePill(status)),
            SettingsRow(
              label: '协议版本',
              detail: state.outdated ? '不一致：请把 App 和主机升到同一版' : null,
              trailing: ValuePill('App $kProtocol · 主机 ${state.serverProtocol ?? '—'}'),
            ),
          ],
        ),
        SettingsGroup(
          title: '这台设备',
          children: [
            SettingsRow(label: '设备名', trailing: ValuePill(me?.device?.name ?? host?.deviceName ?? '—')),
            SettingsRow(
              label: '权限',
              detail: '路由规则、定时任务、记忆和设备管理要“管理”权限，在控制台“设备”页给',
              trailing: ValuePill(me == null ? '—' : '${me.scope.label}${me.device == null ? '（共用令牌）' : ''}'),
            ),
          ],
        ),
        SettingsGroup(
          title: '断开',
          children: [
            SettingsRow(
              label: '断开并忘掉这台主机',
              detail: '本机的令牌会删掉，要再连得重新配对',
              trailing: OutlinedButton(
                style: OutlinedButton.styleFrom(foregroundColor: s.danger),
                onPressed: host == null ? null : () => _forget(context),
                child: const Text('断开'),
              ),
            ),
          ],
        ),
      ],
    );
  }
}

class _AppearancePane extends StatelessWidget {
  const _AppearancePane();

  @override
  Widget build(BuildContext context) {
    return ValueListenableBuilder(
      valueListenable: themeChoice,
      builder: (context, mode, _) => ListView(
        children: [
          SettingsGroup(
            title: '主题',
            children: [
              SettingsRow(
                label: '浅色、深色',
                detail: '跟随系统时，系统切换深色模式后这里也跟着变',
                trailing: SegmentedButton<ThemeMode>(
                  showSelectedIcon: false,
                  segments: const [
                    ButtonSegment(value: ThemeMode.system, label: Text('跟随系统')),
                    ButtonSegment(value: ThemeMode.light, label: Text('浅色')),
                    ButtonSegment(value: ThemeMode.dark, label: Text('深色')),
                  ],
                  selected: {mode},
                  onSelectionChanged: (v) => themeChoice.value = v.first,
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }
}

class _AboutPane extends StatelessWidget {
  const _AboutPane();

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    return ListView(
      children: [
        SettingsGroup(
          title: 'intelligent-position',
          children: [
            Text(
              'AI 原生组织架构：岗位由数字员工承担，人只在关键节点拍板。这个 App 连你自己的主机（ip serve）。',
              style: TextStyle(color: s.inkSecondary, fontSize: 13.5, height: 1.5),
            ),
            const SizedBox(height: 12),
            const SettingsRow(label: '版本', trailing: ValuePill('0.8.0')),
            const SettingsRow(label: '许可', trailing: ValuePill('Apache-2.0')),
          ],
        ),
      ],
    );
  }
}
