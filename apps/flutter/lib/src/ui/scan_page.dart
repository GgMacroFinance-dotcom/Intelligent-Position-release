// “扫配对二维码”页（F-19）：用摄像头扫控制台或 App“设备”页给的配对二维码，扫到配对链接就回到连接页。
// 这里不连主机：回去后照常先请人确认主机地址，再配对。

import 'package:flutter/material.dart';

import '../host.dart';
import '../qr/scanner.dart';

class ScanPage extends StatefulWidget {
  const ScanPage({super.key, required this.scanner});

  final QrScanner scanner;

  @override
  State<ScanPage> createState() => _ScanPageState();
}

class _ScanPageState extends State<ScanPage> {
  String? _error;
  String? _wrong;
  bool _done = false;

  void _onText(String text) {
    if (_done || !mounted) return;
    final link = PairingLink.tryParse(text);
    if (link == null) {
      if (_wrong == null) setState(() => _wrong = '这不是配对二维码。在控制台或 App 的“设备”页点“添加设备”，扫那里出的码。');
      return;
    }
    _done = true;
    Navigator.of(context).pop(link);
  }

  void _onError(String message) {
    if (mounted) setState(() => _error = message);
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final error = _error;
    return Scaffold(
      appBar: AppBar(title: const Text('扫配对二维码')),
      body: SafeArea(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Expanded(
              child: error != null
                  ? Center(
                      child: Padding(
                        padding: const EdgeInsets.all(24),
                        child: Text(
                          error,
                          textAlign: TextAlign.center,
                          style: TextStyle(color: theme.colorScheme.error),
                        ),
                      ),
                    )
                  : ColoredBox(
                      color: Colors.black,
                      child: widget.scanner.view(onText: _onText, onError: _onError),
                    ),
            ),
            Padding(
              padding: const EdgeInsets.all(16),
              child: Text(
                _wrong ?? '对准控制台或 App“设备”页上的配对二维码。扫到后先请你确认主机地址，再连接。',
                style: _wrong == null ? theme.textTheme.bodyMedium : TextStyle(color: theme.colorScheme.error),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
