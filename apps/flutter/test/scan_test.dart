// 扫码配对（F-19）的界面：假相机里“扫到”什么由测试决定。

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/qr/scanner.dart';

import 'harness.dart';

/// 假相机：画面是几个按钮，点一下就当扫到了那段文字，或者当相机打不开。
class FakeScanner implements QrScanner {
  int opened = 0;

  @override
  Widget view({required ValueChanged<String> onText, required ValueChanged<String> onError}) {
    opened++;
    return Column(
      children: [
        TextButton(onPressed: () => onText('https://example.com/hello'), child: const Text('扫到别的码')),
        TextButton(onPressed: () => onText('http://192.168.1.20:8787/#/pair/123456'), child: const Text('扫到配对码')),
        TextButton(onPressed: () => onError('没有相机权限。'), child: const Text('相机打不开')),
      ],
    );
  }
}

void main() {
  testWidgets('不能扫码的平台上不显示“扫配对二维码”', (tester) async {
    final h = Harness();
    await h.start(tester);
    expect(find.text('连接主机'), findsOneWidget);
    expect(find.text('扫配对二维码'), findsNothing);
    await h.stop(tester);
  });

  testWidgets('扫到别的码时提示，扫到配对码后照常先确认主机再配对', (tester) async {
    final scanner = FakeScanner();
    final h = Harness(scanner: scanner);
    await h.start(tester);
    await tester.tap(find.text('扫配对二维码'));
    await tester.pumpAndSettle();
    expect(scanner.opened, 1);

    await tester.tap(find.text('扫到别的码'));
    await tester.pump();
    expect(find.textContaining('这不是配对二维码'), findsOneWidget);
    expect(h.paired, isEmpty);

    await tester.tap(find.text('扫到配对码'));
    await tester.pumpAndSettle();
    // 回到连接页，地址和配对码已填好，先问要不要连这台主机（明文局域网还要勾信任）。
    expect(find.text('连接这台主机？'), findsOneWidget);
    expect(find.text('http://192.168.1.20:8787'), findsWidgets);
    expect(h.paired, isEmpty);
    await tester.tap(find.text('我信任这个网络'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, '连接'));
    await tester.pumpAndSettle();
    expect(h.paired.single.$2, '123456');
    expect(h.paired.single.$1.text, 'http://192.168.1.20:8787');
    expect((await h.state.store.hosts()).single.trusted, isTrue);
    await h.stop(tester);
  });

  testWidgets('确认时取消就留在连接页，地址和配对码留着', (tester) async {
    final h = Harness(scanner: FakeScanner());
    await h.start(tester);
    await tester.tap(find.text('扫配对二维码'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('扫到配对码'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('取消'));
    await tester.pumpAndSettle();
    expect(h.paired, isEmpty);
    String field(String label) => tester.widget<TextField>(find.widgetWithText(TextField, label)).controller!.text;
    expect(field('主机地址'), 'http://192.168.1.20:8787');
    expect(field('配对码'), '123456');
    await h.stop(tester);
  });

  testWidgets('相机打不开时说明原因，能返回', (tester) async {
    final h = Harness(scanner: FakeScanner());
    await h.start(tester);
    await tester.tap(find.text('扫配对二维码'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('相机打不开'));
    await tester.pump();
    expect(find.text('没有相机权限。'), findsOneWidget);
    expect(find.text('扫到配对码'), findsNothing);
    await tester.tap(find.byTooltip('返回'));
    await tester.pumpAndSettle();
    expect(find.text('连接主机'), findsOneWidget);
    expect(h.paired, isEmpty);
    await h.stop(tester);
  });

  test('相机报错的说明：没权限时教人去系统设置', () {
    expect(cameraErrorText(Exception('Permission not granted')), contains('系统设置'));
    expect(cameraErrorText(Exception('坏了')), startsWith('打不开相机'));
  });
}
