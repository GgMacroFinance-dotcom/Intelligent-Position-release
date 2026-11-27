// 天穹的背景层（设计重构，2026-11-25 维护者定）：全部用程序画，不用任何图片。
// 文艺复兴的人仰望的天空：明亮、温暖、有秩序。参考圣洛伦佐老圣器室的星空穹顶（约 1442）和
// 法尔内西纳别墅的天顶（约 1511），参考图在 design-refactor/references/。
//   天空铺满整个窗口：上面群青，往下亮成明亮的天青，最下面带一点暖。
//   弧光：一道金色的光带从左到右拱过天空（不是线），横截面是几层柔和的渐变叠起来，没有硬边。
//         粗细、亮度沿着弧几乎一样（中段宽 40、两头宽 36，两头亮度是中段的 75%，变化平缓），
//         两端几乎不淡出，一直延伸到窗口边；不截断亮度，横截面一直是完整的渐变（2026-11-25 维护者定：中段不要鼓、两端不怎么淡出）。
//         一团亮一点的光沿着弧慢慢走，整道光轻轻起伏。
//   云：三五团象牙白的字符画（每格 4 像素，每行错开半格，用居中的字形，不排成横纹），
//       团和团之间留出大块干净的天空；很慢地飘。（2026-11-25 维护者定：云保持这一版）
//   星：分远、中、近三层，一共十来颗，成簇：远的小、淡、偏冷；中的金色；近的一两颗大八角星带光晕。
// 没有轨道、黄道带、拉丁名、星座人物；太阳以后是单独一层。
// 云的厚薄不在这里算：cloud.frag 每秒几次把每个字符格的云厚画进一张小图（一格一个像素），这里只查表
// （找边要的上下左右四格也是查表），GPU 的活少很多。按 Flutter 的 FragmentProgram 写。

#version 460 core
precision mediump float;

#include <flutter/runtime_effect.glsl>

uniform vec2 uSize;     // 画布大小（逻辑像素）
uniform float uTime;    // 秒
uniform float uDpr;
uniform vec2 uCell;     // 一格字的宽和高（逻辑像素）
uniform vec4 uDome;     // 天空的中心 x、y，半轴 a、b（只用来排天空的明暗和云）
uniform sampler2D uAtlas;
uniform sampler2D uCloud;   // 云厚的小图：一格一个像素（cloud.frag 画的）
uniform vec2 uGrid;         // 小图的大小（格子数）

out vec4 fragColor;

const vec2 GRID = vec2(8.0, 5.0); // 字表 8 列 5 行（第 4、5 行是行星插缝层的字，天穹不用）
const float PI = 3.14159265;

// 颜色（参考图里取的）。
const vec3 ULTRA = vec3(0.200, 0.320, 0.660);        // 群青（老圣器室穹顶的上部）
const vec3 AZURE = vec3(0.420, 0.590, 0.820);        // 天青（法尔内西纳）
const vec3 AZURE_BRIGHT = vec3(0.640, 0.770, 0.910); // 明亮的天青（下部）
const vec3 WARM = vec3(0.960, 0.880, 0.760);         // 靠下的暖光
const vec3 IVORY = vec3(0.980, 0.950, 0.890);        // 云的亮面
const vec3 IVORY_WARM = vec3(0.990, 0.910, 0.820);   // 云的受光边（一点桃色，像法尔内西纳的云）
const vec3 CLOUD_SHADE = vec3(0.700, 0.740, 0.850);  // 云的暗面（带一点天的蓝）
const vec3 GOLD = vec3(0.880, 0.690, 0.310);         // 金箔
const vec3 GOLD_LIGHT = vec3(1.000, 0.900, 0.620);
const vec3 STAR_COLD = vec3(0.860, 0.880, 0.960);    // 远处的星

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

// 字表：0 到 9 由疏到密（· ∙ : • + * o % # @），10 到 13 是线（| ~ / \），14 到 17 是星（· • ✦ ∙）。
float glyph(float g, vec2 local) {
  vec2 cell = vec2(mod(g, GRID.x), floor(g / GRID.x));
  return texture(uAtlas, (cell + local) / GRID).a;
}

// 查某个字符格的云有多厚（格子坐标，每行已经错开半格）。超出小图的格子当作没有云。
float cloudCell(vec2 id) {
  if (id.x < 0.0 || id.y < 0.0 || id.x >= uGrid.x || id.y >= uGrid.y) return 0.0;
  return texture(uCloud, (id + 0.5) / uGrid).r;
}

// 查一点的颜料不均匀（G）和星成簇（B）：按像素位置在小图上插值，变化很慢，插值看不出来。
vec2 slowAt(vec2 q) {
  return texture(uCloud, clamp(q / uCell / uGrid, vec2(0.0), vec2(1.0))).gb;
}

// 金色八角星（程序画，不用字）：中心一点，四道长光、四道短光。
float starShape(vec2 v, float rad) {
  float r = length(v);
  float core = 1.0 - smoothstep(rad * 0.18, rad * 0.32, r);
  float aa = 0.8 / uDpr;
  float w = 0.45;
  float ax = (1.0 - smoothstep(w, w + aa, abs(v.y))) * (1.0 - smoothstep(rad * 0.2, rad, abs(v.x)));
  float ay = (1.0 - smoothstep(w, w + aa, abs(v.x))) * (1.0 - smoothstep(rad * 0.2, rad, abs(v.y)));
  vec2 q = vec2(v.x + v.y, v.x - v.y) * 0.70710678;
  float dx = (1.0 - smoothstep(w, w + aa, abs(q.y))) * (1.0 - smoothstep(rad * 0.1, rad * 0.55, abs(q.x)));
  float dy = (1.0 - smoothstep(w, w + aa, abs(q.x))) * (1.0 - smoothstep(rad * 0.1, rad * 0.55, abs(q.y)));
  return clamp(max(max(core, max(ax, ay)), max(dx, dy) * 0.8), 0.0, 1.0);
}

// 一层星：每 cell 见方一格，大约 chance 的格子有一颗；簇由一层大噪声决定（只有噪声高的地方才放）。
vec4 starLayer(vec2 p, float t, float cell, float chance, float radA, float radB, vec3 colA, vec3 colB,
               float twinkle, float glowAmt, float seed, float cloudCover) {
  vec2 sid = floor(p / cell);
  float h = hash21(sid * vec2(3.1, 7.7) + vec2(11.0 + seed, 5.0));
  vec2 jitter = vec2(hash21(sid + 1.7 + seed), hash21(sid + 9.3 + seed)) * 0.6 + 0.2;
  vec2 sc = (sid + jitter) * cell;
  // 成簇：查小图（原来每个像素一次 3 层噪声）；三层星各自错开一点，不在同一个地方扎堆。
  float cluster = smoothstep(0.45, 0.7, slowAt(sc + vec2(seed * 97.0, seed * 53.0)).y);
  if (h > chance * cluster) return vec4(0.0);
  float rad = mix(radA, radB, hash21(sid + 4.4 + seed));
  float h2 = hash21(sid + 2.2 + seed);
  float tw = twinkle > 0.0 ? 1.0 - twinkle * (0.5 - 0.5 * sin(t * (0.4 + h2 * 0.8) + h * 900.0)) : 1.0;
  float s = starShape(p - sc, rad * (0.85 + 0.15 * tw));
  float glow = exp(-dot(p - sc, p - sc) / (rad * rad * 3.5)) * glowAmt;
  float a = clamp((s * (0.6 + 0.4 * tw) + glow) * (1.0 - cloudCover), 0.0, 1.0);
  return vec4(mix(colA, colB, tw), a);
}

void main() {
  vec2 p = FlutterFragCoord().xy;
  float t = uTime;
  vec2 S = uSize;

  vec2 d = (p - uDome.xy) / uDome.zw;
  float e = length(d);

  // ---------- 天空 ----------
  float yy = clamp(p.y / S.y, 0.0, 1.0);             // 0 在最上面
  vec3 sky = mix(ULTRA, AZURE, smoothstep(0.0, 0.42, yy));
  sky = mix(sky, AZURE_BRIGHT, smoothstep(0.35, 0.85, yy));
  sky = mix(sky, mix(AZURE_BRIGHT, WARM, 0.55), smoothstep(0.72, 1.05, yy) * (1.0 - abs(d.x) * 0.35) * 0.6);
  sky = mix(sky, AZURE_BRIGHT, 0.12 * (1.0 - smoothstep(0.0, 0.9, e)));
  // 细颗粒：一个便宜的哈希（不插值），像颜料里的砂；大块的深浅查小图。
  float grain = hash21(floor(p / 1.6)) - 0.5;
  float wash = slowAt(p).x - 0.5;
  sky *= 1.0 + grain * 0.035 + wash * 0.07;

  // ---------- 弧光 ----------
  // 圆心在窗口下面很远，弧从左边拱过天的上半部到右边（位置不变）。
  vec2 c1 = vec2(S.x * 0.5, S.y * 1.85);
  float r1 = S.y * 1.52;
  float dist = length(p - c1) - r1;                   // 离弧多远（正是外侧）
  float th = atan(p.y - c1.y, p.x - c1.x);            // 弧的最高点在 -PI/2
  float along = (th + PI * 0.5) / 0.75;               // 0 在中间，两边到窗口边大约是 ±1
  // 沿着弧：按窗口的横向位置算（0 在中间，1 在左右边），粗细和亮度都只是平缓地变一点，中段不鼓起来。
  float side = abs(p.x / S.x - 0.5) * 2.0;
  float flat2 = 1.0 - side * side;                    // 中间 1，左右边 0，变化平缓
  float strength = mix(0.75, 1.0, flat2);             // 两头是中段的 75%
  // 只在最贴边的 2% 里轻轻收一点（到边上还留 80%），看起来是一直延伸出窗口。
  strength *= mix(0.8, 1.0, smoothstep(0.0, 0.02, p.x / S.x) * smoothstep(1.0, 0.98, p.x / S.x));
  float halfW = mix(18.0, 20.0, flat2);               // 光带的半宽（逻辑像素）：中段宽 40，两头宽 36
  // 横截面：三层柔和的渐变叠起来，没有硬的芯：中间暖亮，往外越来越淡、越来越宽。
  // 每层的亮度调低，中段最亮处自然在 0.7 左右，不用截断，横截面没有平顶。
  float inner = exp(-pow(dist / (halfW * 0.55), 2.0));
  float mid = exp(-pow(dist / halfW, 2.0));
  float outer = exp(-pow(dist / (halfW * 2.6), 2.0));
  float pulse = 0.92 + 0.08 * sin(t * 0.35);
  float walker = exp(-pow((along - 0.85 * sin(t * 0.045)) * 2.6, 2.0));
  float light = (inner * 0.267 + mid * 0.25 + outer * 0.183) * strength * pulse * (1.0 + 0.35 * walker);
  // 颜色也从里到外渐变：中间浅金，往外是暖的金，再往外只是一点暖。
  vec3 glowCol = mix(mix(WARM, GOLD, 0.55), GOLD_LIGHT, clamp(inner * 0.9 + walker * 0.2, 0.0, 1.0));

  // ---------- 字符格子（每行错开半格） ----------
  vec2 gp = p / uCell;
  float row = floor(gp.y);
  float shift = mod(row, 2.0) * 0.5;
  gp.x += shift;
  vec2 cellId = floor(gp);
  vec2 local = fract(gp);
  vec2 cc = vec2((cellId.x + 0.5 - shift) * uCell.x, (cellId.y + 0.5) * uCell.y);

  float c = cloudCell(cellId);
  vec4 ink = vec4(0.0);
  float cover = 0.0;
  if (c > 0.04) {
    // 边的方向：看左右上下的云厚。
    // 上下两行错开了半格：上下的邻格取同一列，差半格看不出来。
    float cl = cloudCell(cellId - vec2(1.0, 0.0)), cr = cloudCell(cellId + vec2(1.0, 0.0));
    float cu = cloudCell(cellId - vec2(0.0, 1.0)), cd = cloudCell(cellId + vec2(0.0, 1.0));
    float gx = cr - cl, gy = cd - cu;
    float edge = length(vec2(gx, gy));
    // 云的受光：上面亮、带一点暖，下面带一点天的蓝。
    float lit = clamp(0.62 - gy * 3.0, 0.0, 1.0);
    vec3 cloudCol = mix(CLOUD_SHADE, mix(IVORY, IVORY_WARM, smoothstep(0.6, 1.0, lit) * 0.6), lit);
    if (edge > 0.2 && c > 0.15) {
      float ax = abs(gx), ay = abs(gy);
      float g = ay < ax * 0.414 ? 10.0 : (ax < ay * 0.414 ? 11.0 : (gx * gy > 0.0 ? 13.0 : 12.0));
      ink = vec4(mix(cloudCol, IVORY, 0.3), clamp(0.55 + edge, 0.0, 0.95));
      cover = glyph(g, local);
    } else {
      // 云边上字稀、颜色淡，往里越来越密（软的边）。
      ink = vec4(cloudCol, 0.35 + 0.6 * c);
      cover = glyph(floor(c * 9.0 + 0.5), local);
    }
  } else {
    // 干净的天空：极少的淡点，只当颜料的质感。
    float h = hash21(cellId * vec2(7.0, 13.0) + vec2(3.0, 1.0));
    if (h < 0.010) {
      ink = vec4(mix(sky, IVORY, 0.35), 0.28);
      cover = glyph(0.0, local);
    }
  }

  // 远处的星在最后面。
  vec3 col = sky;
  vec4 far = starLayer(p, t, 54.0, 0.22, 1.4, 2.2, STAR_COLD, vec3(1.0), 0.0, 0.0, 1.0, c);
  col = mix(col, far.rgb, far.a * 0.55);

  // 弧光在云的后面：云厚的地方光被盖住。
  col = mix(col, glowCol, clamp(light * (1.0 - c * 0.85), 0.0, 1.0));

  // 中间的星：金色，偶尔闪。
  vec4 midStar = starLayer(p, t, 96.0, 0.28, 2.6, 4.2, GOLD, GOLD_LIGHT, 0.4, 0.18, 7.0, c);
  col = mix(col, midStar.rgb, midStar.a);

  // 云。
  col = mix(col, ink.rgb, clamp(ink.a * cover, 0.0, 1.0));

  // 近处的星：一两颗大的八角星，带光晕，慢慢闪。
  vec4 nearStar = starLayer(p, t, 240.0, 0.12, 5.5, 7.5, GOLD, GOLD_LIGHT, 0.55, 0.45, 13.0, c);
  col = mix(col, nearStar.rgb, nearStar.a);

  fragColor = vec4(col, 1.0);
}
