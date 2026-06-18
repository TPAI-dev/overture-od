import { engine } from "./bridge.js";
import { Spine } from "./spine.js";
import { Charts } from "./charts.js";
import { createEditor, scanFeasibility, platinumFlow } from "./editor.js";
import { createSaves } from "./saves.js";
import { buildLog, downloadLog } from "./log.js";

/* ───────── domain vocab ───────── */
const LANDS = ["plain", "swamp", "hill", "mountain", "forest", "cavern", "water"];

// Timeline horizon. Protection is hours 1..48; hour 49 is OUT OF PROTECTION (the headline);
// hours 50.. are the optional post-OOP planning window (Phase 1: economy, no combat). The plan
// stores `hours` of length = protection + post-OOP. BY DEFAULT there's NO post-OOP window
// (DEFAULT_POST_OOP_HOURS = 0) — the timeline runs through hour 49 (49 ticks incl. OOP) and stops;
// the OOP row always shows because the adapter surfaces the OOP state whether or not a post-OOP
// window exists. The topbar `#postOopInput` is opt-in "extend up to X", from 0 to MAX_POST_OOP_HOURS
// (480 ≈ 20 days). The engine has no limit — nothing in calc/tick reads protection_ticks_remaining
// (which marches negative post-OOP), the daily-bonus reset keeps firing every 24h via
// `remaining % 24 == 0`, and the spine/charts/ledger are data-driven off rows.length. Caveat:
// post-OOP is economy-only (no combat) and NOT oracle-golden-validated, so a long window is a
// "farm uninterrupted" projection — fine for buildup setup, rough far out.
const PROTECTION_HOURS = 48;
const OOP_HOUR = PROTECTION_HOURS + 1; // 49 — the out-of-protection moment
const MAX_POST_OOP_HOURS = 480;        // topbar cap: extend up to 480h (~20 days) past OOP
const DEFAULT_POST_OOP_HOURS = 0;      // a fresh plan stops at OOP (hour 49); extend via the input
const DEFAULT_HOURS = PROTECTION_HOURS + DEFAULT_POST_OOP_HOURS; // 48 — protection only, through OOP

const int = (n) => Math.round(n || 0).toLocaleString("en-US");
const col = {
  land: "#cf8a63", peasant: "#b7b0a0", draftee: "#6aa8d8", plat: "#d9b25a",
  food: "#7ec27a", lumber: "#b88a5e", ore: "#c2705a", mana: "#7e86d6",
  gems: "#c77ec2", dp: "#d8d2c2", op: "#d6856a", dim: "#9aa0ab",
};

const COLUMNS = [
  // land shows on-hand land, with the committed total (incl. incoming exploration) in
  // parens when something's inbound, e.g. "500 (550)".
  {
    key: "land", label: "land", c: col.land, get: (r) => r.land + (r.incoming || 0),
    disp: (r) => ((r.incoming || 0) > 0 ? `${int(r.land)} <span class="paren">(${int(r.land + r.incoming)})</span>` : int(r.land)),
  },
  { key: "peasants", label: "peas", c: col.peasant, get: (r) => r.peasants },
  { key: "draftees", label: "draft", c: col.draftee, get: (r) => r.draftees },
  { key: "spies", label: "spy", c: col.dim, dim: true, get: (r) => (r.military && r.military.spies) || 0 },
  { key: "wizards", label: "wiz", c: col.mana, dim: true, get: (r) => (r.military && r.military.wizards) || 0 },
  { key: "platinum", label: "plat", c: col.plat, get: (r) => r.platinum },
  { key: "platPerHr", label: "p/hr", c: col.plat, dim: true, get: (r) => r.platPerHr },
  { key: "lumber", label: "lmbr", c: col.lumber, get: (r) => r.lumber },
  { key: "food", label: "food", c: col.food, get: (r) => r.food },
  { key: "ore", label: "ore", c: col.ore, get: (r) => r.ore },
  { key: "mana", label: "mana", c: col.mana, get: (r) => r.mana },
  { key: "gems", label: "gems", c: col.gems, get: (r) => r.gems },
  { key: "research", label: "rsch", c: col.dim, dim: true, get: (r) => r.tech || 0 },
  // trained DP (draftees excluded), shown both as raw and after the global modifier
  // (guard towers / walls / Ares / morale). They coincide when no modifier is active.
  { key: "dpRaw", label: "DP raw", c: col.dp, dim: true, get: (r) => Math.round(r.trainedRaw) },
  { key: "dpMod", label: "DP mod", c: col.dp, get: (r) => Math.round(r.trainedModded) },
  // modded offensive power (target-less base) — relevant once the build attacks at OOP
  { key: "opMod", label: "OP mod", c: col.op, get: (r) => Math.round(r.trainedOpModded || 0) },
];

/* ───────── default = a blank canvas. You start where the real game does: 350 barren
   plain acres, nothing built, no actions queued. Open `00 OPENING` to place your free
   starting build, then design hour by hour (every edit re-simulates live). ───────── */
function defaultPlan() {
  return { race: "human", dpTarget: 6000, opening: {}, hours: Array.from({ length: DEFAULT_HOURS }, () => []), oopActions: [] };
}

/* ───────── state ───────── */
let plan = defaultPlan();
let trace = null, prev = null, playhead = 0, tab = "state", playing = null, lastFeasible = null;
let meta = null, editor = null, infeasible = [];
const $ = (s) => document.querySelector(s);
let spine, charts;

/* ───────── history (undo / redo) ─────────
   `plan` is the single source of truth. Every mutation calls recordUndo(label) BEFORE it
   mutates `plan`, snapshotting the pre-mutation plan onto `past`. undo()/redo() swap whole-plan
   snapshots between the past/future stacks. Continuous-value edits (DP target, post-OOP length,
   opening-build counts) COALESCE: a run of the same label collapses to ONE undo step, so typing
   "6000" into the DP field is a single undo, not four. */
const HISTORY_CAP = 100;
const COALESCE_LABELS = new Set(["dp", "postoop", "opening"]);
let past = [], future = [], lastEditLabel = null, metaRace = null;
const clonePlan = (p) => (typeof structuredClone === "function" ? structuredClone(p) : JSON.parse(JSON.stringify(p)));

// Snapshot the CURRENT plan as an undo point. Call this immediately before mutating `plan`.
function recordUndo(label) {
  const coalesce = COALESCE_LABELS.has(label) && label === lastEditLabel && past.length > 0;
  if (!coalesce) {
    past.push(clonePlan(plan));
    if (past.length > HISTORY_CAP) past.shift();
  }
  future = [];           // a fresh edit invalidates the redo stack
  lastEditLabel = label;
  updateHistoryButtons();
}
function updateHistoryButtons() {
  const u = $("#undoBtn"), r = $("#redoBtn");
  if (u) u.disabled = past.length === 0;
  if (r) r.disabled = future.length === 0;
}
async function undo() {
  if (!past.length) return;
  future.push(clonePlan(plan));
  plan = past.pop();
  await restoreFromHistory();
}
async function redo() {
  if (!future.length) return;
  past.push(clonePlan(plan));
  plan = future.pop();
  await restoreFromHistory();
}
// Re-seat the whole UI on the restored plan: resync the topbar inputs, refresh race meta if the
// race changed across the snapshot, re-simulate (no wake-flash), reclamp the playhead, and refresh
// the editor popover in place if it's open.
async function restoreFromHistory() {
  lastEditLabel = null;
  if (!Array.isArray(plan.hours)) plan.hours = [];
  plan.opening = plan.opening || {};
  if (!Array.isArray(plan.oopActions)) plan.oopActions = [];
  $("#raceSelect").value = plan.race;
  $("#dpInput").value = plan.dpTarget;
  $("#postOopInput").value = Math.max(0, plan.hours.length - PROTECTION_HOURS);
  if (metaRace !== plan.race) await loadMeta(plan.race);
  prev = null;
  await recompute();
  setPlayhead(Math.min(playhead, lastRow()));
  if (editor && editor.isOpen()) editor.rerender();
  updateHistoryButtons();
}
async function loadMeta(race) { meta = await engine.meta(race); metaRace = race; }

// Highest ROW index in the current trace (data-driven: protection 0..48 + OOP 49 + post-OOP,
// plus the trailing post-OOP end row the engine adds). Falls back to the OOP hour pre-trace.
const lastRow = () => (trace && trace.rows && trace.rows.length ? trace.rows.length - 1 : OOP_HOUR);
// Editable hours = 1..plan.hours.length (the trailing end row is display-only).
const planHours = () => (plan.hours ? plan.hours.length : DEFAULT_HOURS);

// Per-race resource visibility (improvement #2). ORE is the ONLY per-race resource — it has no
// universal sink (it's purely a unit-training input), so a race whose units never cost ore has no use
// for it and the column hides. Everything else is always shown: platinum/food, lumber (construction),
// mana (spells), and GEMS — everyone builds diamond mines eventually, so gems stays on for every race.
function buildUsesResource(key) {
  if (!trace || !trace.rows) return false;
  const prod = key === "ore" ? "orePerHr" : null;
  return trace.rows.some((r) => (r[key] || 0) > 0 || (prod && (r[prod] || 0) > 0));
}
function showResource(key) {
  if (key === "ore") return !!(meta && meta.resources && meta.resources.ore) || buildUsesResource("ore");
  return true; // platinum, food, lumber, mana, gems — always shown
}

const buildingLandOf = (b) => (meta && meta.buildingLand && meta.buildingLand[b]) || "plain";

async function init() {
  document.body.dataset.reducedMotion = String(matchMedia("(prefers-reduced-motion: reduce)").matches);
  setModeBadge();

  const sel = $("#raceSelect");
  for (const r of await engine.races()) { const o = document.createElement("option"); o.value = r; o.textContent = r.replace("-", " "); sel.appendChild(o); }
  sel.value = plan.race;
  sel.addEventListener("change", async () => { recordUndo("race"); plan.race = sel.value; await loadMeta(plan.race); recompute(0); });

  const dp = $("#dpInput");
  dp.value = plan.dpTarget;
  dp.addEventListener("input", () => { const nd = +dp.value || 0; if (nd !== plan.dpTarget) recordUndo("dp"); plan.dpTarget = nd; recompute(); });

  // post-OOP planning window: grows/shrinks plan.hours beyond the 48 protection hours.
  const postOop = $("#postOopInput");
  postOop.value = Math.max(0, planHours() - PROTECTION_HOURS);
  postOop.addEventListener("input", () => {
    const want = PROTECTION_HOURS + Math.max(0, Math.min(MAX_POST_OOP_HOURS, Math.floor(+postOop.value || 0)));
    if (want !== plan.hours.length) recordUndo("postoop");
    while (plan.hours.length < want) plan.hours.push([]);
    if (plan.hours.length > want) plan.hours.length = want;
    if (playhead > want + 1) setPlayhead(want + 1); // keep the playhead in range
    recompute();
  });

  spine = new Spine($("#spineCanvas"), (h) => setPlayhead(h));
  charts = new Charts($("#chartsGrid"), $("#chartsLegend"));

  document.querySelectorAll(".tab").forEach((t) => t.addEventListener("click", () => switchTab(t.dataset.tab)));

  await loadMeta(plan.race);
  editor = createEditor({ getPlan: () => plan, getTrace: () => trace, getMeta: () => meta, recompute, recordUndo, onNav: (h) => setPlayhead(h) });
  createSaves({
    getPlan: () => plan,
    applyPlan,
    getStats: async (p) => { const t = await engine.simulate(p); return { committed: t.final.committed, dp: Math.round(t.final.trainedModded), feasible: t.final.feasible }; },
    live: engine.live,
    saveBuild: (name, p) => engine.saveBuild(name, p),
    listSaves: () => engine.listSaves(),
    loadBuild: (path) => engine.loadBuild(path),
    deleteSave: (path) => engine.deleteSave(path),
  });

  // reset → blank canvas (keeps race + DP target). Two-click confirm so a build isn't lost by a stray click.
  const resetBtn = $("#resetBtn");
  let resetArmed = false, resetTimer = null;
  const disarmReset = () => { resetArmed = false; resetBtn.classList.remove("arm"); resetBtn.innerHTML = "↺&nbsp;reset"; };
  resetBtn.addEventListener("click", () => {
    if (!resetArmed) {
      resetArmed = true; resetBtn.classList.add("arm"); resetBtn.textContent = "↺ confirm?";
      clearTimeout(resetTimer); resetTimer = setTimeout(disarmReset, 2600);
      return;
    }
    clearTimeout(resetTimer); disarmReset();
    editor.close();
    applyPlan({ race: plan.race, dpTarget: plan.dpTarget, opening: {}, hours: Array.from({ length: DEFAULT_HOURS }, () => []), oopActions: [] });
  });

  // ⇩ log → export an importable protection log (.txt). The game's protection import
  // (LogParserService) replays these exact actions, so the file is formatted to that
  // parser's grammar. Built entirely from the bit-exact engine trace already on screen.
  $("#logBtn").addEventListener("click", () => {
    if (!trace) return;
    const { text, warnings, hours } = buildLog(plan, trace, meta);
    const fname = `overture-protection-log-${plan.race}-${plan.dpTarget || 0}dp.txt`;
    downloadLog(fname, text);
    const warns = [...warnings];
    if (infeasible.length) {
      const f = infeasible[0];
      warns.push(`${infeasible.length} overspent ${infeasible.length === 1 ? "hour" : "hours"} (h${String(f.hour).padStart(2, "0")} ${f.reason}) — the in-game import may stop there.`);
    }
    if (warns.length) toast(`⚠ exported <b>${fname}</b> · ${warns.join(" · ")}`, "warn");
    else toast(`⇩ exported <b>${fname}</b> — ${hours} protection ${hours === 1 ? "hour" : "hours"}, ready to import`, "ok");
  });

  $("#undoBtn").addEventListener("click", () => undo());
  $("#redoBtn").addEventListener("click", () => redo());
  updateHistoryButtons();

  document.addEventListener("keydown", onKey);
  window.addEventListener("resize", () => { renderLedger(); });

  // Per-hour overspend tooltip — delegated on the (persistent) ledger body so it survives re-renders;
  // shows instantly on hover/focus of a ⚠ marker, hides on leave or when the ledger scrolls.
  const lb = $("#ledgerBody");
  lb.addEventListener("mouseover", (e) => { const w = e.target.closest(".k-warn"); if (w) showHourTip(w); });
  lb.addEventListener("mouseout", (e) => { if (e.target.closest(".k-warn")) hideHourTip(); });
  lb.addEventListener("focusin", (e) => { const w = e.target.closest(".k-warn"); if (w) showHourTip(w); });
  lb.addEventListener("focusout", hideHourTip);
  const ls = $("#ledgerScroll"); if (ls) ls.addEventListener("scroll", hideHourTip, { passive: true });

  await recompute(0);
  spine.cv.focus();

  // periodic safety autosave (the debounced one covers active editing; this catches idle drift)
  setInterval(() => { if (engine.live && planHasContent()) engine.autosave(plan); }, 90000);
  // launch: offer to restore the most recent autosaved session (read BEFORE any new autosave)
  offerRestore();
}

function onKey(e) {
  // Undo / redo work globally — even while a topbar/editor input is focused — since the app's
  // plan history is the meaningful one here (our inputs are number/select, not rich text).
  const mod = e.metaKey || e.ctrlKey;
  if (mod && e.key.toLowerCase() === "z") { e.preventDefault(); if (e.shiftKey) redo(); else undo(); return; }
  if (mod && e.key.toLowerCase() === "y") { e.preventDefault(); redo(); return; } // Windows redo
  if (/INPUT|SELECT|TEXTAREA/.test(document.activeElement.tagName)) return;
  if (e.key === "ArrowRight") { setPlayhead(playhead + (e.shiftKey ? 6 : 1)); e.preventDefault(); }
  else if (e.key === "ArrowLeft") { setPlayhead(playhead - (e.shiftKey ? 6 : 1)); e.preventDefault(); }
  else if (e.key === "Home") setPlayhead(0);
  else if (e.key === "End") setPlayhead(lastRow());
  else if (e.key.toLowerCase() === "e") editor.open(playhead); // hour 0 → opening build
  else if (e.key === " ") { e.preventDefault(); togglePlay(); }
  else if (e.key === "1") switchTab("state");
  else if (e.key === "2") switchTab("oop");
}
function togglePlay() {
  if (playing) { clearInterval(playing); playing = null; return; }
  playing = setInterval(() => { if (playhead >= lastRow()) { clearInterval(playing); playing = null; } else setPlayhead(playhead + 1); }, 110);
}

async function recompute(editHour = null) {
  // The engine can reject a plan outright (a HARD sim error — distinct from the soft
  // overspend flag, which comes from a sim that *succeeded*). Catch it so the rejection
  // never escapes as an unhandled promise: keep the last good trace on screen, raise a
  // persistent banner, and still RESOLVE so callers' `.then(render)` runs and the UI stays
  // live. The next successful sim calls clearSimError(), so correcting the offending action
  // makes the error disappear on its own — no stale error left frozen on screen.
  let next;
  try {
    next = await engine.simulate(plan);
  } catch (e) {
    showSimError(e);
    return;
  }
  prev = trace;
  trace = next;
  clearSimError();
  renderAll(editHour);
  autosaveSoon();
}

/* ───────── autosave (desktop) — a rolling backup of the working build, offered back on launch.
   FS-backed and desktop-only; a BLANK canvas never autosaves, so the previous session's autosave
   survives until you actually start building (and the launch "restore" can recover it). ───────── */
let autosaveTimer = null;
function planHasContent() {
  if (plan.opening && Object.values(plan.opening).some((n) => (n || 0) > 0)) return true;
  return (plan.hours || []).some((h) => h && h.length);
}
function autosaveSoon() {
  if (!engine.live || !planHasContent()) return;
  clearTimeout(autosaveTimer);
  autosaveTimer = setTimeout(() => { if (planHasContent()) engine.autosave(plan); }, 4000);
}
async function offerRestore() {
  if (!engine.live || !engine.latestAutosave) return;
  let auto = null;
  try { auto = await engine.latestAutosave(); } catch (_) { return; }
  if (!auto || !auto.plan || !Array.isArray(auto.plan.hours)) return;
  toast(`↻ <b>restore last session?</b> <button class="toast-act" id="restoreBtn">restore</button>`, "warn");
  const b = document.getElementById("restoreBtn");
  if (b) b.onclick = () => { const t = document.getElementById("toast"); if (t) t.classList.remove("show"); applyPlan(auto.plan); };
}

// Load a saved/imported build: swap the plan, resync the topbar inputs + race meta,
// and re-simulate from a clean playhead (editHour=null → no wake flash).
async function applyPlan(p, opts = {}) {
  if (opts.history !== false) recordUndo("replace"); // snapshot the outgoing build so a load/reset/import is undoable
  plan = p;
  if (!Array.isArray(plan.hours)) plan.hours = [];
  while (plan.hours.length < DEFAULT_HOURS) plan.hours.push([]); // extend to the post-OOP horizon
  plan.opening = plan.opening || {};
  if (!Array.isArray(plan.oopActions)) plan.oopActions = [];
  $("#raceSelect").value = plan.race;
  $("#dpInput").value = plan.dpTarget;
  $("#postOopInput").value = Math.max(0, plan.hours.length - PROTECTION_HOURS);
  await loadMeta(plan.race);
  playhead = 0;
  prev = null;
  await recompute();
  setPlayhead(0);
}

function setPlayhead(h) {
  playhead = Math.max(0, Math.min(lastRow(), h));
  $("#playheadHour").textContent = String(playhead).padStart(2, "0");
  $("#playheadRem").textContent = playhead < OOP_HOUR ? `/ ${OOP_HOUR - playhead} to OOP`
    : (playhead === OOP_HOUR ? "/ ✦ OOP" : `/ +${playhead - OOP_HOUR} post-OOP`);
  $("#tabStateHour").textContent = "@" + String(playhead).padStart(2, "0");
  spine && spine.setPlayhead(playhead);
  charts && charts.setPlayhead(playhead);
  markActiveRow();
  if (tab === "state") renderState();
}

/* ───────── render ───────── */
function renderAll(editHour) {
  const f = trace.final;
  infeasible = scanFeasibility(trace); // reads the engine's honest per-row negatives directly
  augmentFlow(trace); // derive cumulative produced/spent + sink totals onto the rows
  renderScore(f);
  spine.setData(trace.rows, markers());
  spine.setPlayhead(playhead);
  charts.setData(trace.rows, { dpTarget: plan.dpTarget || 0 });
  charts.setPlayhead(playhead);
  renderLedger(editHour);
  renderInspector();
}

// Per-hour platinum produced/spent for the flow chart + OOP ledger. Rows are now POST-action
// (A_H), so hour h's production is exactly that row's platPerHr — the engine applies production
// at the post-action state and it lands in row h+1 — read directly, no forward delta. Hour 0
// (the opening / building phase) produces nothing. Spends come from editor.platinumFlow (priced
// at the row's own post-action costs). Writes cumProduced / cumSpent onto each row (the PLATINUM
// FLOW chart reads them) and stashes whole-build totals on trace.flowTotals (the ledger reads them).
function augmentFlow(trace) {
  const flow = platinumFlow(trace);
  const rows = trace.rows, final = trace.final;
  const totals = { produced: 0, claim: 0, bankNet: 0, spent: { explore: 0, construct: 0, rezone: 0, train: 0, improve: 0 } };
  let cp = 0, cs = 0;
  for (let h = 0; h < rows.length; h++) {
    const r = rows[h], fl = flow[h] || { sinks: {}, claim: 0, bankIn: 0, bankOut: 0 };
    const sinks = fl.sinks || {};
    const spentH = (sinks.explore || 0) + (sinks.construct || 0) + (sinks.rezone || 0) + (sinks.train || 0) + (sinks.improve || 0);
    // Hour h's production lands in row h+1; it's exactly this (post-action) row's platPerHr.
    const producedH = h >= 1 ? (r.platPerHr || 0) : 0;
    cp += producedH; cs += spentH;
    r.cumProduced = cp; r.cumSpent = cs;
    totals.produced += producedH; totals.claim += fl.claim || 0; totals.bankNet += (fl.bankIn || 0) - (fl.bankOut || 0);
    for (const k in totals.spent) totals.spent[k] += sinks[k] || 0;
  }
  totals.spentTotal = Object.values(totals.spent).reduce((a, b) => a + b, 0);
  totals.idleFinal = (final && final.platinum != null) ? final.platinum : (rows.length ? rows[rows.length - 1].platinum : 0);
  trace.flowTotals = totals;
}

// One-time mode badge: the desktop app runs the real bit-exact engine; the browser
// falls back to an APPROXIMATE mock that is NOT game-accurate — make that unmistakable.
function setModeBadge() {
  const b = $("#modeBadge");
  if (!b) return;
  if (engine.live) {
    b.className = "mode-badge engine";
    b.textContent = "● BIT-EXACT ENGINE";
    b.title = "Running the real game engine — every value is bit-exact vs the round-50 game.";
  } else {
    b.className = "mode-badge mock";
    b.textContent = "⚠ MOCK PREVIEW — NOT GAME-ACCURATE";
    b.title = "This is an approximate preview for design only. Numbers are NOT the real game's. Run the desktop app (the bit-exact engine) for true values.";
  }
}

// Transient bottom-center notice (export confirmations / warnings). Self-creates its
// node; warnings linger longer than confirmations.
let toastTimer = null;
function toast(html, kind = "ok") {
  let t = document.getElementById("toast");
  if (!t) { t = document.createElement("div"); t.id = "toast"; document.body.appendChild(t); }
  t.className = `toast ${kind} show`;
  t.innerHTML = html;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => t.classList.remove("show"), kind === "warn" ? 8000 : 4200);
}

// Hard simulation failure (the engine rejected the plan). Unlike the per-hour overspend
// warnings (ledger ⚠ tooltips), which come from a *successful* sim and refresh every render, a hard
// error means we have NO fresh trace — so we keep the last valid view and raise this
// persistent banner. It is cleared by clearSimError() on the very next successful sim,
// so fixing the offending action makes it vanish without any manual reset. Built with
// textContent (not innerHTML) so an engine message can never inject markup.
function showSimError(e) {
  const bar = $("#simError");
  if (!bar) return;
  bar.replaceChildren();
  const tag = document.createElement("span"); tag.className = "sim-error-tag"; tag.textContent = "SIM ERROR";
  const msg = document.createElement("span"); msg.className = "sim-error-msg"; msg.textContent = String(e && e.message ? e.message : e);
  const hint = document.createElement("span"); hint.className = "sim-error-hint"; hint.textContent = "showing last valid result — undo the latest action to recover";
  bar.append(tag, msg, hint);
  bar.hidden = false;
}
function clearSimError() {
  const bar = $("#simError");
  if (bar) bar.hidden = true;
}

function markers() {
  const m = [];
  const har = trace.rows.find((r) => r.spells.some((s) => s.key === "harmony")); if (har) m.push({ hour: har.hour, label: "Harmony", color: "#7e86d6" });
  const exp = trace.rows.find((r) => r.incoming > 0); if (exp) m.push({ hour: exp.hour, label: "explore", color: "#6aa8d8" });
  const def = trace.rows.find((r) => r.military.u2 + r.military.u3 > 0); if (def) m.push({ hour: def.hour, label: "defense", color: "#d8d2c2" });
  // OUT OF PROTECTION — hour 49, the headline. `oop: true` makes the spine render it as a
  // bold, unmistakable divider (not a small flag).
  m.push({ hour: OOP_HOUR, label: trace.final.feasible ? "OOP ✓" : "OOP", color: trace.final.feasible ? "#5fd08a" : "#e3a93f", oop: true });
  return m;
}

function renderScore(f) {
  $("#oopLand").textContent = int(f.committed);
  const gate = $("#gate"), gl = $("#gateLabel"), gd = $("#gateDetail");
  const ok = f.feasible;
  const short = f.targetShort;
  gate.dataset.state = ok ? "cleared" : "short";
  gl.textContent = ok ? "CLEARED" : "SHORT";
  gd.textContent = ok ? `${int(f.trainedModded)} ≥ ${int(f.dpTarget)}` : `−${int(short)} DP`;
  if (lastFeasible !== null && lastFeasible !== ok && ok && document.body.dataset.reducedMotion !== "true") {
    gate.classList.remove("latch"); void gate.offsetWidth; gate.classList.add("latch");
  }
  lastFeasible = ok;
  // Overspend warnings are surfaced PER HOUR as a ⚠ marker + hover tooltip on each affected ledger
  // row (see renderLedger) — there is no topbar summary box anymore (it crowded the score bar).
}

function renderLedger(editHour = null) {
  const head = $("#ledgerHead"), body = $("#ledgerBody");
  const cols = COLUMNS.filter((c) => showResource(c.key)); // per-race column visibility (ore/gems)
  head.innerHTML = "<tr>" + `<th><span class="col-key">H</span></th>` +
    cols.map((c) => `<th><span class="col-key" style="--col:${c.c}">${c.label}</span></th>`).join("") + "</tr>";
  const badReason = new Map(infeasible.map((b) => [b.hour, b.reason])); // hour → overspend reason (per-row tooltip)
  let html = "";
  for (const r of trace.rows) {
    // Prominent OUT OF PROTECTION divider right before hour 49 (the headline boundary).
    if (r.hour === OOP_HOUR) {
      html += `<tr class="ledger-oop-sep" aria-hidden="true"><td colspan="${cols.length + 1}"><span>◆ OUT OF PROTECTION · hour 49 ◆</span></td></tr>`;
    }
    const day = r.hour > 0 && r.hour % 24 === 0;
    const hasAct = (r.actions || []).length > 0;
    const reason = badReason.get(r.hour);    // overspend reason for this hour, if any
    const bad = reason != null;
    const warnMsg = reason ? `overspent this hour — ${reason}`.replace(/"/g, "&quot;") : ""; // custom-tooltip text
    const postoop = r.hour >= OOP_HOUR;     // hour 49+ = out of protection
    const oop = r.hour === OOP_HOUR;        // the OOP row itself (the headline)
    // L/P daily-bonus markers — a quiet at-a-glance cue for hours that claim the land or platinum bonus.
    const claimL = (r.actions || []).some((a) => a.type === "claim_land");
    const claimP = (r.actions || []).some((a) => a.type === "claim_platinum");
    const claimMarks = `${claimL ? `<span class="k-claim k-claim-l" title="claims +20 land this hour">L</span>` : ""}${claimP ? `<span class="k-claim k-claim-p" title="claims the platinum bonus this hour">P</span>` : ""}`;
    html += `<tr data-h="${r.hour}" class="${day ? "is-day" : ""} ${r.hour === playhead ? "is-active" : ""} ${bad ? "is-infeas" : ""} ${r.hour === 0 ? "is-opening" : ""} ${postoop ? "is-postoop" : ""} ${oop ? "is-oop" : ""}">`;
    html += `<td class="k-hour ${hasAct ? "has-action" : ""}"><span class="k-hour-n">${String(r.hour).padStart(2, "0")}</span>${claimMarks}${reason ? `<span class="k-warn" tabindex="0" role="img" aria-label="${warnMsg}" data-warn="${warnMsg}">⚠</span>` : ""}</td>`;
    for (const c of cols) {
      const v = c.get(r);
      const neg = v < 0;
      const changed = editHour != null && prev && prev.rows[r.hour] && c.get(prev.rows[r.hour]) !== v && r.hour >= editHour;
      const delay = changed ? Math.min(360, (r.hour - editHour) * 10) : 0;
      html += `<td class="col-c ${neg ? "neg" : ""} ${changed ? "wake" : ""}" style="--col:${c.dim ? "var(--text-faint)" : c.c}${changed ? `;animation-delay:${delay}ms` : ""}">${c.disp ? c.disp(r) : int(v)}</td>`;
    }
    html += "</tr>";
  }
  body.innerHTML = html;
  body.querySelectorAll("tr[data-h]").forEach((tr) => {
    tr.addEventListener("click", () => { const h = +tr.dataset.h; setPlayhead(h); editor.open(h); }); // h=0 → opening build
  });
}
/* Snappy per-hour warning tooltip — instant show/hide on the ⚠ marker (no native-title lag).
   One shared element, positioned beside the hovered marker and clamped to the viewport. */
let hourTip = null;
function showHourTip(el) {
  if (!hourTip) {
    hourTip = document.createElement("div");
    hourTip.className = "hour-tip";
    hourTip.hidden = true;
    document.body.appendChild(hourTip);
  }
  hourTip.textContent = "⚠ " + (el.dataset.warn || "overspent this hour");
  hourTip.hidden = false;
  const r = el.getBoundingClientRect();
  let left = r.right + 8, top = r.top - 4;
  if (left + hourTip.offsetWidth > window.innerWidth - 8) left = r.left - hourTip.offsetWidth - 8; // flip left if it'd clip
  top = Math.min(top, window.innerHeight - hourTip.offsetHeight - 8);
  hourTip.style.left = Math.max(8, left) + "px";
  hourTip.style.top = Math.max(8, top) + "px";
}
function hideHourTip() { if (hourTip) hourTip.hidden = true; }

function markActiveRow() {
  document.querySelectorAll("#ledgerBody tr").forEach((tr) => tr.classList.toggle("is-active", +tr.dataset.h === playhead));
  const active = document.querySelector("#ledgerBody tr.is-active");
  if (active) active.scrollIntoView({ block: "nearest" });
}

/* ───────── inspector ───────── */
function switchTab(t) {
  tab = t;
  document.querySelectorAll(".tab").forEach((el) => el.classList.toggle("is-active", el.dataset.tab === t));
  document.querySelectorAll(".tabpane").forEach((el) => el.classList.toggle("is-active", el.dataset.pane === t));
  renderInspector();
}
function renderInspector() {
  if (tab === "oop") renderOop();
  else renderState();
}

function group(title, rows) {
  return `<div class="stat-group"><h3>${title}</h3>${rows.map(([l, v, d]) => `<div class="stat-row"><span class="lab">${l}</span><span class="val ${d ? "dim" : ""}">${v}</span></div>`).join("")}</div>`;
}
function stateBlock(r, isFinal) {
  const m = r.military;
  const bchips = Object.entries(r.buildings).filter(([, n]) => n > 0).map(([k, n]) => `<span class="bchip"><b>${n}</b> ${k}</span>`).join("");
  const land = `${int(r.land + (r.incoming || 0))} (${int(r.land)} + ${int(r.incoming)} incoming)`;
  let html = "";
  if (isFinal) {
    const slack = Math.round(r.trainedModded - r.dpTarget);
    html += group("verdict", [
      ["committed land", int(r.committed)],
      ["trained DP target", int(r.dpTarget)],
      ["status", r.feasible ? "✓ feasible" : `short −${int(r.targetShort)}`],
      ["DP slack", slack >= 0 ? `<span style="color:var(--green)">+${int(slack)} over target</span>` : `<span style="color:var(--amber)">−${int(-slack)} short</span>`, true],
    ]);
  }
  // barren = land − built − ALL construction (the game's getTotalBarrenLand; acres queued
  // for construction this tick are already excluded). Goes negative if over-built — shown
  // in red rather than clamped, so a flagged over-construction reads honestly.
  const barren = r.barren ?? 0;
  const barrenVal = barren < 0 ? `<span style="color:var(--red)">${int(barren)}</span>` : int(barren);
  html += group("land", [["committed", land], ["barren", barrenVal], ...LANDS.filter((l) => r.landBy[l]).map((l) => [l, int(r.landBy[l]), true])]);
  html += group("defense (trained — draftees excluded)", [
    ["raw", int(r.trainedRaw)], ["modded", int(r.trainedModded)], ["multiplier", "×" + r.mult.toFixed(3), true],
  ]);
  html += group("offense (trained — base, no target)", [
    ["raw", int(r.trainedOpRaw || 0)], ["modded", int(r.trainedOpModded || 0)], ["multiplier", "×" + (r.opMult || 1).toFixed(3), true],
  ]);
  html += `<div class="stat-group"><h3>buildings</h3><div class="chip-row">${bchips || '<span class="empty">none</span>'}</div></div>`;
  html += capsBlock(r);
  html += militaryBlock(r);
  html += group("population", [["peasants", int(r.peasants)], ["max-pop", int(r.maxPop), true], ["morale", int(r.morale), true]]);
  html += employmentBlock(r);
  html += group("production / hr", [
    ["platinum", int(r.platPerHr)], ["food net", int(r.foodNet)],
    ["lumber", int(r.lumberPerHr), true],
    ...(showResource("ore") ? [["ore", int(r.orePerHr), true]] : []),
    ["mana", int(r.manaPerHr), true], ["gems", int(r.gemPerHr || 0), true],
    ["research", int(r.techPerHr || 0), true],
  ]);
  html += group("resources", [
    ["platinum", int(r.platinum)], ["food", int(r.food)], ["lumber", int(r.lumber)],
    ...(showResource("ore") ? [["ore", int(r.ore)]] : []),
    ["mana", int(r.mana)], ["gems", int(r.gems)],
    ["research", int(r.tech || 0)],
    ...((r.boats || 0) > 0 ? [["boats", int(r.boats), true]] : []),
  ]);
  if (r.spells.length) html += `<div class="stat-group"><h3>active spells</h3><div class="spellbar">${r.spells.map((s) => `<span class="spell"><span class="dot" style="background:${col.mana}"></span>${s.key.replace("_", " ")} ${s.dur}h</span>`).join("")}</div></div>`;
  return html;
}
function renderState() { $("#paneState").innerHTML = stateBlock(trace.rows[playhead], false); }
function renderOop() { $("#paneOop").innerHTML = stateBlock(trace.final, true) + flowSummary(trace.flowTotals); }

// Whole-build platinum accounting (#1): produced vs spent-by-sink + idle reserve.
// Reads the totals augmentFlow stashed on the trace.
function flowSummary(t) {
  if (!t) return "";
  const pct = (v) => (t.spentTotal > 0 ? ` <span class="dim">${Math.round((v / t.spentTotal) * 100)}%</span>` : "");
  const sink = (lab, v) => (v > 0 ? [lab, int(v) + pct(v), true] : null);
  const rows = [
    ["produced", int(t.produced)],
    ["spent", int(t.spentTotal)],
    sink("· explore", t.spent.explore), sink("· construct", t.spent.construct),
    sink("· rezone", t.spent.rezone), sink("· train", t.spent.train), sink("· improve", t.spent.improve),
  ].filter(Boolean);
  if (t.claim) rows.push(["daily claims", "+" + int(t.claim), true]);
  if (t.bankNet) rows.push(["banked net", (t.bankNet >= 0 ? "+" : "−") + int(Math.abs(t.bankNet)), true]);
  rows.push(["idle at OOP", int(t.idleFinal)]);
  return group("platinum ledger · 0 → OOP", rows);
}

// Employment as a balance, not a bare ratio (#8): is the bottleneck workers (too many
// peasants for the jobs → add job buildings) or housing (jobs the current cap can never
// staff → add homes / re-house troops with barracks)? Engine-sourced constants. The
// prescription renders as a full-width callout (not a label/value row) so it can't crowd.
function employmentBlock(r) {
  const e = r.employment;
  if (!e) return ""; // engine field absent (older mock) → omit gracefully
  const jpb = e.jobsPerBuilding || 20, hph = e.housingPerHome || 30;
  const open = e.jobs - e.employed;
  const structuralUnfilled = Math.max(0, e.jobs - e.maxPeasantPop);
  let kind, head, detail;
  if (e.peasants > e.jobs) {
    const need = Math.ceil((e.peasants - e.jobs) / jpb);
    kind = "warn"; head = `${int(e.peasants - e.jobs)} surplus workers`;
    detail = `+${need} job building${need === 1 ? "" : "s"} would employ them`;
  } else if (structuralUnfilled > 0) {
    const homes = Math.ceil(structuralUnfilled / hph);
    kind = "warn"; head = `${int(structuralUnfilled)} jobs unstaffable`;
    detail = `+${homes} home${homes === 1 ? "" : "s"} raises the cap (or barracks to re-house troops)`;
  } else if (open > 0) {
    kind = "info"; head = `${int(open)} open jobs`;
    detail = `peasants still growing into them`;
  } else {
    kind = "ok"; head = `fully employed`;
    detail = "";
  }
  const rows = [
    ["employed", `${int(e.employed)} <span class="dim">/ ${int(e.jobs)} jobs</span>`],
    ["peasant housing", `${int(e.peasants)} <span class="dim">/ ${int(e.maxPeasantPop)}</span>`, true],
  ].map(([l, v, d]) => `<div class="stat-row"><span class="lab">${l}</span><span class="val ${d ? "dim" : ""}">${v}</span></div>`).join("");
  const callout = `<div class="emp-balance emp-${kind}"><b>${head}</b>${detail ? `<span>${detail}</span>` : ""}</div>`;
  return `<div class="stat-group"><h3>employment</h3>${rows}${callout}</div>`;
}

// "N buildings from cap" for the ratio-of-land capped buildings (#new). Only buildings
// you've actually invested in are shown — cap headroom is meaningless at zero.
const CAP_LABELS = { guard_tower: "guard tower", gryphon_nest: "gryphon nest", smithy: "smithy", factory: "factory", school: "school" };
// Caps are framed the way players think of them: a building's share of land (GT caps at
// 20% of land, smithy 18%, factory 10%, school 50%) — i.e. "now% / cap%", not the bonus %.
function capsBlock(r) {
  const caps = r.caps;
  if (!caps) return "";
  const land = r.land || 1;
  const rows = [];
  for (const [k, lbl] of Object.entries(CAP_LABELS)) {
    const c = caps[k];
    if (!c || c.count <= 0) continue;
    const curPct = Math.round((c.count / land) * 100);
    const capPct = Math.round((c.capCount / land) * 100);
    const toCap = c.capCount - c.count;
    let status;
    if (toCap > 0) status = `<span class="dim">${int(toCap)} to go</span>`;
    else if (toCap === 0) status = `<span style="color:var(--green)">at cap</span>`;
    else status = `<span style="color:var(--amber)">${int(-toCap)} over</span>`;
    rows.push([lbl, `${int(c.count)} · ${curPct}% / ${capPct}% cap · ${status}`, false]);
  }
  if (!rows.length) return "";
  return group("capped buildings", rows);
}

// Full military readout — every race unit slot (with offense/defense), plus the
// espionage/magic units when present. Reflects all unit types the game tracks, not
// just the two defensive slots the studio used to surface.
function militaryBlock(r) {
  const m = r.military || {};
  const units = (meta && meta.units && meta.units.length)
    ? meta.units
    : [1, 2, 3, 4].map((s) => ({ slot: s, name: "slot " + s, offense: 0, defense: 0 }));
  const rows = units.map((u) => [
    `${u.name} <span class="dim">${u.offense || 0}o/${u.defense || 0}d</span>`,
    int(m["u" + u.slot] || 0),
  ]);
  for (const [k, lbl] of [["spies", "spies"], ["assassins", "assassins"], ["wizards", "wizards"], ["archmages", "archmages"]]) {
    if ((m[k] || 0) > 0) rows.push([lbl, int(m[k]), true]);
  }
  rows.push(["draftees", int(m.draftees || 0), true]);
  return group("military", rows);
}

init();
