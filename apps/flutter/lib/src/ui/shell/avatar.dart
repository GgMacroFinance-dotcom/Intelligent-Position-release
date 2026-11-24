// 员工头像：先用一个占位（圆角方块上一个字），字符画头像在设计重构后面一步做（requirements.txt N-14）。
// 颜色按员工 id 固定，加了新员工别人的颜色不变。

import 'package:flutter/material.dart';

import 'tokens.dart';

/// 头像的几种颜色（不是 dusk bot 的像素头像，只是区分员工用的底色）。
const _palette = [
  Color(0xFF246BF9),
  Color(0xFF894EF0),
  Color(0xFF0D864D),
  Color(0xFFB95B01),
  Color(0xFFE91319),
  Color(0xFF0E7C86),
  Color(0xFF6B6B6B),
];

Color avatarColor(String id) {
  // FNV-1a：跨平台、跨启动都一样（String.hashCode 不保证）。
  var h = 0x811C9DC5;
  for (final c in id.codeUnits) {
    h = ((h ^ c) * 0x01000193) & 0xFFFFFFFF;
  }
  return _palette[h % _palette.length];
}

class AgentAvatar extends StatelessWidget {
  const AgentAvatar({
    super.key,
    required this.id,
    required this.name,
    this.size = 40,
    this.working = false,
    this.waiting = false,
    this.failed = false,
    this.chief = false,
  });

  final String id;
  final String name;
  final double size;

  /// 正在处理一轮：右下角绿点。
  final bool working;

  /// 在问你：右下角黄点。
  final bool waiting;

  /// 上一轮失败：右下角红点。
  final bool failed;

  /// 幕僚长：左上角一个小标记。
  final bool chief;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    final color = avatarColor(id);
    final letter = name.isEmpty ? '?' : String.fromCharCode(name.runes.first);
    final dot = waiting
        ? s.warning
        : failed
        ? s.danger
        : working
        ? s.success
        : null;
    final dotSize = size >= 36 ? 12.0 : 9.0;
    return SizedBox.square(
      dimension: size,
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          Container(
            width: size,
            height: size,
            alignment: Alignment.center,
            decoration: BoxDecoration(
              color: color.withValues(alpha: 0.14),
              borderRadius: BorderRadius.circular(size * 0.3),
            ),
            child: Text(
              letter,
              style: TextStyle(color: color, fontSize: size * 0.42, fontWeight: FontWeight.w600, height: 1),
            ),
          ),
          if (dot != null)
            Positioned(
              right: -2,
              bottom: -2,
              child: Container(
                width: dotSize,
                height: dotSize,
                decoration: BoxDecoration(
                  color: dot,
                  shape: BoxShape.circle,
                  border: Border.all(color: s.panel, width: 2),
                ),
              ),
            ),
          if (chief)
            Positioned(
              left: -3,
              top: -3,
              child: Container(
                padding: const EdgeInsets.all(2),
                decoration: BoxDecoration(color: s.panel, shape: BoxShape.circle),
                child: Icon(Icons.star_rounded, size: size >= 36 ? 12 : 9, color: const Color(0xFFF59312)),
              ),
            ),
        ],
      ),
    );
  }
}
