// 多形态文艺复兴字符画天体着色器（设计重构 2026-11-26）：
// 所有天体（太阳、8 大行星形态、第 9+ 位恒星）共用这一份着色器与 cosmos.dart 字表；
// 逐像素零噪声计算（全部查 body_surface.frag 烘焙的微型表面小图），极致省 GPU。
//   0 太阳：象牙白→暖金→朱红 + 24 道点阵放射光芒
//   1 土星：羊皮纸金丝绸云带 + 3D 倾斜双层卡西尼点阵环（前环压球、后环被遮）
//   2 木星：象牙/赭石/赤陶湍流云带 + 自转大红斑漩涡
//   3 地球：群青海 + 翡翠绿陆地 + 飘动象牙白云 + 极细天青大气边辉
//   4 火星：朱砂赤陶岩漠 + 陨坑明暗 + 北极象牙白冰盖
//   5 水星/月相：参考达·芬奇《地球照》，银霜明面 + 幽蓝暗面月海与环形山
//   6 天王星：孔雀青琉璃冰球 + 58° 陡倾极细冰银单环
//   7 海王星：深邃青金流带 + 亮银高空卷云 + 深蓝大暗斑 + 冰蓝边辉
//   8 金星：香槟珍珠与玫瑰金旋涡厚云 + 柔暖金雾晕
//   9 恒星：象牙冰蓝核心 + 4 长 4 短文艺复兴金箔八角星芒
//
// 细腻版（用于八大行星 1..8；太阳 0 与恒星 9 走 legacyBody，太阳一处不改，恒星仅球边做平滑圆形切割）：
//   - 字来做渐变、底色退后：底色是逐像素平滑的哑光色（暗一档），明暗主要靠字的疏密和字的颜色；
//   - 插缝叠印：主层 A 的字缝里再插一层 B（另一套字形 + 一道慢慢流过的波），两层套印；
//   - 表面小图加密并双线性平滑，上一张和这一张之间连续插值，流动不再一跳一跳；
//   - 行星的字更小（由 Dart 端传入的 uCell 决定）。

#version 460 core
precision mediump float;

#include <flutter/runtime_effect.glsl>

uniform vec2 uBox;      // 方框大小（逻辑像素），天体居中
uniform float uTime;    // 秒
uniform float uDpr;
uniform vec2 uCell;     // 一格字的宽高（逻辑像素）
uniform vec4 uBody;     // x: 球半径, y: 光芒/外环基准长度, z: 选中过渡(0..1), w: 形态编号(0..9)
uniform vec2 uGrid;     // 表面小图像素数
uniform float uStep;    // 细腻版：表面小图一个像素对应多少逻辑像素（旧版传 0）
uniform float uMix;     // 细腻版：上一张 → 这一张表面小图的插值（0..1）
uniform float uHaze;    // 纵深轨道：转到远处时隔着一层空气（0 近/不处理 .. 1 最远），偏天空蓝、略淡
uniform sampler2D uAtlas;
uniform sampler2D uSurface;
uniform sampler2D uSurfacePrev;

out vec4 fragColor;

// 字表 8 列 5 行：0..9 主层由疏到密，10..13 线，14..17 星，18..27 插缝层（横笔画，行星环用），
// 28..37 竖笔画插缝层（球面用，和主层交叉成排线，抵消横纹）。
const vec2 GRID = vec2(8.0, 5.0);
const float RAMP_B = 18.0;
const float RAMP_V = 28.0;
// 竖向抵消（1 开 0 关）：插缝层换竖笔画、字形拉高填满行缝、插缝层的流动沿经线走。
const float VERT = 1.0;
const float PI = 3.14159265;

const vec3 IVORY = vec3(1.000, 0.975, 0.900);
const vec3 GOLD_LIGHT = vec3(1.000, 0.880, 0.560);
const vec3 GOLD = vec3(0.900, 0.680, 0.280);
const vec3 GOLD_DEEP = vec3(0.780, 0.500, 0.180);
const vec3 CINNABAR = vec3(0.760, 0.330, 0.200);

float hash21(vec2 q) {
  q = fract(q * vec2(123.34, 456.21));
  q += dot(q, q + 45.32);
  return fract(q.x * q.y);
}

float glyph(float g, vec2 local) {
  vec2 cell = vec2(mod(g, GRID.x), floor(g / GRID.x));
  return texture(uAtlas, (cell + local) / GRID).a;
}

// 旧版：一格字一像素，最近邻取样。
vec4 surfaceAt(vec2 id) {
  if (id.x < 0.0 || id.y < 0.0 || id.x >= uGrid.x || id.y >= uGrid.y) return vec4(0.5, 0.0, 0.0, 1.0);
  return texture(uSurface, (id + 0.5) / uGrid);
}

// 细腻版：按方框里的逻辑像素位置双线性取样，并在上一张与这一张之间插值。
vec4 surfaceFine(vec2 boxPos) {
  vec2 uv = boxPos / (uGrid * uStep);
  return mix(texture(uSurfacePrev, uv), texture(uSurface, uv), uMix);
}

// 外围柔和光晕 / 大气辉光（按形态定色，确保冷色星球也能从蓝天里跃出）
void haloOf(float style, float r, float R, float sel, float lift, out vec3 rgb, out float alpha) {
  vec3 haloCol = GOLD_LIGHT;
  float haloStrength = 0.22;
  float haloSpread = 0.55;
  if (style < 0.5) {
    haloCol = GOLD_LIGHT; haloStrength = 0.32; haloSpread = 0.75;
  } else if (style < 1.5) {
    haloCol = vec3(1.00, 0.91, 0.70); haloStrength = 0.20; haloSpread = 0.48;
  } else if (style < 2.5) {
    haloCol = vec3(0.98, 0.82, 0.62); haloStrength = 0.20; haloSpread = 0.46;
  } else if (style < 3.5) {
    haloCol = vec3(0.68, 0.93, 1.00); haloStrength = 0.34; haloSpread = 0.42;
  } else if (style < 4.5) {
    haloCol = vec3(0.98, 0.64, 0.46); haloStrength = 0.24; haloSpread = 0.45;
  } else if (style < 5.5) {
    haloCol = vec3(0.92, 0.95, 1.00); haloStrength = 0.26; haloSpread = 0.44;
  } else if (style < 6.5) {
    haloCol = vec3(0.72, 0.98, 0.98); haloStrength = 0.30; haloSpread = 0.46;
  } else if (style < 7.5) {
    haloCol = vec3(0.58, 0.84, 1.00); haloStrength = 0.34; haloSpread = 0.46;
  } else if (style < 8.5) {
    haloCol = vec3(1.00, 0.88, 0.78); haloStrength = 0.28; haloSpread = 0.52;
  } else {
    haloCol = vec3(0.96, 0.97, 1.00); haloStrength = 0.36; haloSpread = 0.72;
  }
  float boxEdgeFade = 1.0 - smoothstep(uBox.x * 0.40, uBox.x * 0.50, r);
  float halo = exp(-pow(max(r - R * 0.92, 0.0) / (R * haloSpread), 1.65)) * haloStrength * boxEdgeFade;
  rgb = haloCol * lift;
  alpha = clamp(halo * (1.0 + 0.35 * sel), 0.0, 1.0);
}

// 球面透视压扁字符（越靠近球边，沿半径方向压得越扁）。
float glyphOnSphere(float g, vec2 local, vec2 cc, float rc, float R) {
  vec2 dir = rc > 0.001 ? cc / rc : vec2(1.0, 0.0);
  vec2 lp = local - 0.5;
  float squash = mix(1.0, 0.55, smoothstep(0.55, 1.0, rc / R));
  float along = dot(lp, dir);
  vec2 l2 = lp + dir * along * (1.0 / squash - 1.0) + 0.5;
  // 竖向抵消：只取字形中间 80% 的高度铺满整格，字被拉高，上下两行的字几乎挨上，行缝不再成横纹。
  vec2 l3 = vec2(l2.x, mix(l2.y, 0.10 + 0.80 * l2.y, VERT));
  return (l2.x >= 0.0 && l2.x <= 1.0 && l2.y >= 0.0 && l2.y <= 1.0) ? glyph(g, l3) : 0.0;
}

// ======================= 旧版：太阳 0 / 水星 5 / 恒星 9（与上一版逐行一致） =======================
vec4 legacyBody(vec2 p) {
  vec2 c = uBox * 0.5;
  vec2 d = p - c;
  float r = length(d);
  float R = uBody.x;
  float rayLen = uBody.y;
  float sel = uBody.z;
  float style = uBody.w;
  float t = uTime;
  float lift = 1.0 + 0.15 * sel;

  vec3 rgb;
  float alpha;
  haloOf(style, r, R, sel, lift, rgb, alpha);

  // 字符格子（每行错开半格，与背景天穹一致）
  vec2 gp = p / uCell;
  float row = floor(gp.y);
  float shift = mod(row, 2.0) * 0.5;
  gp.x += shift;
  vec2 cellId = floor(gp);
  vec2 local = fract(gp);
  vec2 cc = vec2((cellId.x + 0.5 - shift) * uCell.x, (cellId.y + 0.5) * uCell.y) - c;
  float rc = length(cc);

  // 恒星 9：保留旧版外观与点阵八角星芒，仅对球体边缘做平滑圆形切割（修复粗字符格缺口），太阳 0 路径一字不改。
  if (style > 8.5) {
    if (rc >= R && rc < R + rayLen) {
      float th = atan(cc.y, cc.x);
      float seg = PI * 0.5;
      float idx = floor(th / seg + 0.5);
      float off = abs(th - idx * seg) * rc;
      float len = rayLen * (0.78 + 0.18 * sin(t * 1.1 + idx * 1.3));
      float idx2 = floor((th - seg * 0.5) / seg + 0.5);
      float off2 = abs(th - seg * 0.5 - idx2 * seg) * rc;
      float len2 = rayLen * (0.45 + 0.12 * cos(t * 1.3 + idx2 * 2.1));
      float reach = rc - R;
      float a = 0.0;
      if (off < uCell.x * 0.65 && reach < len) a = 1.0 - reach / len;
      else if (off2 < uCell.x * 0.55 && reach < len2) a = 0.75 * (1.0 - reach / len2);
      if (a > 0.0) {
        float cover = glyph(a > 0.5 ? 16.0 : 14.0, local);
        vec3 col = mix(vec3(0.76, 0.88, 1.00), IVORY, a);
        float k = clamp(cover * (0.40 + 0.60 * a), 0.0, 1.0);
        rgb = mix(rgb, col * lift, k);
        alpha = mix(alpha, 1.0, k);
      }
    }

    float edge = 1.0 - smoothstep(R - 1.0, R + 0.35, r);
    if (edge > 0.0) {
      vec2 sc = (rc > R * 0.998) ? cc * (R * 0.998 / max(rc, 0.001)) : cc;
      float rsc = min(rc, R * 0.998);
      vec2 n2 = sc / R;
      float nz = sqrt(max(0.0, 1.0 - dot(n2, n2)));
      vec3 nrm = vec3(n2, nz);
      vec3 lightDir = normalize(vec3(-0.45, -0.55, 0.70));
      float lam = max(0.0, dot(nrm, lightDir));
      float limb = pow(nz, 0.45);
      vec4 surf = surfaceAt(cellId);

      float q = clamp((0.45 + 0.52 * lam + (surf.r - 0.5) * 0.40) * (0.55 + 0.45 * limb), 0.0, 1.0);
      vec3 col = mix(vec3(0.76, 0.88, 1.00), IVORY, q);
      vec3 base = mix(col * 0.82, col, 0.38);
      float cover = glyphOnSphere(clamp(floor(q * 9.0 + 0.5), 0.0, 9.0), local, sc, rsc, R);
      vec3 ink = mix(col, IVORY, 0.36 * q);
      vec3 sphere = mix(base, ink, cover * 0.88) * lift;
      rgb = mix(rgb, sphere, edge);
      alpha = mix(alpha, 1.0, edge);
    }

    return vec4(clamp(rgb, 0.0, 1.0) * alpha, alpha);
  }

  if (rc < R) {
    vec2 n2 = cc / R;
    float nz = sqrt(max(0.0, 1.0 - dot(n2, n2)));
    vec3 nrm = vec3(n2, nz);
    vec3 lightDir = (style > 4.5 && style < 5.5)
        ? normalize(vec3(-0.76, -0.34, 0.55))
        : normalize(vec3(-0.45, -0.55, 0.70));
    float lam = max(0.0, dot(nrm, lightDir));
    float limb = pow(nz, 0.45);
    float rim = pow(1.0 - nz, 2.6);
    vec4 surf = surfaceAt(cellId);

    vec3 col = GOLD;
    float q = 0.5;
    vec3 rimGlow = vec3(0.0);

    if (style < 0.5) {
      // 0 · 太阳
      q = clamp((0.30 + 0.62 * lam + (surf.r - 0.5) * 0.55) * (0.45 + 0.55 * limb), 0.0, 1.0);
      col = q > 0.62 ? mix(GOLD_LIGHT, IVORY, (q - 0.62) / 0.38)
          : q > 0.32 ? mix(GOLD, GOLD_LIGHT, (q - 0.32) / 0.30)
                     : mix(CINNABAR, GOLD_DEEP, q / 0.32);
    } else {
      // 5 · 水星 / 银霜月相（备用旧版分支）
      float phase = smoothstep(0.02, 0.28, lam);
      vec3 craterTone = mix(vec3(0.46, 0.52, 0.63), vec3(0.96, 0.97, 0.99), clamp(surf.r * 0.78 + surf.g * 0.26, 0.0, 1.0));
      vec3 earthshine = vec3(0.20, 0.29, 0.48) * (0.78 + 0.32 * surf.r);
      col = mix(earthshine, craterTone * (0.82 + 0.24 * lam), phase);
      rimGlow = vec3(0.84, 0.92, 1.00) * rim * (0.28 + 0.40 * phase);
      q = clamp(0.18 + phase * 0.62 + surf.g * 0.20, 0.0, 1.0);
    }

    vec3 base = mix(col * 0.82, col, 0.38);
    float cover = glyphOnSphere(clamp(floor(q * 9.0 + 0.5), 0.0, 9.0), local, cc, rc, R);
    vec3 ink = mix(col, IVORY, 0.36 * q);
    vec3 sphere = (mix(base, ink, cover * 0.88) + rimGlow) * lift;
    float edge = 1.0 - smoothstep(R - 1.2, R + 0.4, r);
    rgb = mix(rgb, sphere, edge);
    alpha = mix(alpha, 1.0, edge);
  } else if (style < 0.5 && rc < R + rayLen) {
    // 太阳 24 道点阵放射光芒
    float th = atan(cc.y, cc.x) - t * 0.02;
    float seg = PI / 6.0;
    float idx = floor(th / seg + 0.5);
    float off = abs(th - idx * seg) * rc;
    float len = rayLen * (0.62 + 0.3 * sin(t * 0.5 + idx * 1.7));
    float idx2 = floor((th - seg * 0.5) / seg + 0.5);
    float off2 = abs(th - seg * 0.5 - idx2 * seg) * rc;
    float len2 = rayLen * (0.38 + 0.18 * sin(t * 0.7 + idx2 * 2.3));
    float reach = rc - R;
    float a = 0.0;
    if (off < uCell.x * 0.7 && reach < len) a = 1.0 - reach / len;
    else if (off2 < uCell.x * 0.6 && reach < len2) a = 0.7 * (1.0 - reach / len2);
    if (a > 0.0) {
      float cover = glyph(a > 0.55 ? 15.0 : 14.0, local);
      vec3 col = mix(GOLD, GOLD_LIGHT, a);
      float k = clamp(cover * (0.35 + 0.65 * a), 0.0, 1.0);
      rgb = mix(rgb, col * lift, k);
      alpha = mix(alpha, 1.0, k);
    }
  }

  return vec4(clamp(rgb, 0.0, 1.0) * alpha, alpha);
}

// ======================= 细腻版：八大行星 1/2/3/4/5/6/7/8 =======================

// 某一点（相对球心的逻辑像素位置）的行星颜色。q 是这一点该用多密的字（0..1）。
vec3 shadePlanet(float style, vec2 pos, float R, vec4 surf, out float q, out vec3 rimGlow) {
  vec2 n2 = pos / R;
  float nz = sqrt(max(0.0, 1.0 - dot(n2, n2)));
  vec3 nrm = vec3(n2, nz);
  // 水星 5 保留侧逆光，呈现达·芬奇《地球照》冷暖明暗交界；其余行星走左上柔和主光。
  vec3 lightDir = (style > 4.5 && style < 5.5)
      ? normalize(vec3(-0.76, -0.34, 0.55))
      : normalize(vec3(-0.45, -0.55, 0.70));
  float lam = max(0.0, dot(nrm, lightDir));
  float limb = pow(nz, 0.45);
  float rim = pow(1.0 - nz, 2.6);
  vec3 col = GOLD;
  q = 0.5;
  rimGlow = vec3(0.0);

  if (style < 1.5) {
    // 1 · 土星：暖羊皮纸金与琥珀丝绸云带
    float tone = clamp((0.28 + 0.68 * lam) * (0.55 + 0.45 * limb), 0.0, 1.0);
    vec3 bandCol = mix(vec3(0.73, 0.56, 0.33), vec3(0.99, 0.93, 0.76), surf.r);
    col = mix(bandCol * 0.56, mix(bandCol, IVORY, 0.25), tone);
    rimGlow = vec3(1.00, 0.92, 0.72) * rim * 0.22;
    q = clamp(tone * 0.72 + surf.r * 0.28, 0.0, 1.0);
  } else if (style < 2.5) {
    // 2 · 木星：焦糖赭石/奶油象牙云带 + 自转朱砂大红斑
    float tone = clamp((0.24 + 0.72 * lam) * (0.52 + 0.48 * limb), 0.0, 1.0);
    vec3 belt = surf.r > 0.60
        ? mix(vec3(0.90, 0.75, 0.55), vec3(1.00, 0.95, 0.84), (surf.r - 0.60) / 0.40)
        : surf.r > 0.30
        ? mix(vec3(0.75, 0.44, 0.24), vec3(0.90, 0.75, 0.55), (surf.r - 0.30) / 0.30)
        : mix(vec3(0.49, 0.24, 0.14), vec3(0.75, 0.44, 0.24), surf.r / 0.30);
    belt = mix(belt, vec3(0.88, 0.34, 0.18), surf.g * 0.85);
    col = mix(belt * 0.48, belt * 1.08, tone);
    rimGlow = vec3(1.00, 0.86, 0.66) * rim * 0.20;
    q = clamp(tone * 0.68 + surf.r * 0.24 + surf.g * 0.20, 0.0, 1.0);
  } else if (style < 3.5) {
    // 3 · 地球：群青深海 + 孔雀翡翠陆地 + 飘动象牙白云 + 天青大气边辉
    float isLand = smoothstep(0.49, 0.54, surf.r);
    vec3 sea = mix(vec3(0.11, 0.26, 0.58), vec3(0.22, 0.50, 0.84), surf.r / 0.52);
    vec3 land = mix(vec3(0.20, 0.54, 0.38), vec3(0.66, 0.76, 0.44), clamp((surf.r - 0.50) * 2.2, 0.0, 1.0));
    vec3 ground = mix(sea, land, isLand);
    ground = mix(ground, IVORY, clamp(surf.g * 0.85 + surf.b * 0.82, 0.0, 0.92));
    float tone = clamp((0.24 + 0.76 * lam) * (0.56 + 0.44 * limb), 0.0, 1.0);
    col = mix(ground * 0.44, ground * 1.08, tone);
    rimGlow = vec3(0.56, 0.90, 1.00) * rim * (0.45 + 0.55 * lam);
    q = clamp(tone * 0.65 + surf.g * 0.30 + isLand * 0.12, 0.0, 1.0);
  } else if (style < 4.5) {
    // 4 · 火星：朱砂赤陶 + 暗铁锈地貌 + 北极白冰盖
    float tone = clamp((0.22 + 0.74 * lam) * (0.52 + 0.48 * limb), 0.0, 1.0);
    vec3 dust = mix(vec3(0.48, 0.19, 0.12), vec3(0.92, 0.52, 0.32), surf.r);
    dust = mix(dust, vec3(0.99, 0.96, 0.91), surf.g * 0.92);
    col = mix(dust * 0.46, dust * 1.08, tone);
    rimGlow = vec3(0.98, 0.66, 0.46) * rim * 0.32 * lam;
    q = clamp(tone * 0.68 + surf.r * 0.24 + surf.g * 0.22, 0.0, 1.0);
  } else if (style < 5.5) {
    // 5 · 水星 / 银霜月相：达·芬奇地球照冷暖明暗交界（细腻圆边 + 平滑月海与环形山）
    float phase = smoothstep(0.02, 0.32, lam);
    vec3 craterTone = mix(
        vec3(0.46, 0.52, 0.64),
        vec3(0.97, 0.98, 1.00),
        clamp(surf.r * 0.76 + surf.g * 0.28, 0.0, 1.0));
    vec3 earthshine = vec3(0.20, 0.30, 0.50) * (0.76 + 0.34 * surf.r);
    col = mix(earthshine, craterTone * (0.80 + 0.26 * lam), phase);
    col *= 0.62 + 0.38 * limb;
    rimGlow = vec3(0.84, 0.92, 1.00) * rim * (0.28 + 0.42 * phase);
    q = clamp(0.18 + phase * 0.60 + surf.g * 0.22, 0.0, 1.0);
  } else if (style < 6.5) {
    // 6 · 天王星：孔雀青琉璃冰球
    float tone = clamp((0.30 + 0.68 * lam) * (0.58 + 0.42 * limb), 0.0, 1.0);
    vec3 ice = mix(vec3(0.20, 0.62, 0.74), vec3(0.76, 0.97, 0.98), surf.r);
    ice = mix(ice, IVORY, surf.g * 0.45);
    col = mix(ice * 0.56, ice * 1.06, tone);
    rimGlow = vec3(0.76, 0.99, 1.00) * rim * 0.52;
    q = clamp(tone * 0.72 + surf.r * 0.24, 0.0, 1.0);
  } else if (style < 7.5) {
    // 7 · 海王星：深邃青金琉璃 + 高速银白卷云 + 深蓝大暗斑
    float tone = clamp((0.24 + 0.74 * lam) * (0.54 + 0.46 * limb), 0.0, 1.0);
    vec3 blue = mix(vec3(0.13, 0.28, 0.64), vec3(0.32, 0.60, 0.96), surf.r);
    blue = mix(blue, vec3(0.08, 0.16, 0.42), surf.b * 0.72);
    blue = mix(blue, vec3(0.90, 0.96, 1.00), surf.g * 0.78);
    col = mix(blue * 0.48, blue * 1.10, tone);
    rimGlow = vec3(0.56, 0.85, 1.00) * rim * 0.60;
    q = clamp(tone * 0.66 + surf.g * 0.34, 0.0, 1.0);
  } else {
    // 8 · 金星：香槟珍珠与玫瑰金旋涡厚云
    float tone = clamp((0.32 + 0.66 * lam) * (0.58 + 0.42 * limb), 0.0, 1.0);
    vec3 veil = mix(vec3(0.84, 0.60, 0.48), vec3(1.00, 0.94, 0.82), surf.r);
    veil = mix(veil, IVORY, surf.g * 0.45);
    col = mix(veil * 0.58, veil * 1.06, tone);
    rimGlow = vec3(1.00, 0.90, 0.76) * rim * 0.42;
    q = clamp(tone * 0.70 + surf.r * 0.28, 0.0, 1.0);
  }
  return col;
}

// 行星环的一层：layer 0 是主层字，layer 1 是插缝层字（另一套字形 + 沿环绕行的细碎流光），layer 2 是逐像素的淡底纱。
// 返回 (颜色, 覆盖度)。
vec4 ringLayer(vec2 cc, vec2 local, float R, float tiltRad, float squashY, float innerF, float gapA, float gapB,
               float outerF, vec3 colInner, vec3 colOuter, float lift, float t, float layer, out float ringY) {
  float cs = cos(tiltRad), sn = sin(tiltRad);
  vec2 rp = vec2(cc.x * cs - cc.y * sn, cc.x * sn + cc.y * cs);
  ringY = rp.y;
  float rr = length(vec2(rp.x, rp.y * squashY)) / R;
  if (rr < innerF || rr > outerF) return vec4(0.0);
  float band = (gapB > gapA && rr > gapA && rr < gapB) ? 0.12 : 1.0;
  float edgeFade = smoothstep(innerF, innerF + 0.07, rr) * (1.0 - smoothstep(outerF - 0.08, outerF, rr));
  float stripe = 0.68 + 0.32 * sin(rr * 34.0 + layer * 1.6);
  float ang = atan(rp.y * squashY, rp.x);
  float flow = layer * 0.20 * sin(ang * 7.0 - t * 0.6 + rr * 13.0);
  float density = clamp(edgeFade * band * stripe + flow * edgeFade * band, 0.0, 1.0);
  if (density < 0.06) return vec4(0.0);
  float tCol0 = clamp((rr - innerF) / max(0.01, outerF - innerF), 0.0, 1.0);
  if (layer > 1.5) {
    // 逐像素的一层极淡底纱：环的轮廓是平滑的，不再是一格一格的方块
    float side = 0.78 + 0.28 * clamp((-rp.x * 0.6 - rp.y * 0.4) / (R * outerF), -1.0, 1.0);
    return vec4(mix(colInner, colOuter, tCol0) * side * lift, density * 0.20);
  }
  float gIdx = layer * RAMP_B + clamp(floor(density * 6.0 + 1.0), 0.0, 8.0);
  float cov = glyph(gIdx, local);
  float tCol = clamp((rr - innerF) / max(0.01, outerF - innerF), 0.0, 1.0);
  float lightSide = 0.78 + 0.28 * clamp((-rp.x * 0.6 - rp.y * 0.4) / (R * outerF), -1.0, 1.0);
  vec3 c = mix(colInner, colOuter, tCol) * lightSide * lift;
  c = mix(c, IVORY * lift, layer * 0.18 * density);
  float a = layer > 0.5
      ? clamp(density * cov, 0.0, 0.96) * 0.62
      : clamp(density * cov, 0.0, 0.96);
  return vec4(c, a);
}

void over(inout vec3 rgb, inout float alpha, vec4 layerPx) {
  if (layerPx.a <= 0.0) return;
  rgb = mix(rgb, layerPx.rgb, layerPx.a);
  alpha = clamp(alpha + layerPx.a * (1.0 - alpha), 0.0, 1.0);
}

vec4 fineBody(vec2 p) {
  vec2 c = uBox * 0.5;
  vec2 d = p - c;
  float r = length(d);
  float R = uBody.x;
  float sel = uBody.z;
  float style = uBody.w;
  float t = uTime;
  float lift = 1.0 + 0.15 * sel;

  vec3 rgb;
  float alpha;
  haloOf(style, r, R, sel, lift, rgb, alpha);

  // 主层 A：每行错开半格（与背景天穹一致）
  vec2 gp = p / uCell;
  float row = floor(gp.y);
  float shift = mod(row, 2.0) * 0.5;
  gp.x += shift;
  vec2 cellA = floor(gp);
  vec2 localA = fract(gp);
  vec2 ccA = vec2((cellA.x + 0.5 - shift) * uCell.x, (cellA.y + 0.5) * uCell.y) - c;
  // 插缝层 B：落在 A 字之间的空隙里——上下两行 A 字的正中间、左右也错开，
  // 所以 B 不会和 A 连成横线，而是把 A 的行缝织满（像铜版画的交叉排线）。
  vec2 gpB = p / uCell - vec2(0.0, 0.5);
  float rowB = floor(gpB.y);
  float shiftB = mod(rowB, 2.0) * 0.5 + 0.25;
  gpB.x += shiftB;
  vec2 cellB = floor(gpB);
  vec2 localB = fract(gpB);
  vec2 ccB = vec2((cellB.x + 0.5 - shiftB) * uCell.x, (cellB.y + 1.0) * uCell.y) - c;

  // 带环星球：土星 1 双环 / 天王星 6 陡倾冰环，两层字叠在一起。
  bool hasRing = style < 1.5 || (style > 5.5 && style < 6.5);
  float ringYA = 0.0;
  float ringYB = 0.0;
  vec4 ringA = vec4(0.0);
  vec4 ringB = vec4(0.0);
  float ringYS = 0.0;
  vec4 ringS = vec4(0.0);
  if (hasRing) {
    if (style < 1.5) {
      ringA = ringLayer(ccA, localA, R, -0.37, 3.15, 1.24, 1.56, 1.66, 1.98,
                        vec3(1.00, 0.95, 0.80), vec3(0.84, 0.66, 0.38), lift, t, 0.0, ringYA);
      ringB = ringLayer(ccB, localB, R, -0.37, 3.15, 1.24, 1.56, 1.66, 1.98,
                        vec3(1.00, 0.95, 0.80), vec3(0.84, 0.66, 0.38), lift, t, 1.0, ringYB);
      ringS = ringLayer(d, vec2(0.5), R, -0.37, 3.15, 1.24, 1.56, 1.66, 1.98,
                        vec3(1.00, 0.95, 0.80), vec3(0.84, 0.66, 0.38), lift, t, 2.0, ringYS);
    } else {
      ringA = ringLayer(ccA, localA, R, 1.02, 3.35, 1.44, 0.0, 0.0, 1.66,
                        vec3(0.88, 0.99, 1.00), vec3(0.56, 0.88, 0.96), lift, t, 0.0, ringYA);
      ringB = ringLayer(ccB, localB, R, 1.02, 3.35, 1.44, 0.0, 0.0, 1.66,
                        vec3(0.88, 0.99, 1.00), vec3(0.56, 0.88, 0.96), lift, t, 1.0, ringYB);
      ringS = ringLayer(d, vec2(0.5), R, 1.02, 3.35, 1.44, 0.0, 0.0, 1.66,
                        vec3(0.88, 0.99, 1.00), vec3(0.56, 0.88, 0.96), lift, t, 2.0, ringYS);
    }
    // 后半环画在球后面
    if (ringYS < 0.0) over(rgb, alpha, ringS);
    if (ringYB < 0.0) over(rgb, alpha, ringB);
    if (ringYA < 0.0) over(rgb, alpha, ringA);
  }

  // 球体：逐像素平滑的哑光底 + B 插缝层 + A 主层
  float edge = 1.0 - smoothstep(R - 1.0, R + 0.3, r);
  if (edge > 0.0) {
    float qP;
    vec3 rimP;
    vec3 colP = shadePlanet(style, d, R, surfaceFine(p), qP, rimP);
    // 中间调的字压淡后，底色稍提亮一点，整体亮度不掉
    vec3 sphere = colP * mix(0.70, 0.76, VERT);

    float rcB = length(ccB);
    if (rcB < R) {
      float qB;
      vec3 rimB;
      vec3 colB = shadePlanet(style, ccB, R, surfaceFine(ccB + c), qB, rimB);
      // 一道慢慢流过的波，让插缝层的字疏密跟着起伏，和主层错开、套印
      float waveH = sin(ccB.x * 0.23 - t * 1.2 + ccB.y * 0.11);
      // 竖向抵消：波沿球面经线走（贴着球、两头收拢的竖弧线），随时间慢慢绕球转过去
      vec2 nB = ccB / R;
      float lonB = atan(nB.x, sqrt(max(0.0, 1.0 - dot(nB, nB))));
      float waveV = sin(lonB * 9.0 - t * 0.55);
      float wave = mix(waveH, waveV, VERT);
      float qB2 = clamp(qB * 0.94 + wave * mix(0.08, 0.06, VERT), 0.0, 1.0);
      float rampB = mix(RAMP_B, RAMP_V, VERT);
      // 去格纹：插缝层只是点缀——只在经线波峰经过处和亮面出现，且每格按随机数决定出不出现，
      // 所以它是成片流过、若隐若现的竖向笔触，不会织成一整张网。
      float crest = smoothstep(0.55, 0.95, waveV * 0.5 + 0.5);
      float lit = smoothstep(0.55, 0.90, qB);
      float presence = clamp(crest * 0.85 + lit * 0.40, 0.0, 1.0);
      float hB = hash21(cellB + vec2(3.7, 11.3));
      float showB = mix(1.0, smoothstep(hB - 0.08, hB + 0.08, presence), VERT);
      float levelB = floor(qB2 * 9.0 + 0.5 + (hash21(cellB + 7.1) - 0.5) * 1.2 * VERT);
      vec2 jitB = (vec2(hash21(cellB + 1.9), hash21(cellB + 5.3)) - 0.5) * vec2(0.14, 0.10) * VERT;
      float covB = glyphOnSphere(rampB + clamp(levelB, 0.0, 9.0), localB + jitB, ccB, rcB, R) * showB;
      vec3 inkB = mix(colB * 1.06, IVORY, 0.06 + 0.24 * qB2);
      sphere = mix(sphere, inkB, covB * (mix(0.42, 0.46, VERT) + 0.30 * qB2));
    }

    float rcA = length(ccA);
    if (rcA < R) {
      float qA;
      vec3 rimA;
      vec3 colA = shadePlanet(style, ccA, R, surfaceFine(ccA + c), qA, rimA);
      // 去格纹：相邻格子在相近的几级疏密里随机挑，位置也各自错开一点点，眼睛抓不到重复的格子
      float levelA = floor(qA * 9.0 + 0.5 + (hash21(cellA + 2.3) - 0.5) * 1.3 * VERT);
      vec2 jitA = (vec2(hash21(cellA + 4.1), hash21(cellA + 8.7)) - 0.5) * vec2(0.14, 0.10) * VERT;
      float covA = glyphOnSphere(clamp(levelA, 0.0, 9.0), localA + jitA, ccA, rcA, R);
      vec3 inkA = mix(colA * 1.12, IVORY, 0.10 + 0.30 * qA);
      // 中间调的字压淡，字的对比留给亮面和明暗交界（交界处 q 约 0.2~0.4）
      float accent = max(smoothstep(0.55, 0.90, qA), 1.0 - smoothstep(0.08, 0.30, abs(qA - 0.30)));
      float wA = mix(0.92, 0.58 + 0.34 * accent, VERT);
      sphere = mix(sphere, inkA, covA * wA);
    }

    sphere = (sphere + rimP) * lift;
    rgb = mix(rgb, sphere, edge);
    alpha = mix(alpha, 1.0, edge);
  }

  // 前半环画在球前面
  if (hasRing) {
    if (ringYS >= 0.0) over(rgb, alpha, ringS);
    if (ringYB >= 0.0) over(rgb, alpha, ringB);
    if (ringYA >= 0.0) over(rgb, alpha, ringA);
  }

  return vec4(clamp(rgb, 0.0, 1.0) * alpha, alpha);
}

void main() {
  vec2 p = FlutterFragCoord().xy;
  float style = uBody.w;
  // 太阳（0）与旧版恒星（9）走 legacyBody，八大行星（1..8）统一走细腻平滑版 fineBody。
  bool legacy = style < 0.5 || style > 8.5;
  vec4 o = legacy ? legacyBody(p) : fineBody(p);
  if (uHaze > 0.0) {
    // 输出是预乘透明度的颜色：往“天空蓝 × 透明度”混，再整体略淡
    o.rgb = mix(o.rgb, vec3(0.62, 0.74, 0.90) * o.a, uHaze * 0.45);
    o *= 1.0 - 0.22 * uHaze;
  }
  fragColor = o;
}
