// charts.js — trajectory small-multiples sharing the spine's 48-hour axis. Each
// chart is spec-driven: one or more series (line or filled area) on a shared
// y-scale, an optional horizontal reference line (e.g. the DP target) and a
// crossover marker, plus a playhead cursor + "now" readout slaved to the inspected
// hour. New series read fields the engine/mock already emit, except the platinum
// flow series (cumProduced / cumSpent), which app.js derives onto each row.
const fmt = (n) => {
  const a = Math.abs(n);
  if (a >= 1e6) return (n / 1e6).toFixed(a >= 1e7 ? 0 : 1) + "m";
  if (a >= 1000) return (n / 1000).toFixed(a >= 1e5 ? 0 : 1) + "k";
  return String(Math.round(n));
};

const C = {
  land: "#cf8a63", peasant: "#b7b0a0", draftee: "#6aa8d8", plat: "#d9b25a",
  dp: "#d8d2c2", mult: "#8fb3c9", spent: "#c2705a", amber: "#e3a93f", emp: "#7ec27a",
};
const committed = (r) => r.land + (r.incoming || 0);
const built = (r) => { const b = r.buildings || {}; let s = 0; for (const k in b) s += b[k]; return s; };
const emp = (r, k) => (r.employment && r.employment[k]) || 0;

// spec: { key, label, color (caption/now tint), zero?, minSpan?,
//         series:[{color,get,fill?,width?}], ref?:{get(ctx),color,label},
//         cross?(rows,ctx)->hour|null, now?(row,ctx)->string }
const METRICS = [
  { key: "land", label: "LAND", color: C.land, series: [{ color: C.land, get: committed }] },
  {
    // peasants vs total pop vs the peasant-housing ceiling. The gap between the peasants
    // line and the ceiling fill is free growth room — i.e. where Harmony (pop growth)
    // still buys peasants; once peasants reach the ceiling, Harmony does nothing.
    key: "population", label: "POPULATION", color: C.peasant,
    series: [
      { color: "rgba(183,176,160,.16)", get: (r) => emp(r, "maxPeasantPop"), fill: true }, // peasant housing ceiling
      { color: C.draftee, get: (r) => r.peasants + emp(r, "populationMilitary") },          // total population
      { color: C.peasant, get: (r) => r.peasants, width: 1.7 },                             // peasants
    ],
    now: (r) => { const p = r.peasants, gap = Math.max(0, emp(r, "maxPeasantPop") - p); return gap > 0 ? fmt(p) + " +" + fmt(gap) : fmt(p) + " max"; },
  },
  { key: "draftees", label: "DRAFTEES", color: C.draftee, series: [{ color: C.draftee, get: (r) => r.draftees }] },
  { key: "platHr", label: "PLATINUM/HR", color: C.plat, zero: true, series: [{ color: C.plat, get: (r) => r.platPerHr }] },
  {
    key: "dp", label: "TRAINED DP", color: C.dp, zero: true,
    series: [{ color: C.dp, get: (r) => r.trainedModded }],
    ref: { get: (ctx) => ctx.dpTarget, color: C.amber, label: "target" },
    cross: (rows, ctx) => { const t = ctx.dpTarget; if (!t) return null; const i = rows.findIndex((r) => r.trainedModded >= t); return i < 0 ? null : i; },
  },

  {
    key: "flow", label: "PLATINUM FLOW", color: C.plat, zero: true,
    series: [
      { color: "rgba(217,178,90,.40)", get: (r) => r.platinum, fill: true }, // idle reserve (area)
      { color: C.plat, get: (r) => r.cumProduced || 0 },                      // produced (cumulative)
      { color: C.spent, get: (r) => r.cumSpent || 0 },                        // spent (cumulative)
    ],
    now: (r) => fmt(r.platinum) + " idle",
  },
  { key: "explorePlat", label: "EXPLORE PLAT/AC", color: C.plat, zero: true, series: [{ color: C.plat, get: (r) => (r.costs && r.costs.explorePlat) || 0 }] },
  { key: "exploreDraft", label: "EXPLORE DRAFT/AC", color: C.draftee, zero: true, series: [{ color: C.draftee, get: (r) => (r.costs && r.costs.exploreDraftee) || 0 }] },
  {
    key: "landuse", label: "LAND USE", color: C.land, zero: true,
    series: [
      { color: "rgba(207,138,99,.32)", get: built, fill: true }, // built (working) acres
      { color: C.land, get: committed },                         // total committed (line)
    ],
    now: (r) => fmt(built(r)) + "/" + fmt(committed(r)),
  },
  {
    // jobs (capacity) vs employed (actual) vs peasant-housing ceiling. The gap between
    // jobs and employed = unfilled jobs (worker-limited); the gap between the housing
    // ceiling and jobs = room for surplus workers with no jobs (job-building-limited).
    key: "employment", label: "EMPLOYMENT", color: C.emp, zero: true,
    series: [
      { color: "rgba(126,194,122,.20)", get: (r) => emp(r, "maxPeasantPop"), fill: true }, // housing ceiling
      { color: C.peasant, get: (r) => emp(r, "jobs") },                                     // jobs (capacity)
      { color: C.emp, get: (r) => emp(r, "employed"), width: 1.7 },                         // employed (actual)
    ],
    now: (r) => fmt(emp(r, "employed")) + "/" + fmt(emp(r, "jobs")),
  },
];

// Compact key for the compound (multi-series) charts; the single-series charts read
// off their caption + colored "now" value.
const LEGEND = [
  ["idle", "rgba(217,178,90,.7)"], ["produced", C.plat], ["spent", C.spent],
  ["built", "rgba(207,138,99,.7)"], ["committed", C.land],
  ["peasants", C.peasant], ["total pop", C.draftee], ["peasant cap", "rgba(183,176,160,.55)"],
  ["jobs", C.peasant], ["employed", C.emp], ["housing", "rgba(126,194,122,.5)"],
];

export class Charts {
  constructor(grid, legend) {
    this.cells = METRICS.map((m) => {
      const cell = document.createElement("div"); cell.className = "chart-cell";
      const cap = document.createElement("span"); cap.className = "chart-cap"; cap.textContent = m.label;
      const now = document.createElement("span"); now.className = "chart-now"; now.style.color = m.color;
      const cv = document.createElement("canvas");
      cell.append(cap, now, cv); grid.appendChild(cell);
      return { m, cv, ctx: cv.getContext("2d"), now };
    });
    if (legend) legend.innerHTML = LEGEND.map(([l, c]) => `<span class="legend-item"><span class="swatch" style="background:${c}"></span>${l}</span>`).join("");
    this.rows = []; this.ph = 0; this.ctx = {};
    // Redraw (each canvas re-measures itself) whenever the grid actually resizes —
    // covers the responsive column swap and the 2-row reflow, not just window resizes.
    if (typeof ResizeObserver !== "undefined") {
      this._ro = new ResizeObserver(() => this.draw());
      this._ro.observe(grid);
    } else {
      window.addEventListener("resize", () => this.draw());
    }
  }
  setData(rows, ctx) { this.rows = rows; this.ctx = ctx || {}; this.draw(); }
  setPlayhead(h) { this.ph = h; this.draw(); }
  draw() {
    if (!this.rows.length) return;
    for (const { m, cv, ctx, now } of this.cells) {
      const dpr = window.devicePixelRatio || 1, w = cv.clientWidth, h = cv.clientHeight;
      if (!w) continue;
      cv.width = w * dpr; cv.height = h * dpr; ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);

      // shared y-scale over every series + the reference value
      const seriesVals = m.series.map((s) => this.rows.map(s.get));
      const allVals = seriesVals.flat();
      const refVal = m.ref ? m.ref.get(this.ctx) : null;
      const hasRef = refVal != null && isFinite(refVal);
      if (hasRef) allVals.push(refVal);
      let lo = m.zero ? 0 : Math.min(...allVals);
      let hi = Math.max(...allVals, lo + 1);
      if (m.minSpan && hi - lo < m.minSpan) { const mid = (hi + lo) / 2; lo = mid - m.minSpan / 2; hi = mid + m.minSpan / 2; }
      const padT = 18, padB = 6, padX = 2;
      // Divisor = highest row index (data-driven: protection + OOP + post-OOP), so the chart
      // stretches with the timeline horizon in lockstep with the spine.
      const maxHour = Math.max(1, this.rows.length - 1);
      const xOf = (i) => padX + (i / maxHour) * (w - 2 * padX);
      const yOf = (v) => padT + (h - padT - padB) * (1 - (v - lo) / (hi - lo));

      // baseline
      ctx.strokeStyle = "#1d2128"; ctx.beginPath(); ctx.moveTo(0, h - padB); ctx.lineTo(w, h - padB); ctx.stroke();
      // playhead cursor
      const px = xOf(this.ph);
      ctx.strokeStyle = "rgba(207,138,99,.5)"; ctx.beginPath(); ctx.moveTo(px, 0); ctx.lineTo(px, h); ctx.stroke();
      // crossover marker (e.g. first hour DP ≥ target)
      if (m.cross) {
        const ch = m.cross(this.rows, this.ctx);
        if (ch != null) { const cx = xOf(ch); ctx.strokeStyle = (m.ref && m.ref.color) || "#5fd08a"; ctx.globalAlpha = .55; ctx.beginPath(); ctx.moveTo(cx, padT - 4); ctx.lineTo(cx, h - padB); ctx.stroke(); ctx.globalAlpha = 1; }
      }
      // reference line
      if (hasRef) {
        const ry = yOf(refVal);
        ctx.strokeStyle = m.ref.color; ctx.setLineDash([3, 3]); ctx.globalAlpha = .85;
        ctx.beginPath(); ctx.moveTo(0, ry); ctx.lineTo(w, ry); ctx.stroke();
        ctx.setLineDash([]); ctx.globalAlpha = 1;
      }
      // series (fills first, lines on top)
      m.series.forEach((s, si) => {
        const vals = seriesVals[si];
        if (s.fill) {
          ctx.beginPath(); ctx.moveTo(xOf(0), h - padB);
          vals.forEach((v, i) => ctx.lineTo(xOf(i), yOf(v)));
          ctx.lineTo(xOf(vals.length - 1), h - padB); ctx.closePath();
          ctx.fillStyle = s.color; ctx.fill();
        } else {
          ctx.beginPath(); vals.forEach((v, i) => (i ? ctx.lineTo(xOf(i), yOf(v)) : ctx.moveTo(xOf(i), yOf(v))));
          ctx.strokeStyle = s.color; ctx.lineWidth = s.width || 1.5; ctx.stroke();
        }
      });
      // playhead dots (line series only)
      m.series.forEach((s, si) => {
        if (s.fill) return;
        const v = seriesVals[si][Math.min(this.ph, this.rows.length - 1)];
        ctx.fillStyle = s.color; ctx.beginPath(); ctx.arc(px, yOf(v), 2.4, 0, 7); ctx.fill();
      });
      // now readout
      const row = this.rows[Math.min(this.ph, this.rows.length - 1)];
      now.textContent = m.now ? m.now(row, this.ctx) : fmt(m.series[0].get(row));
    }
  }
}
