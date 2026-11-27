// 天穹的云（设计重构，2026-11-25）：每个字符格算一次云有多厚，画进一张小图（一格一个像素）。
// 主着色器（sky.frag）读这张小图，不再每格算 5 次噪声（中心加上下左右，找边用）。
// G、B 两个通道放天空的颜料不均匀和星成簇的地方：这两样原来每个像素都算一次 3 层噪声，最费 GPU。
// 云飘得很慢，这张图每秒只更新几次（cosmos.dart）。
// 这里的算法和以前 sky.frag 里的 cloudAt 一样：云的位置、形状不变。

#version 460 core
precision mediump float;

#include <flutter/runtime_effect.glsl>

uniform vec2 uGrid;     // 小图的大小（格子数）
uniform vec2 uCell;     // 一格字的宽和高（逻辑像素）
uniform float uTime;    // 秒
uniform vec4 uDome;     // 天空的中心 x、y，半轴 a、b（逻辑像素）

out vec4 fragColor;

float hash21(vec2 p) {
  p = fract(p * vec2(123.34, 456.21));
  p += dot(p, p + 45.32);
  return fract(p.x * p.y);
}

float noise(vec2 p) {
  vec2 i = floor(p), f = fract(p);
  vec2 u = f * f * (3.0 - 2.0 * f);
  float a = hash21(i), b = hash21(i + vec2(1.0, 0.0)), c = hash21(i + vec2(0.0, 1.0)), d = hash21(i + vec2(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

float fbm3(vec2 p) {
  float v = 0.0, a = 0.5;
  for (int i = 0; i < 3; i++) {
    v += a * noise(p);
    p = p * 2.03 + vec2(17.3, 9.1);
    a *= 0.5;
  }
  return v / 0.875;
}

// 云在一点有多厚：大尺度的噪声决定云团在哪里（只有几团），小尺度的决定团里的起伏；
// 靠下、靠两边容易有云，上面中间的天空干净。很慢地飘。
float cloudAt(vec2 q, float t) {
  vec2 d = (q - uDome.xy) / uDome.zw;
  vec2 drift = vec2(t * 0.010, t * 0.003);
  float big = fbm3(q / 520.0 + drift * 0.6 + vec2(3.7, 1.9));
  float detail = fbm3(q / 110.0 + drift + vec2(-1.3, 5.1));
  float place = smoothstep(0.0, 0.95, d.y) * 0.55 + smoothstep(0.55, 1.1, abs(d.x)) * 0.35 - smoothstep(0.2, -0.8, d.y) * 0.25;
  float v = big * 0.62 + detail * 0.38 + place * 0.28;
  return smoothstep(0.64, 0.84, v);
}

void main() {
  // 这一个像素就是一个字符格：(列, 行)。每行错开半格，和 sky.frag 里的格子一致。
  vec2 id = floor(FlutterFragCoord().xy);
  float shift = mod(id.y, 2.0) * 0.5;
  vec2 cc = vec2((id.x + 0.5 - shift) * uCell.x, (id.y + 0.5) * uCell.y);
  float c = cloudAt(cc, uTime);
  // 不随时间变的两样，顺便放在同一张图里，主着色器不用每个像素再算：
  // G：天空的颜料不均匀（大块的深浅），B：星成簇的地方（三层星共用，各层再错开一点）。
  float wash = fbm3(cc / 140.0);
  float cluster = fbm3(cc / 420.0 + vec2(1.3, 1.1));
  fragColor = vec4(c, wash, cluster, 1.0);
}
