// hourgrid.js — a compact ±N-hour entry column for ONE action lane (the selected building / terrain /
// unit), embedded directly in the hour editor. Rows = the hours around the editor's current hour; you
// type a count per hour, multi-select a span and fill it, or paste a column from Excel — and the live
// land / platinum / (lumber|draftees) / DP cells recompute beside it. The spreadsheet motion, scoped to
// the hour you're already editing instead of a separate screen.
//
// Selection/cursor state is owned by the explicit click/key handlers (focus events are unreliable under
// programmatic focus), so multi-select works the same whether driven by mouse, keyboard, or paste.

const int = (n) => Math.round(n || 0).toLocaleString("en-US");
const pad = (n) => String(n).padStart(2, "0");
const esc = (s) => String(s == null ? "" : s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

export function mountHourGrid(host, opts) {
  // opts: { center, radius=6, maxHour, oopHour=49, label, color, stateCols:[{key,label,c,get(r)}],
  //   read(h)->n, write(h,n), rowAt(h)->row, recordUndo(label), recompute(editHour)->Promise, afterCommit(h) }
  const radius = opts.radius || 6;
  const maxHour = Math.max(1, opts.maxHour || 48);
  const center = Math.max(1, Math.min(maxHour, opts.center || 1));
  const lo = Math.max(1, center - radius), hi = Math.min(maxHour, center + radius);
  const oopHour = opts.oopHour || 49;
  const stateCols = opts.stateCols || [];
  let anchor = null, active = null; // selection endpoints (hour ints; single column)

  let body = "";
  for (let h = lo; h <= hi; h++) {
    const v = opts.read(h);
    const cls = [h === center ? "hg-cur" : "", h % 24 === 0 ? "hg-day" : "", h === oopHour ? "hg-oop" : "", h > oopHour ? "hg-post" : ""].join(" ");
    body += `<tr class="hg-row ${cls}">
      <td class="hg-h" data-h="${h}">${pad(h)}${h === center ? '<span class="hg-now">◂</span>' : ""}</td>
      <td class="hg-cell"><input class="hg-in" type="text" inputmode="numeric" autocomplete="off" data-h="${h}" value="${v > 0 ? v : ""}" placeholder="0" aria-label="${esc(opts.label)} hour ${pad(h)}"></td>
      ${opts.maxAt ? `<td class="hg-max"><button type="button" class="hg-maxbtn" data-h="${h}" title="fill to max">—</button></td>` : ""}
      ${stateCols.map((c) => `<td class="hg-state" id="hg-${h}-${c.key}">—</td>`).join("")}
    </tr>`;
  }
  host.innerHTML = `
    <table class="hg-table">
      <thead><tr>
        <th class="hg-h-th">H</th>
        <th class="hg-col-th" style="--col:var(${opts.color || "--c-land"})">${esc(opts.label)}</th>
        ${opts.maxAt ? `<th class="hg-max-th">max</th>` : ""}
        ${stateCols.map((c) => `<th class="hg-state-th" style="--col:var(${c.c})">${c.label}</th>`).join("")}
      </tr></thead>
      <tbody>${body}</tbody>
    </table>
    <div class="hg-foot">
      <span class="hg-hint"><kbd>↵</kbd> next hr&nbsp; <kbd>←→</kbd> hours&nbsp; <kbd>⇧</kbd>+<kbd>↑↓</kbd> span&nbsp; <kbd>⌘D</kbd> fill&nbsp; · tap a <b>max</b> to fill it</span>
      <span class="hg-fill" id="hgFill" hidden></span>
    </div>`;
  const tbody = host.querySelector(".hg-table tbody");
  const inpOf = (h) => tbody.querySelector(`.hg-in[data-h="${h}"]`);
  const fillEl = host.querySelector("#hgFill");

  function refreshState() {
    for (let h = lo; h <= hi; h++) {
      const r = opts.rowAt(h); if (!r) continue;
      for (const c of stateCols) {
        const cell = host.querySelector(`#hg-${h}-${c.key}`); if (!cell) continue;
        const v = Math.round(c.get(r) || 0);
        cell.textContent = int(v);
        cell.classList.toggle("neg", v < 0);
      }
    }
    if (opts.maxAt) for (let h = lo; h <= hi; h++) {
      const m = opts.maxAt(h) || {}, b = host.querySelector(`.hg-maxbtn[data-h="${h}"]`);
      if (b) { b.textContent = int(m.n || 0); b.title = m.why ? `max — limited by ${m.why}` : "fill to max"; }
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
      const i = inpOf(h); if (i) i.parentElement.classList.add("sel");
      const hc = tbody.querySelector(`.hg-h[data-h="${h}"]`); if (hc) hc.classList.add("sel");
    }
    if (!r) { fillEl.hidden = true; fillEl.innerHTML = ""; }
    else { fillEl.hidden = false; fillEl.innerHTML = `<span class="hg-fill-n">${r.hi - r.lo + 1} hrs</span><input id="hgFillVal" class="hg-fill-in" type="text" inputmode="numeric" autocomplete="off" placeholder="value" aria-label="fill value"><button id="hgFillBtn" class="hg-fill-btn" type="button">fill ↓</button>`; }
  }
  function commit(i) {
    const h = +i.dataset.h, v = Math.max(0, Math.floor(+i.value || 0));
    i.value = v > 0 ? String(v) : "";
    if (v === opts.read(h)) return; // idempotent guard (blur + Enter both fire)
    opts.recordUndo("edit");
    opts.write(h, v);
    opts.recompute(h).then(() => { refreshState(); opts.afterCommit && opts.afterCommit(h); });
  }
  function fillRange(value) {
    const r = selRange(); if (!r) return;
    opts.recordUndo("edit");
    for (let h = r.lo; h <= r.hi; h++) { opts.write(h, value); const i = inpOf(h); if (i) i.value = value > 0 ? String(value) : ""; }
    opts.recompute(r.lo).then(() => { refreshState(); opts.afterCommit && opts.afterCommit(r.lo); });
  }
  function focusCell(h, extend) {
    h = Math.max(lo, Math.min(hi, h));
    const t = inpOf(h); if (!t) return;
    active = h; if (!extend) anchor = h;
    t.focus(); t.select();
    highlight();
  }

  host.addEventListener("input", (e) => { const i = e.target.closest(".hg-in"); if (!i) return; const c = i.value.replace(/[^0-9]/g, ""); if (c !== i.value) i.value = c; });
  host.addEventListener("change", (e) => { const i = e.target.closest(".hg-in"); if (i) commit(i); });
  host.addEventListener("mousedown", (e) => { const i = e.target.closest(".hg-in"); if (!i) return; const h = +i.dataset.h; active = h; if (!(e.shiftKey && anchor != null)) anchor = h; setTimeout(highlight, 0); });
  host.addEventListener("keydown", (e) => {
    if (e.target.id === "hgFillVal" && e.key === "Enter") { e.preventDefault(); return fillRange(Math.max(0, Math.floor(+e.target.value || 0))); }
    const i = e.target.closest(".hg-in"); if (!i) return;
    const h = +i.dataset.h;
    if (e.key === "Enter") { e.preventDefault(); focusCell(h + (e.shiftKey ? -1 : 1), false); }
    else if (e.key === "ArrowDown") { e.preventDefault(); focusCell(h + 1, e.shiftKey); }
    else if (e.key === "ArrowUp") { e.preventDefault(); focusCell(h - 1, e.shiftKey); }
    else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
      // ← / → navigate between hours (step the editor's focal hour). Commit this cell's typed value
      // first so it isn't lost when the window re-centers; step immediately if nothing changed.
      // stopPropagation so the global key handler doesn't ALSO step (it would double up once the
      // re-render defocuses this cell).
      e.preventDefault(); e.stopPropagation();
      const dir = e.key === "ArrowRight" ? 1 : -1;
      const v = Math.max(0, Math.floor(+i.value || 0));
      const changed = v !== opts.read(h);
      if (changed) { opts.recordUndo("edit"); opts.write(h, v); i.value = v > 0 ? String(v) : ""; }
      const go = () => opts.onStepHour && opts.onStepHour(dir);
      changed ? opts.recompute(h).then(go) : go();
    }
    else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "d") { e.preventDefault(); if (selRange()) fillRange(opts.read(anchor)); }
  });
  host.addEventListener("click", (e) => {
    if (e.target.id === "hgFillBtn") { const f = host.querySelector("#hgFillVal"); return fillRange(Math.max(0, Math.floor(+((f && f.value) || 0)))); }
    const mb = e.target.closest(".hg-maxbtn"); // tap an hour's "max" → fill that hour to its max legal count
    if (mb && opts.maxAt) {
      const h = +mb.dataset.h, v = Math.max(0, (opts.maxAt(h) || { n: 0 }).n | 0);
      if (v !== opts.read(h)) { opts.recordUndo("edit"); opts.write(h, v); }
      const inp = inpOf(h); if (inp) inp.value = v > 0 ? String(v) : "";
      opts.recompute(h).then(() => { refreshState(); opts.afterCommit && opts.afterCommit(h); });
    }
  });
  host.addEventListener("paste", (e) => {
    const i = e.target.closest && e.target.closest(".hg-in"); if (!i) return;
    const text = (e.clipboardData || window.clipboardData).getData("text"); if (text == null) return;
    const nums = text.replace(/\r/g, "").split("\n").map((s) => s.trim()).filter((s) => s !== "");
    if (!nums.length) return;
    e.preventDefault();
    const startH = +i.dataset.h;
    opts.recordUndo("edit");
    nums.forEach((raw, k) => { const h = startH + k; if (h < lo || h > hi) return; const v = Math.max(0, Math.floor(+raw.replace(/[^0-9]/g, "") || 0)); opts.write(h, v); const c = inpOf(h); if (c) c.value = v > 0 ? String(v) : ""; });
    opts.recompute(startH).then(() => { refreshState(); opts.afterCommit && opts.afterCommit(startH); });
  });

  refreshState();
  requestAnimationFrame(() => focusCell(center, false)); // land on the hour you opened, after the editor settles
  return { focusCenter: () => focusCell(center, false) };
}
