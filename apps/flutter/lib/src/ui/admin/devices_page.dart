// “设备”页（F-18，只给“管理”权限）：添加设备（出配对码和二维码）、改名、改权限、吊销。
// 和网页控制台“设备与通知”页的设备部分一样（rust/web/src/app.js 的 renderDevices）。

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../protocol/client.dart';
import '../../protocol/models.dart';
import '../common.dart';

class DevicesPage extends StatefulWidget {
  const DevicesPage({super.key, this.pollEvery = const Duration(seconds: 3)});

  /// 配对码有效期间多久看一次设备列表。
  final Duration pollEvery;

  @override
  State<DevicesPage> createState() => _DevicesPageState();
}

class _DevicesPageState extends State<DevicesPage> {
  List<DeviceInfo>? _devices;
  String? _error;
  NewPairing? _pairing;
  Timer? _poll;
  final _name = TextEditingController();

  @override
  void initState() {
    super.initState();
    unawaited(_load());
  }

  @override
  void dispose() {
    _poll?.cancel();
    _name.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    try {
      final list = await AppScope.read(context).client.listDevices();
      if (!mounted) return;
      setState(() {
        _devices = list;
        _error = null;
      });
    } on RequestError catch (e) {
      if (mounted) setState(() => _error = '读不到设备列表：$e');
    }
  }

  /// 改了列表的请求都回复改完的列表；失败时弹提示。
  Future<void> _apply(Future<List<DeviceInfo>> Function(IpClient c) f, String what) async {
    final state = AppScope.read(context);
    try {
      final list = await f(state.client);
      if (mounted) setState(() => _devices = list);
    } on RequestError catch (e) {
      if (mounted) showError(context, '$what失败：$e');
    }
  }

  Future<void> _createPairing() async {
    final name = _name.text.trim();
    if (name.isEmpty) return;
    final state = AppScope.read(context);
    try {
      // 扫码地址用主机的地址：控制台就在那里，另一台设备扫了能直接打开。
      final p = await state.client.createPairing(name, linkBase: state.host?.address.text);
      if (!mounted) return;
      setState(() => _pairing = p);
      _name.clear();
      _watchPairing();
    } on RequestError catch (e) {
      if (mounted) showError(context, '生成配对码失败：$e');
    }
  }

  /// 配对码有效期间定时看设备列表，那台设备配上了或者过期了就收起配对码。
  void _watchPairing() {
    _poll?.cancel();
    final before = {for (final d in _devices ?? const <DeviceInfo>[]) d.id};
    _poll = Timer.periodic(widget.pollEvery, (_) async {
      final p = _pairing;
      if (p == null || DateTime.now().millisecondsSinceEpoch > p.expiresMs) return _endPairing();
      await _load();
      if ((_devices ?? const []).any((d) => !before.contains(d.id))) _endPairing();
    });
  }

  void _endPairing() {
    _poll?.cancel();
    _poll = null;
    if (mounted) setState(() => _pairing = null);
    unawaited(_load());
  }

  Future<bool> _confirm(String title, String body, String ok) async =>
      await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          title: Text(title),
          content: Text(body),
          actions: [
            TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('取消')),
            FilledButton(onPressed: () => Navigator.pop(context, true), child: Text(ok)),
          ],
        ),
      ) ==
      true;

  Future<void> _setScope(DeviceInfo d, Scope scope) async {
    if (scope == d.scope) return;
    if (scope == Scope.admin && !await _confirm('给「${d.name}」“管理”权限？', '它就能改路由规则、定时任务、记忆，管理所有设备。', '给管理权限')) {
      return;
    }
    if (!mounted) return;
    final state = AppScope.read(context);
    await _apply((c) => c.setDeviceScope(d.id, scope), '改权限');
    // 改的是这台自己：权限跟着变，降成“日常”后这一页就不该再看了。
    if (d.id == state.me?.device?.id) {
      try {
        await state.refreshMe();
      } on RequestError {
        // 断线重连后会重新拉。
      }
      if (mounted && !state.isAdmin) Navigator.of(context).maybePop();
    }
  }

  Future<void> _rename(DeviceInfo d) async {
    final name = await showDialog<String>(
      context: context,
      builder: (_) => _RenameDialog(initial: d.name),
    );
    final n = name?.trim();
    if (n == null || n.isEmpty || n == d.name) return;
    await _apply((c) => c.renameDevice(d.id, n), '改名');
  }

  Future<void> _revoke(DeviceInfo d) async {
    if (!await _confirm('吊销「${d.name}」？', '它的连接会立即断开，要再用得重新配对。', '吊销')) return;
    await _apply((c) => c.revokeDevice(d.id), '吊销');
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final theme = Theme.of(context);
    final devices = _devices;
    final myId = state.me?.device?.id;
    return ListView(
      padding: const EdgeInsets.only(bottom: 24),
      children: [
        if (_pairing case final p?)
          _PairingCard(pairing: p, onDone: _endPairing)
        else
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 16, 16, 8),
            child: Row(
              children: [
                Expanded(
                  child: TextField(
                    controller: _name,
                    maxLength: 40,
                    decoration: const InputDecoration(labelText: '添加设备', hintText: '设备名，例如：张三的手机'),
                    onSubmitted: (_) => _createPairing(),
                  ),
                ),
                const SizedBox(width: 12),
                FilledButton(onPressed: _createPairing, child: const Text('生成配对码')),
              ],
            ),
          ),
        const Divider(),
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
          child: Text(devices == null ? '配对过的设备' : '配对过的设备（${devices.length}）', style: theme.textTheme.titleMedium),
        ),
        if (_error != null)
          ListTile(
            title: Text(_error!, style: TextStyle(color: theme.colorScheme.error)),
          )
        else if (devices == null)
          const Padding(
            padding: EdgeInsets.all(24),
            child: Center(child: CircularProgressIndicator()),
          )
        else if (devices.isEmpty)
          const ListTile(title: Text('还没有。'))
        else
          for (final d in devices)
            ListTile(
              key: ValueKey(d.id),
              title: Text(d.id == myId ? '${d.name}（这台）' : d.name),
              subtitle: Text(
                [
                  if (d.online) '在线' else if (d.lastSeenMs != null) '最后在线 ${formatTime(d.lastSeenMs!)}',
                  if (d.pairedAtMs != null) '${formatTime(d.pairedAtMs!)} 配对',
                ].join(' · '),
              ),
              leading: Icon(
                d.online ? Icons.smartphone : Icons.smartphone_outlined,
                color: d.online ? theme.colorScheme.primary : null,
              ),
              trailing: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  DropdownButton<Scope>(
                    value: d.scope,
                    onChanged: (s) => s == null ? null : _setScope(d, s),
                    items: [for (final s in Scope.values) DropdownMenuItem(value: s, child: Text(s.label))],
                  ),
                  PopupMenuButton<String>(
                    tooltip: '更多',
                    onSelected: (v) => v == 'rename' ? _rename(d) : _revoke(d),
                    itemBuilder: (_) => const [
                      PopupMenuItem(value: 'rename', child: Text('改名')),
                      PopupMenuItem(value: 'revoke', child: Text('吊销')),
                    ],
                  ),
                ],
              ),
            ),
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
          child: Text(
            '新配对的设备是“日常”权限：对话、审批、处理任务、看报告。“管理”另外能改路由规则、定时任务、记忆，管理设备。'
            '手机要能访问这台主机：用公网 HTTPS 或 Tailscale，见部署文档。',
            style: theme.textTheme.bodySmall,
          ),
        ),
      ],
    );
  }
}

class _PairingCard extends StatelessWidget {
  const _PairingCard({required this.pairing, required this.onDone});

  final NewPairing pairing;
  final VoidCallback onDone;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final until = TimeOfDay.fromDateTime(DateTime.fromMillisecondsSinceEpoch(pairing.expiresMs));
    final link = pairing.link;
    final qr = pairing.qr;
    return Card(
      margin: const EdgeInsets.all(16),
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('给「${pairing.name}」配对', style: theme.textTheme.titleMedium),
            const SizedBox(height: 8),
            SelectableText(
              pairing.code,
              style: theme.textTheme.displaySmall?.copyWith(letterSpacing: 6, fontFeatures: const []),
            ),
            const SizedBox(height: 8),
            Text(
              '${until.format(context)} 前有效，只能用一次。在那台设备上打开 App 或控制台，填主机地址和这 6 位数字；或者扫下面的二维码。',
              style: theme.textTheme.bodySmall,
            ),
            if (qr != null && qr.isNotEmpty) ...[const SizedBox(height: 12), Center(child: QrView(rows: qr))],
            if (link != null) ...[
              const SizedBox(height: 8),
              Row(
                children: [
                  Expanded(child: SelectableText(link, style: theme.textTheme.bodySmall)),
                  IconButton(
                    tooltip: '复制配对链接',
                    icon: const Icon(Icons.copy),
                    onPressed: () => Clipboard.setData(ClipboardData(text: link)),
                  ),
                ],
              ),
            ],
            const SizedBox(height: 8),
            Align(
              alignment: Alignment.centerRight,
              child: TextButton(onPressed: onDone, child: const Text('完成')),
            ),
          ],
        ),
      ),
    );
  }
}

/// 画主机给的二维码：黑块白底，四周留 4 格白边，深色主题下也能扫。
class QrView extends StatelessWidget {
  const QrView({super.key, required this.rows, this.cell = 5});

  final List<String> rows;
  final double cell;

  @override
  Widget build(BuildContext context) {
    final n = rows.length + 8;
    return Semantics(
      label: '配对二维码',
      child: CustomPaint(size: Size.square(n * cell), painter: _QrPainter(rows, cell)),
    );
  }
}

class _QrPainter extends CustomPainter {
  _QrPainter(this.rows, this.cell);

  final List<String> rows;
  final double cell;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRect(Offset.zero & size, Paint()..color = const Color(0xFFFFFFFF));
    final black = Paint()..color = const Color(0xFF000000);
    for (final (y, row) in rows.indexed) {
      for (var x = 0; x < row.length; x++) {
        if (row.codeUnitAt(x) == 0x31) {
          canvas.drawRect(Rect.fromLTWH((x + 4) * cell, (y + 4) * cell, cell, cell), black);
        }
      }
    }
  }

  @override
  bool shouldRepaint(_QrPainter old) => old.rows != rows || old.cell != cell;
}

/// 改设备名的对话框。输入框的控制器跟着对话框走，关的动画放完才释放。
class _RenameDialog extends StatefulWidget {
  const _RenameDialog({required this.initial});

  final String initial;

  @override
  State<_RenameDialog> createState() => _RenameDialogState();
}

class _RenameDialogState extends State<_RenameDialog> {
  late final _controller = TextEditingController(text: widget.initial);

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: const Text('改设备名'),
    content: TextField(
      controller: _controller,
      autofocus: true,
      maxLength: 40,
      decoration: const InputDecoration(labelText: '新的设备名'),
      onSubmitted: (v) => Navigator.pop(context, v),
    ),
    actions: [
      TextButton(onPressed: () => Navigator.pop(context), child: const Text('取消')),
      FilledButton(onPressed: () => Navigator.pop(context, _controller.text), child: const Text('保存')),
    ],
  );
}
