// 天体表面肌理烘焙（设计重构 2026-11-26）：每格字算一次，画进一张极小的离屏图（约 24x24 像素，一格一像素）。
// body.frag 每帧只做 O(1) 查表，彻底免去逐像素噪声运算；所有 10 种天体形态共用这一份着色器。
// 输出四通道（r/g/b 承载不同星球的高度场、风暴斑、云层、冰盖、月海等，零额外带宽）。

#version 460 core
precision mediump float;

#include <flutter/runtime_effect.glsl>

uniform vec2 uGrid;     // 小图格子数
uniform vec2 uCell;     // 一格字的宽高（逻辑像素）
uniform vec2 uBox;      // 天体方框大小（逻辑像素）
uniform float uTime;    // 秒
uniform float uRadius;  // 球半径（逻辑像素）
uniform float uStyle;   // 形态编号（0 太阳, 1 土星环, 2 木星带纹, 3 地球海陆, 4 火星岩漠, 5 月相水星, 6 天王冰星, 7 海王深蓝, 8 金星厚云, 9 恒星）
uniform float uSeed;    // 每颗星球的相位偏移
uniform float uStep;    // 细腻版（行星）：一个像素对应多少逻辑像素，按方框直角网格排；旧版（太阳/水星/恒星）传 0，一格字一像素

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

float fbm2(vec2 p) {
  return noise(p) * 0.65 + noise(p * 2.13 + 5.7) * 0.35;
}

void main() {
  vec2 id = floor(FlutterFragCoord().xy);
  vec2 n2;
  float rr;
  if (uStep > 0.0) {
    // 细腻版：像素中心 = (id + 0.5) * uStep；球外的像素贴到球边上取值，双线性取样时球边不会被拉灰。
    vec2 cc = (id + 0.5) * uStep - uBox * 0.5;
    n2 = cc / uRadius;
    rr = dot(n2, n2);
    if (rr >= 0.998) {
      n2 *= 0.999 / sqrt(rr);
      rr = dot(n2, n2);
    }
  } else {
    float shift = mod(id.y, 2.0) * 0.5;
    vec2 cc = vec2((id.x + 0.5 - shift) * uCell.x, (id.y + 0.5) * uCell.y) - uBox * 0.5;
    n2 = cc / uRadius;
    rr = dot(n2, n2);
    if (uStyle > 8.5) {
      // 旧版恒星 9 做圆形切割时，跨球边的字符格仍会落在圆内，贴到球边内侧取值避免边沿发灰。
      if (rr >= 0.998) {
        n2 *= 0.999 / sqrt(rr);
        rr = dot(n2, n2);
      }
    } else if (rr >= 1.0) {
      fragColor = vec4(0.5, 0.0, 0.0, 1.0);
      return;
    }
  }
  float nz = sqrt(1.0 - rr);
  float t = uTime;
  float sd = uSeed * 6.28318;
  float lat = asin(clamp(n2.y, -1.0, 1.0));

  float rOut = 0.5;
  float gOut = 0.0;
  float bOut = 0.0;

  if (uStyle < 0.5) {
    // 0 · 太阳：米粒组织慢慢翻涌
    float lon = atan(n2.x, nz) + t * 0.035;
    vec2 sp = vec2(lon * 3.2, lat * 3.2);
    float a = noise(sp * 2.0 + vec2(t * 0.05, -t * 0.03));
    float b = noise(sp * 4.3 + vec2(-t * 0.08, t * 0.06) + 7.1);
    rOut = a * 0.62 + b * 0.38;
  } else if (uStyle < 1.5) {
    // 1 · 土星：柔和丝绸水平云带
    float lon = atan(n2.x, nz) + t * 0.045 + sd;
    float warp = (noise(vec2(lon * 1.4, lat * 3.5)) - 0.5) * 0.28;
    float y = lat + warp * 0.35;
    float bands = 0.5 + 0.30 * sin(y * 9.5) + 0.16 * cos(y * 17.0 + 1.2);
    rOut = clamp(bands + (noise(vec2(lon * 2.6, y * 7.0)) - 0.5) * 0.14, 0.0, 1.0);
  } else if (uStyle < 2.5) {
    // 2 · 木星：强烈湍流云带 + 自转大红斑（gOut）
    float lon = atan(n2.x, nz) + t * 0.065 + sd;
    float turb = (fbm2(vec2(lon * 2.1, lat * 4.5)) - 0.5) * 0.38;
    float y = lat + turb * 0.30;
    float bands = 0.5 + 0.34 * sin(y * 11.5) + 0.20 * sin(y * 23.0 + 0.9);
    rOut = clamp(bands, 0.0, 1.0);
    // 大红斑位于南纬约 0.30，随经度自转划过正面
    float dLon = mod(lon + 3.14159, 6.28318) - 3.14159;
    vec2 sdPos = vec2(dLon * 2.15, (lat - 0.30) * 4.4);
    float spotR = length(sdPos);
    float swirl = noise(sdPos * 3.5 + vec2(t * 0.08, 0.0));
    gOut = clamp((1.0 - smoothstep(0.28, 0.85, spotR)) * (0.78 + 0.22 * swirl), 0.0, 1.0);
    bOut = turb + 0.5;
  } else if (uStyle < 3.5) {
    // 3 · 地球：大陆海洋（rOut）+ 独立飘动高空白云（gOut）+ 两极冰霜（bOut）
    float lon = atan(n2.x, nz) + t * 0.030 + sd;
    vec2 p = vec2(lon * 1.9, lat * 2.2);
    float land = fbm2(p + vec2(0.0, 1.4));
    float detail = noise(p * 3.4 + 3.1);
    rOut = clamp(land * 0.78 + detail * 0.22, 0.0, 1.0);
    float cloudLon = atan(n2.x, nz) + t * 0.052 + sd + 2.3;
    float cl = fbm2(vec2(cloudLon * 1.6, lat * 3.6) + vec2(4.2, -1.7));
    gOut = smoothstep(0.56, 0.78, cl);
    bOut = smoothstep(1.02, 1.32, abs(lat) + (detail - 0.5) * 0.22);
  } else if (uStyle < 4.5) {
    // 4 · 火星：朱砂岩漠与暗色玄武岩高原 + 北极白冰盖（gOut）
    float lon = atan(n2.x, nz) + t * 0.028 + sd;
    vec2 p = vec2(lon * 2.2, lat * 2.4);
    float terrain = fbm2(p + 1.8);
    float craters = abs(noise(p * 4.8 + 9.2) - 0.5) * 2.0;
    rOut = clamp(terrain * 0.75 + craters * 0.25, 0.0, 1.0);
    // 屏幕 y 向上为负，lat < 0 为北极
    gOut = smoothstep(-0.92, -1.16, lat + (terrain - 0.5) * 0.25);
    bOut = craters;
  } else if (uStyle < 5.5) {
    // 5 · 银霜月相星 / 水星：暗月海（rOut）与环形山亮脊（gOut）
    float lon = atan(n2.x, nz) + t * 0.020 + sd;
    vec2 p = vec2(lon * 2.1, lat * 2.1);
    float maria = fbm2(p + 4.1);
    float cr = noise(p * 5.2 + 2.6);
    float rim = 1.0 - abs(cr - 0.52) * 5.5;
    rOut = clamp(maria, 0.0, 1.0);
    gOut = clamp(rim, 0.0, 1.0);
  } else if (uStyle < 6.5) {
    // 6 · 天王星：清透孔雀青冰云细纹
    float lon = atan(n2.x, nz) + t * 0.042 + sd;
    float streak = 0.5 + 0.24 * sin(lat * 7.5 + noise(vec2(lon * 1.3, lat * 2.5)) * 1.1);
    rOut = clamp(streak, 0.0, 1.0);
    gOut = smoothstep(0.66, 0.88, noise(vec2(lon * 2.4, lat * 5.2) + 3.0));
  } else if (uStyle < 7.5) {
    // 7 · 海王星：深青金流带 + 亮银卷云（gOut）+ 大暗斑（bOut）
    float lon = atan(n2.x, nz) + t * 0.058 + sd;
    float bands = 0.5 + 0.28 * sin(lat * 10.5 + noise(vec2(lon * 1.8, lat * 3.0)) * 1.4);
    rOut = clamp(bands, 0.0, 1.0);
    gOut = smoothstep(0.68, 0.86, noise(vec2(lon * 1.4, lat * 7.5) + 6.4));
    float dLon = mod(lon + 2.1, 6.28318) - 3.14159;
    float dSpot = length(vec2(dLon * 2.5, (lat + 0.22) * 4.8));
    bOut = 1.0 - smoothstep(0.25, 0.82, dSpot);
  } else if (uStyle < 8.5) {
    // 8 · 金星：香槟玫瑰金厚云旋涡
    float lon = atan(n2.x, nz) + t * 0.062 + sd;
    float chevron = lon * 1.6 + abs(lat) * 2.4;
    float w = fbm2(vec2(chevron, lat * 3.2) + vec2(0.0, 2.1));
    float w2 = noise(vec2(lon * 2.8, lat * 5.0 - w * 1.8));
    rOut = clamp(w * 0.65 + w2 * 0.35, 0.0, 1.0);
    gOut = smoothstep(0.58, 0.82, w2);
  } else {
    // 9 · 恒星：柔和快速闪烁的光球肌理
    float lon = atan(n2.x, nz) + t * 0.08 + sd;
    rOut = fbm2(vec2(lon * 3.5, lat * 3.5) + vec2(t * 0.12, -t * 0.09));
  }

  fragColor = vec4(rOut, gOut, bOut, 1.0);
}
