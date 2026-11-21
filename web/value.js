"use strict";
// Scrying Bull value page. Vanilla JS, no dependencies.

const NS = "http://www.w3.org/2000/svg";
const $ = (s) => document.querySelector(s);
const css = (v) => getComputedStyle(document.documentElement).getPropertyValue(v).trim();
const pct = (x, d = 1) => (x == null || !isFinite(x) ? "–" : (x > 0 ? "+" : "") + (x * 100).toFixed(d) + "%");
const pctPlain = (x, d = 1) => (x == null || !isFinite(x) ? "–" : (x * 100).toFixed(d) + "%");
const num = (x, d = 2) => (x == null || !isFinite(x) ? "–" : (+x).toFixed(d));
const signCls = (x) => (x > 0 ? "pos" : x < 0 ? "neg" : "");
const state = { screen: [], monthly: [], signals: [], sort: { k: "rank", asc: true } };

function el(tag, attrs = {}, parent) {
  const e = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  if (parent) parent.appendChild(e);
  return e;
}
function h(tag, props = {}, parent) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === "text") e.textContent = v;
    else if (k === "cls") e.className = v;
    else e.setAttribute(k, v);
  }
  if (parent) parent.appendChild(e);
  return e;
}
function niceTicks(lo, hi, n = 5) {
  const span = hi - lo || 1;
  const step0 = Math.pow(10, Math.floor(Math.log10(span / n)));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * step0).find((s) => span / s <= n) || step0 * 10;
  const out = [];
  for (let v = Math.ceil(lo / step) * step; v <= hi + 1e-9; v += step) out.push(+v.toFixed(10));
  return out;
}

// ---------- tooltip ----------
const tip = $("#tip");
function showTip(evt, build) {
  tip.replaceChildren();
  build(tip);
  tip.hidden = false;
  const pad = 14, r = tip.getBoundingClientRect();
  let x = evt.clientX + pad, y = evt.clientY + pad;
  if (x + r.width > innerWidth - 8) x = evt.clientX - r.width - pad;
  if (y + r.height > innerHeight - 8) y = evt.clientY - r.height - pad;
  tip.style.left = Math.max(8, x) + "px";
  tip.style.top = Math.max(8, y) + "px";
}
const hideTip = () => (tip.hidden = true);
function tipRow(parent, color, value, label) {
  const row = h("div", { cls: "row" }, parent);
  const key = h("i", {}, row);
  key.style.background = color || "transparent";
  h("b", { text: value }, row);
  h("span", { cls: "t", text: label }, row);
}

// ---------- load ----------
async function getJSON(path) {
  const r = await fetch(path, { cache: "no-cache" });
  if (!r.ok) throw new Error(`${path}: ${r.status}`);
  return r.json();
}

async function load() {
  const [screen, signals, monthly, strat] = await Promise.all([
    getJSON("data/value/screen.json"),
    getJSON("data/value/signals.json"),
    getJSON("data/value/monthly.json"),
    getJSON("data/value/strategies.json"),
  ]);
  state.screen = screen.rows;
  state.signals = signals;
  state.monthly = monthly;
  state.strategies = strat;
  $("#screen-date").textContent = `(as of ${screen.date})`;
  $("#generated").textContent = new Date(strat.generated).toLocaleString();
  tiles(screen);
  signalTable();
  const sel = $("#signal");
  const order = ["value", "earnings_yield", "book_to_market", "sales_yield", "fcf_yield", "ebit_ev", "fscore", "momentum"];
  for (const k of order) {
    const s = signals.find((x) => x.signal === k);
    if (s) h("option", { value: k, text: s.label }, sel);
  }
  sel.addEventListener("change", icChart);
  for (const id of ["#q", "#limit", "#healthy"]) $(id).addEventListener("input", screenTable);
  document.querySelectorAll("#screen th[data-k]").forEach((th) =>
    th.addEventListener("click", () => {
      const k = th.dataset.k;
      state.sort = { k, asc: state.sort.k === k ? !state.sort.asc : ["rank", "ticker", "name"].includes(k) };
      screenTable();
    })
  );
  renderCharts();
  strategyTable();
  screenTable();
}

function sig(name, period) {
  return state.signals.find((s) => s.signal === name && s.period === period);
}

function tiles(screen) {
  const v = sig("value", "Validation"), t = sig("value", "Train");
  const best = state.signals.filter((s) => s.period === "Validation" && s.signal !== "momentum").sort((a, b) => b.ic_t_stat - a.ic_t_stat)[0];
  const items = [
    [String(screen.rows.length), `stocks scored on ${screen.date}`],
    [t ? `${num(t.mean_ic, 3)} (t ${num(t.ic_t_stat, 1)})` : "–", "composite value IC, train"],
    [v ? `${num(v.mean_ic, 3)} (t ${num(v.ic_t_stat, 1)})` : "–", "composite value IC, validation"],
    [v ? pct(v.spread_per_year) : "–", "cheapest minus most expensive fifth, per year (validation)"],
    [best ? best.label : "–", "strongest signal in validation"],
  ];
  const root = $("#tiles");
  root.replaceChildren();
  for (const [val, k] of items) {
    const d = h("div", { cls: "tile" }, root);
    h("div", { cls: "v", text: val }, d);
    h("div", { cls: "k", text: k }, d);
  }
}

function signalTable() {
  const tb = $("#signals tbody");
  tb.replaceChildren();
  const order = ["value", "earnings_yield", "book_to_market", "sales_yield", "fcf_yield", "ebit_ev", "fscore", "momentum"];
  for (const k of order) {
    const t = sig(k, "Train"), v = sig(k, "Validation");
    if (!t && !v) continue;
    const tr = h("tr", {}, tb);
    h("td", { cls: "l", text: (t || v).label }, tr);
    for (const s of [t, v]) {
      h("td", { text: s ? num(s.mean_ic, 3) : "–", cls: s ? signCls(s.mean_ic) : "" }, tr);
      const tt = h("td", { text: s ? num(s.ic_t_stat, 1) : "–" }, tr);
      if (s && Math.abs(s.ic_t_stat) >= 2) tt.style.fontWeight = "600";
      h("td", { text: s ? pct(s.spread_per_year) : "–", cls: s ? signCls(s.spread_per_year) : "" }, tr);
    }
  }
}

function renderCharts() {
  icChart();
  quintileChart();
}

// ---------- IC over time ----------
function icChart() {
  const root = $("#ic-chart");
  root.replaceChildren();
  const key = "ic_" + ($("#signal").value || "value");
  const rows = state.monthly.filter((r) => r[key] != null).map((r) => ({ d: new Date(r.date + "T00:00:00Z"), v: r[key], period: r.period, date: r.date }));
  if (!rows.length) return h("div", { cls: "empty", text: "No data for this signal." }, root);
  rows.forEach((r, i) => {
    const w = rows.slice(Math.max(0, i - 11), i + 1);
    r.avg = w.length >= 12 ? w.reduce((a, x) => a + x.v, 0) / w.length : null;
  });
  const W = root.clientWidth || 800, H = Math.min(320, Math.max(220, W * 0.38)), m = { l: 44, r: 12, t: 10, b: 24 };
  const lo = Math.min(-0.1, ...rows.map((r) => r.v)), hi = Math.max(0.1, ...rows.map((r) => r.v));
  const x0 = rows[0].d.getTime(), x1 = rows[rows.length - 1].d.getTime();
  const x = (t) => m.l + ((t - x0) / (x1 - x0 || 1)) * (W - m.l - m.r);
  const y = (v) => H - m.b - ((v - lo) / (hi - lo)) * (H - m.t - m.b);
  const svg = el("svg", { viewBox: `0 0 ${W} ${H}`, role: "img", "aria-label": `Monthly information coefficient of ${$("#signal").selectedOptions[0]?.textContent}` }, root);
  const vStart = rows.find((r) => r.period === "Validation");
  if (vStart) {
    el("rect", { x: x(vStart.d.getTime()), y: m.t, width: W - m.r - x(vStart.d.getTime()), height: H - m.t - m.b, fill: css("--band") }, svg);
    el("text", { x: x(vStart.d.getTime()) + 6, y: m.t + 12 }, svg).textContent = "Validation";
  }
  for (const t of niceTicks(lo, hi, 5)) {
    el("line", { x1: m.l, x2: W - m.r, y1: y(t), y2: y(t), class: "grid" }, svg);
    el("text", { x: m.l - 6, y: y(t) + 4, "text-anchor": "end" }, svg).textContent = t.toFixed(2);
  }
  const y0 = new Date(x0).getUTCFullYear(), yN = new Date(x1).getUTCFullYear();
  const every = Math.max(1, Math.ceil((yN - y0 + 1) / Math.max(2, Math.floor((W - m.l - m.r) / 60))));
  for (let yr = y0 + 1; yr <= yN; yr += every) {
    const t = Date.UTC(yr, 0, 1);
    el("text", { x: x(t), y: H - 6, "text-anchor": "middle" }, svg).textContent = yr;
  }
  el("line", { x1: m.l, x2: W - m.r, y1: y(0), y2: y(0), stroke: css("--text-muted") }, svg);
  const bw = Math.max(1, (W - m.l - m.r) / rows.length - 1);
  for (const r of rows) {
    el("rect", { x: x(r.d.getTime()) - bw / 2, y: Math.min(y(0), y(r.v)), width: bw, height: Math.max(1, Math.abs(y(r.v) - y(0))), fill: r.v >= 0 ? css("--series-1") : css("--neg"), "fill-opacity": 0.55 }, svg);
  }
  const avg = rows.filter((r) => r.avg != null);
  if (avg.length > 1) {
    el("path", { d: "M" + avg.map((r) => `${x(r.d.getTime()).toFixed(1)},${y(r.avg).toFixed(1)}`).join("L"), fill: "none", stroke: css("--text-primary"), "stroke-width": 2 }, svg);
  }
  const hair = el("line", { y1: m.t, y2: H - m.b, stroke: css("--text-muted"), visibility: "hidden" }, svg);
  const hit = el("rect", { x: m.l, y: 0, width: W - m.l - m.r, height: H, fill: "transparent" }, svg);
  hit.addEventListener("pointermove", (evt) => {
    const p = svg.createSVGPoint(); p.x = evt.clientX; p.y = evt.clientY;
    const q = p.matrixTransform(svg.getScreenCTM().inverse());
    const t = x0 + ((q.x - m.l) / (W - m.l - m.r)) * (x1 - x0);
    const r = rows.reduce((a, b) => (Math.abs(b.d - t) < Math.abs(a.d - t) ? b : a));
    hair.setAttribute("x1", x(r.d.getTime())); hair.setAttribute("x2", x(r.d.getTime())); hair.setAttribute("visibility", "visible");
    showTip(evt, (box) => {
      h("div", { cls: "t", text: r.date.slice(0, 7) }, box);
      tipRow(box, r.v >= 0 ? css("--series-1") : css("--neg"), num(r.v, 3), "IC that month");
      if (r.avg != null) tipRow(box, css("--text-primary"), num(r.avg, 3), "12-month average");
    });
  });
  hit.addEventListener("pointerleave", () => { hair.setAttribute("visibility", "hidden"); hideTip(); });
}

// ---------- quintile growth ----------
function quintileChart() {
  const root = $("#q-chart");
  root.replaceChildren();
  const rows = state.monthly.filter((r) => r.q1 != null);
  if (rows.length < 2) return h("div", { cls: "empty", text: "Not enough months yet." }, root);
  const names = ["q1", "q2", "q3", "q4", "q5"];
  const labels = { q1: "Q1 cheapest", q2: "Q2", q3: "Q3", q4: "Q4", q5: "Q5 most expensive" };
  const series = names.map((k) => {
    let eq = 1;
    const pts = [[new Date(rows[0].date + "T00:00:00Z").getTime(), 1, rows[0].date]];
    for (let i = 0; i < rows.length; i++) {
      eq *= 1 + rows[i][k];
      const next = rows[i + 1] ? rows[i + 1].date : null;
      const d = next || rows[i].date;
      pts.push([new Date(d + "T00:00:00Z").getTime() + (next ? 0 : 30 * 864e5), eq, next || "latest"]);
    }
    return { k, label: labels[k], color: css("--" + k), pts };
  });
  const lg = $("#q-legend");
  lg.replaceChildren();
  for (const s of series) { const sp = h("span", {}, lg); const i = h("i", {}, sp); i.style.background = s.color; sp.appendChild(document.createTextNode(s.label)); }
  const W = root.clientWidth || 800, H = Math.min(340, Math.max(220, W * 0.4)), m = { l: 44, r: 118, t: 10, b: 24 };
  const all = series.flatMap((s) => s.pts);
  const x0 = all[0][0], x1 = Math.max(...all.map((p) => p[0]));
  const ylo = Math.log(Math.min(...all.map((p) => p[1]))), yhi = Math.log(Math.max(...all.map((p) => p[1])));
  const x = (t) => m.l + ((t - x0) / (x1 - x0 || 1)) * (W - m.l - m.r);
  const y = (v) => H - m.b - ((Math.log(v) - ylo) / (yhi - ylo || 1)) * (H - m.t - m.b);
  const svg = el("svg", { viewBox: `0 0 ${W} ${H}`, role: "img", "aria-label": "Growth of one dollar in each value quintile, log scale" }, root);
  for (const t of [0.5, 1, 2, 4, 8, 16, 32].filter((v) => Math.log(v) >= ylo - 0.01 && Math.log(v) <= yhi + 0.01)) {
    el("line", { x1: m.l, x2: W - m.r, y1: y(t), y2: y(t), class: "grid" }, svg);
    el("text", { x: m.l - 6, y: y(t) + 4, "text-anchor": "end" }, svg).textContent = "$" + t;
  }
  const y0 = new Date(x0).getUTCFullYear(), yN = new Date(x1).getUTCFullYear();
  const every = Math.max(1, Math.ceil((yN - y0 + 1) / Math.max(2, Math.floor((W - m.l - m.r) / 60))));
  for (let yr = y0 + 1; yr <= yN; yr += every) el("text", { x: x(Date.UTC(yr, 0, 1)), y: H - 6, "text-anchor": "middle" }, svg).textContent = yr;
  // End labels, nudged apart so they never overlap.
  const ends = series.map((s) => ({ s, y: y(s.pts[s.pts.length - 1][1]) })).sort((a, b) => a.y - b.y);
  for (let i = 1; i < ends.length; i++) ends[i].y = Math.max(ends[i].y, ends[i - 1].y + 13);
  for (const s of [...series].reverse()) {
    el("path", { d: "M" + s.pts.map((p) => `${x(p[0]).toFixed(1)},${y(p[1]).toFixed(1)}`).join("L"), fill: "none", stroke: s.color, "stroke-width": s.k === "q1" || s.k === "q5" ? 2.25 : 1.5 }, svg);
  }
  for (const e of ends) {
    const last = e.s.pts[e.s.pts.length - 1];
    el("text", { x: x(last[0]) + 6, y: e.y + 4, class: "lbl" }, svg).textContent = `${e.s.k.toUpperCase()} $${last[1].toFixed(2)}`;
  }
  const hair = el("line", { y1: m.t, y2: H - m.b, stroke: css("--text-muted"), visibility: "hidden" }, svg);
  const hit = el("rect", { x: m.l, y: 0, width: W - m.l - m.r, height: H, fill: "transparent" }, svg);
  hit.addEventListener("pointermove", (evt) => {
    const p = svg.createSVGPoint(); p.x = evt.clientX; p.y = evt.clientY;
    const q = p.matrixTransform(svg.getScreenCTM().inverse());
    const t = x0 + ((q.x - m.l) / (W - m.l - m.r)) * (x1 - x0);
    const idx = series[0].pts.reduce((bi, pt, i, arr) => (Math.abs(pt[0] - t) < Math.abs(arr[bi][0] - t) ? i : bi), 0);
    const px = x(series[0].pts[idx][0]);
    hair.setAttribute("x1", px); hair.setAttribute("x2", px); hair.setAttribute("visibility", "visible");
    showTip(evt, (box) => {
      h("div", { cls: "t", text: String(series[0].pts[idx][2]).slice(0, 7) }, box);
      for (const s of series) tipRow(box, s.color, "$" + s.pts[idx][1].toFixed(2), s.label);
    });
  });
  hit.addEventListener("pointerleave", () => { hair.setAttribute("visibility", "hidden"); hideTip(); });
}

// ---------- tables ----------
function strategyTable() {
  const tb = $("#strategies tbody");
  tb.replaceChildren();
  const rows = [...state.strategies.strategies].sort((a, b) => b.train.sharpe - a.train.sharpe);
  for (const s of rows) {
    const tr = h("tr", { title: s.description }, tb);
    const name = h("td", { cls: "l id", text: s.id }, tr);
    if (s.id === "value.universe_equal" || s.id === "baseline.buy_hold") name.style.fontWeight = "600";
    h("td", { text: num(s.train.sharpe) }, tr);
    h("td", { text: pct(s.train.cagr), cls: signCls(s.train.cagr) }, tr);
    h("td", { text: num(s.val.sharpe) }, tr);
    h("td", { text: pct(s.val.cagr), cls: signCls(s.val.cagr) }, tr);
    h("td", { text: pct(s.val.mdd), cls: "neg" }, tr);
    h("td", { text: Math.round(s.beats_random_val * 100) + "%" }, tr);
    h("td", { text: s.paper && s.paper.days > 0 ? pct(s.paper.ret, 2) : "–", cls: s.paper ? signCls(s.paper.ret) : "" }, tr);
    h("td", { text: s.paper ? String(s.paper.days) : "–" }, tr);
  }
}

function screenTable() {
  const q = $("#q").value.trim().toLowerCase(), limit = +$("#limit").value, healthy = $("#healthy").checked;
  let rows = state.screen.filter((r) => (!healthy || (r.fscore ?? 0) >= 6) && (!q || r.ticker.toLowerCase().includes(q) || (r.name || "").toLowerCase().includes(q)));
  rows = rows.slice(0, limit);
  const { k, asc } = state.sort;
  rows.sort((a, b) => {
    const va = a[k], vb = b[k];
    if (va == null && vb == null) return 0;
    if (va == null) return 1;
    if (vb == null) return -1;
    const c = typeof va === "string" ? va.localeCompare(vb) : va - vb;
    return asc ? c : -c;
  });
  document.querySelectorAll("#screen th[data-k]").forEach((th) => {
    th.classList.toggle("sorted", th.dataset.k === k);
    th.classList.toggle("asc", th.dataset.k === k && asc);
  });
  const tb = $("#screen tbody");
  tb.replaceChildren();
  for (const r of rows) {
    const tr = h("tr", {}, tb);
    h("td", { cls: "l", text: String(r.rank) }, tr);
    h("td", { cls: "l id", text: r.ticker }, tr);
    h("td", { cls: "l name", text: r.name || "", title: r.name || "" }, tr);
    h("td", { text: r.value_pct != null ? Math.round(r.value_pct * 100) : "–" }, tr);
    for (const f of ["earnings_yield", "book_to_market", "fcf_yield", "ebit_ev"]) {
      h("td", { text: f === "book_to_market" ? num(r[f]) : pctPlain(r[f]), cls: r[f] < 0 ? "neg" : "" }, tr);
    }
    const fs = h("td", {}, tr);
    if (r.fscore != null) h("span", { cls: "pill " + (r.fscore >= 7 ? "good" : r.fscore <= 3 ? "bad" : ""), text: String(r.fscore) }, fs);
    else fs.textContent = "–";
    h("td", { text: pct(r.momentum_12_1, 0), cls: signCls(r.momentum_12_1) }, tr);
    h("td", { text: r.market_cap_bn != null ? r.market_cap_bn.toFixed(1) : "–" }, tr);
    h("td", { cls: "l", text: r.flag || "" }, tr);
  }
  if (!rows.length) {
    const tr = h("tr", {}, tb);
    h("td", { cls: "l", colspan: "12", text: "No stocks match the filters." }, tr);
  }
}

let resizeTimer;
addEventListener("resize", () => { clearTimeout(resizeTimer); resizeTimer = setTimeout(() => state.monthly.length && renderCharts(), 200); });
load().catch((e) => { document.querySelector("main").prepend(h("p", { cls: "banner", text: "Failed to load value results: " + e.message + ". Run `scry value run`, then `scry site`." })); });
