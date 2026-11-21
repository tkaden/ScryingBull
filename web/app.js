"use strict";
// Scrying Bull results site. Vanilla JS, no dependencies.

const NS = "http://www.w3.org/2000/svg";
const $ = (s) => document.querySelector(s);
const state = { data: null, rows: [], sort: { k: "bt.val.sharpe", asc: false }, selected: null, curves: {} };

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
const get = (o, path) => path.split(".").reduce((a, k) => (a == null ? a : a[k]), o);
const pct = (x, d = 1) => (x == null || !isFinite(x) ? "–" : (x > 0 ? "+" : "") + (x * 100).toFixed(d) + "%");
const num = (x, d = 2) => (x == null || !isFinite(x) ? "–" : x.toFixed(d));
const signCls = (x) => (x > 0 ? "pos" : x < 0 ? "neg" : "");
const css = (v) => getComputedStyle(document.documentElement).getPropertyValue(v).trim();

function quantile(sorted, q) {
  if (!sorted.length) return NaN;
  const i = (sorted.length - 1) * q, lo = Math.floor(i), hi = Math.ceil(i);
  return sorted[lo] + (sorted[hi] - sorted[lo]) * (i - lo);
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
  key.style.background = color;
  h("b", { text: value }, row);
  h("span", { cls: "t", text: label }, row);
}

// ---------- data ----------
async function load() {
  const res = await fetch("data/summary.json", { cache: "no-cache" });
  state.data = await res.json();
  const d = state.data;
  if (d.source !== "yahoo") {
    const b = $("#banner");
    b.hidden = false;
    b.textContent = d.source === "synthetic"
      ? "These results use SYNTHETIC data generated for testing. They say nothing about real markets."
      : "Data source unknown.";
  }
  $("#train-range").textContent = `${d.eval_start} to ${d.validation_start}`;
  $("#bt-range").textContent = `${d.validation_start} to ${d.data_end ?? "–"}`;
  $("#val-range").textContent = `(${d.validation_start} → ${d.data_end ?? "now"})`;
  const v = d.validation;
  if (v) $("#tv-note").textContent += ` Rank correlation: ${num(v.rank_corr)}. The top 10 by train Sharpe averaged ${num(v.top10_val_sharpe)} in validation, against a median of ${num(v.median_val_sharpe)} for all strategies${v.buy_hold_val_sharpe != null ? " and " + num(v.buy_hold_val_sharpe) + " for buy & hold" : ""}.`;
  $("#costs").textContent = `${d.costs.slippage_bps} bps slippage + ${d.costs.commission_bps} bps commission`;
  $("#universe").textContent = d.universe.join(", ");
  $("#generated").textContent = new Date(d.generated).toLocaleString();
  const fams = [...new Set(d.strategies.map((s) => s.family))].sort();
  for (const f of fams) h("option", { value: f, text: f }, $("#family"));
  tiles();
  for (const id of ["#family", "#search", "#hideRandom"]) $(id).addEventListener("input", render);
  document.querySelectorAll("th[data-k]").forEach((th) =>
    th.addEventListener("click", () => {
      const k = th.dataset.k;
      state.sort = { k, asc: state.sort.k === k ? !state.sort.asc : k === "id" || k === "family" };
      table();
    })
  );
  $("#d-close").addEventListener("click", () => select(null));
  const fromHash = decodeURIComponent(location.hash.slice(1));
  render();
  if (fromHash) select(fromHash, false);
}

function tiles() {
  const d = state.data, s = d.strategies, v = d.validation || {};
  const paperDays = Math.max(0, ...s.map((x) => x.paper?.days ?? 0));
  const items = [
    [s.length, "strategies tracked"],
    [v.buy_hold_val_sharpe != null ? num(v.buy_hold_val_sharpe) : "–", "buy & hold Sharpe (validation)"],
    [v.rank_corr != null ? num(v.rank_corr) : "–", "train → validation rank correlation"],
    [`${(v.robust || []).length} / ${v.strategies ?? "–"}`, "beat 95% of random in train AND validation"],
    [paperDays, `paper trading days${d.paper_date ? " (to " + d.paper_date + ")" : ""}`],
  ];
  const root = $("#tiles");
  root.replaceChildren();
  for (const [val, k] of items) {
    const t = h("div", { cls: "tile" }, root);
    h("div", { cls: "v", text: String(val) }, t);
    h("div", { cls: "k", text: k }, t);
  }
}

function visible() {
  const fam = $("#family").value, q = $("#search").value.trim().toLowerCase(), hide = $("#hideRandom").checked;
  return state.data.strategies.filter(
    (s) => (!fam || s.family === fam) && (!q || s.id.toLowerCase().includes(q) || s.description.toLowerCase().includes(q)) && !(hide && s.random)
  );
}

function render() {
  state.rows = visible();
  strip();
  trainVal();
  scatter();
  table();
}

// ---------- strip chart: Sharpe by family ----------
function strip() {
  const root = $("#strip");
  root.replaceChildren();
  const all = state.data.strategies.filter((s) => s.bt);
  const rows = state.rows.filter((s) => s.bt);
  if (!rows.length) return h("div", { cls: "empty", text: "No strategies match the filters." }, root);
  const rnd = all.filter((s) => s.random).map((s) => s.bt.val.sharpe).sort((a, b) => a - b);
  const bh = all.find((s) => s.id === "baseline.buy_hold");
  const byFam = {};
  for (const s of rows) (byFam[s.family] ||= []).push(s);
  const fams = Object.keys(byFam).sort((a, b) => quantile(byFam[b].map((s) => s.bt.val.sharpe).sort((x, y) => x - y), 0.5) - quantile(byFam[a].map((s) => s.bt.val.sharpe).sort((x, y) => x - y), 0.5));
  const W = root.clientWidth || 800, left = Math.min(130, W * 0.3), right = 16, top = 24, rowH = 30;
  const H = top + fams.length * rowH + 24;
  const xs = all.map((s) => s.bt.val.sharpe);
  const lo = Math.min(...xs, 0), hi = Math.max(...xs, 0.1);
  const x = (v) => left + ((v - lo) / (hi - lo)) * (W - left - right);
  const svg = el("svg", { viewBox: `0 0 ${W} ${H}`, role: "img", "aria-label": "Validation Sharpe ratio of each strategy, grouped by family" }, root);
  for (const t of niceTicks(lo, hi, Math.max(3, Math.floor(W / 110)))) {
    el("line", { x1: x(t), x2: x(t), y1: top - 6, y2: H - 20, class: "grid" }, svg);
    el("text", { x: x(t), y: H - 4, "text-anchor": "middle" }, svg).textContent = t;
  }
  if (rnd.length) {
    el("rect", { x: x(quantile(rnd, 0.05)), y: top - 6, width: Math.max(1, x(quantile(rnd, 0.95)) - x(quantile(rnd, 0.05))), height: fams.length * rowH + 2, fill: css("--band") }, svg);
    el("text", { x: x(quantile(rnd, 0.5)), y: top - 10, "text-anchor": "middle" }, svg).textContent = "random 5–95%";
  }
  if (bh) {
    el("line", { x1: x(bh.bt.val.sharpe), x2: x(bh.bt.val.sharpe), y1: top - 6, y2: H - 20, stroke: css("--text-secondary"), "stroke-dasharray": "4 3", "stroke-width": 1.5 }, svg);
  }
  const pts = [];
  fams.forEach((f, r) => {
    const cy = top + r * rowH + rowH / 2;
    el("text", { x: left - 10, y: cy + 4, "text-anchor": "end", class: "lbl" }, svg).textContent = f;
    byFam[f].forEach((s) => {
      const cx = x(s.bt.val.sharpe);
      const sel = s.id === state.selected;
      el("circle", {
        cx, cy, r: sel ? 6 : 4,
        fill: s.random ? css("--null") : css("--series-1"),
        "fill-opacity": sel ? 1 : 0.55,
        stroke: sel ? css("--text-primary") : css("--surface-1"),
        "stroke-width": sel ? 2 : 1,
      }, svg);
      pts.push({ cx, cy, s });
    });
  });
  hoverNearest(svg, pts, W, H, (s) => [[css("--series-1"), num(s.bt.val.sharpe), "validation Sharpe"], [css("--text-muted"), num(s.bt.train.sharpe), "train Sharpe"]]);
}

// Nearest-point hover + click on a scatter-like SVG.
function hoverNearest(svg, pts, W, H, rowsFor) {
  const hit = el("rect", { x: 0, y: 0, width: W, height: H, fill: "transparent" }, svg);
  const ring = el("circle", { r: 8, fill: "none", stroke: css("--text-primary"), "stroke-width": 1.5, visibility: "hidden" }, svg);
  const nearest = (evt) => {
    const p = svg.createSVGPoint();
    p.x = evt.clientX; p.y = evt.clientY;
    const q = p.matrixTransform(svg.getScreenCTM().inverse());
    let best = null, bd = 24 * 24;
    for (const pt of pts) {
      const d = (pt.cx - q.x) ** 2 + (pt.cy - q.y) ** 2;
      if (d < bd) { bd = d; best = pt; }
    }
    return best;
  };
  hit.addEventListener("pointermove", (evt) => {
    const b = nearest(evt);
    if (!b) { ring.setAttribute("visibility", "hidden"); hideTip(); hit.style.cursor = ""; return; }
    ring.setAttribute("cx", b.cx); ring.setAttribute("cy", b.cy); ring.setAttribute("visibility", "visible");
    hit.style.cursor = "pointer";
    showTip(evt, (t) => {
      h("div", { text: b.s.id, style: "font-family:ui-monospace,monospace;margin-bottom:4px" }, t);
      for (const [c, v, l] of rowsFor(b.s)) tipRow(t, c, v, l);
    });
  });
  hit.addEventListener("pointerleave", () => { ring.setAttribute("visibility", "hidden"); hideTip(); });
  hit.addEventListener("click", (evt) => { const b = nearest(evt); if (b) select(b.s.id); });
}

// ---------- scatters ----------
function scatterXY(root, rows, fx, fy, xLabel, yLabel, tipFor) {
  root.replaceChildren();
  const W = root.clientWidth || 800, H = Math.min(420, Math.max(260, W * 0.5)), m = { l: 44, r: 16, t: 12, b: 34 };
  const xs = rows.map(fx), ys = rows.map(fy);
  const lo = Math.min(...xs, ...ys, 0), hi = Math.max(...xs, ...ys, 0.5);
  const x = (v) => m.l + ((v - lo) / (hi - lo)) * (W - m.l - m.r);
  const y = (v) => H - m.b - ((v - lo) / (hi - lo)) * (H - m.t - m.b);
  const svg = el("svg", { viewBox: `0 0 ${W} ${H}`, role: "img", "aria-label": `${xLabel} versus ${yLabel} per strategy` }, root);
  for (const t of niceTicks(lo, hi, 6)) {
    el("line", { x1: x(t), x2: x(t), y1: m.t, y2: H - m.b, class: "grid" }, svg);
    el("line", { x1: m.l, x2: W - m.r, y1: y(t), y2: y(t), class: "grid" }, svg);
    el("text", { x: x(t), y: H - m.b + 16, "text-anchor": "middle" }, svg).textContent = t;
    el("text", { x: m.l - 6, y: y(t) + 4, "text-anchor": "end" }, svg).textContent = t;
  }
  el("line", { x1: x(lo), y1: y(lo), x2: x(hi), y2: y(hi), stroke: css("--text-muted"), "stroke-dasharray": "4 3" }, svg);
  el("text", { x: (W + m.l) / 2, y: H - 2, "text-anchor": "middle", class: "lbl" }, svg).textContent = xLabel;
  el("text", { x: m.l + 6, y: m.t + 10, class: "lbl" }, svg).textContent = yLabel;
  const pts = rows.map((s) => {
    const cx = x(fx(s)), cy = y(fy(s)), sel = s.id === state.selected;
    el("circle", { cx, cy, r: sel ? 6 : 4, fill: s.random ? css("--null") : css("--series-1"), "fill-opacity": sel ? 1 : 0.55, stroke: sel ? css("--text-primary") : css("--surface-1"), "stroke-width": sel ? 2 : 1 }, svg);
    return { cx, cy, s };
  });
  hoverNearest(svg, pts, W, H, tipFor);
}

function trainVal() {
  const rows = state.rows.filter((s) => s.bt);
  if (!rows.length) return $("#tv").replaceChildren(h("div", { cls: "empty", text: "No strategies match the filters." }));
  scatterXY($("#tv"), rows, (s) => s.bt.train.sharpe, (s) => s.bt.val.sharpe, "Train Sharpe", "Validation Sharpe",
    (s) => [[css("--series-1"), num(s.bt.train.sharpe), "train Sharpe"], [css("--series-2"), num(s.bt.val.sharpe), "validation Sharpe"]]);
}

const MIN_PAPER_DAYS = 20;
function scatter() {
  const root = $("#scatter");
  const rows = state.rows.filter((s) => s.bt && s.paper && s.paper.days >= MIN_PAPER_DAYS);
  if (!rows.length) {
    const days = Math.max(0, ...state.data.strategies.map((s) => s.paper?.days ?? 0));
    return root.replaceChildren(h("div", { cls: "empty", text: `Paper trading has ${days} day(s) of history so far. This chart appears after ${MIN_PAPER_DAYS}.` }));
  }
  scatterXY(root, rows, (s) => s.bt.val.sharpe, (s) => s.paper.sharpe, "Validation Sharpe", "Paper Sharpe",
    (s) => [[css("--series-1"), num(s.bt.val.sharpe), "validation Sharpe"], [css("--series-2"), num(s.paper.sharpe), `paper Sharpe (${s.paper.days} days)`]]);
}

// ---------- table ----------
function table() {
  const { k, asc } = state.sort;
  const rows = [...state.rows].sort((a, b) => {
    const va = get(a, k), vb = get(b, k);
    if (va == null && vb == null) return 0;
    if (va == null) return 1;
    if (vb == null) return -1;
    const c = typeof va === "string" ? va.localeCompare(vb) : va - vb;
    return asc ? c : -c;
  });
  document.querySelectorAll("th[data-k]").forEach((th) => {
    th.classList.toggle("sorted", th.dataset.k === k);
    th.classList.toggle("asc", th.dataset.k === k && asc);
  });
  const tb = $("#table tbody");
  tb.replaceChildren();
  for (const s of rows) {
    const tr = h("tr", { tabindex: "0" }, tb);
    if (s.id === state.selected) tr.className = "sel";
    const td = (text, cls = "") => h("td", { text, cls }, tr);
    td(s.id, "l id");
    td(s.family, "l");
    td(num(s.bt?.train.sharpe));
    td(pct(s.bt?.val.cagr), signCls(s.bt?.val.cagr));
    td(num(s.bt?.val.sharpe));
    td(pct(s.bt?.val.mdd), "neg");
    td(s.bt ? Math.round(s.bt.beats_random_val * 100) + "%" : "–");
    td(pct(s.paper?.ret, 2), signCls(s.paper?.ret));
    td(s.paper && s.paper.days >= 2 ? num(s.paper.sharpe) : "–");
    td(String(s.paper?.days ?? "–"));
    tr.addEventListener("click", () => select(s.id));
    tr.addEventListener("keydown", (e) => { if (e.key === "Enter") select(s.id); });
  }
}

// ---------- detail ----------
async function curve(id) {
  if (!state.curves[id]) {
    const r = await fetch(`data/curves/${encodeURIComponent(id)}.json`);
    state.curves[id] = r.ok ? await r.json() : { bt: [], paper: [] };
  }
  return state.curves[id];
}

async function select(id, scroll = true) {
  state.selected = id;
  history.replaceState(null, "", id ? "#" + encodeURIComponent(id) : location.pathname);
  render();
  const box = $("#detail");
  const s = id && state.data.strategies.find((x) => x.id === id);
  if (!s) { box.hidden = true; return; }
  box.hidden = false;
  $("#d-title").textContent = s.id;
  $("#d-desc").textContent = `${s.family} · ${s.description}`;
  const stats = $("#d-stats");
  stats.replaceChildren();
  const items = s.bt ? [
    [pct(s.bt.train.cagr), "Train CAGR"], [num(s.bt.train.sharpe), "Train Sharpe"], [pct(s.bt.train.mdd), "Train max DD"],
    [Math.round(s.bt.beats_random_train * 100) + "%", "Train beats random"],
    [pct(s.bt.val.cagr), "Val CAGR"], [num(s.bt.val.sharpe), "Val Sharpe"], [pct(s.bt.val.mdd), "Val max DD"],
    [Math.round(s.bt.beats_random_val * 100) + "%", "Val beats random"],
    [pct(s.bt.exposure, 0), "Avg exposure"], [num(s.bt.turnover, 1) + "x", "Turnover / yr"],
  ] : [];
  if (s.paper) items.push([pct(s.paper.ret, 2), `Paper return (${s.paper.days}d)`], [s.paper.inception, "Paper since"]);
  for (const [v, l] of items) {
    const d = h("div", {}, stats);
    h("b", { text: v }, d);
    h("span", { text: l }, d);
  }
  const years = $("#d-years");
  years.replaceChildren();
  for (const [y, r] of Object.entries(s.bt?.yearly ?? {})) {
    const d = h("div", {}, years);
    h("span", { cls: "muted", text: y + " " }, d);
    h("b", { cls: signCls(r), text: pct(r) }, d);
  }
  const [c, bh] = await Promise.all([curve(id), curve("baseline.buy_hold")]);
  if (state.selected !== id) return;
  const series = (key) => {
    const out = [{ name: "this strategy", color: css("--series-1"), pts: c[key] }];
    if (id !== "baseline.buy_hold" && c[key].length) {
      const start = c[key][0][0];
      out.push({ name: "buy & hold", color: css("--series-2"), pts: bh[key].filter((p) => p[0] >= start) });
    }
    return out;
  };
  lineChart($("#d-bt"), series("bt"), "Backtest equity, indexed to 1", "No data.", state.data.validation_start);
  lineChart($("#d-paper"), series("paper"), "Paper equity, indexed to 1", "No paper history yet: this strategy's first orders fill at the next open.");
  if (scroll) box.scrollIntoView({ behavior: "smooth", block: "start" });
}

function lineChart(root, series, label, emptyText = "No data.", marker = null) {
  root.replaceChildren();
  series = series.filter((s) => s.pts && s.pts.length >= 2).map((s) => ({ ...s, pts: s.pts.map(([d, v]) => [new Date(d).getTime(), v / s.pts[0][1], d]) }));
  if (!series.length) return h("div", { cls: "empty", text: emptyText }, root);
  if (series.length > 1) {
    const lg = h("div", { cls: "legend" }, root);
    for (const s of series) { const sp = h("span", {}, lg); const i = h("i", {}, sp); i.style.background = s.color; sp.appendChild(document.createTextNode(s.name)); }
  }
  const W = root.clientWidth || 800, H = Math.min(300, Math.max(200, W * 0.36)), m = { l: 44, r: 92, t: 10, b: 24 };
  const all = series.flatMap((s) => s.pts);
  const x0 = Math.min(...all.map((p) => p[0])), x1 = Math.max(...all.map((p) => p[0]));
  const y0 = Math.min(...all.map((p) => p[1])), y1 = Math.max(...all.map((p) => p[1]));
  const pad = (y1 - y0) * 0.05 || 0.01;
  const x = (t) => m.l + ((t - x0) / (x1 - x0 || 1)) * (W - m.l - m.r);
  const y = (v) => H - m.b - ((v - (y0 - pad)) / (y1 - y0 + 2 * pad)) * (H - m.t - m.b);
  const svg = el("svg", { viewBox: `0 0 ${W} ${H}`, role: "img", "aria-label": label }, root);
  for (const t of niceTicks(y0 - pad, y1 + pad, 5)) {
    el("line", { x1: m.l, x2: W - m.r, y1: y(t), y2: y(t), class: "grid" }, svg);
    el("text", { x: m.l - 6, y: y(t) + 4, "text-anchor": "end" }, svg).textContent = t;
  }
  const spanDays = (x1 - x0) / 864e5;
  const fmt = (t) => { const d = new Date(t); return spanDays > 400 ? String(d.getUTCFullYear()) : d.toISOString().slice(5, 10); };
  const nx = Math.max(2, Math.floor((W - m.l - m.r) / 90));
  for (let k = 0; k <= nx; k++) {
    const t = x0 + ((x1 - x0) * k) / nx;
    el("text", { x: x(t), y: H - 6, "text-anchor": "middle" }, svg).textContent = fmt(t);
  }
  el("line", { x1: m.l, x2: W - m.r, y1: y(1), y2: y(1), stroke: css("--text-muted"), "stroke-dasharray": "2 3" }, svg);
  if (marker) {
    const mt = new Date(marker).getTime();
    if (mt > x0 && mt < x1) {
      el("line", { x1: x(mt), x2: x(mt), y1: m.t, y2: H - m.b, stroke: css("--text-secondary"), "stroke-dasharray": "2 3" }, svg);
      el("text", { x: x(mt) + 4, y: m.t + 10, class: "lbl" }, svg).textContent = "validation →";
    }
  }
  for (const s of [...series].reverse()) {
    el("path", { d: "M" + s.pts.map((p) => `${x(p[0]).toFixed(1)},${y(p[1]).toFixed(1)}`).join("L"), fill: "none", stroke: s.color, "stroke-width": 2, "stroke-linejoin": "round" }, svg);
    const last = s.pts[s.pts.length - 1];
    el("text", { x: x(last[0]) + 6, y: y(last[1]) + 4, class: "lbl" }, svg).textContent = `${last[1].toFixed(2)}x`;
  }
  // Crosshair.
  const hair = el("line", { y1: m.t, y2: H - m.b, stroke: css("--text-muted"), visibility: "hidden" }, svg);
  const dots = series.map((s) => el("circle", { r: 4, fill: s.color, stroke: css("--surface-1"), "stroke-width": 2, visibility: "hidden" }, svg));
  const hit = el("rect", { x: m.l, y: 0, width: W - m.l - m.r, height: H, fill: "transparent" }, svg);
  const near = (pts, t) => { let lo = 0, hi = pts.length - 1; while (hi - lo > 1) { const mid = (lo + hi) >> 1; if (pts[mid][0] < t) lo = mid; else hi = mid; } return Math.abs(pts[lo][0] - t) < Math.abs(pts[hi][0] - t) ? pts[lo] : pts[hi]; };
  hit.addEventListener("pointermove", (evt) => {
    const p = svg.createSVGPoint(); p.x = evt.clientX; p.y = evt.clientY;
    const q = p.matrixTransform(svg.getScreenCTM().inverse());
    const t = x0 + ((q.x - m.l) / (W - m.l - m.r)) * (x1 - x0);
    const anchor = near(series[0].pts, t);
    hair.setAttribute("x1", x(anchor[0])); hair.setAttribute("x2", x(anchor[0])); hair.setAttribute("visibility", "visible");
    showTip(evt, (tt) => {
      h("div", { cls: "t", text: anchor[2] }, tt);
      series.forEach((s, i) => {
        const pt = near(s.pts, anchor[0]);
        dots[i].setAttribute("cx", x(pt[0])); dots[i].setAttribute("cy", y(pt[1])); dots[i].setAttribute("visibility", "visible");
        tipRow(tt, s.color, pt[1].toFixed(3) + "x", s.name);
      });
    });
  });
  hit.addEventListener("pointerleave", () => { hair.setAttribute("visibility", "hidden"); dots.forEach((d) => d.setAttribute("visibility", "hidden")); hideTip(); });
}

let resizeTimer;
addEventListener("resize", () => { clearTimeout(resizeTimer); resizeTimer = setTimeout(() => { if (state.data) { render(); if (state.selected) select(state.selected, false); } }, 200); });

load().catch((e) => { document.body.appendChild(h("p", { cls: "wrap banner", text: "Failed to load results: " + e })); });
