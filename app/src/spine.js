// spine.js — the signature surface. One horizontal 48-hour axis carrying the
// land-growth curve, per-hour action density, event flags, and the playhead that
// the whole instrument is slaved to.

const C = {
  land: "#cf8a63", landFill: "rgba(207,138,99,.16)", draftee: "#6aa8d8", dp: "#d8d2c2",
  mana: "#7e86d6", plat: "#d9b25a", food: "#7ec27a", ore: "#c2705a", green: "#5fd08a", amber: "#e3a93f",
  line: "#262a32", faint: "#5e6571", dim: "#9aa0ab", grid: "#1d2128", panel: "#14161b",
};
const PAD = 18, TOP = 8, CURVE_H = 28, TICK_Y = 44, FLAG_Y = 42, SPELL_Y0 = 60, LANE_H = 8, MAX_LANES = 4;
// Self-spell bars are coloured by the resource their effect touches (on-theme with the rest of the
// instrument): Midas → platinum gold, Gaia's → food green, Mining → ore, Ares → defence, the rest mana.
const SPELL_COLOR = { midas_touch: C.plat, gaias_watch: C.food, mining_strength: C.ore, ares_call: C.dp, harmony: C.mana };
const SPELL_SHORT = { midas_touch: "Midas", gaias_watch: "Gaia's", mining_strength: "Mining", ares_call: "Ares", harmony: "Harmony" };
const spellColor = (key) => SPELL_COLOR[key] || C.mana;
const spellLabel = (key) => SPELL_SHORT[key] || key.replace(/_/g, " ");

export class Spine {
  constructor(canvas, onScrub) {
    this.cv = canvas; this.ctx = canvas.getContext("2d");
    this.rows = []; this.markers = []; this.ph = 0; this.hover = null; this.dragging = false;
    this.onScrub = onScrub;
    const set = (e) => { const h = this.hourFromX(this._localX(e)); onScrub(h); };
    canvas.addEventListener("pointerdown", (e) => { this.dragging = true; canvas.setPointerCapture(e.pointerId); set(e); });
    canvas.addEventListener("pointermove", (e) => { this.hover = this.hourFromX(this._localX(e)); if (this.dragging) set(e); else this.draw(); });
    canvas.addEventListener("pointerup", (e) => { this.dragging = false; });
    canvas.addEventListener("pointerleave", () => { this.hover = null; this.draw(); });
    canvas.addEventListener("keydown", (e) => {
      let d = 0;
      if (e.key === "ArrowRight") d = e.shiftKey ? this._nextEvent(1) : 1;
      else if (e.key === "ArrowLeft") d = e.shiftKey ? this._nextEvent(-1) : -1;
      else if (e.key === "Home") return onScrub(0);
      else if (e.key === "End") return onScrub(this.mh());
      else return;
      e.preventDefault(); onScrub(Math.max(0, Math.min(this.mh(), this.ph + d)));
    });
    // Re-measure whenever the canvas's own box changes size — not just on window
    // resize. A window-only listener misses container reflows (responsive grid swap,
    // scrollbar appearing, late font load), leaving the backing store at a stale width
    // that CSS then stretches → the horizontal-smear distortion. ResizeObserver fires
    // on the real element size, including the first layout pass.
    if (typeof ResizeObserver !== "undefined") {
      this._ro = new ResizeObserver(() => this.resize());
      this._ro.observe(canvas);
    } else {
      window.addEventListener("resize", () => this.resize());
    }
  }
  _localX(e) { const r = this.cv.getBoundingClientRect(); return e.clientX - r.left; }
  _nextEvent(dir) {
    const hrs = this.markers.map((m) => m.hour).sort((a, b) => a - b);
    const next = dir > 0 ? hrs.find((h) => h > this.ph) : [...hrs].reverse().find((h) => h < this.ph);
    return next === undefined ? 0 : next - this.ph;
  }
  setData(rows, markers) { this.rows = rows; this.markers = markers || []; this.resize(); }
  setPlayhead(h) { this.ph = h; this.draw(); }
  resize() {
    const dpr = window.devicePixelRatio || 1;
    const w = this.cv.clientWidth, h = this.cv.clientHeight;
    if (!w || !h) return; // not laid out yet — the observer will fire again with a real size
    const bw = Math.round(w * dpr), bh = Math.round(h * dpr);
    if (this.cv.width !== bw || this.cv.height !== bh) { this.cv.width = bw; this.cv.height = bh; }
    this.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    this.W = w; this.H = h; this.draw();
  }
  // Max hour = highest row index, data-driven (protection + OOP + post-OOP). The axis math
  // and the inverse scrub mapping must share this divisor or the timeline won't stretch right.
  mh() { return Math.max(1, this.rows.length - 1); }
  xOf(h) { return PAD + (h / this.mh()) * (this.W - 2 * PAD); }
  hourFromX(x) { const m = this.mh(); return Math.max(0, Math.min(m, Math.round(((x - PAD) / (this.W - 2 * PAD)) * m))); }

  // Contiguous runs (start hour → first hour absent) where each self-spell is active, from the
  // per-row active-spell list. Recasts on expiry read as one continuous span; a gap breaks it.
  spellSpans() {
    const spans = [], active = {};
    this.rows.forEach((r, i) => {
      const present = new Set((r.spells || []).map((s) => s.key));
      for (const key in active) if (!present.has(key)) { spans.push({ key, start: active[key], end: i }); delete active[key]; }
      present.forEach((key) => { if (active[key] == null) active[key] = i; });
    });
    for (const key in active) spans.push({ key, start: active[key], end: this.rows.length });
    return spans.sort((a, b) => a.start - b.start || a.key.localeCompare(b.key));
  }

  draw() {
    const { ctx } = this; if (!this.W) return;
    ctx.clearRect(0, 0, this.W, this.H);
    ctx.fillStyle = C.panel; ctx.fillRect(0, 0, this.W, this.H);

    const mh = this.mh();
    const oopM = this.markers.find((m) => m.oop);
    const oopH = oopM ? oopM.hour : 49;
    // post-OOP region wash — everything from OOP onward reads as "after protection"
    if (mh > oopH) {
      ctx.fillStyle = "rgba(95,208,138,.05)";
      ctx.fillRect(this.xOf(oopH), 0, this.xOf(mh) - this.xOf(oopH), this.H);
    }
    // subtle day shading inside protection (the 24–48h block)
    ctx.fillStyle = "rgba(255,255,255,.012)"; ctx.fillRect(this.xOf(24), 0, this.xOf(Math.min(48, mh)) - this.xOf(24), this.H);

    // hour grid + labels
    ctx.textBaseline = "middle"; ctx.font = "10px 'IBM Plex Mono', monospace";
    for (let h = 0; h <= mh; h++) {
      const x = this.xOf(h);
      ctx.strokeStyle = h % 6 === 0 ? C.line : C.grid;
      ctx.beginPath(); ctx.moveTo(x, TICK_Y - 3); ctx.lineTo(x, TICK_Y + 3); ctx.stroke();
      if (h % 6 === 0) { ctx.fillStyle = C.faint; ctx.textAlign = "center"; ctx.fillText(String(h), x, TICK_Y + 12); }
    }

    // land-growth curve (area + line)
    if (this.rows.length) {
      const lands = this.rows.map((r) => r.land + (r.incoming || 0));
      const lo = 350, hi = Math.max(lo + 1, ...lands);
      const yOf = (v) => TOP + CURVE_H - ((v - lo) / (hi - lo)) * CURVE_H;
      ctx.beginPath(); ctx.moveTo(this.xOf(0), TOP + CURVE_H);
      this.rows.forEach((r, i) => ctx.lineTo(this.xOf(i), yOf(lands[i])));
      ctx.lineTo(this.xOf(this.rows.length - 1), TOP + CURVE_H); ctx.closePath();
      ctx.fillStyle = C.landFill; ctx.fill();
      ctx.beginPath(); this.rows.forEach((r, i) => (i ? ctx.lineTo(this.xOf(i), yOf(lands[i])) : ctx.moveTo(this.xOf(i), yOf(lands[i]))));
      ctx.strokeStyle = C.land; ctx.lineWidth = 1.5; ctx.stroke();
    }

    // self-spell duration band — each active spell is a horizontal bar over the hours it's up,
    // packed into lanes so concurrent spells stack rather than collide.
    const spans = this.spellSpans();
    const laneEnd = [];
    ctx.textBaseline = "middle";
    spans.forEach((sp) => {
      let lane = laneEnd.findIndex((end) => end <= sp.start);
      if (lane === -1) lane = laneEnd.length;
      if (lane >= MAX_LANES) return; // overflow guard (rare: >4 concurrent spells)
      laneEnd[lane] = sp.end;
      const y = SPELL_Y0 + lane * LANE_H, bh = LANE_H - 2;
      const x0 = this.xOf(sp.start), x1 = this.xOf(Math.min(sp.end, mh)), w = Math.max(3, x1 - x0);
      ctx.globalAlpha = 0.92; ctx.fillStyle = spellColor(sp.key); ctx.fillRect(x0, y, w, bh); ctx.globalAlpha = 1;
      ctx.save(); ctx.beginPath(); ctx.rect(x0, y, w, bh); ctx.clip();
      ctx.fillStyle = "#0c0d10"; ctx.font = "700 8px 'Space Grotesk', sans-serif"; ctx.textAlign = "left";
      ctx.fillText(spellLabel(sp.key), x0 + 4, y + bh / 2 + 0.5);
      ctx.restore();
    });

    // event flags
    ctx.font = "9px 'Space Grotesk', sans-serif"; ctx.textAlign = "left";
    this.markers.forEach((m) => {
      const x = this.xOf(m.hour);
      if (m.oop) {
        // OUT OF PROTECTION — the headline boundary: a bold full-height dashed divider + label.
        ctx.save();
        ctx.strokeStyle = m.color; ctx.lineWidth = 2; ctx.setLineDash([4, 3]);
        ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, this.H); ctx.stroke();
        ctx.setLineDash([]);
        ctx.fillStyle = m.color; ctx.font = "700 9px 'Space Grotesk', sans-serif";
        ctx.textAlign = "left"; ctx.textBaseline = "top"; ctx.fillText(m.label, x + 4, 1);
        ctx.restore();
        return;
      }
      ctx.strokeStyle = m.color; ctx.globalAlpha = .5; ctx.beginPath(); ctx.moveTo(x, TOP); ctx.lineTo(x, FLAG_Y); ctx.stroke(); ctx.globalAlpha = 1;
      ctx.fillStyle = m.color; ctx.beginPath(); ctx.moveTo(x, TOP - 2); ctx.lineTo(x + 5, TOP - 6); ctx.lineTo(x, TOP - 6); ctx.closePath(); ctx.fill();
      ctx.fillStyle = C.dim; ctx.fillText(m.label, x + 4, TOP + 2);
    });

    // hover ghost
    if (this.hover != null && this.hover !== this.ph) {
      const x = this.xOf(this.hover);
      ctx.strokeStyle = C.faint; ctx.setLineDash([2, 3]); ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, this.H); ctx.stroke(); ctx.setLineDash([]);
    }

    // playhead
    const px = this.xOf(this.ph);
    ctx.strokeStyle = C.land; ctx.lineWidth = 1.5; ctx.beginPath(); ctx.moveTo(px, 0); ctx.lineTo(px, this.H); ctx.stroke();
    ctx.fillStyle = C.land; ctx.beginPath(); ctx.moveTo(px - 5, 0); ctx.lineTo(px + 5, 0); ctx.lineTo(px, 7); ctx.closePath(); ctx.fill();
    // playhead hour bubble
    const lbl = String(this.ph).padStart(2, "0");
    ctx.font = "600 11px 'IBM Plex Mono', monospace"; ctx.textAlign = "center"; ctx.textBaseline = "middle";
    const bw = 22; ctx.fillStyle = C.land; ctx.fillRect(px - bw / 2, this.H - 16, bw, 14);
    ctx.fillStyle = "#1a120c"; ctx.fillText(lbl, px, this.H - 8);
  }
}
