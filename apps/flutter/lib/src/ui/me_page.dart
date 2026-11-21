// “我”页：连的哪台主机、这台设备叫什么、什么权限，断开并忘掉主机。

import 'package:flutter/material.dart';

import '../protocol/client.dart';
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
