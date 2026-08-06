// hourgrid.js — a compact ±N-hour entry window over one OR SEVERAL action lanes (buildings /
// terrains / units / an improvement), embedded directly in the hour editor. Rows = the hours
// around the editor's current hour; columns = the visible lanes plus the live consequence
// columns. The FOCUSED lane carries the cursor and the per-hour max column. ←/→ move the
// cursor across lanes when more than one is visible (a single-lane window keeps ←/→ = step
// the hour); ↑/↓/↵ walk hours and glide the window past its edges. A multi-hour selection
// gains fill ↓ and move → (retype the span's counts into another lane).
//
// Selection/cursor state is owned by the explicit click/key handlers (focus events are unreliable
// under programmatic focus), so multi-select works the same whether driven by mouse, keyboard, or
// paste. Lane cells may be rule-derived (lane.autoAt): they display the engine-resolved amount in
// auto styling until a manual value overrides them.

const int = (n) => Math.round(n || 0).toLocaleString("en-US");
const pad = (n) => String(n).padStart(2, "0");
const esc = (s) => String(s == null ? "" : s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

export function mountHourGrid(host, opts) {
  // opts: { center, radius=6, maxHour, oopHour=49, stateCols:[{key,label,c,get(r),fmt?}],
  //   lanes:[{ key, label, color, read(h)->n, write(h,n,raw), maxAt(h)?, autoAt(h)?->n|null, zeroAt(h)? }],
  //   focusLane: key, onFocusLane(key, hour), moveTargets:[{key,label}], onMoveSpan(from,to,lo,hi),
  //   rowAt(h)->row, recordUndo(label), recompute(editHour)->Promise, afterCommit(h),
  //   onStepHour(dir, fromHour) re-centers the host editor on fromHour+dir,
  //   onCursor(h) tracks the focused row, initialSel:{lo,hi}? restores a span after a remount }
  // Remounts replace the host's content but listeners live on the host node itself — abort the
  // previous mount's listeners or every remount stacks another set of stale closures (whose
  // lane/window state then fights the live one).
  if (host._hgAbort) host._hgAbort.abort();
  const hgAbort = new AbortController();
  host._hgAbort = hgAbort;
  const on = (type, fn) => host.addEventListener(type, fn, { signal: hgAbort.signal });
  const radius = opts.radius || 6;
  const maxHour = Math.max(1, opts.maxHour || 48);
  const center = Math.max(1, Math.min(maxHour, opts.center || 1));
  const lo = Math.max(1, center - radius), hi = Math.min(maxHour, center + radius);
  const oopHour = opts.oopHour || 49;
  const stateCols = opts.stateCols || [];
  const lanes = opts.lanes || [];
  const focusKey = lanes.some((l) => l.key === opts.focusLane) ? opts.focusLane : (lanes[0] || {}).key;
  const focusedLane = lanes.find((l) => l.key === focusKey) || lanes[0];
  const laneBy = (key) => lanes.find((l) => l.key === key);
  const hasMax = !!(focusedLane && focusedLane.maxAt);
  let anchor = null, active = null, anchorLane = focusKey; // selection endpoints (hour ints; single lane)

  // What a cell currently SHOWS: the manual value, else the rule-resolved amount (auto),
  // else an explicit rule-skip zero. Commit guards compare against this — never against the
  // manual value alone, or blurring an untouched auto cell would silently convert it to manual.
  const displayOf = (lane, h) => {
    const manual = lane.read(h);
    if (manual > 0) return { v: manual, auto: false, zero: false };
    const auto = lane.autoAt ? lane.autoAt(h) : null;
    if (auto != null) return { v: auto, auto: true, zero: false };
    return { v: 0, auto: false, zero: !!(lane.zeroAt && lane.zeroAt(h)) };
  };
  const cellText = (d) => (d.v > 0 ? String(d.v) : (d.zero ? "0" : ""));

  let body = "";
  for (let h = lo; h <= hi; h++) {
    const cls = [h === center ? "hg-cur" : "", h % 24 === 0 ? "hg-day" : "", h === oopHour ? "hg-oop" : "", h > oopHour ? "hg-post" : ""].join(" ");
    body += `<tr class="hg-row ${cls}">
      <td class="hg-h" data-h="${h}">${pad(h)}${h === center ? '<span class="hg-now">◂</span>' : ""}</td>
      ${lanes.map((l) => {
        const d = displayOf(l, h);
        const laneCls = (l.key === focusKey ? "" : " hg-lane-dim") + (d.auto ? " hg-auto" : "");
        return `<td class="hg-cell${laneCls}" data-h="${h}" data-lane="${esc(l.key)}"><input class="hg-in" type="text" inputmode="numeric" autocomplete="off" data-h="${h}" data-lane="${esc(l.key)}" value="${cellText(d)}" placeholder="0" aria-label="${esc(l.label)} hour ${pad(h)}"></td>`;
      }).join("")}
      ${hasMax ? `<td class="hg-max"><button type="button" class="hg-maxbtn" data-h="${h}" title="fill to max">—</button></td>` : ""}
      ${stateCols.map((c) => `<td class="hg-state" id="hg-${h}-${c.key}">—</td>`).join("")}
    </tr>`;
  }
  host.innerHTML = `
    <table class="hg-table">
      <thead><tr>
        <th class="hg-h-th">H</th>
        ${lanes.map((l) => `<th class="hg-col-th ${l.key === focusKey ? "hg-focus-th" : ""}" style="--col:var(${l.color || "--c-land"})">${esc(l.label)}</th>`).join("")}
        ${hasMax ? `<th class="hg-max-th">max</th>` : ""}
        ${stateCols.map((c) => `<th class="hg-state-th" style="--col:var(${c.c})">${c.label}</th>`).join("")}
      </tr></thead>
      <tbody>${body}</tbody>
    </table>
    <div class="hg-foot">
      <span class="hg-hint"><kbd>↵</kbd> next hr&nbsp; ${lanes.length > 1 ? "<kbd>←→</kbd> lanes&nbsp;" : "<kbd>←→</kbd> hours&nbsp;"} <kbd>⇧</kbd>+<kbd>↑↓</kbd> span&nbsp; <kbd>⌘D</kbd> fill${hasMax ? "&nbsp; · tap a <b>max</b> to fill it" : ""}</span>
      <span class="hg-fill" id="hgFill" hidden></span>
    </div>`;
  const tbody = host.querySelector(".hg-table tbody");
  const inpAt = (h, laneKey) => tbody.querySelector(`.hg-in[data-h="${h}"][data-lane="${CSS.escape(laneKey)}"]`);
  const inpOf = (h) => inpAt(h, focusKey);
  const fillEl = host.querySelector("#hgFill");

  function refreshState() {
    for (let h = lo; h <= hi; h++) {
      const r = opts.rowAt(h); if (!r) continue;
      for (const c of stateCols) {
        const cell = host.querySelector(`#hg-${h}-${c.key}`); if (!cell) continue;
        const v = c.get(r) || 0;
        cell.textContent = c.fmt ? c.fmt(v) : int(v);
        cell.classList.toggle("neg", v < 0);
      }
    }
    if (hasMax) for (let h = lo; h <= hi; h++) {
      const m = focusedLane.maxAt(h) || {}, b = host.querySelector(`.hg-maxbtn[data-h="${h}"]`);
      if (b) { b.textContent = int(m.n || 0); b.title = m.why ? `max — limited by ${m.why}` : "fill to max"; }
    }
    // Rule-derived cells re-resolve after every recompute (an upstream edit changes the stock
    // they invest). Never clobber the input the user is typing in.
    for (const l of lanes) {
      if (!l.autoAt && !l.zeroAt) continue;
      for (let h = lo; h <= hi; h++) {
        const i = inpAt(h, l.key); if (!i || i === document.activeElement) continue;
        const d = displayOf(l, h);
        i.value = cellText(d);
        i.parentElement.classList.toggle("hg-auto", d.auto);
      }
    }
  }
  function selRange() {
    if (anchor == null || active == null) return null;
    const a = Math.min(anchor, active), b = Math.max(anchor, active);
    return a === b ? null : { lo: a, hi: b };
  }
  function highlight() {
    tbody.querySelectorAll(".sel").forEach((e) => e.classList.remove("sel"));
    const r = selRange();
    if (r) for (let h = r.lo; h <= r.hi; h++) {
      const i = inpAt(h, anchorLane); if (i) i.parentElement.classList.add("sel");
      const hc = tbody.querySelector(`.hg-h[data-h="${h}"]`); if (hc) hc.classList.add("sel");
    }
    if (!r) { fillEl.hidden = true; fillEl.innerHTML = ""; return; }
    const src = laneBy(anchorLane) || focusedLane;
    const total = (() => { let s = 0; for (let h = r.lo; h <= r.hi; h++) s += displayOf(src, h).v; return s; })();
    const targets = (opts.moveTargets || []).filter((t) => t.key !== anchorLane);
    fillEl.hidden = false;
    fillEl.innerHTML = `<span class="hg-fill-n">${r.hi - r.lo + 1} hrs · ${int(total)} ${esc(src.label)}</span><input id="hgFillVal" class="hg-fill-in" type="text" inputmode="numeric" autocomplete="off" placeholder="value" aria-label="fill value"><button id="hgFillBtn" class="hg-fill-btn" type="button">fill ↓</button>${targets.length && opts.onMoveSpan ? `<span class="hg-mv-cap">move →</span>${targets.map((t) => `<button type="button" class="hg-mv" data-mv="${esc(t.key)}">${esc(t.label)}</button>`).join("")}` : ""}`;
  }
  function commit(i) {
    const lane = laneBy(i.dataset.lane); if (!lane) return;
    const h = +i.dataset.h, raw = i.value, v = Math.max(0, Math.floor(+raw || 0));
    const d = displayOf(lane, h);
    if (v === d.v && !(raw === "" && d.zero)) { i.value = cellText(d); return; } // idempotent guard (blur + Enter both fire)
    opts.recordUndo("edit");
    lane.write(h, v, raw);
    opts.recompute(h).then(() => { refreshState(); opts.afterCommit && opts.afterCommit(h); });
  }
  function fillRange(value) {
    const r = selRange(); if (!r) return;
    const lane = laneBy(anchorLane); if (!lane) return;
    opts.recordUndo("edit");
    for (let h = r.lo; h <= r.hi; h++) { lane.write(h, value, String(value)); const i = inpAt(h, lane.key); if (i) i.value = value > 0 ? String(value) : ""; }
    opts.recompute(r.lo).then(() => { refreshState(); opts.afterCommit && opts.afterCommit(r.lo); });
  }
  // The ◂ current-hour marker follows the cursor: the editor's focal hour tracks the focused
  // row (via onCursor), so the marked row must be the focused one, not the mount-time center.
  function setCurRow(h) {
    const old = tbody.querySelector(".hg-row.hg-cur");
    if (old) { old.classList.remove("hg-cur"); const s = old.querySelector(".hg-now"); if (s) s.remove(); }
    const hc = tbody.querySelector(`.hg-h[data-h="${h}"]`);
    if (hc) { hc.parentElement.classList.add("hg-cur"); hc.insertAdjacentHTML("beforeend", '<span class="hg-now">◂</span>'); }
  }
  function focusCell(h, extend) {
    h = Math.max(lo, Math.min(hi, h));
    const t = inpOf(h); if (!t) return;
    active = h; if (!extend) { anchor = h; anchorLane = focusKey; }
    t.focus(); t.select();
    setCurRow(h);
    highlight();
    opts.onCursor && opts.onCursor(h);
  }
  // Commit a cell's typed value ahead of a window/lane re-render (which would silently drop
  // it — detaching a focused input does NOT fire its change event). Returns the recompute
  // promise when something changed, null when the cell was already canonical.
  function commitInline(i, h) {
    const lane = laneBy(i.dataset.lane); if (!lane) return null;
    const raw = i.value, v = Math.max(0, Math.floor(+raw || 0));
    const d = displayOf(lane, h);
    if (v === d.v && !(raw === "" && d.zero)) return null;
    opts.recordUndo("edit");
    lane.write(h, v, raw);
    i.value = cellText(displayOf(lane, h));
    return opts.recompute(h);
  }
  // Vertical walking (Enter/↑/↓) keeps the cursor row CENTERED: every step commits the cell,
  // then re-centers the window on the next hour — the rows scroll under a fixed cursor instead
  // of the cursor drifting off-center (which made the next re-center a jarring snap).
  // A commit made at an edge where navigation is refused still has to publish itself: refresh the
  // window's state columns AND the host's budget/queue chrome, or the edit would sit invisible
  // until some later action happened to re-render them.
  const afterEdgeCommit = () => { refreshState(); opts.afterCommit && opts.afterCommit(active == null ? center : active); };
  function stepHour(i, h, dir) {
    // Commit FIRST, unconditionally: at hour 1 / the last hour the keystroke is still swallowed
    // by preventDefault, so bailing before the commit would silently eat a typed value (detaching
    // a focused input never fires its change event).
    const p = commitInline(i, h);
    if (h + dir < 1 || h + dir > maxHour || !opts.onStepHour) { if (p) p.then(afterEdgeCommit); return; }
    const go = () => opts.onStepHour(dir, h);
    p ? p.then(go) : go();
  }

  on("input", (e) => { const i = e.target.closest(".hg-in"); if (!i) return; const c = i.value.replace(/[^0-9]/g, ""); if (c !== i.value) i.value = c; });
  on("change", (e) => { const i = e.target.closest(".hg-in"); if (i) commit(i); });
  on("mousedown", (e) => {
    const i = e.target.closest(".hg-in"); if (!i) return;
    const h = +i.dataset.h, laneKey = i.dataset.lane;
    if (laneKey !== focusKey && opts.onFocusLane) {
      // clicking another lane refocuses it (the max column and cursor follow the lane)
      e.preventDefault();
      const cur = document.activeElement && document.activeElement.classList && document.activeElement.classList.contains("hg-in") ? document.activeElement : null;
      const p = cur ? commitInline(cur, +cur.dataset.h) : null;
      const go = () => opts.onFocusLane(laneKey, h);
      p ? p.then(go) : go();
      return;
    }
    active = h; if (!(e.shiftKey && anchor != null)) { anchor = h; anchorLane = laneKey; }
    setCurRow(h); setTimeout(highlight, 0);
    opts.onCursor && opts.onCursor(h);
  });
  on("keydown", (e) => {
    if (e.target.id === "hgFillVal" && e.key === "Enter") { e.preventDefault(); return fillRange(Math.max(0, Math.floor(+e.target.value || 0))); }
    const i = e.target.closest(".hg-in"); if (!i) return;
    const h = +i.dataset.h;
    if (e.key === "Enter") { e.preventDefault(); stepHour(i, h, e.shiftKey ? -1 : 1); }
    else if (e.key === "ArrowDown") { e.preventDefault(); e.shiftKey ? focusCell(h + 1, true) : stepHour(i, h, 1); }
    else if (e.key === "ArrowUp") { e.preventDefault(); e.shiftKey ? focusCell(h - 1, true) : stepHour(i, h, -1); }
    else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
      // Multi-lane: ←/→ move the cursor across lanes (the focused lane follows). Single lane:
      // ←/→ step the editor's focal hour FROM THE FOCUSED CELL (not the window's stale center —
      // that teleported the cursor; the "arrow keys skip hours" bug). Commit the typed value
      // first so a re-render can't drop it. stopPropagation so the global key handler doesn't
      // ALSO step once the re-render defocuses this cell.
      e.preventDefault(); e.stopPropagation();
      const dir = e.key === "ArrowRight" ? 1 : -1;
      if (lanes.length > 1) {
        const idx = lanes.findIndex((l) => l.key === focusKey);
        const next = lanes[idx + dir];
        const p = commitInline(i, h); // commit before the edge bail, same reason as stepHour
        if (!next || !opts.onFocusLane) { if (p) p.then(afterEdgeCommit); return; }
        const go = () => opts.onFocusLane(next.key, h);
        p ? p.then(go) : go();
        return;
      }
      const p = commitInline(i, h);
      const go = () => opts.onStepHour && opts.onStepHour(dir, h);
      p ? p.then(go) : go();
    }
    else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "d") { e.preventDefault(); if (selRange()) { const l = laneBy(anchorLane); if (l) fillRange(displayOf(l, anchor).v); } }
  });
  on("click", (e) => {
    if (e.target.id === "hgFillBtn") { const f = host.querySelector("#hgFillVal"); return fillRange(Math.max(0, Math.floor(+((f && f.value) || 0)))); }
    const mv = e.target.closest(".hg-mv"); // retype the selected span into another lane
    if (mv && opts.onMoveSpan) { const r = selRange(); if (r) opts.onMoveSpan(anchorLane, mv.dataset.mv, r.lo, r.hi); return; }
    const mb = e.target.closest(".hg-maxbtn"); // tap an hour's "max" → fill the focused lane to its max legal count
    if (mb && hasMax) {
      const h = +mb.dataset.h, v = Math.max(0, (focusedLane.maxAt(h) || { n: 0 }).n | 0);
      if (v !== focusedLane.read(h)) { opts.recordUndo("edit"); focusedLane.write(h, v, String(v)); }
      const inp = inpOf(h); if (inp) inp.value = v > 0 ? String(v) : "";
      opts.recompute(h).then(() => { refreshState(); opts.afterCommit && opts.afterCommit(h); });
    }
  });
  on("paste", (e) => {
    const i = e.target.closest && e.target.closest(".hg-in"); if (!i) return;
    const lane = laneBy(i.dataset.lane); if (!lane) return;
    const text = (e.clipboardData || window.clipboardData).getData("text"); if (text == null) return;
    const nums = text.replace(/\r/g, "").split("\n").map((s) => s.trim()).filter((s) => s !== "");
    if (!nums.length) return;
    e.preventDefault();
    const startH = +i.dataset.h;
    opts.recordUndo("edit");
    nums.forEach((raw, k) => { const h = startH + k; if (h < lo || h > hi) return; const v = Math.max(0, Math.floor(+raw.replace(/[^0-9]/g, "") || 0)); lane.write(h, v, raw); const c = inpAt(h, lane.key); if (c) c.value = v > 0 ? String(v) : ""; });
    opts.recompute(startH).then(() => { refreshState(); opts.afterCommit && opts.afterCommit(startH); });
  });

  refreshState();
  requestAnimationFrame(() => { // land on the hour you opened (or restore a span), after the editor settles
    if (opts.initialSel && opts.initialSel.lo != null) {
      const s = opts.initialSel;
      focusCell(Math.max(lo, Math.min(hi, s.hi)), false);
      anchor = Math.max(lo, Math.min(hi, s.lo)); anchorLane = focusKey;
      highlight();
    } else focusCell(center, false);
  });
  return { focusCenter: () => focusCell(center, false) };
}
