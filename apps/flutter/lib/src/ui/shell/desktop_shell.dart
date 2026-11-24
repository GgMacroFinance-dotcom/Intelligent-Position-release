// 桌面外壳（设计重构，2026-11-24 维护者定）：宽窗口里是字符画的文艺复兴天穹（cosmos.dart，将来就是星图），
// 左下角两个气泡：“我”和连接状态。
// 待办、星图、对话、任务、岗位这些功能以后用别的方式展示；在那之前，窗口变窄（< Sizes.shellMin）时
// 还是原来的导航，功能都在（features.txt）。点“我”或连接状态打开设置弹窗。

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../protocol/client.dart';
import '../common.dart';
import 'cosmos.dart';
import 'settings.dart';
import 'tokens.dart';

class DesktopShell extends StatefulWidget {
  const DesktopShell({super.key, required this.onOpenThread});

  /// 点了系统通知、或者设置里“管理”各页点了“看对话”：记下那个对话，窗口变窄时直接打开它。
  final void Function(String threadId) onOpenThread;

  @override
  State<DesktopShell> createState() => _DesktopShellState();
}

class _DesktopShellState extends State<DesktopShell> {
  void _settings() => showSettings(context);

  void _handlePendingOpen() {
    final state = AppScope.read(context);
    if (state.pendingOpen == null) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      final open = state.takePendingOpen();
      if (open == null) return;
      Navigator.of(context).popUntil((r) => r.isFirst);
      final (threadId,) = open;
      if (threadId != null) widget.onOpenThread(threadId);
    });
  }

  @override
  Widget build(BuildContext context) {
    AppScope.of(context);
    _handlePendingOpen();
    return CallbackShortcuts(
      bindings: {const SingleActivator(LogicalKeyboardKey.comma, meta: true): _settings},
      child: Focus(
        autofocus: true,
        child: Scaffold(
          backgroundColor: Pigment.lapisDeep,
          body: Stack(
            children: [
              const Positioned.fill(child: CosmosBackdrop()),
              Align(
                alignment: Alignment.bottomLeft,
                child: Padding(
                  padding: const EdgeInsets.fromLTRB(12, 0, 12, 14),
                  child: SizedBox(width: Sizes.sidebar - 24, child: _Footer(onSettings: _settings)),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// 左下角：“我”（打开设置）和连接状态的气泡。
class _Footer extends StatelessWidget {
  const _Footer({required this.onSettings});

  final VoidCallback onSettings;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final (String label, Color color) = switch (state.status) {
      _ when state.rejected != null => ('被主机拒绝', s.danger),
      ConnStatus.open => ('已连接', s.success),
      ConnStatus.connecting => ('正在连接', s.warning),
      ConnStatus.closed => ('断开了', s.danger),
    };
    final host = state.host?.address.origin.host ?? '';
    return Row(
      children: [
        Tooltip(
          message: '我',
          child: Material(
            color: s.composer,
            shape: CircleBorder(side: BorderSide(color: s.edge, width: 0.5)),
            child: InkWell(
              customBorder: const CircleBorder(),
              onTap: onSettings,
              child: SizedBox.square(dimension: 36, child: Icon(Icons.person_outline, size: 18, color: s.ink)),
            ),
          ),
        ),
        const SizedBox(width: 8),
        Expanded(
          child: Material(
            color: s.composer,
            shape: StadiumBorder(side: BorderSide(color: s.edge, width: 0.5)),
            child: InkWell(
              customBorder: const StadiumBorder(),
              onTap: onSettings,
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 9),
                child: Row(
                  children: [
                    Container(
                      width: 7,
                      height: 7,
                      decoration: BoxDecoration(color: color, shape: BoxShape.circle),
                    ),
                    const SizedBox(width: 8),
                    Expanded(
                      child: Text(
                        host.isEmpty ? label : '$label · $host',
                        overflow: TextOverflow.ellipsis,
                        style: TextStyle(color: s.ink, fontSize: 13, fontWeight: FontWeight.w500),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ],
    );
  }
}
