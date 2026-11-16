// 推荐模型配置决策器：按 Bot 的角色、任务类型、数据敏感度、调用量、时延和质量要求，
// 给出推荐的模型、effort 档位、理由和成本。纯函数，无 DOM 依赖，可在 node 里直接测试。
// 模型表来自 Claude API 文档（2026-10 缓存），价格为每百万 token 美元；换厂商或换型号只改 MODELS。
(function (root) {
  "use strict";

  // 模型表（价格：每百万 token 美元，2026-11-09 查得）。
  // src：claude = Claude API 文档缓存价目；official = 厂商官方定价页（部分为摘要）；third = 第三方汇总，需复核。
  // 档位（tier 1 小 / 2 中 / 3 强 / 4 最强）是按价格和命名推断的，没做过评测，上线前要用自己的任务验证。
  const MODELS = {
    haiku:   { vendor: "claude", id: "claude-haiku-5-5",  label: "Claude Haiku 5.5",  inPer1M: 0.1,  outPer1M: 0.5,  src: "claude",   note: "最快最便宜，适合固定流程与高频任务" },
    sonnet:  { vendor: "claude", id: "claude-sonnet-5-5", label: "Claude Sonnet 5.5", inPer1M: 2,    outPer1M: 10,   src: "claude",   note: "速度与能力均衡，适合写作、摘要、工具调用" },
    opus:    { vendor: "claude", id: "claude-opus-5-5",   label: "Claude Opus 5.5",   inPer1M: 4,    outPer1M: 20,   src: "claude",   note: "规划、判断与最终把关" },
    fable:   { vendor: "claude", id: "claude-fable-5-1",  label: "Claude Fable 5.1",  inPer1M: 10,   outPer1M: 50,   src: "claude",   note: "最强，留给错了代价很高的环节" },
    luna:    { vendor: "openai", id: "gpt-6-luna",        label: "GPT-6 Luna",        inPer1M: 0.1,  outPer1M: 0.5,  src: "official", note: "OpenAI 小档，高频固定任务" },
    terra:   { vendor: "openai", id: "gpt-6.1-terra",     label: "GPT-6.1 Terra",     inPer1M: 1,    outPer1M: 5,    src: "official", note: "OpenAI 中档" },
    sol:     { vendor: "openai", id: "gpt-6.1-sol",       label: "GPT-6.1 Sol",       inPer1M: 2,    outPer1M: 10,   src: "official", note: "OpenAI 上位中档到旗舰之间" },
    astra:   { vendor: "openai", id: "gpt-6-astra",       label: "GPT-6 Astra",       inPer1M: 10,   outPer1M: 50,   src: "official", note: "OpenAI 最强" },
    glite:   { vendor: "google", id: "gemini-3.1-flash-lite", label: "Gemini 3.1 Flash-Lite", inPer1M: 0.25, outPer1M: 1.5, src: "official", note: "Google 小档（价格页更新于 2026-06）" },
    gflash:  { vendor: "google", id: "gemini-3.5-flash",  label: "Gemini 3.5 Flash",  inPer1M: 1.5,  outPer1M: 9,    src: "official", note: "Google 中档（价格页更新于 2026-06）" },
    gpro:    { vendor: "google", id: "gemini-3.1-pro-preview", label: "Gemini 3.1 Pro（预览）", inPer1M: 2, outPer1M: 12, src: "official", note: "Google 强档，200K 以内的价格；预览版型号会变" },
    dflash:  { vendor: "deepseek", id: "deepseek-flash",  label: "DeepSeek Flash",    inPer1M: 0.3,  outPer1M: 1.2,  src: "third",    note: "DeepSeek 小档，按高峰价；价格来自第三方汇总" },
    dpro:    { vendor: "deepseek", id: "deepseek-v4-pro", label: "DeepSeek V4 Pro",   inPer1M: 1.32, outPer1M: 3.96, src: "third",    note: "DeepSeek 旗舰，按高峰价；价格来自第三方汇总" },
  };
  const VENDORS = {
    claude:   { label: "Claude",   ladder: ["haiku", "sonnet", "opus", "fable"] },
    openai:   { label: "OpenAI GPT", ladder: ["luna", "terra", "sol", "astra"] },
    google:   { label: "Google Gemini", ladder: ["glite", "gflash", "gpro", "gpro"] },
    deepseek: { label: "DeepSeek", ladder: ["dflash", "dflash", "dpro", "dpro"] },
  };
  const SRC_LABEL = { claude: "Claude API 文档", official: "厂商官方定价页", third: "第三方汇总，需复核" };
  // 国内的 Kimi / Qwen / GLM / MiniMax 价格只查到第三方汇总、没有确切的 API 型号 ID，先不放进模型表。
  const ORDER = ["haiku", "sonnet", "opus", "fable"]; // 兼容旧代码：Claude 阶梯
  const EFFORTS = ["low", "medium", "high", "xhigh", "max"];

  // 每次调用的假设 token 量（示意，换成真实日志后更准）
  const TOKENS = {
    coordinator: { input: 8000, output: 2000 },
    executor: { input: 3000, output: 500 },
  };

  // 任务类型 -> 执行 Bot 的起点档位与 effort
  const TASKS = {
    plan:      { label: "规划与拆解",       tier: "opus",   effort: "high" },
    write:     { label: "写作与观点",       tier: "sonnet", effort: "medium" },
    tool:      { label: "工具调用与代码",   tier: "sonnet", effort: "high" },
    summarize: { label: "长文摘要与抽取",   tier: "sonnet", effort: "medium" },
    data:      { label: "数据拉取与计算",   tier: "haiku",  effort: "low" },
    format:    { label: "格式化与改写",     tier: "haiku",  effort: "low" },
  };

  const clamp = (n, lo, hi) => Math.max(lo, Math.min(hi, n));
  const ladderOf = (vendor) => (VENDORS[vendor] || VENDORS.claude).ladder;
  // "auto"：每一档里选最便宜的模型
  function pickModel(vendor, tier) {
    if (vendor === "auto") {
      const cands = Object.keys(VENDORS).map((v) => VENDORS[v].ladder[tier]);
      return cands.sort((a, b) => MODELS[a].inPer1M * 3 + MODELS[a].outPer1M - (MODELS[b].inPer1M * 3 + MODELS[b].outPer1M))[0];
    }
    return ladderOf(vendor)[tier];
  }
  const money = (v) => "$" + v.toFixed(v < 1 ? 3 : 2);

  function callCost(modelKey, role) {
    const m = MODELS[modelKey], t = TOKENS[role];
    return (t.input * m.inPer1M + t.output * m.outPer1M) / 1e6;
  }

  function decide(input) {
    const q = Object.assign(
      { role: "executor", task: "data", sensitivity: "public", volume: "mid", latency: "background", quality: "normal", vendor: "claude" },
      input || {}
    );
    const reasons = [];
    const warnings = [];
    let tier, effort;

    if (q.role === "coordinator") {
      tier = 2;
      effort = EFFORTS.indexOf("high");
      reasons.push("协调 Bot 负责拆解、分派和汇总，按架构的核心分工用云端最顶尖的模型：第 3 档（强）、推理强度 high。");
    } else {
      const t = TASKS[q.task] || TASKS.data;
      tier = ORDER.indexOf(t.tier);
      effort = EFFORTS.indexOf(t.effort);
      reasons.push(`执行 Bot 做「${t.label}」，起点第 ${tier + 1} 档（${["小", "中", "强", "最强"][tier]}）、推理强度 ${t.effort}。`);
    }

    if (q.quality === "critical") {
      tier = clamp(tier + 1, 0, 3);
      effort = clamp(effort + 1, 0, 3);
      reasons.push("质量要求是「错了代价很高」：升一档模型，effort 提高一级。");
    } else if (q.quality === "high") {
      effort = clamp(effort + 1, 0, 3);
      reasons.push("质量要求高：模型不变，effort 提高一级。");
    } else if (q.role === "executor" && q.volume === "high") {
      if (tier > 0) {
        tier -= 1;
        reasons.push("调用量大且质量要求一般：执行 Bot 降一档模型控制成本。");
      }
    }

    if (q.latency === "interactive" && q.quality !== "critical") {
      if (effort > 0) {
        effort -= 1;
        reasons.push("需要即时响应：effort 降一级，减少等待。");
      }
    }

    if (q.sensitivity === "internal") {
      warnings.push("内部数据：确认调用走的是客户自己的企业或 Team 账号（或 API），并关闭训练用途。");
    }
    let blocked = false;
    if (q.sensitivity === "secret") {
      blocked = true;
      warnings.push("涉密数据不出内网：现阶段所有推荐都是云端模型，不满足。请先脱敏，或等 1B 本地专家模型上线。这一步需要人类决策者确认。");
    }

    const modelKey = pickModel(q.vendor, tier);
    const effortName = EFFORTS[effort];
    const cost = callCost(modelKey, q.role);
    const baselineKey = pickModel(q.vendor, 2);
    const baseline = callCost(baselineKey, q.role);
    const down = tier > 0 && pickModel(q.vendor, tier - 1) !== modelKey ? pickModel(q.vendor, tier - 1) : null;
    const up = tier < 3 && pickModel(q.vendor, tier + 1) !== modelKey ? pickModel(q.vendor, tier + 1) : null;

    return {
      input: q,
      modelKey,
      model: MODELS[modelKey],
      effort: effortName,
      tier: tier + 1,
      baseline: MODELS[baselineKey],
      srcLabel: SRC_LABEL[MODELS[modelKey].src],
      // Claude 的 thinking 默认就是自适应；其他厂商的推理强度参数名不同，按各自文档映射
      thinking: MODELS[modelKey].vendor === "claude" ? "adaptive（默认，不需要设置）" : "按该厂商文档把推理强度映射到对应参数",
      plannedLocal: q.role === "executor" && !blocked ? "后续可换成 1B 专家模型（规划中）" : null,
      blocked,
      reasons,
      warnings,
      costPerCall: cost,
      costPer1k: cost * 1000,
      baselinePer1k: baseline * 1000,
      cheaper: down ? { key: down, label: MODELS[down].label, per1k: callCost(down, q.role) * 1000 } : null,
      stronger: up ? { key: up, label: MODELS[up].label, per1k: callCost(up, q.role) * 1000 } : null,
    };
  }

  // 宏观研报岗位里不同 Bot 的推荐配置表
  const MACRO_BOTS = [
    { name: "协调 Bot", desc: "拆解、分派、汇总、写核心观点", input: { role: "coordinator", task: "plan", quality: "normal", volume: "low", latency: "background" } },
    { name: "数据拉取 Bot", desc: "调用 Finance Macro Data API", input: { role: "executor", task: "data", volume: "high", quality: "normal" } },
    { name: "指标计算 Bot", desc: "同比环比、异动标注", input: { role: "executor", task: "data", volume: "high", quality: "high" } },
    { name: "政策摘要 Bot", desc: "逐份摘要政策文件", input: { role: "executor", task: "summarize", volume: "mid", quality: "normal" } },
    { name: "图表与排版 Bot", desc: "图表说明、研报格式", input: { role: "executor", task: "format", volume: "high", quality: "normal" } },
  ];

  const api = { MODELS, VENDORS, SRC_LABEL, ORDER, EFFORTS, TASKS, TOKENS, MACRO_BOTS, decide, money };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else root.IP_DECIDER = api;
})(typeof window !== "undefined" ? window : globalThis);
