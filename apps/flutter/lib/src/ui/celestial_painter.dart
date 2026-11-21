// 画太阳、八大行星和恒星。全部用程序画，不用图片：缩放到多大都清楚，也不用操心图片的版权。

import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../star_map.dart';

/// 在 `c` 处画一个半径 `r` 的天体。`t` 是秒，用来让太阳的日冕转、恒星闪。
void paintCelestial(Canvas canvas, Celestial body, Offset c, double r, double t) {
  switch (body.kind) {
    case CelestialKind.sun:
      _paintSun(canvas, c, r, t);
    case CelestialKind.star:
      _paintStar(canvas, Color(body.colors.first), c, r, t);
    case CelestialKind.planet:
      _paintPlanet(canvas, body, c, r);
  }
}

/// 单独一个天体图标，给气泡顶栏、图例用。
class CelestialIcon extends StatelessWidget {
  const CelestialIcon(this.body, {super.key, this.size = 28, this.t = 0});

  final Celestial body;
  final double size;
  final double t;

  @override
  Widget build(BuildContext context) => CustomPaint(size: Size.square(size), painter: _IconPainter(body, t));
}

class _IconPainter extends CustomPainter {
  _IconPainter(this.body, this.t);
  final Celestial body;
  final double t;

  @override
  void paint(Canvas canvas, Size size) {
    // 恒星和太阳的光晕比本体大，图标里留出位置。
    final r = size.shortestSide / 2 * (body.kind == CelestialKind.planet ? 0.82 : 0.5);
    paintCelestial(canvas, body, size.center(Offset.zero), r, t);
  }

  @override
  bool shouldRepaint(_IconPainter old) => old.body != body || old.t != t;
}

void _paintSun(Canvas canvas, Offset c, double r, double t) {
  // 外层光晕：渐变淡到透明，画成矩形就够了。不用 drawCircle：Impeller 的 GL 后端在一些驱动上画带渐变的大圆会出方块。
  final halo = Rect.fromCircle(center: c, radius: r * 2.6);
  canvas.drawRect(
    halo,
    Paint()
      ..shader = const RadialGradient(
        colors: [Color(0x66FBBF24), Color(0x22F97316), Color(0x00F97316)],
        stops: [0.0, 0.45, 1.0],
      ).createShader(halo),
  );
  // 慢慢转的日冕光芒。
  final rays = Paint()..color = const Color(0x40FDE68A);
  for (var i = 0; i < 16; i++) {
    final a = t * 0.08 + i * math.pi / 8;
    final len = r * (1.45 + 0.18 * math.sin(t * 1.3 + i));
    final p = Path()
      ..moveTo(c.dx + math.cos(a - 0.09) * r * 0.95, c.dy + math.sin(a - 0.09) * r * 0.95)
      ..lineTo(c.dx + math.cos(a) * len, c.dy + math.sin(a) * len)
      ..lineTo(c.dx + math.cos(a + 0.09) * r * 0.95, c.dy + math.sin(a + 0.09) * r * 0.95)
      ..close();
    canvas.drawPath(p, rays);
  }
  // 本体：白热的中心到橙色的边。
  canvas.drawCircle(
    c,
    r,
    Paint()
      ..shader = const RadialGradient(
        center: Alignment(-0.25, -0.3),
        colors: [Color(0xFFFFFBEB), Color(0xFFFDE047), Color(0xFFFBBF24), Color(0xFFEA580C)],
        stops: [0.0, 0.35, 0.7, 1.0],
      ).createShader(Rect.fromCircle(center: c, radius: r)),
  );
  // 几块米粒组织一样的亮斑。
  final spots = Paint()..color = const Color(0x33FFFFFF);
  for (var i = 0; i < 5; i++) {
    final a = i * 1.7 + t * 0.05;
    canvas.drawCircle(c + Offset(math.cos(a), math.sin(a)) * r * 0.5, r * 0.12, spots);
  }
}

void _paintStar(Canvas canvas, Color color, Offset c, double r, double t) {
  final twinkle = 0.85 + 0.15 * math.sin(t * 2.1 + color.toARGB32() % 7);
  final glow = Rect.fromCircle(center: c, radius: r * 2.4 * twinkle);
  canvas.drawRect(
    glow,
    Paint()
      ..shader = RadialGradient(
        colors: [color.withValues(alpha: 0.75), color.withValues(alpha: 0.22), color.withValues(alpha: 0)],
        stops: const [0.0, 0.35, 1.0],
      ).createShader(glow),
  );
  // 十字星芒和两道淡一点的斜芒。
  void spike(double angle, double length, double width, double alpha) {
    final dir = Offset(math.cos(angle), math.sin(angle));
    final normal = Offset(-dir.dy, dir.dx) * width;
    final p = Path()
      ..moveTo(c.dx + dir.dx * length, c.dy + dir.dy * length)
      ..lineTo(c.dx + normal.dx, c.dy + normal.dy)
      ..lineTo(c.dx - dir.dx * length, c.dy - dir.dy * length)
      ..lineTo(c.dx - normal.dx, c.dy - normal.dy)
      ..close();
    canvas.drawPath(p, Paint()..color = Color.lerp(color, Colors.white, 0.5)!.withValues(alpha: alpha));
  }

  spike(0, r * 2.2 * twinkle, r * 0.12, 0.85);
  spike(math.pi / 2, r * 2.2 * twinkle, r * 0.12, 0.85);
  spike(math.pi / 4, r * 1.2 * twinkle, r * 0.08, 0.45);
  spike(-math.pi / 4, r * 1.2 * twinkle, r * 0.08, 0.45);
  // 白热的核心。
  canvas.drawCircle(
    c,
    r * 0.55,
    Paint()
      ..shader = RadialGradient(
        colors: [Colors.white, Color.lerp(color, Colors.white, 0.4)!, color],
        stops: const [0.0, 0.55, 1.0],
      ).createShader(Rect.fromCircle(center: c, radius: r * 0.55)),
  );
}

void _paintPlanet(Canvas canvas, Celestial body, Offset c, double r) {
  final colors = body.colors.map(Color.new).toList();
  final rect = Rect.fromCircle(center: c, radius: r);
  final isSaturn = body.id == 'saturn';
  final isUranus = body.id == 'uranus';

  // 大气的一圈薄光。
  final air = Rect.fromCircle(center: c, radius: r * 1.35);
  canvas.drawRect(
    air,
    Paint()
      ..shader = RadialGradient(
        colors: [colors.first.withValues(alpha: 0.28), colors.first.withValues(alpha: 0)],
        stops: const [0.7, 1.0],
      ).createShader(air),
  );

  // 土星环、天王星环的后半圈画在星体后面。
  if (isSaturn) _ring(canvas, c, r, colors[2], back: true, tilt: -0.35, width: 1.0);
  if (isUranus) _ring(canvas, c, r, colors.first, back: true, tilt: 1.35, width: 0.45, thin: true);

  canvas.save();
  canvas.clipPath(Path()..addOval(rect));
  canvas.drawRect(
    rect,
    Paint()
      ..shader = RadialGradient(
        center: const Alignment(-0.35, -0.4),
        colors: [Color.lerp(colors.first, Colors.white, 0.25)!, colors.first, colors[1]],
        stops: const [0.0, 0.5, 1.0],
      ).createShader(rect),
  );
  switch (body.id) {
    case 'mercury':
      final crater = Paint()..color = colors[1].withValues(alpha: 0.55);
      for (final (dx, dy, s) in [(-0.3, -0.2, 0.18), (0.35, 0.1, 0.12), (-0.05, 0.45, 0.15), (0.2, -0.45, 0.09)]) {
        canvas.drawCircle(c + Offset(dx, dy) * r, s * r, crater);
      }
    case 'venus':
      final swirl = Paint()
        ..color = Colors.white.withValues(alpha: 0.18)
        ..style = PaintingStyle.stroke
        ..strokeWidth = r * 0.16;
      canvas.drawArc(rect.deflate(r * 0.25), 0.4, 2.2, false, swirl);
      canvas.drawArc(rect.deflate(r * 0.55), 3.4, 2.0, false, swirl);
    case 'earth':
      final land = Paint()..color = colors[1];
      canvas.drawOval(Rect.fromCenter(center: c + Offset(-0.3, -0.15) * r, width: r * 0.8, height: r * 0.6), land);
      canvas.drawOval(Rect.fromCenter(center: c + Offset(0.35, 0.3) * r, width: r * 0.55, height: r * 0.75), land);
      final cloud = Paint()
        ..color = Colors.white.withValues(alpha: 0.6)
        ..style = PaintingStyle.stroke
        ..strokeCap = StrokeCap.round
        ..strokeWidth = r * 0.1;
      canvas.drawArc(rect.deflate(r * 0.2), -2.6, 1.3, false, cloud);
      canvas.drawArc(rect.deflate(r * 0.45), 0.6, 1.1, false, cloud);
    case 'mars':
      canvas.drawOval(
        Rect.fromCenter(center: c + Offset(0, -0.88) * r, width: r * 0.8, height: r * 0.35),
        Paint()..color = Colors.white.withValues(alpha: 0.85),
      );
      canvas.drawCircle(c + Offset(0.25, 0.2) * r, r * 0.22, Paint()..color = colors[1].withValues(alpha: 0.45));
    case 'jupiter':
      for (final (y, h, i) in [(-0.6, 0.16, 1), (-0.25, 0.2, 0), (0.1, 0.18, 1), (0.45, 0.22, 0), (0.78, 0.14, 1)]) {
        canvas.drawRect(
          Rect.fromLTWH(c.dx - r, c.dy + y * r, r * 2, h * r),
          Paint()..color = (i == 1 ? colors[1] : Colors.white).withValues(alpha: i == 1 ? 0.55 : 0.18),
        );
      }
      canvas.drawOval(
        Rect.fromCenter(center: c + Offset(0.3, 0.32) * r, width: r * 0.42, height: r * 0.24),
        Paint()..color = colors[2].withValues(alpha: 0.85),
      );
    case 'saturn':
      for (final y in [-0.45, 0.0, 0.42]) {
        canvas.drawRect(
          Rect.fromLTWH(c.dx - r, c.dy + y * r, r * 2, r * 0.16),
          Paint()..color = colors[1].withValues(alpha: 0.35),
        );
      }
    case 'neptune':
      canvas.drawOval(
        Rect.fromCenter(center: c + Offset(-0.2, 0.25) * r, width: r * 0.45, height: r * 0.25),
        Paint()..color = const Color(0xFF0B1E5B).withValues(alpha: 0.7),
      );
      canvas.drawArc(
        rect.deflate(r * 0.3),
        -0.6,
        1.0,
        false,
        Paint()
          ..color = Colors.white.withValues(alpha: 0.35)
          ..style = PaintingStyle.stroke
          ..strokeWidth = r * 0.07,
      );
  }
  // 明暗交界：右下暗下去，看上去是个球。
  canvas.drawRect(
    rect,
    Paint()
      ..shader = const RadialGradient(
        center: Alignment(-0.45, -0.5),
        radius: 1.15,
        colors: [Color(0x00000000), Color(0x00000000), Color(0xB3000000)],
        stops: [0.0, 0.55, 1.0],
      ).createShader(rect),
  );
  canvas.restore();

  if (isSaturn) _ring(canvas, c, r, colors[2], back: false, tilt: -0.35, width: 1.0);
  if (isUranus) _ring(canvas, c, r, colors.first, back: false, tilt: 1.35, width: 0.45, thin: true);
}

/// 行星环：一个倾斜的扁椭圆，`back` 时只画在星体后面的那半圈。
void _ring(
  Canvas canvas,
  Offset c,
  double r,
  Color color, {
  required bool back,
  required double tilt,
  required double width,
  bool thin = false,
}) {
  canvas.save();
  canvas.translate(c.dx, c.dy);
  canvas.rotate(tilt);
  final oval = Rect.fromCenter(center: Offset.zero, width: r * (2.1 + width * 0.9), height: r * (0.55 + width * 0.25));
  final paint = Paint()
    ..style = PaintingStyle.stroke
    ..strokeWidth = r * (thin ? 0.07 : 0.22)
    ..color = color.withValues(alpha: thin ? 0.55 : 0.85);
  canvas.drawArc(oval, back ? math.pi : 0, math.pi, false, paint);
  if (!thin) {
    canvas.drawArc(
      oval.inflate(r * 0.2),
      back ? math.pi : 0,
      math.pi,
      false,
      paint
        ..strokeWidth = r * 0.08
        ..color = color.withValues(alpha: 0.45),
    );
  }
  canvas.restore();
}
