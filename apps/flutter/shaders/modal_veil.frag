// 「新建 Bot」弹窗液态流光底幕（方案 B + 甲：文艺复兴暗金夜雾 + 随选中星球轻染色，2026-11-27）：
//   - 深邃午夜靛蓝半透丝绒基底，叠在 BackdropFilter 磨砂玻璃之上；
//   - 双层慢速域扭曲（domain warp）驱动三股极低浓度暗流：深群青、孔雀幽青，与一缕极隐蔽的文艺复兴暗香槟金星尘丝缕
//     （金蓝过渡层经淡暮紫桥接，绝不混出浑浊暗绿）；
//   - 顶部大星球背后融入超大半径的当前选中星球色晕（uAccent），切换星球时在 Dart 侧做 620ms 平滑插值染色；
//   - 顶部自带极微弱 Liquid Glass 玻璃边折射高光，末段叠三角分布亚像素抖动，彻底消除暗色 8-bit 渐变断层。

#version 460 core
precision mediump float;

#include <flutter/runtime_effect.glsl>

uniform vec2 uSize;     // 弹窗逻辑像素宽高
uniform float uTime;    // 共享轻量时钟（秒）
uniform vec3 uAccent;   // 当前选中星球的柔和氛围色（0..1，切星时平滑过渡）
uniform float uHeroY;   // 居中大星球在弹窗里的归一化纵坐标（约 0.22..0.28）

out vec4 fragColor;

float hash21(vec2 p) {
  p = fract(p * vec2(123.34, 456.21));
  p += dot(p, p + 45.32);
  return fract(p.x * p.y);
}

void main() {
  vec2 p = FlutterFragCoord().xy;
  vec2 S = max(uSize, vec2(1.0));
  vec2 uv = p / S;
  vec2 aspect = vec2(S.x / S.y, 1.0);
  vec2 q = (uv - 0.5) * aspect;

  // 柔缓但清晰可辨的液态流动节奏：暗金丝缕与行进光峰持续缓缓游移，既活又克制。
  float wt = uTime * 0.44;

  // 1. 深邃午夜靛蓝基底（上微透、下渐深）
  vec3 baseTop = vec3(0.068, 0.108, 0.205);
  vec3 baseMid = vec3(0.048, 0.078, 0.155);
  vec3 baseBot = vec3(0.032, 0.052, 0.112);
  float gradT = clamp(uv.y * 0.78 + uv.x * 0.22, 0.0, 1.0);
  vec3 col = gradT < 0.52
      ? mix(baseTop, baseMid, gradT / 0.52)
      : mix(baseMid, baseBot, (gradT - 0.52) / 0.48);

  // 2. 双层有机流体域扭曲（无网格接缝、纯连续液态丝绸波纹）
  vec2 w = q * 2.20;
  w += 0.42 * vec2(sin(w.y * 1.50 + wt * 0.95), cos(w.x * 1.38 - wt * 0.82));
  w += 0.24 * vec2(cos(w.y * 2.65 - wt * 0.76 + 1.35), sin(w.x * 2.45 + wt * 0.90 + 2.15));

  float silkA = 0.5 + 0.5 * sin(w.x * 1.95 + w.y * 1.30 - wt * 0.85);
  float silkB = 0.5 + 0.5 * cos(w.x * 1.50 - w.y * 2.05 + wt * 0.72 + 0.9);

  // 3. 暗流一：深群青蓝（左上到中腹缓慢舒展）
  vec2 pUltra = vec2(-0.30 + 0.16 * sin(wt * 0.58), -0.18 + 0.13 * cos(wt * 0.46));
  float fUltra = exp(-dot(q - pUltra, q - pUltra) * 2.35) * (0.58 + 0.42 * silkA);
  col = mix(col, vec3(0.130, 0.295, 0.635), fUltra * 0.23);

  // 4. 暗流二：孔雀幽青雾（右翼与中下部缓缓回旋）
  vec2 pTeal = vec2(0.32 + 0.15 * cos(wt * 0.52 + 1.1), 0.22 + 0.15 * sin(wt * 0.44 + 0.6));
  float fTeal = exp(-dot(q - pTeal, q - pTeal) * 2.65) * (0.54 + 0.46 * silkB);
  col = mix(col, vec3(0.100, 0.420, 0.530), fTeal * 0.16);

  // 5. 暗流三（B 核心气质）：双股交错流动的文艺复兴暗金丝缕
  // 主流沿左下→右上柔弧缓行，并带一道沿丝带持续向前滑动的行进光峰，流动感一眼可辨且不刺眼；
  // 外缘经淡暮紫桥接，避免金与蓝直混发绿，保持暗金鎏光的克制高级感。
  float alongA = q.x * 0.68 - q.y * 0.73;
  float ribbonA = (q.x * 0.73 + q.y * 0.68)
      + 0.24 * sin(w.x * 1.25 - wt * 0.88)
      + 0.13 * cos(w.y * 1.80 + wt * 0.68);
  float centerA = 0.22 * sin(wt * 0.52 - 0.4);
  float dA = ribbonA - centerA;
  float travelA = 0.34 + 0.66 * pow(0.5 + 0.5 * sin(alongA * 2.55 - wt * 1.28 + w.x * 0.55), 1.55);
  float duskBridge = exp(-dA * dA * 5.2) * (0.52 + 0.48 * silkA);
  float goldCoreA = exp(-dA * dA * 18.0) * (0.45 + 0.55 * silkB) * travelA;
  float goldHaloA = exp(-dA * dA * 7.5) * (0.48 + 0.52 * silkA) * (0.45 + 0.55 * travelA);

  // 副流：更轻的一缕反向游移香槟暗金薄纱，与主流交错出液态丝绒层次
  float alongB = -q.x * 0.55 - q.y * 0.84;
  float ribbonB = (q.x * 0.84 - q.y * 0.55)
      + 0.21 * cos(w.y * 1.45 - wt * 0.74 + 1.6)
      + 0.11 * sin(w.x * 1.95 + wt * 0.62);
  float centerB = 0.26 * cos(wt * 0.44 + 1.2);
  float dB = ribbonB - centerB;
  float travelB = 0.30 + 0.70 * pow(0.5 + 0.5 * sin(alongB * 2.80 + wt * 1.05 + w.y * 0.50), 1.65);
  float goldCoreB = exp(-dB * dB * 24.0) * (0.46 + 0.54 * silkA) * travelB;

  float diagMask = 0.62 + 0.38 * smoothstep(0.06, 0.58, length(q - vec2(-0.04, 0.04)));
  col = mix(col, vec3(0.245, 0.165, 0.320), clamp((duskBridge + goldCoreB * 0.45) * diagMask * 0.16, 0.0, 0.24));
  vec3 antiqueGold = vec3(0.865, 0.640, 0.355);
  vec3 champagneCrest = vec3(0.985, 0.835, 0.560);
  vec3 goldTint = mix(antiqueGold, champagneCrest, clamp(travelA * 0.55, 0.0, 0.65));
  col += antiqueGold * (goldHaloA * diagMask * 0.115);
  col += goldTint * (goldCoreA * diagMask * 0.185);
  col += mix(antiqueGold, champagneCrest, 0.35) * (goldCoreB * diagMask * 0.110);

  // 6. 甲 · 随当前选中星球轻染色的顶部液态星晕（与整张流动底幕浑然一体，零圆形硬边）
  vec2 heroPos = vec2(0.048 * sin(wt * 0.62), (uHeroY - 0.5) + 0.028 * cos(wt * 0.50));
  vec2 dqAura = (q - heroPos) + 0.052 * vec2(sin(w.y * 1.6 + wt * 0.55), cos(w.x * 1.5 - wt * 0.48));
  float dAura2 = dot(dqAura, dqAura);
  float broadAura = exp(-dAura2 * 2.45) * (0.76 + 0.24 * silkA);
  float closeAura = exp(-dAura2 * 12.5) * (0.82 + 0.18 * silkB);
  vec3 softAccent = mix(uAccent, vec3(0.84, 0.91, 1.00), 0.24);
  col += uAccent * (broadAura * 0.145);
  col += softAccent * (closeAura * 0.115);

  // 7. 顶部 Liquid Glass 玻璃微折射高光（含一丝沿顶边缓移的暖香槟微辉）+ 四角极轻收暗
  float topSheen = exp(-uv.y * 15.0) * (0.050 + 0.026 * cos(uv.x * 3.14159 - wt * 0.72));
  col += mix(vec3(0.76, 0.86, 1.00), vec3(0.98, 0.87, 0.66), 0.42) * topSheen;
  float vig = smoothstep(0.45, 1.05, length(q));
  col *= 1.0 - vig * 0.15;

  // 8. 三角分布亚像素抖动（±1.5 / 255），彻底消除深色渐变在 8-bit 屏幕上的色带断层
  float dither = (hash21(p) + hash21(p + vec2(17.3, 31.7)) - 1.0) * (1.5 / 255.0);
  col = clamp(col + dither, 0.0, 1.0);

  // 顶部略透、底部更沉稳的磨砂不透明度（87%..92.5%）
  float alpha = mix(0.870, 0.925, smoothstep(0.0, 1.0, uv.y * 0.72 + vig * 0.28));
  fragColor = vec4(col * alpha, alpha);
}
