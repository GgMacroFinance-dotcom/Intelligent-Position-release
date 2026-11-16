// I.P. 主页原型的全部文案与演示数据。改文案只需改这个文件。
// 带 demo: true 或 "示意" 的数字是占位值，上线前换成实测数据。
window.IP_DATA = {
  links: {
    github: "https://github.com/GgMacroFinance-dotcom/Intelligent-Position-release",
    apiDocs: "#api",
    contact: "#contact",
  },

  // 定位：有影响力的开源项目 + 一套全新的 AI 原生组织架构；FPDE 驻场、云主机托管是顺道的商业化
  hero: {
    eyebrow: "OPEN SOURCE · AI-NATIVE ORGANIZATION",
    titleA: "一套全新的",
    titleB: "AI 原生组织架构，",
    titleC: "开源共建。",
    sub: "I.P. 把公司里的岗位重新设计成 AI 原生的单元：协调 Bot 用云端最顶尖的模型，执行 Bot 现阶段也调用云端模型，并为后训练的专家小模型预留了位置；人类决策者只在关键节点修改方案。整个 harness 用 Rust 重写。宏观研报岗位是第一个展示。",
    tags: ["AI 原生组织架构", "协调 / 执行两层 Bot", "Rust 重写整个 harness", "70/20/5/5 上下文设计理念"],
    install: "cargo install ip-cli && ip init --position macro-research",
    trust: "Apache-2.0 开源 · 宏观研报岗位验收中 · 欢迎提交 PR",
  },

  arch: {
    title: "五层架构：把一个岗位重新设计成 AI 原生单元",
    sub: "传统公司里，人按部门分工，AI 只是工具。在 I.P. 架构里，岗位本身是 AI 原生的：协调 Bot 负责调度，执行 Bot 负责干活，人类决策者只在关键节点修改方案。",
  },
  paradigm: [
    ["组织方式", "人按部门分工，AI 是工具", "岗位是 AI 原生单元，人是决策者"],
    ["交付单元", "一次对话 / 一个 Agent", "一个岗位：职责、规范、电脑、记忆"],
    ["上下文", "临时写的 prompt", "70/20/5/5 设计理念 + 规章约束"],
    ["算力", "一个模型包打天下", "协调与执行分两层，都调用云端模型；执行层预留专家小模型"],
    ["运行环境", "共享环境", "1 岗 1 台云 Linux 主机"],
    ["人的角色", "全程盯着对话", "自动路由先给方案，人类决策者修改"],
  ],
  // oss：开源项目里有什么；biz：顺道的商业化服务（可为空）
  // pending：还没定的问题，页面上以"待定"显示，不写死
  layers: [
    {
      n: "L1", name: "岗位定义与上下文治理", key: "70/20/5/5 设计理念 · 规章约束 · 团队 Context · 权限分层",
      what: "定义一个岗位知道什么、按什么规矩做事。设计理念是 70/20/5/5：大部分来自公开 JD，一部分来自企业 FPDE，少量是联创意图，再留一点给 I.P. 助理自由发挥。团队 Context 管理系统负责多岗位协同，并按组织架构分层授权。",
      oss: "上下文组装器、岗位描述格式、JD 采集、团队 Context", biz: "FPDE 驻场：把企业规范注入岗位",
      pending: [],
    },
    {
      n: "L2", name: "协调 Bot + 执行 Bot", key: "协调：云端顶尖模型 · 执行：现阶段调用云端模型 · 预留 1B 专家",
      what: "负责调度协调的 Bot 用云端最顶尖的模型。下面负责执行的 Bot 现阶段也调用云端模型，可以选更快、更便宜的档位。执行层预留了专家小模型的接口：以后可以换成后训练的 1B 左右专家模型（约 3.6G 显存），我们预计未来 2–3 年会涌现大量这样的小模型。",
      oss: "Router、云端模型接入、专家模型接口", biz: "按岗位定制后训练（规划中）",
      pending: [],
    },
    {
      n: "L3", name: "Rust harness 与云 Linux 主机", key: "整个 harness 用 Rust 重写 · 1 岗 1 机 · information grab",
      what: "整个 harness 用 Rust 从零重写，充分利用硬件效率。每个 I.P. 一台独立的云 Linux 主机做环境隔离，协调与执行都在这里跑。Harness 负责连接企业内部数据，I.P. information grab 负责采集外部信息。",
      oss: "完整 Rust harness：调度、沙盒、连接器", biz: "云 Linux 主机托管与运维；海外云主机上配好客户自有的 Claude / GPT 账号",
      pending: ["怎么连公司 Database", "接哪些外部 API（测试版已部署：Finance Macro Data API）", "每台主机 ?核 ?G GPU 存储"],
    },
    {
      n: "L4", name: "人类决策者与多端协同", key: "自动路由出方案 · 人类决策者修改 · 桌面 / 手机 App",
      what: "人类注意力点的机制是：自动路由先给出一套完整方案，人类决策者修改、确认后执行。员工在桌面 App 编排 I.P.，在手机 App 上随时审批方案、查看进度。",
      oss: "自动路由方案、桌面 App、手机 App", biz: "",
      pending: [],
    },
    {
      n: "L5", name: "岗位（先从宏观研报开始）", key: "宏观研报 · 其他岗位按路线图铺开",
      what: "架构是主体，岗位是它的产出。现阶段先把整体架构建好，只用宏观研报智能岗位做展示，其他岗位以后按同一套架构依次铺开。",
      oss: "岗位描述文件与岗位库", biz: "",
      pending: [],
    },
  ],
  proof: {
    title: "展示：宏观研报智能岗位",
    sub: "现阶段只用一个岗位展示整套架构的真实生产力。",
    featured: {
      name: "宏观研报 I.P.", en: "MACRO RESEARCH · IN ACCEPTANCE", status: "验收中", statusKind: "ind",
      line: "追踪中国与全球宏观数据发布，整理政策文件，生成研究简报。美国和全球数据来自 Finance Macro Data API（测试版已部署），中国数据来自国家统计局。正在用真实模型和真实数据验收。",
      flow: [
        ["L1 上下文", "研究员公开 JD + 研究所报告口径 + 指标字典"],
        ["L2 算力", "协调 Bot 用云端顶尖模型（GPT-6 Astra）拆任务、写观点；执行 Bot 调用云端快速档模型（GPT-6 Luna）拉数据、做计算"],
        ["L3 Harness", "Rust harness · 独立云 Linux 主机 · Finance Macro Data API"],
        ["L4 人", "自动路由给出方案，研究员修改后确认"],
      ],
    },
    roadmapTitle: "后续岗位 · 路线图",
    roadmapNote: "整体架构建好之后，按同一套架构依次铺开。",
    roadmap: ["运维、测试", "网络安全", "债券承做", "数据分析", "美国税务"],
  },

  // 70/20/5/5 生成器
  slots: [
    { key: "jd", pct: 70, name: "公开 JD", full: "公开 JD 信息搜集", color: "var(--steel)" },
    { key: "fpde", pct: 20, name: "FPDE", full: "前场部署工程 / 企业规范", color: "var(--rust)" },
    { key: "cofounder", pct: 5, name: "联创", full: "Cofounder 战略意图", color: "var(--amber)" },
    { key: "free", pct: 5, name: "自由", full: "I.P. 助理自由发挥", color: "var(--ok)" },
  ],
  positionPresets: {
    "宏观研报": {
      id: "macro-research",
      jdCount: 188,
      jd: ["宏观数据跟踪与解读", "政策文件梳理", "周报 / 月报撰写", "资产配置观点支持"],
      fpde: ["研究所报告模板与口径", "内部数据库指标字典", "Finance Macro Data API 指标映射"],
      cofounder: ["关注利率与信用周期", "观点必须注明数据来源与日期"],
      free: ["发现异常数据点时主动补充图表"],
    },
  },
  // 输入任意岗位名时的通用模板
  genericPosition: {
    jdCount: 120,
    jd: ["抽取行业通用职责", "提炼技能树", "整理基础工作流", "汇总常用工具"],
    fpde: ["企业内部流程文档", "术语表与命名规范", "历史交付样例"],
    cofounder: ["本季度业务目标", "风控边界"],
    free: ["在既定流程外提出改进建议"],
  },

  // Router 沙盘
  // 核心分工：协调 Bot 用云端最顶尖的模型；执行 Bot 现阶段调用云端模型，预留后训练的 1B 专家模型
  models: {
    claude: { label: "Claude", kind: "cloud", costPerTask: 0.4 },
    gpt: { label: "GPT", kind: "cloud", costPerTask: 0.35 },
    gemini: { label: "Gemini", kind: "cloud", costPerTask: 0.3 },
    cloudFast: { label: "云端 · 快速档", kind: "cloud-fast", costPerTask: 0.05 },
    expert1b: { label: "1B 专家（规划中）", kind: "local", costPerTask: 0.0 },
  },
  executorModel: "cloudFast",
  plannedExecutorModel: "expert1b",
  routerTasks: [
    {
      title: "写一份本周中国宏观数据简报",
      coordinator: { name: "拆解任务、分派执行 Bot、汇总并写核心观点", model: "claude", reason: "协调需要最强的规划与判断，用云端顶尖模型" },
      executors: [
        { name: "拉取 CPI / PPI / 社融数据", expert: "数据拉取专家", reason: "调用 Finance Macro Data API，固定流程" },
        { name: "计算同比环比，标出异动", expert: "指标计算专家", reason: "确定性计算，快速档足够" },
        { name: "逐份摘要本周政策文件", expert: "政策摘要专家", reason: "格式固定，快速档足够" },
        { name: "生成图表与说明文字", expert: "图表专家", reason: "高频重复，用快速档控制成本" },
      ],
    },
    {
      title: "跟踪美国非农数据发布，生成快评",
      coordinator: { name: "确定快评框架，汇总各项结论并定稿", model: "claude", reason: "要综合判断市场含义，用云端顶尖模型" },
      executors: [
        { name: "发布后拉取非农、失业率、时薪", expert: "数据拉取专家", reason: "定时任务，固定接口" },
        { name: "对比市场预期与前值", expert: "指标计算专家", reason: "确定性计算" },
        { name: "生成走势图", expert: "图表专家", reason: "模板化输出" },
      ],
    },
    {
      title: "解读本月央行货币政策执行报告",
      coordinator: { name: "列出解读提纲，判断政策取向变化", model: "claude", reason: "需要深度理解与判断，用云端顶尖模型" },
      executors: [
        { name: "抽取报告中的关键表述", expert: "政策摘要专家", reason: "抽取任务，快速档足够" },
        { name: "与上期报告逐段比对措辞", expert: "文本比对专家", reason: "规则明确，快速档足够" },
        { name: "整理相关利率与流动性数据", expert: "数据拉取专家", reason: "调用 Finance Macro Data API" },
        { name: "排版成研报格式", expert: "格式专家", reason: "轻量改写，速度快" },
      ],
    },
  ],

  pillars: [
    {
      big: "2 层",
      title: "协调与执行分层",
      body: "协调 Bot 用云端最顶尖的模型做规划和判断；执行 Bot 现阶段调用云端模型。执行层预留了专家小模型接口，以后可以换成后训练的 1B 模型。",
      spec: [["协调层", "云端顶尖模型"], ["执行层", "云端模型（现阶段）"], ["预留", "1B 专家 · 约 3.6G 显存"]],
    },
    {
      big: "Rust",
      title: "整个 harness 用 Rust 重写",
      body: "调度、沙盒、连接器、Router，整个 harness 用 Rust 从零写起，充分利用硬件效率、并发能力和内存安全。",
      spec: [["语言", "Rust"], ["范围", "整个 harness"], ["目标", "硬件效率"]],
    },
    {
      big: "方案 → 修改",
      title: "人类决策者只改方案",
      body: "自动路由先给出一套完整方案，人类决策者在桌面或手机上修改、确认。人不再盯着每一步。",
      spec: [["机器", "自动路由出方案"], ["人", "修改 / 确认"], ["终端", "桌面 + 手机"]],
    },
  ],

  // 抽屉里展示的岗位（目前只有宏观研报）
  positions: [
    {
      name: "宏观研报", en: "Macro Research",
      line: "追踪数据发布，生成研究简报，直连 Finance Macro Data API。",
      mix: [70, 20, 5, 5],
      ctx: "公开研究员 JD + 研究所报告口径 + 指标字典 + Finance Macro Data API",
    },
  ],

  community: {
    title: "一起把 AI 原生组织架构建起来",
    sub: "I.P. 首先是一个开源项目。我们想和更多人一起，把公司的组织方式重新设计一遍。",
    ways: [
      { k: "RUST HARNESS", t: "写 harness", d: "调度、沙盒、连接器、Router 全部用 Rust。欢迎系统和性能方向的贡献。" },
      { k: "POSITION SPEC", t: "定义一个岗位", d: "按 70/20/5/5 设计理念写一份岗位描述文件，提交到岗位库。" },
      { k: "EXPERT MODEL", t: "训练专家小模型", d: "为执行层后训练 1B 左右的专家模型（规划中），附上评测结果。" },
      { k: "ORG DESIGN", t: "讨论组织设计", d: "协调与执行怎么分工、人在哪些节点决策，这些规则还在细化，欢迎一起讨论。" },
    ],
  },

  // 开源项目 vs 顺道的商业化服务
  compare: [
    ["Harness", "完整 Rust harness：调度、沙盒、连接器", "多台云 Linux 主机的托管与运维"],
    ["上下文", "上下文组装器 + JD 采集 + 岗位描述格式", "FPDE 驻场：把企业规范注入岗位"],
    ["路由与算力", "自动路由 + 协调 / 执行分工", "在海外云主机上配好客户自有的 Claude / GPT 账号"],
    ["模型", "云端模型接入 + 专家模型接口", "按岗位定制后训练（规划中）"],
    ["参与方式", "社区共建，Apache-2.0", "按项目合作"],
  ],

  api: {
    // 接口路径和测试部署一致；返回值是 2026-11-11 用同一版代码（0.2.0）在本机实测截取的，有删节
    curl: 'curl "https://whichapi-macro.onrender.com/v1/tools/get_fred_series?series_id=DGS10&limit=3" \\\n  -H "Authorization: Bearer $MACRO_API_KEY"',
    response: '{\n  "source": "Federal Reserve Bank of St. Louis (FRED)",\n  "series_id": "DGS10",\n  "order": "newest first",\n  "records": [\n    { "observation_date": "2026-11-09", "value": 4.88 },\n    { "observation_date": "2026-11-06", "value": 4.86 },\n    { "observation_date": "2026-11-05", "value": 4.84 }\n  ]\n}',
  },
};
