// editor.js — the action console. A tabbed modal exposing every legal action for a
// given hour, with EXACT resource-constraint enforcement.
//
// Enforcement model: the engine renders each row as the POST-instant-action state A_H
// (the entering wallet with that hour's instant actions already replayed on top — spell
// mana spent, dailies claimed, rezones moved, queued-build/explore/train PAYMENTS made,
// and the land-scaled costs re-priced after any same-tick claim_land). So row[hour] IS
// the wallet remaining after everything queued this hour, and `row.costs` already prices
// the NEXT action at the post-claim land size. The editor therefore reads the wallet
// straight off the row — no JS replay of the committed actions (which is what used to
// drift / overstate) — and only previews the ONE uncommitted action on top of it. The
// engine never clamps the soft actions under protection (it lets the spend go negative),
// so the row carries honest negatives and feasibility is read directly off them.

import { renderTechTree } from "./techtree.js";
import { listOpenings, saveOpening, deleteOpening, openingAcres } from "./openings.js";
import { mountHourGrid } from "./hourgrid.js";

const RES = [
  ["platinum", "--c-plat"], ["lumber", "--c-lumber"], ["ore", "--c-ore"],
  ["mana", "--c-mana"], ["gems", "--c-gems"], ["draftees", "--c-draftee"], ["tech", "--text-dim"],
];
const LAND_TYPES = ["plain", "swamp", "hill", "mountain", "forest", "cavern", "water"];
// Actions the engine NEVER clamps to affordability under protection — it executes them
// and lets the spent resource / barren land go negative (ConstructActionService et al.
// only gate outside protection; the protection importer replays the queue as-is). The
// editor mirrors that exactly: it shows the resulting negative budget and FLAGS the hour,
// but never blocks the edit — a later adjustment may fund it. The engine-gated actions
// (spell/bank/improve/research) and the physical ones (destroy/release/daily) are NOT here.
const SOFT_OVERSPEND = new Set(["construct", "rezone", "explore", "train"]);
// Timeline: protection is hours 1..48; hour 49 = OUT OF PROTECTION; 50.. = post-OOP window.
const PROTECTION_HOURS = 48, OOP_HOUR = 49;
// Self-spells are now data-driven per race (meta().spells) — see the Magic tab.
const BANKABLE = ["platinum", "lumber", "ore", "gems"];
// Bank exchange (BankingCalculator; bonus = 1 in protection): received = floor(amt·sell·buy).
const BANK_SELL = { platinum: 0.5, lumber: 0.5, ore: 0.5, gems: 2.0, food: 0.0 };
const BANK_BUY = { platinum: 1.0, lumber: 1.0, ore: 1.0, gems: 0.0, food: 0.5 };
const IMPROVEMENTS = ["science", "keep", "walls", "spires", "forges", "harbor"];

const TABS = [
  ["build", "Build"], ["rezone", "Rezone"], ["explore", "Explore"], ["train", "Train"],
  ["magic", "Magic"], ["bank", "Bank"], ["daily", "Daily"], ["manage", "Manage"], ["techs", "Techs"],
];

const int = (n) => Math.round(n || 0).toLocaleString("en-US");
// HTML-escape any plan-derived string before it lands in innerHTML. Plan fields can come from a
// hand-edited or imported *.overture.json, so they're untrusted. The CSP already blocks inline
// script, so this is defense-in-depth — but it also keeps a stray "<" or "&" in a label from
// silently corrupting the queue markup.
const esc = (s) => String(s == null ? "" : s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
const el = (html) => { const t = document.createElement("template"); t.innerHTML = html.trim(); return t.content.firstElementChild; };

export function createEditor(deps) {
  // deps: { getPlan(), getTrace(), getMeta(), recompute(editHour), recordUndo(label) }
  // recordUndo(label) snapshots the plan for undo/redo BEFORE a mutation (see app.js history).
  const scrim = document.getElementById("editorScrim");
  const root = document.getElementById("editor");
  let hour = 1, tab = "build", manageKind = "draft_rate", buildSel = "home", buildDir = "build", trainDir = "train";
  let exploreSel = "plain", trainSel = 1; // the lane currently picked in the Explore / Train windows

  const plan = () => deps.getPlan();
  const meta = () => deps.getMeta() || { units: [], techs: [], buildingLand: {} };
  const buildingLand = (b) => (meta().buildingLand || {})[b] || "plain";
  const homeLand = () => meta().homeLand || "plain";

  // Lane-aware live "consequence" columns for the embedded ±6h entry window (mirrors the ledger): the
  // 3rd column tracks what the lane spends — lumber for Build, draftees for Explore/Train.
  const SC = {
    land: { key: "land", label: "land", c: "--c-land", get: (r) => (r.land || 0) + (r.incoming || 0) },
    plat: { key: "plat", label: "plat", c: "--c-plat", get: (r) => r.platinum },
    draft: { key: "draft", label: "draft", c: "--c-draftee", get: (r) => r.draftees },
    lumber: { key: "lumber", label: "lmbr", c: "--c-lumber", get: (r) => r.lumber },
    dp: { key: "dp", label: "DP", c: "--c-dp", get: (r) => Math.round(r.trainedModded || 0) },
  };
  const buildStateCols = [SC.land, SC.plat, SC.lumber, SC.dp];
  const spendStateCols = [SC.land, SC.plat, SC.draft, SC.dp];
  // Read/upsert one lane's quantity in a given hour's action list (match recognizes the lane's actions,
  // make(n) builds a fresh one) — the embedded window's cell read/write.
  const laneRead = (h, match) => (plan().hours[h - 1] || []).filter(match).reduce((s, a) => s + (a.n | 0), 0);
  function laneWrite(h, match, make, value) {
    const list = plan().hours[h - 1] || (plan().hours[h - 1] = []);
    const keep = []; let done = false;
    for (const a of list) { if (match(a)) { if (!done && value > 0) { a.n = value; keep.push(a); done = true; } } else keep.push(a); }
    if (!done && value > 0) keep.push(make(value));
    plan().hours[h - 1] = keep;
  }
  const rowAt = (h) => { const rows = deps.getTrace().rows; return rows[Math.min(rows.length - 1, h)]; };
  // Shared opts for every embedded window: centered on the current editor hour, refreshing the
  // budget strip + queue (not the form) after each commit so the window keeps focus.
  const windowOpts = (extra) => Object.assign({
    center: hour, radius: 6, maxHour: plan().hours.length, oopHour: OOP_HOUR,
    rowAt, recordUndo: deps.recordUndo, recompute: deps.recompute, afterCommit: () => renderChrome(),
  }, extra);

  /* ───────── wallet (read straight off the post-instant-action row) ───────── */
  // Row H is the POST-instant-action state A_H (the engine replays hour H's instant actions
  // onto the entering wallet, re-pricing land-scaled costs after any same-tick claim_land). So
  // row[hour] already IS the wallet remaining after everything queued this hour, and its costs
  // price the NEXT action at the post-claim land size — no JS replay needed.
  function entryRow() {
    const rows = deps.getTrace().rows;
    return rows[Math.min(rows.length - 1, hour)];
  }
  function freshWallet(row) {
    const m = row.military || {};
    return {
      platinum: row.platinum, lumber: row.lumber, ore: row.ore, mana: row.mana,
      gems: row.gems, tech: row.tech || 0, draftees: row.draftees, peasants: row.peasants || 0,
      free: { ...row.freeLandByType },
      buildings: { ...row.buildings },
      units: { u1: m.u1 || 0, u2: m.u2 || 0, u3: m.u3 || 0, u4: m.u4 || 0, draftees: row.draftees },
      dailyPlatinum: row.dailyPlatinum, dailyLand: row.dailyLand,
      techs: [...(row.techs || [])],
      costs: row.costs,
    };
  }
  // Apply one action to a wallet. Returns null if affordable, or a string reason if it
  // overspends. `dry` probes for the reason WITHOUT mutating. When NOT dry, the actions
  // the engine never clamps (SOFT_OVERSPEND: construct/rezone/explore/train) ALWAYS apply
  // their resource/land deltas — even into the negative — so the wallet mirrors the
  // lenient engine and the "remaining this hour" budget shows the true (possibly negative)
  // figure instead of silently skipping an unaffordable queued action (which overstated
  // the budget). Engine-gated/clamped actions (spell/bank/improve/research) and physical
  // ones (destroy/release/daily) still only apply when valid.
  function applyOne(w, a, dry) {
    const c = w.costs, n = a.n | 0;
    const need = (res, amt) => amt > w[res] ? `${res} short by ${int(amt - w[res])}` : null;
    let r = null;
    switch (a.type) {
      case "construct": {
        const lt = buildingLand(a.building);
        if (n > w.free[lt]) r = `only ${int(w.free[lt])} free ${lt} land`;
        else r = need("platinum", n * c.constructPlat) || need("lumber", n * c.constructLumber);
        if (!dry) { w.platinum -= n * c.constructPlat; w.lumber -= n * c.constructLumber; w.free[lt] -= n; }
        break;
      }
      case "rezone": {
        if (n > w.free[a.from]) r = `only ${int(w.free[a.from])} barren ${a.from} land`;
        else r = need("platinum", n * c.rezonePlat);
        if (!dry) { w.platinum -= n * c.rezonePlat; w.free[a.from] -= n; w.free[a.to] += n; }
        break;
      }
      case "explore": {
        r = need("platinum", n * c.explorePlat) || need("draftees", n * c.exploreDraftee);
        if (!dry) { w.platinum -= n * c.explorePlat; w.draftees -= n * c.exploreDraftee; w.units.draftees -= n * c.exploreDraftee; }
        break;
      }
      case "train": {
        const t = c.train[a.slot] || {}; // {resource: per-unit cost} — any of platinum/ore/mana/lumber/gems
        r = need("draftees", n);
        for (const [res, per] of Object.entries(t)) if (!r) r = need(res, n * per);
        if (!dry) { for (const [res, per] of Object.entries(t)) w[res] -= n * per; w.draftees -= n; w.units.draftees -= n; }
        break;
      }
      case "spell": {
        // self-spells are gated: if mana is short the cast is simply skipped (a
        // no-op, never a negative balance), so this is informational, not a block.
        const cost = (c.spell || {})[a.spell] || 0;
        if (!dry && cost <= w.mana) w.mana -= cost;
        break;
      }
      case "bank": {
        const src = (a.source || "").replace("resource_", ""), tgt = (a.target || "").replace("resource_", "");
        const amt = a.amount | 0;
        if (amt > (w[src] ?? 0)) r = `${src} short by ${int(amt - (w[src] ?? 0))}`;
        // Credit the target same-hour (the engine applies it immediately), so a later
        // bank→train/build in the same hour isn't falsely blocked.
        if (!r && !dry) { w[src] -= amt; w[tgt] = (w[tgt] ?? 0) + Math.floor(amt * (BANK_SELL[src] || 0) * (BANK_BUY[tgt] || 0)); }
        break;
      }
      case "improve": {
        if ((a.amount | 0) > (w[a.resource] ?? 0)) r = `${a.resource} short by ${int((a.amount | 0) - (w[a.resource] ?? 0))}`;
        if (!r && !dry) w[a.resource] -= a.amount | 0;
        break;
      }
      case "research": {
        if (w.techs.includes(a.tech)) r = "already researched";
        else if (c.techCost > w.tech) r = `tech short by ${int(c.techCost - w.tech)}`;
        if (!r && !dry) { w.tech -= c.techCost; w.techs.push(a.tech); }
        break;
      }
      case "destroy": {
        if (n > (w.buildings[a.building] || 0)) r = `only ${int(w.buildings[a.building] || 0)} ${a.building}`;
        // Razed buildings free their land as barren (same-hour), so destroy→build/rezone
        // on the freed land isn't falsely blocked.
        if (!r && !dry) { w.buildings[a.building] -= n; const lt = buildingLand(a.building); w.free[lt] = (w.free[lt] || 0) + n; }
        break;
      }
      case "release": {
        const src = a.unit === "draftees" ? "draftees" : "u" + a.slot;
        if (n > (w.units[src] || 0)) r = `only ${int(w.units[src] || 0)} available`;
        if (!r && !dry) { w.units[src] -= n; if (src === "draftees") {} else { w.units.draftees += n; w.draftees += n; } }
        break;
      }
      case "claim_platinum": { if (w.dailyPlatinum) r = "already claimed today"; if (!r && !dry) { w.dailyPlatinum = true; w.platinum += (w.peasants || 0) * 4; w.tech += 350; } break; }
      case "claim_land": { if (w.dailyLand) r = "already claimed today"; if (!r && !dry) { w.dailyLand = true; const h = homeLand(); w.free[h] = (w.free[h] || 0) + 20; } break; }
      case "draft_rate": break;
    }
    return r;
  }
  // Wallet remaining after the hour's already-queued actions (the budget a NEW action draws
  // from). Row H = A_H already has every committed action of this hour applied by the engine,
  // so this is just the row's wallet — replaying the actions here would double-count them.
  function remainingWallet() {
    return freshWallet(entryRow());
  }
  // Largest LEGAL n for a quantity action, plus which constraint binds (a resource
  // shortfall or a physical limit like barren acres / building count). Every candidate
  // ceiling is computed from the same exact within-hour cost table the gate uses.
  function maxDetailed(w, a) {
    const c = w.costs, f = (x) => Math.max(0, Math.floor(x));
    let cands;
    switch (a.type) {
      case "construct": { const lt = buildingLand(a.building); cands = [
        { n: w.free[lt], why: `free ${lt} land` },
        { n: f(w.platinum / Math.max(1, c.constructPlat)), why: "platinum" },
        { n: f(w.lumber / Math.max(1, c.constructLumber)), why: "lumber" }]; break; }
      case "rezone": cands = [
        { n: w.free[a.from], why: `barren ${a.from} land` },
        { n: f(w.platinum / Math.max(1, c.rezonePlat)), why: "platinum" }]; break;
      case "explore": cands = [
        { n: f(w.platinum / Math.max(1, c.explorePlat)), why: "platinum" },
        { n: f(w.draftees / Math.max(1, c.exploreDraftee)), why: "draftees" }]; break;
      case "train": { const t = c.train[a.slot] || {}; cands = [{ n: w.draftees, why: "draftees" }];
        for (const [res, per] of Object.entries(t)) cands.push({ n: f(w[res] / Math.max(1, per)), why: res }); break; }
      case "bank": { const s = (a.source || "").replace("resource_", ""); cands = [{ n: w[s] ?? 0, why: s }]; break; }
      case "improve": cands = [{ n: w[a.resource] ?? 0, why: a.resource }]; break;
      case "destroy": cands = [{ n: w.buildings[a.building] || 0, why: (a.building || "").replace(/_/g, " ") }]; break;
      case "release": cands = [{ n: a.unit === "draftees" ? w.units.draftees : (w.units["u" + a.slot] || 0), why: "available" }]; break;
      default: return { n: 0, why: "" };
    }
    cands = cands.map((x) => ({ n: Math.max(0, x.n | 0), why: x.why }));
    return cands.reduce((m, x) => (x.n < m.n ? x : m), cands[0]);
  }

  /* ───────── plan access ───────── */
  const acts = () => (plan().hours[hour - 1] || (plan().hours[hour - 1] = []));
  function commit(a) {
    deps.recordUndo("edit");
    acts().push(a);
    deps.recompute(hour).then(render);
  }
  function removeAt(i) { deps.recordUndo("edit"); acts().splice(i, 1); deps.recompute(hour).then(render); }
  // Inline-edit a queued action's quantity straight from the QUEUED list — same commit path as
  // adding (snapshot for undo → mutate → re-simulate → re-render). Seamless: no delete-and-re-add.
  function editQty(i, key, raw) {
    const list = acts();
    if (!list[i]) return;
    deps.recordUndo("edit");
    const n = Math.max(0, Math.floor(+raw || 0));
    list[i][key] = key === "rate" ? Math.min(100, n) : n;
    deps.recompute(hour).then(render);
  }
  // Label + the one inline-editable numeric field (if any) + a colour `kind`, for a queued action.
  function describeAct(a) {
    const bld = (b) => (b || "").replace(/_/g, " ");
    switch (a.type) {
      case "construct": return { desc: `build ${bld(a.building)}`, edit: { key: "n", val: a.n }, kind: "k-build" };
      case "destroy": return { desc: `destroy ${bld(a.building)}`, edit: { key: "n", val: a.n }, kind: "k-danger" };
      case "rezone": return { desc: `rezone ${a.from}→${a.to}`, edit: { key: "n", val: a.n } };
      case "explore": return { desc: `explore${a.land ? " " + a.land : ""}`, edit: { key: "n", val: a.n } };
      case "train": return { desc: `train ${typeof a.slot === "string" ? a.slot : "slot " + a.slot}`, edit: { key: "n", val: a.n }, kind: "k-build" };
      case "release": return { desc: `release ${a.unit === "draftees" ? "draftees" : "slot " + a.slot}`, edit: { key: "n", val: a.n }, kind: "k-demob" };
      case "bank": return { desc: `bank ${(a.source || "").replace("resource_", "")}→${(a.target || "").replace("resource_", "")}`, edit: { key: "amount", val: a.amount } };
      case "improve": return { desc: `invest ${a.resource}→${Object.keys(a.data || {})[0] || ""}`, edit: { key: "amount", val: a.amount } };
      case "draft_rate": return { desc: "draft rate", edit: { key: "rate", val: a.rate, unit: "%" } };
      case "spell": return { desc: `cast ${bld(a.spell)}`, edit: null };
      case "research": { const t = (meta().techs || []).find((x) => x.key === a.tech); return { desc: `research ${t ? t.name : bld(a.tech)}`, edit: null }; }
      case "claim_platinum": return { desc: "claim platinum", edit: null };
      case "claim_land": return { desc: "claim land", edit: null };
      default: return { desc: a.type, edit: null };
    }
  }

  /* ───────── render ───────── */
  function open(h) {
    if (root.hidden) { buildDir = "build"; trainDir = "train"; } // a FRESH open always defaults to producing, never destroy/release
    hour = Math.max(0, Math.min(plan().hours.length, h)); // hour 0 = opening; 1..N editable (post-OOP incl.)
    root.hidden = false; scrim.hidden = false;
    scrim.onclick = close;
    render();
  }
  function close() { root.hidden = true; scrim.hidden = true; }
  // Re-render the open popover in place (e.g. after an external undo/redo changed this hour's
  // queued actions). Clamps the hour in case post-OOP hours were removed by the undo.
  function rerender() { if (root.hidden) return; hour = Math.max(0, Math.min(plan().hours.length, hour)); render(); }

  // Per-race resource visibility (mirrors app.js showResource): ORE is the only conditional one — it
  // hides for races whose units never cost it (Merfolk etc.) unless the build actually holds ore. Hides
  // ore from the budget strip + bank/improve pickers so a Merfolk player never sees an ore field. Everything
  // else stays — platinum/lumber/mana (universal) and gems (everyone builds diamond mines).
  function showRes(key) {
    if (key !== "ore") return true;
    const res = meta().resources || {};
    const r = entryRow();
    return !!res.ore || (r.ore || 0) > 0 || (r.orePerHr || 0) > 0;
  }
  function budgetStrip(w) {
    const chip = ([k, v]) => `<span class="bud-chip ${w[k] < 0 ? "neg" : ""}" style="--c:var(${v})"><i></i><b>${int(w[k])}</b><span>${k}</span></span>`;
    return `<div class="ed-budget"><span class="ed-budget-cap">remaining this hour</span><div class="bud-row">${RES.filter(([k]) => showRes(k)).map(chip).join("")}</div></div>`;
  }

  // Hour 0 is the free + instant opening build: fill your 350 starting acres with
  // ANY buildings — each is auto-zoned to its land type (the engine's openingBuild
  // places buildings free and the calcs are land-type-agnostic). This is the only
  // free build; everything after costs platinum/lumber + 12h via the hour tabs.
  const BLD_FX = {
    home: "+30 housing", alchemy: "+45 plat/hr", farm: "+80 food/hr", smithy: "cheaper training", masonry: "+improvements",
    tower: "+25 mana/hr", temple: "+pop growth", wizard_guild: "+5 mana/hr · wizards",
    ore_mine: "+60 ore/hr", gryphon_nest: "+offense",
    guard_tower: "+defense", factory: "cheaper build/rezone", shrine: "hero bonus", barracks: "military housing",
    lumberyard: "+50 lumber/hr", forest_haven: "+25 lumber · spies",
    diamond_mine: "+15 gems/hr", school: "+research points", dock: "+food · boats",
  };
  function renderOpening() {
    const pln = plan();
    pln.opening = pln.opening || {};
    const allB = Object.keys(meta().buildingLand || {});
    const groups = LAND_TYPES.map((t) => [t, allB.filter((b) => buildingLand(b) === t)]).filter(([, bs]) => bs.length);
    const cur = (b) => pln.opening[b] || 0;
    const total = () => allB.reduce((s, b) => s + cur(b), 0);
    const typeAcres = (t) => allB.filter((b) => buildingLand(b) === t).reduce((s, b) => s + cur(b), 0);
    root.innerHTML = `
      <div class="ed-head">
        <div class="ed-hour"><span>OPENING BUILD</span><span class="ed-sub">350 free starting acres · auto-zoned</span></div>
        <button class="ed-x" aria-label="close">✕</button>
      </div>
      <div class="ed-budget"><span class="ed-budget-cap">acres placed (free · instant)</span>
        <div class="bud-row">
          <span class="bud-chip" id="chipPlaced" style="--c:var(--c-land)"><i></i><b>0</b><span>/ 350 placed</span></span>
          <span class="bud-chip" id="chipBarren" style="--c:var(--text-dim)"><i></i><b>0</b><span>→ homes (auto)</span></span>
        </div>
      </div>
      <div class="ed-form">
        <div class="ed-open-note">Fill your 350 free starting acres with <b>any</b> buildings — each is auto-zoned to its land type (temples → swamp, guard towers → hill, ore mines → mountain…). This is the only free + instant build; everything after costs platinum/lumber + 12h on the hour tabs. The game requires all 350 acres built at the opening, so any acres you don't place <b>auto-fill as homes</b> — destroy + rezone them on later hours to change them.</div>
        <div class="ed-tpl">
          <div class="ed-tpl-head">
            <span class="ed-tpl-cap">opening templates</span>
            <div class="ed-tpl-save"><input id="tplName" class="ed-tpl-name" placeholder="name this opening…" autocomplete="off"><button class="ed-tpl-btn" id="tplSave" type="button">save</button></div>
          </div>
          <div class="ed-tpl-list" id="tplList"></div>
        </div>
        ${groups.map(([t, bs]) => `
          <div class="ed-open-group">
            <div class="ed-open-gh"><span>${t}</span><span class="ed-open-ga" data-acres="${t}">0 acres</span></div>
            ${bs.map((b) => `<label class="ed-open-row"><span class="ed-open-name">${b.replace(/_/g, " ")}</span><input type="number" min="0" inputmode="numeric" data-b="${b}" value="${cur(b)}"><span class="ed-open-desc">${BLD_FX[b] || ""}</span></label>`).join("")}
          </div>`).join("")}
        <div class="ed-feedback" id="opFb"></div>
        <button class="ed-add" id="edToFirst">go to hour 01 ▸</button>
      </div>`;
    const sync = () => {
      const t = total(), barren = 350 - t;
      const cp = root.querySelector("#chipPlaced"), cb = root.querySelector("#chipBarren"), fb = root.querySelector("#opFb");
      cp.querySelector("b").textContent = int(t); cp.classList.toggle("neg", t > 350);
      cb.querySelector("b").textContent = int(barren); cb.classList.toggle("neg", barren < 0);
      root.querySelectorAll("[data-acres]").forEach((e) => { e.textContent = int(typeAcres(e.dataset.acres)) + " acres"; });
      fb.innerHTML = t > 350
        ? `<span class="fb-bad">✕ ${int(t - 350)} over the 350-acre cap</span>`
        : `<span class="fb-ok">✓ ${int(barren)} unplaced ${barren === 1 ? "acre" : "acres"} auto-fill as homes</span>`;
    };
    root.querySelector(".ed-x").onclick = close;
    root.querySelector("#edToFirst").onclick = () => { open(1); deps.onNav && deps.onNav(1); };
    // opening templates: save the current placement under a name, or stamp a saved one in. Lives
    // inline here (the opening editor) — the most seamless place to reuse a starting build.
    const tplName = root.querySelector("#tplName");
    const defaultTplName = () => `${plan().race} opening ${listOpenings().filter((t) => t.race === plan().race).length + 1}`;
    function renderTplList() {
      const host = root.querySelector("#tplList"); if (!host) return;
      const tpls = listOpenings(), race = plan().race;
      host.innerHTML = tpls.length
        ? tpls.map((t) => `<span class="ed-tpl-chip ${t.race && t.race !== race ? "alt" : ""}"><button class="ed-tpl-apply" data-apply="${t.id}" title="apply ${esc(t.name)}${t.race ? " (" + esc(t.race) + ")" : ""}">${esc(t.name)} <i>${int(t.acres || openingAcres(t.opening))}ac</i></button><button class="ed-tpl-del" data-del="${t.id}" title="delete">✕</button></span>`).join("")
        : `<span class="ed-tpl-empty">no templates yet — place buildings below, then “save”</span>`;
      host.querySelectorAll("[data-apply]").forEach((b) => (b.onclick = () => {
        const t = listOpenings().find((x) => x.id === b.dataset.apply); if (!t) return;
        deps.recordUndo("opening");
        pln.opening = JSON.parse(JSON.stringify(t.opening || {}));
        deps.recompute(0); renderOpening();
      }));
      host.querySelectorAll("[data-del]").forEach((b) => (b.onclick = () => { deleteOpening(b.dataset.del); renderTplList(); }));
    }
    root.querySelector("#tplSave").onclick = () => {
      const nm = (tplName.value || "").trim() || defaultTplName();
      saveOpening(nm, plan().race, pln.opening); tplName.value = ""; renderTplList();
    };
    tplName.onkeydown = (e) => { if (e.key === "Enter") root.querySelector("#tplSave").click(); };
    renderTplList();
    root.querySelectorAll("input[data-b]").forEach((inp) => {
      inp.oninput = () => {
        const b = inp.dataset.b;
        let val = Math.max(0, parseInt(inp.value || "0", 10) || 0);
        const others = total() - cur(b);
        if (others + val > 350) { val = Math.max(0, 350 - others); inp.value = String(val); } // keep ≤ 350
        deps.recordUndo("opening");
        pln.opening[b] = val;
        deps.recompute(0);  // live: the whole instrument reflows off the new opening
        sync();             // update the editor's own chips in place (keeps input focus)
      };
    });
    sync();
  }

  function render() {
    if (hour === 0) return renderOpening();
    const w = remainingWallet();
    const nActs = acts().length;
    root.innerHTML = `
      <div class="ed-head">
        <div class="ed-hour">
          <button class="ed-step" data-step="-1" aria-label="previous hour">◀</button>
          <span>HOUR <strong>${String(hour).padStart(2, "0")}</strong></span>
          <button class="ed-step" data-step="1" aria-label="next hour">▶</button>
          <span class="ed-sub">${nActs ? nActs + " queued · " : ""}${hour < OOP_HOUR ? `${OOP_HOUR - hour}h to OOP` : (hour === OOP_HOUR ? "✦ out of protection" : `+${hour - OOP_HOUR}h post-OOP`)}</span>
        </div>
        <button class="ed-x" aria-label="close">✕</button>
      </div>
      <div id="edBudget"></div>
      <div class="ed-main">
        <nav class="ed-rail" role="tablist">${TABS.map(([k, l]) => `<button class="ed-tab ${k === tab ? "on" : ""}" data-tab="${k}" role="tab">${l}</button>`).join("")}</nav>
        <div class="ed-form" id="edForm"></div>
        <div class="ed-queue" id="edQueue"></div>
      </div>`;
    root.querySelector(".ed-x").onclick = close;
    root.querySelectorAll(".ed-step").forEach((b) => (b.onclick = () => { open(hour + (+b.dataset.step)); deps.onNav && deps.onNav(hour); })); // sync the main-window playhead
    root.querySelectorAll(".ed-tab").forEach((b) => (b.onclick = () => { tab = b.dataset.tab; render(); }));
    renderChrome();
    renderForm(w);
  }
  // The QUEUED-this-hour list (every action type for the current hour, inline-editable).
  function queueListHtml() {
    const a = acts(), nActs = a.length;
    return `<div class="ed-queue-cap">QUEUED @ HOUR ${String(hour).padStart(2, "0")}</div>
      <div class="ed-queue-list">${nActs ? a.map((x, i) => {
        const d = describeAct(x); const desc = esc(d.desc); // plan-derived — escape before innerHTML
        return `<div class="q-line ${d.kind || ""}"><span class="q-desc">${desc}</span>${d.edit ? `<input class="q-num" type="number" inputmode="numeric" data-i="${i}" data-k="${esc(d.edit.key)}" value="${esc(d.edit.val)}" aria-label="${desc} amount">${d.edit.unit ? `<span class="q-unit">${esc(d.edit.unit)}</span>` : ""}` : ""}<button class="q-x" data-i="${i}" aria-label="remove">✕</button></div>`;
      }).join("") : '<div class="ed-empty">no actions yet — pick a tab and add one</div>'}</div>`;
  }
  // Re-render the budget strip + queue in place (after an embedded-window edit) WITHOUT rebuilding the
  // form, so the window keeps its focus/selection. Re-prices off the freshly recomputed trace.
  function renderChrome() {
    const w = remainingWallet();
    const b = document.getElementById("edBudget"); if (b) b.innerHTML = budgetStrip(w);
    const q = document.getElementById("edQueue"); if (q) q.innerHTML = queueListHtml();
    root.querySelectorAll(".q-x").forEach((x) => (x.onclick = () => removeAt(+x.dataset.i)));
    root.querySelectorAll(".q-num").forEach((inp) => (inp.onchange = () => editQty(+inp.dataset.i, inp.dataset.k, inp.value)));
  }

  // A reusable add-form: builds the action from inputs, shows the max legal amount,
  // live-validates, commits. opts.qtyField = the input id holding the quantity → a
  // "max legal: N · limited by X" affordance (click N to fill it in).
  function mountForm(bodyHtml, collect, opts = {}) {
    const form = document.getElementById("edForm");
    form.innerHTML = `${bodyHtml}
      ${opts.qtyField ? `<div class="ed-maxrow">max legal <button class="ed-maxbtn" id="edMax" type="button">—</button><span class="ed-maxwhy" id="edMaxWhy"></span></div>` : ""}
      <div class="ed-feedback" id="edFb"></div>
      <button class="ed-add ${opts.variant || ""}" id="edAdd">${opts.verb || "add action"}</button>`;
    const fb = form.querySelector("#edFb"), add = form.querySelector("#edAdd");
    const refresh = () => {
      const a = collect();
      if (!a) { add.disabled = true; fb.innerHTML = ""; return; }
      if (opts.qtyField) {
        const m = maxDetailed(remainingWallet(), a);
        const mb = form.querySelector("#edMax"), mw = form.querySelector("#edMaxWhy");
        mb.textContent = int(m.n);
        mw.textContent = m.why ? `· limited by ${m.why}` : "";
        mb.onclick = () => { const fld = document.getElementById(opts.qtyField); if (fld) { fld.value = String(m.n); refresh(); } };
      }
      const reason = applyOne(remainingWallet(), a, true);
      const soft = SOFT_OVERSPEND.has(a.type);
      if (reason && !soft) {
        // engine-gated / physical limit — genuinely can't queue this (e.g. bank more than held)
        add.disabled = true;
        let hint = "";
        if (a.n != null) { const m = maxDetailed(remainingWallet(), a); if (m.n > 0) hint = ` · max ${int(m.n)}`; }
        fb.innerHTML = `<span class="fb-bad">✕ ${reason}${hint}</span>`;
      } else if (reason && soft) {
        // an overspend the engine still executes (resources/land go negative). Allow the
        // edit — a later adjustment may fund it — and flag the hour instead of blocking it.
        add.disabled = false;
        let hint = "";
        if (a.n != null) { const m = maxDetailed(remainingWallet(), a); if (m.n > 0) hint = ` · max ${int(m.n)} now`; }
        fb.innerHTML = `<span class="fb-warn">⚠ ${reason}${hint} · queues anyway — hour flagged until funded</span>`;
      } else {
        add.disabled = false;
        fb.innerHTML = `<span class="fb-ok">✓ ${opts.note ? opts.note(a) : "affordable"}</span>`;
      }
    };
    form.querySelectorAll("input,select").forEach((i) => { i.oninput = refresh; i.onchange = refresh; });
    add.onclick = () => { const a = collect(); if (!a) return; if (applyOne(remainingWallet(), a, true) && !SOFT_OVERSPEND.has(a.type)) return; commit(a); };
    refresh();
    return refresh;
  }

  const numField = (id, label, val, color) => `<label class="ed-field"><span>${label}</span><input id="${id}" type="number" inputmode="numeric" value="${val}" ${color ? `style="--c:var(${color})"` : ""}></label>`;
  const selField = (id, label, opts) => `<label class="ed-field"><span>${label}</span><select id="${id}">${opts}</select></label>`;
  const v = (id) => { const e = document.getElementById(id); return e ? e.value : null; };
  const vn = (id) => Math.max(0, parseInt(v(id) || "0", 10) || 0);

  // Segmented direction switch (Build|Destroy, Train|Release). The reverse side carries a colour
  // variant (danger/demob) so the flipped mode is unmistakable; wire the buttons with wireDirSwitch.
  const dirSwitch = (current, opts) =>
    `<div class="ed-switch" role="tablist">${opts.map(([k, label, vr]) =>
      `<button type="button" class="ed-sw ${k === current ? `on ${vr || ""}` : ""}" data-dir="${k}" role="tab">${label}</button>`).join("")}</div>`;
  const wireDirSwitch = (cb) => document.querySelectorAll("#edForm .ed-switch .ed-sw").forEach((b) => (b.onclick = () => cb(b.dataset.dir)));

  function renderForm(w) {
    const c = entryRow().costs;
    const free = entryRow().freeLandByType;
    // Reset the reverse-mode tint each render; the destroy/release branches re-apply it.
    const edFormEl = document.getElementById("edForm");
    edFormEl.classList.remove("mode-danger", "mode-demob");
    if (tab === "build") {
      // Build|Destroy directional switch — both directions share the building axis; destroy is
      // the inverse of construct (raze → barren), one flip away instead of buried in Manage.
      const sw = dirSwitch(buildDir, [["build", "Build", ""], ["destroy", "Destroy", "danger"]]);
      if (buildDir === "build") {
        const buildings = Object.keys(meta().buildingLand || {});
        if (!buildings.includes(buildSel)) buildSel = buildings[0] || "home";
        const groups = LAND_TYPES.map((t) => [t, buildings.filter((b) => buildingLand(b) === t)]).filter(([, bs]) => bs.length);
        const picker = `<div class="bld-picker">${groups.map(([t, bs]) => `
          <div class="bld-group"><span class="bld-gh">${t}</span><div class="bld-chips">${bs.map((b) => `<button type="button" class="bld-chip ${b === buildSel ? "on" : ""}" data-b="${b}">${b.replace(/_/g, " ")}</button>`).join("")}</div></div>`).join("")}</div>`;
        const host = document.getElementById("edForm");
        host.innerHTML = `${sw}${picker}<div class="ed-note" id="buildNote"></div><div class="ed-hg" id="edHg"></div>`;
        const mountSel = () => {
          const n = document.getElementById("buildNote");
          if (n) n.textContent = `${int(c.constructPlat)} plat + ${int(c.constructLumber)} lumber each · sits on ${buildingLand(buildSel)} land · 12h to build · type a count down the hours`;
          mountHourGrid(document.getElementById("edHg"), windowOpts({
            label: buildSel.replace(/_/g, " "), color: "--c-land", stateCols: buildStateCols,
            read: (h) => laneRead(h, (a) => a.type === "construct" && a.building === buildSel),
            write: (h, val) => laneWrite(h, (a) => a.type === "construct" && a.building === buildSel, () => ({ type: "construct", building: buildSel, n: val }), val),
          }));
        };
        document.querySelectorAll(".bld-chip").forEach((ch) => (ch.onclick = () => {
          buildSel = ch.dataset.b;
          document.querySelectorAll(".bld-chip").forEach((x) => x.classList.toggle("on", x === ch));
          mountSel();
        }));
        mountSel();
      } else {
        // DESTROY — raze owned buildings to barren land (instant, free, undo-able).
        edFormEl.classList.add("mode-danger");
        const owned = Object.entries(entryRow().buildings).filter(([, n]) => n > 0);
        mountForm(
          `${sw}<div class="ed-grid2">${selField("p1", "building", owned.map(([b, n]) => `<option value="${b}">${b.replace(/_/g, " ")} (${int(n)})</option>`).join("") || `<option value="">none built</option>`)}${numField("p2", "count", 1, "--red")}</div>
           <div class="ed-note">razes buildings → barren land · instant, free · undo-able</div>`,
          () => ({ type: "destroy", building: v("p1"), n: vn("p2") }),
          { verb: "raze buildings", qtyField: "p2", variant: "danger", note: (a) => `raze ${int(a.n)} ${(a.building || "").replace(/_/g, " ")} → +${int(a.n)} barren` }
        );
      }
      wireDirSwitch((d) => { buildDir = d; renderForm(w); });
    } else if (tab === "rezone") {
      renderRezoneTable(c);
    } else if (tab === "explore") {
      // Every land type is legally explorable in the round-50 game: LandHelper::getLandTypes()
      // returns all 7 and ExploreActionService accepts land_<type> for any of them (cost is
      // land-total-based, not type-based). Don't pre-restrict the option set — offer all 7.
      const host = document.getElementById("edForm");
      const chips = LAND_TYPES.map((t) => `<button type="button" class="bld-chip ${t === exploreSel ? "on" : ""}" data-t="${t}">${t}</button>`).join("");
      host.innerHTML = `<div class="bld-group"><span class="bld-gh">explore — pick a land type</span><div class="bld-chips ed-terrain">${chips}</div></div><div class="ed-note" id="expNote"></div><div class="ed-hg" id="edHg"></div>`;
      const mountSel = () => {
        const n = document.getElementById("expNote");
        if (n) n.textContent = `${int(c.explorePlat)} plat + ${int(c.exploreDraftee)} draftees per acre · arrives in 12h as ${exploreSel} (skips a rezone) · costs morale · type acres down the hours`;
        mountHourGrid(document.getElementById("edHg"), windowOpts({
          label: exploreSel, color: "--c-draftee", stateCols: spendStateCols,
          read: (h) => laneRead(h, (a) => a.type === "explore" && (a.land || "plain") === exploreSel),
          write: (h, val) => laneWrite(h, (a) => a.type === "explore" && (a.land || "plain") === exploreSel, () => ({ type: "explore", land: exploreSel, n: val }), val),
        }));
      };
      host.querySelectorAll(".bld-chip").forEach((ch) => (ch.onclick = () => { exploreSel = ch.dataset.t; host.querySelectorAll(".bld-chip").forEach((x) => x.classList.toggle("on", x === ch)); mountSel(); }));
      mountSel();
    } else if (tab === "train") {
      // Train|Release directional switch — release is the inverse of train (units → draftees →
      // peasants), one flip away instead of buried in Manage.
      const sw = dirSwitch(trainDir, [["train", "Train", ""], ["release", "Release", "demob"]]);
      if (trainDir === "train") {
        // Exclude not_trainable units (e.g. Planewalker's summoned slots) — the game can't train them.
        const units = (meta().units || []).filter((u) => u.trainable !== false).map((u) => ({ slot: u.slot, name: u.name }));
        // Spies & wizards (500 platinum + 1 draftee) — trainable in protection.
        if (c.train && c.train.spies) units.push({ slot: "spies", name: "Spy" });
        if (c.train && c.train.wizards) units.push({ slot: "wizards", name: "Wizard" });
        if (!units.some((u) => String(u.slot) === String(trainSel))) trainSel = units[0] ? units[0].slot : 1;
        const chips = units.map((u) => `<button type="button" class="bld-chip ${String(u.slot) === String(trainSel) ? "on" : ""}" data-s="${u.slot}">${u.name}</button>`).join("");
        const host = document.getElementById("edForm");
        host.innerHTML = `${sw}<div class="bld-group"><span class="bld-gh">train — pick a unit</span><div class="bld-chips ed-terrain">${chips}</div></div><div class="ed-note" id="trainNote"></div><div class="ed-hg" id="edHg"></div>`;
        const mountSel = () => {
          const t = c.train[trainSel] || {};
          const n = document.getElementById("trainNote");
          if (n) n.textContent = `${Object.entries(t).map(([res, per]) => `${int(per)} ${res}`).join(" + ") || "—"} + 1 draftee each · spies/wizards & draftees are NOT counted toward the DP target · type counts down the hours`;
          mountHourGrid(document.getElementById("edHg"), windowOpts({
            label: (units.find((u) => String(u.slot) === String(trainSel)) || {}).name || ("slot " + trainSel), color: "--c-dp", stateCols: spendStateCols,
            read: (h) => laneRead(h, (a) => a.type === "train" && String(a.slot) === String(trainSel)),
            write: (h, val) => laneWrite(h, (a) => a.type === "train" && String(a.slot) === String(trainSel), () => ({ type: "train", slot: trainSel, n: val }), val),
          }));
        };
        host.querySelectorAll(".bld-chip").forEach((ch) => (ch.onclick = () => { const x = ch.dataset.s; trainSel = /^\d+$/.test(x) ? +x : x; host.querySelectorAll(".bld-chip").forEach((y) => y.classList.toggle("on", y === ch)); mountSel(); }));
        mountSel();
      } else {
        // RELEASE — disband units → draftees, draftees → peasants (instant, free, undo-able).
        edFormEl.classList.add("mode-demob");
        const r = entryRow(), m = r.military || {};
        const units = [["draftees", `draftees (${int(r.draftees)})`], ["1", `slot 1 (${int(m.u1 || 0)})`], ["2", `slot 2 (${int(m.u2 || 0)})`], ["3", `slot 3 (${int(m.u3 || 0)})`], ["4", `slot 4 (${int(m.u4 || 0)})`]];
        mountForm(
          `${sw}<div class="ed-grid2">${selField("p1", "release", units.map(([k, l]) => `<option value="${k}">${l}</option>`).join(""))}${numField("p2", "count", 10, "--c-draftee")}</div>
           <div class="ed-note">units → draftees, draftees → peasants · instant, free · undo-able</div>`,
          () => { const u = v("p1"); return u === "draftees" ? { type: "release", unit: "draftees", n: vn("p2") } : { type: "release", slot: +u, n: vn("p2") }; },
          { verb: "release troops", qtyField: "p2", variant: "demob", note: (a) => `release ${int(a.n)} ${a.unit === "draftees" ? "draftees → peasants" : "slot " + a.slot + " → draftees"}` }
        );
      }
      wireDirSwitch((d) => { trainDir = d; renderForm(w); });
    } else if (tab === "magic") {
      const active = Object.fromEntries((entryRow().spells || []).map((s) => [s.key, s.dur]));
      // Per-race castable self-spells (common + racial), data-driven from meta. The c.spell gate
      // is protection-aware: `invalid_protection` racial spells (e.g. Undead's Death and Decay)
      // only have a cost entry — and so only appear here — at post-OOP hours (hour 49+). The ✦
      // marks those out-of-protection-only spells.
      const spells = (meta().spells || []).filter((sp) => (c.spell || {})[sp.key] != null);
      const opt = spells.map((sp) => `<option value="${sp.key}">${sp.name}${sp.invalidProtection ? " ✦" : ""}${sp.desc ? " — " + sp.desc : ""}</option>`).join("")
        || `<option value="">no self-spells${hour < OOP_HOUR ? " castable in protection" : ""}</option>`;
      mountForm(
        `<div class="ed-grid1">${selField("p1", "self-spell", opt)}</div>
         <div class="ed-note" id="spellNote"></div>`,
        () => ({ type: "spell", spell: v("p1") }),
        { verb: "cast spell", note: (a) => { const cost = (c.spell || {})[a.spell] || 0, have = remainingWallet().mana; return cost <= have ? `casts — ${int(cost)} of ${int(have)} mana, lasts 12h` : `⚠ only ${int(have)} mana (needs ${int(cost)}) — will NOT cast this hour`; } }
      );
      const invalidProt = new Set(spells.filter((sp) => sp.invalidProtection).map((sp) => sp.key));
      const upd = () => { const k = v("p1"); const n = document.getElementById("spellNote"); const act = active[k]; const cost = (c.spell || {})[k] || 0, have = remainingWallet().mana; if (n) n.innerHTML = `${int(cost)} mana · ${cost <= have ? `<span style="color:var(--green)">✓ ${int(have)} available</span>` : `<span style="color:var(--amber)">⚠ only ${int(have)} — won't cast yet</span>`}${act ? ` · active ${act}h (re-cast refreshes)` : ""}${invalidProt.has(k) ? ` · <span style="color:var(--amber)">✦ out-of-protection spell</span>` : ""}`; };
      document.getElementById("p1").addEventListener("change", upd); upd();
    } else if (tab === "bank") {
      const so = BANKABLE.filter(showRes).map((s) => `<option value="resource_${s}">${s}</option>`).join("");
      const toRes = ["ore", "lumber", "platinum", "food"].filter(showRes);
      const toDefault = toRes.includes("ore") ? "ore" : (toRes[0] || "lumber");
      const to = toRes.map((s) => `<option value="resource_${s}" ${s === toDefault ? "selected" : ""}>${s}</option>`).join("");
      mountForm(
        `<div class="ed-grid3">${selField("p1", "from", so)}${selField("p2", "to", to)}${numField("p3", "amount", 5000)}</div>
         <div class="ed-note">exchange at the realm rate (≈2:1 most pairs) · instant</div>`,
        () => ({ type: "bank", source: v("p1"), target: v("p2"), amount: vn("p3") }),
        { verb: "bank resources", qtyField: "p3", note: (a) => `spend ${int(a.amount)} ${a.source.replace("resource_", "")}` }
      );
    } else if (tab === "daily") {
      const r = entryRow();
      document.getElementById("edForm").innerHTML = `
        <div class="ed-daily">
          <button class="ed-daily-btn" data-claim="claim_platinum" ${r.dailyPlatinum ? "disabled" : ""}>
            <b>Claim platinum bonus</b><span>${r.dailyPlatinum ? "already claimed this day" : `+${int(r.peasants * 4)} platinum (peasants × 4) +350 tech`}</span></button>
          <button class="ed-daily-btn" data-claim="claim_land" ${r.dailyLand ? "disabled" : ""}>
            <b>Claim land bonus</b><span>${r.dailyLand ? "already claimed this day" : "+20 plain land (instant)"}</span></button>
        </div>
        <div class="ed-note">daily bonuses reset at hour 1 and hour 25</div>`;
      document.querySelectorAll(".ed-daily-btn").forEach((b) => (b.onclick = () => { if (!b.disabled) commit({ type: b.dataset.claim }); }));
    } else if (tab === "manage") {
      const sub = `<div class="ed-grid1">${selField("mk", "action", [["draft_rate", "Set draft rate"], ["improve", "Invest (improvements)"], ["research", "Research tech"]].map(([k, l]) => `<option value="${k}" ${k === manageKind ? "selected" : ""}>${l}</option>`).join(""))}</div>`;
      const host = el(`<div>${sub}<div id="manageBody"></div></div>`);
      document.getElementById("edForm").innerHTML = "";
      document.getElementById("edForm").appendChild(host);
      document.getElementById("mk").onchange = (e) => { manageKind = e.target.value; renderForm(w); };
      renderManage(c);
    } else if (tab === "techs") {
      // The tech-tree replica, scoped to THIS hour: research lands as a per-hour action committed
      // through the same path as every other editor action (commit → recordUndo → re-sim → render).
      const r = entryRow();
      renderTechTree(document.getElementById("edForm"), {
        techs: meta().techs || [],
        researched: new Set(r.techs || []),
        points: r.tech || 0,
        cost: c.techCost || 0,
        targetHour: hour,
        onResearch: (key) => commit({ type: "research", tech: key }),
        width: 680,
      });
    }
  }

  function tabFreeLandHint() {
    const free = remainingWallet().free; // barren land left AFTER this hour's queued rezones/builds
    const host = document.getElementById("edForm");
    const hint = el(`<div class="ed-landbar">${LAND_TYPES.map((l) => `<span class="lb ${free[l] < 0 ? "neg" : ""}"><b>${int(free[l])}</b> ${l}</span>`).join("")}</div>`);
    host.insertBefore(hint, host.querySelector(".ed-feedback"));
  }

  // Game-style rezone (mirrors the in-game Re-zone Land page): one row per land type with Owned %,
  // Barren, and remove/add inputs — all 7 types visible at once, no dropdowns. The remove/add
  // distribution (which must balance) is decomposed into the engine's {from,to,n} rezone actions for
  // this hour, replacing any rezones already queued here.
  function renderRezoneTable(c) {
    const host = document.getElementById("edForm");
    const r = entryRow();
    const free = r.freeLandByType || {};
    const landBy = r.landBy || {};
    const total = r.land || 1;
    const removes = {}, adds = {};
    for (const a of acts()) if (a.type === "rezone") { removes[a.from] = (removes[a.from] || 0) + (a.n | 0); adds[a.to] = (adds[a.to] || 0) + (a.n | 0); }
    const rows = LAND_TYPES.map((t) => {
      const pct = total ? Math.round((landBy[t] || 0) / total * 100) : 0;
      const home = t === homeLand() ? `<span class="rz-home">home</span>` : "";
      return `<tr>
        <td class="rz-type">${t}${home}</td>
        <td class="rz-num">${int(landBy[t] || 0)} <span class="rz-pct">${pct}%</span></td>
        <td class="rz-num">${int(free[t] || 0)}</td>
        <td class="rz-cell"><input class="rz-remove" type="text" inputmode="numeric" autocomplete="off" data-t="${t}" value="${removes[t] || ""}" placeholder="0" aria-label="remove barren ${t}"></td>
        <td class="rz-arrow">→</td>
        <td class="rz-cell"><input class="rz-add" type="text" inputmode="numeric" autocomplete="off" data-t="${t}" value="${adds[t] || ""}" placeholder="0" aria-label="add ${t}"></td>
      </tr>`;
    }).join("");
    host.innerHTML = `
      <table class="rz-table">
        <thead><tr><th>land type</th><th>owned</th><th>barren</th><th>remove</th><th></th><th>add to</th></tr></thead>
        <tbody>${rows}</tbody>
      </table>
      <div class="rz-balance" id="rzBalance"></div>
      <div class="ed-note">${int(c.rezonePlat)} plat per barren acre · instant · the amount you remove must equal the amount you add</div>`;
    const gather = (cls) => { let s = 0; const m = {}; host.querySelectorAll(cls).forEach((i) => { const x = Math.max(0, Math.floor(+i.value || 0)); if (x > 0) m[i.dataset.t] = x; s += x; }); return { s, m }; };
    const balance = () => {
      const rem = gather(".rz-remove"), add = gather(".rz-add");
      const bal = document.getElementById("rzBalance");
      if (bal) {
        const cost = int(Math.max(rem.s, add.s) * c.rezonePlat);
        bal.innerHTML = rem.s === add.s
          ? (add.s > 0 ? `<span class="fb-ok">✓ ${int(add.s)} acres rezoned · ${cost} plat</span>` : `<span class="rz-dim">enter acres — barren of one type converts into another</span>`)
          : `<span class="fb-warn">⚠ removing ${int(rem.s)} ≠ adding ${int(add.s)} · balance them (${int(Math.abs(rem.s - add.s))} off)</span>`;
      }
      return { rem, add };
    };
    const apply = () => {
      const { rem, add } = balance();
      const rl = Object.entries(rem.m).map(([t, q]) => ({ t, q })), al = Object.entries(add.m).map(([t, q]) => ({ t, q }));
      const out = []; let i = 0, j = 0;
      while (i < rl.length && j < al.length) { // greedy-pair removes→adds into from→to conversions
        const n = Math.min(rl[i].q, al[j].q);
        if (n > 0 && rl[i].t !== al[j].t) out.push({ type: "rezone", from: rl[i].t, to: al[j].t, n });
        rl[i].q -= n; al[j].q -= n; if (rl[i].q === 0) i++; if (al[j].q === 0) j++;
      }
      deps.recordUndo("edit");
      plan().hours[hour - 1] = acts().filter((a) => a.type !== "rezone").concat(out);
      deps.recompute(hour).then(renderChrome);
    };
    host.querySelectorAll(".rz-remove,.rz-add").forEach((i) => {
      i.oninput = () => { const cl = i.value.replace(/[^0-9]/g, ""); if (cl !== i.value) i.value = cl; balance(); };
      i.onchange = apply;
    });
    balance();
  }

  function renderManage(c) {
    const body = document.getElementById("manageBody");
    const r = entryRow();
    if (manageKind === "draft_rate") {
      body.innerHTML = `<div class="ed-grid1">${numField("p1", "draft rate %", r.draftRate || 90)}</div><div class="ed-note">draftees grow 1%/hr of peasants while military %% &lt; draft rate</div><div class="ed-feedback" id="edFb"></div><button class="ed-add" id="edAdd">set draft rate</button>`;
      wireManageAdd(() => ({ type: "draft_rate", rate: Math.max(0, Math.min(100, vn("p1"))) }));
    } else if (manageKind === "improve") {
      const so = BANKABLE.filter(showRes).map((s) => `<option value="${s}">${s}</option>`).join("");
      const io = IMPROVEMENTS.map((s) => `<option>${s}</option>`).join("");
      body.innerHTML = `<div class="ed-grid3">${selField("p1", "spend", so)}${selField("p2", "into", io)}${numField("p3", "amount", 5000)}</div><div class="ed-note">invests resources into a realm improvement</div>${MAX_ROW}<div class="ed-feedback" id="edFb"></div><button class="ed-add" id="edAdd">invest</button>`;
      wireManageAdd(() => ({ type: "improve", resource: v("p1"), data: oneObj(v("p2"), vn("p3")), amount: vn("p3") }), "p3");
    } else if (manageKind === "research") {
      const techs = (meta().techs || []).filter((t) => !(r.techs || []).includes(t.key));
      body.innerHTML = techs.length
        ? `<div class="ed-grid1">${selField("p1", "tech", techs.map((t) => `<option value="${t.key}">${t.name || t.key}</option>`).join(""))}</div><div class="ed-note">costs ${int(c.techCost)} tech points · prerequisites enforced by the engine</div><div class="ed-feedback" id="edFb"></div><button class="ed-add" id="edAdd">research</button>`
        : `<div class="ed-empty">no techs available here${(meta().techs || []).length ? " (all researched)" : " — research is modeled by the engine backend, not the preview"}</div>`;
      if (techs.length) wireManageAdd(() => ({ type: "research", tech: v("p1") }));
    }
  }
  function oneObj(k, val) { const o = {}; o[k] = val; return o; }
  const MAX_ROW = `<div class="ed-maxrow">max legal <button class="ed-maxbtn" id="edMax" type="button">—</button><span class="ed-maxwhy" id="edMaxWhy"></span></div>`;
  function wireManageAdd(collect, qtyField, noteFn) {
    const fb = document.getElementById("edFb"), add = document.getElementById("edAdd");
    if (!add) return;
    const refresh = () => {
      const a = collect(); if (!a) { add.disabled = true; return; }
      if (qtyField) {
        const m = maxDetailed(remainingWallet(), a);
        const mb = document.getElementById("edMax"), mw = document.getElementById("edMaxWhy");
        if (mb) { mb.textContent = int(m.n); mw.textContent = m.why ? `· limited by ${m.why}` : ""; mb.onclick = () => { const f = document.getElementById(qtyField); if (f) { f.value = String(m.n); refresh(); } }; }
      }
      const reason = applyOne(remainingWallet(), a, true);
      add.disabled = !!reason;
      let hint = ""; if (qtyField && reason) { const m = maxDetailed(remainingWallet(), a); if (m.n > 0) hint = ` · max ${int(m.n)}`; }
      fb.innerHTML = reason ? `<span class="fb-bad">✕ ${reason}${hint}</span>` : `<span class="fb-ok">✓ ${noteFn ? noteFn(a) : "ok"}</span>`;
    };
    document.querySelectorAll("#manageBody input,#manageBody select").forEach((i) => { i.oninput = refresh; i.onchange = refresh; });
    add.onclick = () => { const a = collect(); if (a && !applyOne(remainingWallet(), a, true)) commit(a); };
    refresh();
  }

  return { open, close, rerender, isOpen: () => !root.hidden, get hour() { return hour; } };
}

export function actLabel(a, meta) {
  if (a.type === "construct") return `build <b>${int(a.n)}</b> ${a.building.replace(/_/g, " ")}`;
  if (a.type === "rezone") return `rezone <b>${int(a.n)}</b> ${a.from}→${a.to}`;
  if (a.type === "explore") return `explore <b>${int(a.n)}</b>${a.land ? " " + a.land : ""}`;
  if (a.type === "train") return `train <b>${int(a.n)}</b> ${typeof a.slot === "string" ? a.slot : "slot " + a.slot}`;
  if (a.type === "spell") return `cast <b>${(a.spell || "").replace(/_/g, " ")}</b>`;
  if (a.type === "bank") return `bank <b>${int(a.amount)}</b> ${(a.source || "").replace("resource_", "")}→${(a.target || "").replace("resource_", "")}`;
  if (a.type === "destroy") return `destroy <b>${int(a.n)}</b> ${a.building.replace(/_/g, " ")}`;
  if (a.type === "release") return `release <b>${int(a.n)}</b> ${a.unit === "draftees" ? "draftees" : "slot " + a.slot}`;
  if (a.type === "draft_rate") return `draft rate → <b>${a.rate}%</b>`;
  if (a.type === "improve") return `invest <b>${int(a.amount)}</b> ${a.resource} → ${Object.keys(a.data || {})[0] || ""}`;
  if (a.type === "research") {
    const t = meta && (meta.techs || []).find((x) => x.key === a.tech);
    return `research <b>${t ? t.name : (a.tech || "").replace(/_/g, " ")}</b>`;
  }
  if (a.type === "claim_platinum") return "claim platinum bonus";
  if (a.type === "claim_land") return "claim land bonus";
  return a.type;
}

// Per-hour platinum SPEND breakdown, reusing the row's own cost table. Row H = A_H, so out[H]
// is hour H's spends priced at this row's (post-instant-action) costs. Returns, per hour h:
//   sinks: platinum consumed by each true sink (explore/construct/rezone/train/improve)
//   claim: daily-platinum bonus inflow (peasants×4)
//   bankIn/bankOut: platinum gained from / spent on bank exchanges
// Production is NOT recovered here anymore: rows are post-action, so hour h's production is
// simply r.platPerHr (the engine applies production at the post-action state, landing in row
// h+1) — app.js reads it directly. This function feeds only the spend/claim/bank breakdown for
// the platinum-ledger summary + the cumulative-spent curve.
export function platinumFlow(trace) {
  const rows = trace.rows || [];
  const zero = () => ({ sinks: { explore: 0, construct: 0, rezone: 0, train: 0, improve: 0 }, claim: 0, bankIn: 0, bankOut: 0 });
  const out = [];
  for (let h = 0; h < rows.length; h++) {
    const f = zero();
    const acts = (rows[h] && rows[h].actions) || []; // hour h's actions ride on row h
    const c = rows[h] && rows[h].costs;
    // daily-platinum claim pays peasants×4 at the ENTERING peasant count (enter.peasants),
    // which the engine stamps on the row; fall back to the displayed (post-action) peasants.
    const peas = (rows[h] && ((rows[h].enter && rows[h].enter.peasants) ?? rows[h].peasants)) || 0;
    if (c) for (const a of acts) {
      const n = a.n | 0;
      switch (a.type) {
        case "explore": f.sinks.explore += n * c.explorePlat; break;
        case "construct": f.sinks.construct += n * c.constructPlat; break;
        case "rezone": f.sinks.rezone += n * c.rezonePlat; break;
        case "train": { const t = c.train[a.slot] || {}; f.sinks.train += n * (t.platinum || 0); break; }
        case "improve": if (a.resource === "platinum") f.sinks.improve += a.amount | 0; break;
        case "bank": {
          const src = (a.source || "").replace("resource_", ""), tgt = (a.target || "").replace("resource_", ""), amt = a.amount | 0;
          if (src === "platinum") f.bankOut += amt;
          if (tgt === "platinum") f.bankIn += Math.floor(amt * (BANK_SELL[src] || 0) * (BANK_BUY[tgt] || 0));
          break;
        }
        case "claim_platinum": f.claim += peas * 4; break;
      }
    }
    out.push(f);
  }
  return out;
}

// Scan the trace for hours that OVERSPEND. Each row is the POST-instant-action state A_H, and
// the engine never clamps the soft actions under protection — it lets the spend ride into the
// negative — so an overspent hour is simply one whose A_H drives a spendable resource or a
// barren-land type negative. We read that straight off the engine row (the single source of
// truth) instead of re-deriving it in JS: A_H = (entering wallet) − (this hour's spends), so
// a negative means you spent more this tick than you had ENTERING it (this hour's production
// only lands next tick, so it can't fund this hour's actions). The row's own red figures show
// the detail; this just collects the hour + the first offending resource for the top-bar flag.
// (Extra args are ignored — kept so older call sites don't break.)
export function scanFeasibility(trace) {
  const bad = [];
  const rows = (trace && trace.rows) || [];
  for (let h = 1; h < rows.length; h++) {
    const r = rows[h];
    if (!r) continue;
    const reason = rowOverspend(r);
    if (reason) bad.push({ hour: r.hour, reason });
  }
  return bad;
}
// First spendable resource / barren land type the row drives negative, as a short label.
function rowOverspend(r) {
  for (const [name, v] of [["platinum", r.platinum], ["lumber", r.lumber], ["ore", r.ore], ["draftees", r.draftees]]) {
    if ((v ?? 0) < 0) return `${name} −${int(-v)}`;
  }
  const free = r.freeLandByType || {};
  for (const lt of LAND_TYPES) if ((free[lt] ?? 0) < 0) return `${lt} land −${int(-free[lt])}`;
  return null;
}
