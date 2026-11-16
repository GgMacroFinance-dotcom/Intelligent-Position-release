// I.P. 主页原型交互。无依赖，直接在浏览器打开 index.html 即可运行。
(function () {
  "use strict";
  const D = window.IP_DATA;
  const $ = (s, r = document) => r.querySelector(s);
  const el = (tag, cls, html) => {
    const n = document.createElement(tag);
    if (cls) n.className = cls;
    if (html != null) n.innerHTML = html;
    return n;
  };
  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
  const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const timers = new Set();
  const later = (fn, ms) => { const t = setTimeout(() => { timers.delete(t); fn(); }, reduced ? 0 : ms); timers.add(t); };
  const clearTimers = () => { timers.forEach(clearTimeout); timers.clear(); };

  function copyText(text, btn) {
    const done = () => { const o = btn.textContent; btn.textContent = "已复制"; setTimeout(() => (btn.textContent = o), 1400); };
    try {
      navigator.clipboard.writeText(text).then(done, () => selectFallback(btn));
    } catch (e) { selectFallback(btn); }
  }
  function selectFallback(btn) {
    const target = btn.closest(".install, .code-card").querySelector("code, pre");
    const r = document.createRange(); r.selectNodeContents(target);
    const s = window.getSelection(); s.removeAllRanges(); s.addRange(r);
    btn.textContent = "已选中，按 Ctrl+C";
  }

  // ---------- 静态文案 ----------
  document.querySelectorAll("[data-k]").forEach((n) => {
    const v = n.dataset.k.split(".").reduce((o, k) => (o ? o[k] : undefined), D);
    if (v != null) n.textContent = v;
  });
  ["nav-gh", "hero-gh", "cmp-gh", "community-gh"].forEach((id) => { const a = document.getElementById(id); if (a) a.href = D.links.github; });
  const tagUl = $("#hero-tags");
  D.hero.tags.forEach((t) => tagUl.appendChild(el("li", "", esc(t))));
  $("#install-cmd").textContent = D.hero.install;
  $("#copy-install").addEventListener("click", (e) => copyText(D.hero.install, e.currentTarget));

  // ---------- Tabs ----------
  const tabs = [["tab-gen", "panel-gen"], ["tab-router", "panel-router"], ["tab-decider", "panel-decider"]];
  tabs.forEach(([t, p]) => {
    $("#" + t).addEventListener("click", () => {
      tabs.forEach(([t2, p2]) => {
        $("#" + t2).setAttribute("aria-selected", String(t2 === t));
        $("#" + p2).hidden = p2 !== p;
      });
      if (t === "tab-router") runRouter();
    });
  });

  // ---------- 70/20/5/5 生成器 ----------
  const bar = $("#gen-bar"), slotGrid = $("#gen-slots"), yamlPre = $("#gen-yaml");
  D.slots.forEach((s) => {
    const seg = el("div", "", `${s.pct}%`);
    seg.style.background = s.color;
    seg.dataset.pct = s.pct;
    bar.appendChild(seg);
  });

  const presetBox = $("#gen-presets");
  Object.keys(D.positionPresets).forEach((name) => {
    const b = el("button", "", esc(name));
    b.type = "button";
    b.addEventListener("click", () => { $("#gen-input").value = name; generate(name); });
    presetBox.appendChild(b);
  });

  function slug(name) {
    const p = D.positionPresets[name];
    if (p) return p.id;
    const ascii = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
    return ascii || "custom-position";
  }

  function buildYaml(name, p) {
    const k = (s) => `<span class="y-k">${s}</span>`;
    const s = (v) => `<span class="y-s">${esc(v)}</span>`;
    const c = (v) => `<span class="y-c"># ${esc(v)}</span>`;
    const list = (arr) => arr.map((x) => `    - ${s(x)}`).join("\n");
    return [
      `${k("position")}: ${s(slug(name))}`,
      `${k("title")}: ${s(name)}`,
      `${k("context")}:`,
      `  ${k("jd")}:        ${c(`70% · 公开 JD，已采集 ${p.jdCount} 条`)}`,
      `    ${k("weight")}: 0.70  ${c("设计理念，非精确配比")}`,
      `    ${k("duties")}:`, list(p.jd).replace(/^ {4}/gm, "      "),
      `  ${k("fpde")}:      ${c("20% · 企业规范注入")}`,
      `    ${k("weight")}: 0.20`,
      `    ${k("sources")}:`, list(p.fpde).replace(/^ {4}/gm, "      "),
      `  ${k("cofounder")}: ${c("5% · 联创战略意图")}`,
      `    ${k("weight")}: 0.05`,
      `    ${k("intent")}:`, list(p.cofounder).replace(/^ {4}/gm, "      "),
      `  ${k("free")}:      ${c("5% · 自由发挥预算")}`,
      `    ${k("weight")}: 0.05`,
      `${k("runtime")}:`,
      `  ${k("sandbox")}: ${s("cloud-linux")}`,
      `  ${k("resources")}: ${s("TBD")}  ${c("?核 ?G GPU 存储，按岗位负载评估后确定")}`,
      `${k("router")}: ${s("auto")}  ${c("可在 Router 沙盘中逐项修改")}`,
    ].join("\n");
  }
  let lastYamlText = "";

  function generate(rawName) {
    clearTimers();
    const name = (rawName || "").trim() || "债券承做";
    const p = D.positionPresets[name] || D.genericPosition;
    presetBox.querySelectorAll("button").forEach((b) => b.classList.toggle("on", b.textContent === name));

    // 进度条：依次填充
    const segs = [...bar.children];
    segs.forEach((seg) => (seg.style.width = "0"));
    let delay = 60;
    segs.forEach((seg) => { later(() => (seg.style.width = seg.dataset.pct + "%"), delay); delay += 320; });

    // 槽位卡片：先显示占位，再逐条出现
    slotGrid.innerHTML = "";
    const lists = { jd: p.jd, fpde: p.fpde, cofounder: p.cofounder, free: p.free };
    const metas = {
      jd: `抓取 ${p.jdCount} 条 JD → 提炼职责`,
      fpde: `注入 ${p.fpde.length} 份企业规范`,
      cofounder: "管理层偏好与风控边界",
      free: "框架外的探索空间",
    };
    D.slots.forEach((s, i) => {
      const card = el("div", "slot");
      card.innerHTML = `<div class="slot-h"><span class="slot-pct" style="color:${s.color}">${s.pct}%</span><span class="slot-name">${esc(s.full)}</span></div><div class="slot-meta">${esc(metas[s.key])}</div>`;
      const ul = el("ul");
      lists[s.key].forEach((item, j) => {
        const li = el("li", "pending", "…");
        ul.appendChild(li);
        later(() => { li.textContent = item; li.classList.remove("pending"); }, 200 + i * 320 + j * 140);
      });
      card.appendChild(ul);
      slotGrid.appendChild(card);
    });

    yamlPre.innerHTML = buildYaml(name, p);
    lastYamlText = yamlPre.textContent;
  }
  $("#gen-form").addEventListener("submit", (e) => { e.preventDefault(); generate($("#gen-input").value); });
  $("#copy-yaml").addEventListener("click", (e) => copyText(lastYamlText, e.currentTarget));

  // ---------- Router 沙盘 ----------
  const taskSel = $("#router-task"), tbody = $("#router-body"), applyBtn = $("#router-apply");
  D.routerTasks.forEach((t, i) => taskSel.appendChild(new Option(t.title, i)));
  let current = [];
  const money = (v) => "$" + v.toFixed(2);

  function costOf(modelKey) { return D.models[modelKey].costPerTask; }
  function updateCost() {
    const top = costOf(current[0].model);
    const allTop = current.length * top;
    const mix = current.reduce((sum, r) => sum + costOf(r.model), 0);
    const planned = top + (current.length - 1) * costOf(D.plannedExecutorModel);
    $("#cost-cloud").textContent = money(allTop);
    $("#cost-mix").textContent = money(mix);
    $("#cost-local").textContent = money(planned);
  }
  function resetApply() {
    applyBtn.classList.remove("is-done");
    applyBtn.textContent = "确认方案";
  }

  function groupRow(label, note) {
    const tr = el("tr", "tier-row");
    tr.innerHTML = `<td colspan="6"><span class="tier">${esc(label)}</span><span class="tier-note">${esc(note)}</span></td>`;
    return tr;
  }

  function runRouter() {
    const task = D.routerTasks[Number(taskSel.value) || 0];
    const coord = { ...task.coordinator, tier: "coord", rec: task.coordinator.model };
    const execs = task.executors.map((e) => ({ ...e, model: D.executorModel, rec: D.executorModel, tier: "exec" }));
    current = [coord, ...execs];
    tbody.innerHTML = "";
    resetApply();
    let n = 0;
    current.forEach((row, i) => {
      if (i === 0) tbody.appendChild(groupRow("协调层", "调度协调的 Bot · 默认云端最顶尖的模型"));
      if (i === 1) tbody.appendChild(groupRow("执行层", "执行的 Bot · 现阶段调用云端快速档模型，预留 1B 专家模型"));
      const tr = el("tr", (reduced ? "" : "row-in") + (row.tier === "coord" ? " coord" : ""));
      tr.style.animationDelay = n++ * 90 + "ms";
      const opts = Object.entries(D.models)
        .map(([k, m]) => `<option value="${k}"${k === row.model ? " selected" : ""}>${esc(m.label)}</option>`).join("");
      tr.innerHTML = `
        <td class="idx">${row.tier === "coord" ? "C" : String(i).padStart(2, "0")}</td>
        <td>${esc(row.name)}</td>
        <td><span class="model-pill"></span>${row.expert ? `<div class="expert">${esc(row.expert)}</div>` : ""}</td>
        <td class="reason">${esc(row.reason)}</td>
        <td class="num cost-cell"></td>
        <td><label class="sr" for="rsel-${i}">修改这一步的模型</label><select id="rsel-${i}">${opts}</select></td>`;
      const pill = tr.querySelector(".model-pill"), cell = tr.querySelector(".cost-cell");
      const paint = () => {
        const m = D.models[row.model];
        pill.textContent = m.label;
        pill.className = "model-pill " + m.kind + (row.model !== row.rec ? " changed" : "");
        cell.textContent = money(m.costPerTask);
      };
      paint();
      tr.querySelector("select").addEventListener("change", (e) => { row.model = e.target.value; paint(); updateCost(); resetApply(); });
      tbody.appendChild(tr);
    });
    updateCost();
  }
  taskSel.addEventListener("change", runRouter);
  $("#router-run").addEventListener("click", runRouter);
  applyBtn.addEventListener("click", () => {
    const changed = current.filter((r) => r.model !== r.rec).length;
    applyBtn.classList.add("is-done");
    applyBtn.textContent = changed ? `已确认 · 你改了 ${changed} 处` : `已确认 ${current.length} 个 Bot 的方案`;
  });

  // ---------- 架构：范式对比 + 五层 ----------
  const pb = $("#paradigm-body");
  D.paradigm.forEach(([k, a, b]) => {
    const tr = el("tr");
    tr.innerHTML = `<td>${esc(k)}</td><td class="old">${esc(a)}</td><td class="ent">${esc(b)}</td>`;
    pb.appendChild(tr);
  });
  const stackList = $("#stack-list"), stackDetail = $("#stack-detail");
  const rows = [];
  function selectLayer(i) {
    const L = D.layers[i];
    rows.forEach((b) => b.setAttribute("aria-selected", String(Number(b.dataset.i) === i)));
    const pend = L.pending.length
      ? `<div class="sub-h">还没定的问题</div><ul class="pending-list">${L.pending.map((x) => `<li><span class="pending-chip">待定</span>${esc(x)}</li>`).join("")}</ul>`
      : "";
    stackDetail.innerHTML = `
      <p class="eyebrow">${esc(L.n)} · ${esc(L.key)}</p>
      <h3>${esc(L.name)}</h3>
      <p>${esc(L.what)}</p>
      <dl class="spec"><dt>开源项目</dt><dd>${esc(L.oss)}</dd>${L.biz ? `<dt>商业化（顺道）</dt><dd>${esc(L.biz)}</dd>` : ""}</dl>
      ${pend}`;
  }
  // 从上往下画成一座栈：L5 岗位在顶，L1 在底
  D.layers.forEach((L, i) => {
    const b = el("button", "stack-row" + (i === D.layers.length - 1 ? " top" : ""));
    b.type = "button";
    b.dataset.i = i;
    b.setAttribute("role", "tab");
    b.innerHTML = `<span class="ln">${esc(L.n)}</span><span class="sn">${esc(L.name)}</span>${L.pending.length ? `<span class="pending-chip">${L.pending.length} 项待定</span>` : ""}`;
    b.addEventListener("click", () => selectLayer(i));
    rows.push(b);
  });
  rows.slice().reverse().forEach((b) => stackList.appendChild(b));
  selectLayer(0);

  // ---------- 推荐模型配置决策器 ----------
  const DC = window.IP_DECIDER;
  const decOut = $("#dec-out"), decSel = {
    vendor: $("#d-vendor"), role: $("#d-role"), task: $("#d-task"), sensitivity: $("#d-sens"), volume: $("#d-vol"), latency: $("#d-lat"), quality: $("#d-q"),
  };
  Object.entries(DC.TASKS).forEach(([k, t]) => decSel.task.appendChild(new Option(t.label, k)));
  decSel.task.value = "data";
  Object.entries(DC.VENDORS).forEach(([k, v]) => decSel.vendor.appendChild(new Option(v.label, k)));
  decSel.vendor.appendChild(new Option("自动（每档选最便宜）", "auto"));
  decSel.vendor.value = "claude";
  $("#dec-caveat").textContent = "数据说明：档位（小 / 中 / 强 / 最强）按价格和命名推断，没做过评测，上线前用自己的任务验证；Gemini 价格页更新于 2026-06；DeepSeek 价格来自第三方汇总；Kimi / Qwen / GLM / MiniMax 暂未接入（只有第三方价格、没有确切的 API 型号）。";
  function readDec() { const o = {}; Object.keys(decSel).forEach((k) => (o[k] = decSel[k].value)); return o; }
  function renderDec() {
    const r = DC.decide(readDec());
    const pct = r.baselinePer1k ? Math.round((1 - r.costPer1k / r.baselinePer1k) * 100) : 0;
    const cmp = pct > 0 ? `比同厂商第 3 档 省 ${pct}%` : pct < 0 ? `比同厂商第 3 档 贵 ${-pct}%` : "与全用 Opus 持平";
    decOut.innerHTML = `
      <div class="dec-head ${r.blocked ? "blocked" : ""}">
        <span class="dec-model">${esc(r.model.label)}</span>
        <code>${esc(r.model.id)}</code>
        <span class="dec-effort">effort ${esc(r.effort)}</span>
      </div>
      <p class="dim">${esc(r.model.note)}。价格来源：${esc(r.srcLabel)}。基准：${esc(r.baseline.label)}。thinking：${esc(r.thinking)}。</p>
      ${r.blocked ? '<p class="dec-block"><span class="pending-chip">需要确认</span>不满足数据要求，下面的推荐只作参考。</p>' : ""}
      <ul class="dec-reasons">${r.reasons.map((x) => `<li>${esc(x)}</li>`).join("")}</ul>
      ${r.warnings.map((w) => `<p class="dec-warn">${esc(w)}</p>`).join("")}
      <div class="dec-cost"><div><span class="k">每千次调用</span><span class="v hot">${DC.money(r.costPer1k)}</span></div><div><span class="k">${esc(cmp)}</span><span class="v">${DC.money(r.baselinePer1k)}</span></div></div>
      <p class="dim dec-alt">${r.cheaper ? `想更省：${esc(r.cheaper.label)}（${DC.money(r.cheaper.per1k)}）。` : ""}${r.stronger ? `想更稳：${esc(r.stronger.label)}（${DC.money(r.stronger.per1k)}）。` : ""}${r.plannedLocal ? esc(r.plannedLocal) + "。" : ""}</p>`;
  }
  Object.values(decSel).forEach((s) => s.addEventListener("change", renderDec));
  const decBody = $("#dec-body");
  function renderTable() {
    decBody.innerHTML = "";
    const vendor = decSel.vendor.value;
    DC.MACRO_BOTS.forEach((b) => {
      const r = DC.decide(Object.assign({}, b.input, { vendor }));
      const tr = el("tr", "dec-row");
      tr.tabIndex = 0;
      tr.innerHTML = `<td>${esc(b.name)}<div class="expert">${esc(b.desc)}</div></td><td><span class="model-pill cloud">${esc(r.model.label)}</span></td><td class="idx">${esc(r.effort)}</td><td class="num">${DC.money(r.costPer1k)}</td><td class="num">${DC.money(r.baselinePer1k)}</td>`;
      const load = () => { Object.keys(decSel).forEach((k) => { if (k !== "vendor" && b.input[k] != null) decSel[k].value = b.input[k]; }); renderDec(); };
      tr.addEventListener("click", load);
      tr.addEventListener("keydown", (e) => { if (e.key === "Enter") load(); });
      decBody.appendChild(tr);
    });
  }
  decSel.vendor.addEventListener("change", renderTable);
  renderTable();
  renderDec();

  // ---------- 底座卡片 ----------
  const pillars = $("#pillars");
  D.pillars.forEach((p) => {
    const c = el("article", "pillar");
    c.innerHTML = `
      <div class="pillar-big">${esc(p.big)}${p.demo ? '<span class="demo-flag">示意</span>' : ""}</div>
      <h3>${esc(p.title)}</h3>
      <p>${esc(p.body)}</p>
      <dl class="spec">${p.spec.map(([k, v]) => `<dt>${esc(k)}</dt><dd>${esc(v)}</dd>`).join("")}</dl>`;
    pillars.appendChild(c);
  });

  // ---------- 展示岗位 ----------
  const grid = $("#pos-grid");
  const miniBar = (mix) => `<div class="mini-bar">${mix.map((v, i) => `<div style="width:${v}%;background:${D.slots[i].color}"></div>`).join("")}</div>`;
  // 主展示：宏观研报
  const F = D.proof.featured;
  $("#featured").innerHTML = `
    <div class="feat-main">
      <div class="pos-top"><div><p class="eyebrow">${esc(F.en)}</p><h3>${esc(F.name)}</h3></div><span class="badge ${esc(F.statusKind || "live")}">● ${esc(F.status)}</span></div>
      <p>${esc(F.line)}</p>
      <div class="cta-row"><a class="btn btn-solid" href="#api">调用同一套数据 API</a><button class="btn btn-ghost" type="button" id="feat-open">查看 Context 构成</button></div>
    </div>
    <dl class="feat-flow">${F.flow.map(([k, v]) => `<div><dt>${esc(k)}</dt><dd>${esc(v)}</dd></div>`).join("")}</dl>`;
  $("#feat-open").addEventListener("click", (e) => openDrawer(0, e.currentTarget));

  // 其他岗位：只作为路线图弱化展示
  grid.innerHTML = `
    <div class="roadmap-head"><span class="sub-h">${esc(D.proof.roadmapTitle)}</span><span class="dim">${esc(D.proof.roadmapNote)}</span></div>
    <ol class="roadmap">${D.proof.roadmap.map((n) => `<li>${esc(n)}</li>`).join("")}</ol>`;

  const drawer = $("#drawer");
  let lastFocus = null;
  function openDrawer(idx, opener) {
    const p = D.positions[idx];
    $("#drawer-body").innerHTML = `
      <p class="eyebrow">${esc(p.en)}</p>
      <h3 id="drawer-title">${esc(p.name)}</h3>
      <p class="dim">${esc(p.line)}</p>
      <div class="sub-h">Context 构成</div>
      <div class="bar">${p.mix.map((v, i) => `<div style="width:${v}%;background:${D.slots[i].color}">${v}%</div>`).join("")}</div>
      <div class="legend">${D.slots.map((s) => `<span><i style="background:${s.color}"></i>${esc(s.full)}</span>`).join("")}</div>
      <p>${esc(p.ctx)}</p>
      <div class="sub-h">云 Linux 主机配置</div>
      <div class="hw">${["CPU", "内存", "GPU", "存储"].map((l) => `<div><span>${l}</span><b>?</b></div>`).join("")}</div>
      <p class="dim">按岗位负载评估后确定，暂不写死。<span class="pending-chip">待定</span></p>
      <div class="sub-h">下一步</div>
      <div class="cta-row"><button class="btn btn-solid" type="button" data-try="${esc(p.name)}">在生成器里试一下</button><a class="btn btn-ghost" id="drawer-gh" href="${esc(D.links.github)}" target="_blank" rel="noopener">在 GitHub 上查看</a></div>`;
    lastFocus = opener;
    drawer.hidden = false;
    $("#drawer-x").focus();
  }
  function closeDrawer() { drawer.hidden = true; if (lastFocus) lastFocus.focus(); }
  $("#drawer-x").addEventListener("click", closeDrawer);
  $("#drawer-scrim").addEventListener("click", closeDrawer);
  document.addEventListener("keydown", (e) => { if (e.key === "Escape" && !drawer.hidden) closeDrawer(); });
  $("#drawer-body").addEventListener("click", (e) => {
    const t = e.target.closest("[data-close],[data-try]");
    if (!t) return;
    if (t.dataset.try) {
      const name = t.dataset.try.split(" / ")[0];
      const key = Object.keys(D.positionPresets).find((k) => k.includes(name) || name.includes(k)) || name;
      $("#gen-input").value = key;
      $("#tab-gen").click();
      generate(key);
      drawer.hidden = true;
      document.getElementById("demo").scrollIntoView({ behavior: reduced ? "auto" : "smooth" });
    } else {
      closeDrawer();
    }
  });

  // ---------- 参与共建 ----------
  const C = D.community;
  $("#community-title").textContent = C.title;
  $("#community-sub").textContent = C.sub;
  $("#ways").innerHTML = C.ways.map((w) => `<article class="way"><p class="eyebrow">${esc(w.k)}</p><h3>${esc(w.t)}</h3><p>${esc(w.d)}</p></article>`).join("");

  // ---------- 对比表 ----------
  const cmp = $("#cmp-body");
  D.compare.forEach(([k, a, b]) => {
    const tr = el("tr");
    tr.innerHTML = `<td>${esc(k)}</td><td>${esc(a)}</td><td class="ent">${esc(b)}</td>`;
    cmp.appendChild(tr);
  });

  // ---------- API ----------
  $("#api-curl").textContent = D.api.curl;
  $("#api-resp").textContent = D.api.response;

  // ---------- 表单 ----------
  $("#contact-form").addEventListener("submit", (e) => {
    e.preventDefault();
    $("#form-msg").textContent = "原型：表单尚未接后端。正式版提交后由架构团队联系你。";
  });

  // 首次进入就展示一个已生成的岗位
  generate("宏观研报");
  runRouter();
})();
