// 太阳天体（ui/shell/celestial_body.dart）：选中时整体变亮的过渡要真的走完，
// 面板开着时太阳停住（frozen），但停住不能把选中的过渡也停掉（以前用 TickerMode 关掉整个太阳，过渡停在 0）。

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/ui/shell/celestial_body.dart';
import 'package:ip_app/src/ui/shell/cosmos.dart';

void main() {
  setUp(() => CosmosBackdrop.animate = false);
  tearDown(() => CosmosBackdrop.animate = true);

  Widget sun({required bool selected, required bool frozen}) => MaterialApp(
    home: Center(
      child: CelestialBody(key: const ValueKey('sun'), radius: 80, selected: selected, frozen: frozen, onTap: () {}),
    ),
  );

  double lift(WidgetTester tester) => (tester.state(find.byKey(const ValueKey('sun'))) as dynamic).selection as double;

  testWidgets('选中并停住时，变亮的过渡照样走完；取消后退回去', (tester) async {
    await tester.pumpWidget(sun(selected: false, frozen: false));
    expect(lift(tester), 0.0);

    // 和外壳里一样：选中的同时停住。
    await tester.pumpWidget(sun(selected: true, frozen: true));
    await tester.pump(const Duration(milliseconds: 100));
    expect(lift(tester), inExclusiveRange(0.0, 1.0));
    await tester.pumpAndSettle();
    expect(lift(tester), 1.0);

    await tester.pumpWidget(sun(selected: false, frozen: false));
    await tester.pumpAndSettle();
    expect(lift(tester), 0.0);
  });
}
