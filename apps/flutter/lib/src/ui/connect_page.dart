// “连接主机”页：填主机地址和 6 位配对码，或者粘贴控制台给的配对链接（apps-plan 4.2、9.3）。

import 'package:flutter/material.dart';

import '../host.dart';
import '../pairing.dart';
import 'common.dart';

class ConnectPage extends StatefulWidget {
  const ConnectPage({super.key});

  @override
  State<ConnectPage> createState() => _ConnectPageState();
}

class _ConnectPageState extends State<ConnectPage> {
  final _address = TextEditingController();
  final _code = TextEditingController();
  bool _busy = false;
  String? _error;

  @override
  void dispose() {
    _address.dispose();
    _code.dispose();
    super.dispose();
  }

  /// 地址栏里粘贴了整条配对链接时，拆成地址和配对码。
  void _onAddressChanged(String text) {
    final link = PairingLink.tryParse(text);
    if (link == null) return;
    _address.text = link.host.text;
    _code.text = link.code;
    setState(() => _error = null);
  }

  Future<void> _submit() async {
    final HostAddress address;
    try {
      address = HostAddress.parse(_address.text);
    } on FormatException catch (e) {
      setState(() => _error = e.message);
      return;
    }
    final code = normalizePairingCode(_code.text);
    if (code == null) {
      setState(() => _error = '配对码是 6 位数字');
      return;
    }
    final trusted = await _confirm(address);
    if (trusted == null || !mounted) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await AppScope.read(context).pair(address, code, trusted: trusted);
    } on PairingError catch (e) {
      if (mounted) setState(() => _error = e.message);
    } catch (e) {
      if (mounted) setState(() => _error = '配对失败：$e');
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  /// 先请人确认要连的主机；明文连局域网地址时要勾“我信任这个网络”。取消时返回 null。
  Future<bool?> _confirm(HostAddress address) => showDialog<bool>(
    context: context,
    builder: (context) {
      var trusted = false;
      return StatefulBuilder(
        builder: (context, setState) => AlertDialog(
          title: const Text('连接这台主机？'),
          content: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              SelectableText(address.text, style: Theme.of(context).textTheme.titleMedium),
              const SizedBox(height: 12),
              const Text('确认这是你自己的主机。配对码只能用一次，连上后这台设备会拿到自己的令牌。'),
              if (address.needsTrust) ...[
                const SizedBox(height: 12),
                Text(
                  '这个地址不是 HTTPS，也不是 Tailscale（*.ts.net），令牌和消息会明文经过网络。',
                  style: TextStyle(color: Theme.of(context).colorScheme.error),
                ),
                CheckboxListTile(
                  contentPadding: EdgeInsets.zero,
                  value: trusted,
                  onChanged: (v) => setState(() => trusted = v ?? false),
                  title: const Text('我信任这个网络'),
                ),
              ],
            ],
          ),
          actions: [
            TextButton(onPressed: () => Navigator.pop(context), child: const Text('取消')),
            FilledButton(
              onPressed: !address.needsTrust || trusted ? () => Navigator.pop(context, trusted) : null,
              child: const Text('连接'),
            ),
          ],
        ),
      );
    },
  );

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Scaffold(
      body: SafeArea(
        child: Center(
          child: SingleChildScrollView(
            padding: const EdgeInsets.all(24),
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 420),
              child: AutofillGroup(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Text('连接主机', style: theme.textTheme.headlineSmall),
                    const SizedBox(height: 8),
                    Text(
                      '在主机上运行 ip device pair --name <设备名>，或者在控制台“设备”页点“添加设备”，'
                      '把地址和 6 位配对码填在这里。配对码 2 分钟内有效。',
                      style: TextStyle(color: theme.hintColor),
                    ),
                    const SizedBox(height: 24),
                    TextField(
                      controller: _address,
                      enabled: !_busy,
                      keyboardType: TextInputType.url,
                      autocorrect: false,
                      autofillHints: const [AutofillHints.url],
                      decoration: const InputDecoration(
                        labelText: '主机地址',
                        hintText: 'https://ip.example.com',
                        helperText: '也可以直接粘贴控制台给的配对链接',
                        border: OutlineInputBorder(),
                      ),
                      onChanged: _onAddressChanged,
                    ),
                    const SizedBox(height: 16),
                    TextField(
                      controller: _code,
                      enabled: !_busy,
                      keyboardType: TextInputType.number,
                      maxLength: 7,
                      decoration: const InputDecoration(
                        labelText: '配对码',
                        hintText: '6 位数字',
                        border: OutlineInputBorder(),
                      ),
                      onSubmitted: (_) => _submit(),
                    ),
                    if (_error != null) ...[
                      const SizedBox(height: 8),
                      Text(_error!, style: TextStyle(color: theme.colorScheme.error)),
                    ],
                    const SizedBox(height: 16),
                    FilledButton(
                      onPressed: _busy ? null : _submit,
                      child: _busy
                          ? const SizedBox.square(dimension: 20, child: CircularProgressIndicator(strokeWidth: 2))
                          : const Text('配对并连接'),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
