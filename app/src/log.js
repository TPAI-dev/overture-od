// log.js — Export the current build as an OpenDominion *protection import log* (.txt).
//
// The real game (round-50) can IMPORT a protection log and auto-execute the actions:
//   ProtectionController::postImportLog → LogParserService::parseLog
//                                       → AutomationService::processLog
// So this generator is written to match that parser's grammar EXACTLY — the same
// grammar the game's own exporter (LogParserService::writeLog feeding
// dominion_attr_sentence_from_array) emits — so the output round-trips through the
// importer and the game replays the build for you.
//
// What the parser actually consumes (every other token on a line is cosmetic and
// ignored — see the regexes in LogParserService):
//   • header        /Protection Hour: (\d+)/                  → the hour number only
//   • hour 0        must be exactly ONE "Construction of … started." (the free
//                   opening build). processStartingBuildings then requires the placed
//                   building count to equal the starting land (350).
//   • construction  /Construction of ([\w\s,-]*) started/      → "N <Building>" (plural)
//   • exploration   /Exploration for ([\w\s,-]*) begun…/        → "N <Land>"  (plural)
//   • rezone        /changes in land are as following: …/        → net "±N <Land>"
//   • training      /Training of ([\w\s,]*) begun…/              → "N <Unit>"  (singular)
//   • destruction   /Destruction of ([\w\s,-]*) is complete/     → "N <Building>"
//   • bank          /([\w\s,]*) have been traded for (\d+) (\w+)/ → "N <res> … N <res>"
//   • magic         /successfully cast (.*) at a cost of (\d+)/   → spell display name
//   • daily         /You have been awarded with (\d+) (\w+)/      → platinum vs. land
//   • release       /You successfully released ([\w\s,]*)/        → "N <Unit|Draftees>"
//   • draftrate     /Draftrate changed to (\d+)/
//   • invest        /You invested (\d+) (\w+) into (\w+)/
// There is NO research/tech action in the parser, so research actions are omitted —
// emitting any line the parser can't classify makes it reject the whole import.
//
// Display names follow the game's dominion_attr_sentence_from_array(…, simLog:true):
// buildings & land are FORCE-PLURAL + Title Case; trained units are FORCE-SINGULAR +
// Title Case; resources stay lowercase. (The round-45 sample log shows singular
// "Forest"/"Mountain"; the LIVE round-50 helper pluralizes — "Forests"/"Mountains" —
// and the importer's rtrim('s') accepts both, so we follow the live game.)
//
// Costs shown are bit-exact from the engine's per-hour cost table (desktop app); the
// fields the parser ignores (bank "received", daily amounts, the training cost line)
// are filled with the game's true formulas anyway so the file reads like a real log.

// building key → display (plural, Title Case) — matches Str::plural∘Str::singular.
const BUILDING_DISPLAY = {
  home: "Homes", alchemy: "Alchemies", farm: "Farms", smithy: "Smithies",
  masonry: "Masonries", tower: "Towers", temple: "Temples", guard_tower: "Guard Towers",
  ore_mine: "Ore Mines", lumberyard: "Lumberyards", wizard_guild: "Wizard Guilds",
  gryphon_nest: "Gryphon Nests", diamond_mine: "Diamond Mines", school: "Schools",
  factory: "Factories", shrine: "Shrines", barracks: "Barracks", dock: "Docks",
  forest_haven: "Forest Havens",
};
// land key → display (plural, Title Case; water is special-cased in the game's map).
const LAND_DISPLAY = {
  plain: "Plains", swamp: "Swamps", hill: "Hills", mountain: "Mountains",
  forest: "Forests", cavern: "Caverns", water: "Water",
};
const LAND_ORDER = ["plain", "swamp", "hill", "mountain", "forest", "cavern", "water"];
// bank exchange table (BankingCalculator::getResources; exchange bonus = 1 in
// protection, no techs/wonders): received = floor(amount · sell[src] · buy[tgt]).
const SELL = { platinum: 0.5, lumber: 0.5, ore: 0.5, gems: 2.0, food: 0.0 };
const BUY = { platinum: 1.0, lumber: 1.0, ore: 1.0, gems: 0.0, food: 0.5 };
// action types whose consecutive runs collapse into one line (a single game submission).
const MERGE = new Set(["construct", "explore", "rezone", "train", "destroy", "release"]);

const ig = (n) => String(Math.round(n || 0));
const titleCase = (s) => String(s).split(/[\s_]+/).filter(Boolean).map((w) => w[0].toUpperCase() + w.slice(1)).join(" ");
const bldName = (k) => BUILDING_DISPLAY[k] || titleCase(k);
const landName = (k) => LAND_DISPLAY[k] || titleCase(k);
const spellName = (k) => titleCase(k); // == format_string(action_key) = ucwords(key)

// "12:00:00 AM 6/7/2025" — the game clock format (H:MM:SS AM/PM M/D/YYYY).
function fmtClock(d, utc) {
  const g = (m) => (utc ? d[`getUTC${m}`]() : d[`get${m}`]());
  let h = g("Hours");
  const ampm = h < 12 ? "AM" : "PM";
  h = h % 12 || 12;
  const pad = (n) => String(n).padStart(2, "0");
  return `${h}:${pad(g("Minutes"))}:${pad(g("Seconds"))} ${ampm} ${g("Month") + 1}/${g("Date")}/${g("FullYear")}`;
}

// Build the importable log text + any user-facing warnings.
export function buildLog(plan, trace, meta) {
  const rows = (trace && trace.rows) || [];
  const units = (meta && meta.units) || [];
  const home = (meta && meta.homeLand) || "plain";
  const unitName = (slot) => {
    if (slot === "spies") return "Spies";
    if (slot === "wizards") return "Wizards";
    const u = units.find((u) => u.slot === +slot);
    return u ? u.name : `Unit ${slot}`;
  };

  // One UTC instant per hour; Domtime rendered in UTC (hour 1 = midnight), Local in the
  // browser's timezone — reproducing the sample's two-clock header. Purely cosmetic (the
  // parser reads only the hour number), so any consistent base works.
  const now = new Date();
  const baseUTC = Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate(), 0, 0, 0);
  const header = (h) => {
    const t = new Date(baseUTC + (h - 1) * 3600000);
    return `====== Protection Hour: ${h}  ( Local Time: ${fmtClock(t, false)} )  ( Domtime: ${fmtClock(t, true)} ) ======`;
  };

  const out = [];
  const warnings = [];
  const emit = (h, lines) => { if (lines.length) { out.push(header(h)); for (const l of lines) out.push(l); out.push(""); } };

  // ── hour 0: the free + instant opening build ───────────────────────────────
  const opening = plan.opening || {};
  const ob = Object.entries(opening).filter(([, n]) => (n | 0) > 0);
  if (ob.length) {
    const sum = ob.reduce((s, [, n]) => s + (n | 0), 0);
    const list = ob.map(([k, n]) => `${ig(n)} ${bldName(k)}`).join(", ");
    emit(0, [`Construction of ${list} started.`]);
    if (sum !== 350) warnings.push(`Opening build places ${sum}/350 acres — the game builds ALL 350 starting acres at hour 0 and rejects any other count, so fill the opening to exactly 350 or hour 0 won't import.`);
  } else {
    warnings.push("No opening build — add a 350-acre starting build (the 00 OPENING row) for a complete importable log.");
  }

  // ── all hours ───────────────────────────────────────────────────────────────
  // Export EVERY hour in the game's format. The in-game import applies the protection hours
  // (1–48) plus hour 49 (the OOP cast + your first post-protection actions) and harmlessly
  // ignores hours 50+, so a full log stays import-valid AND is convenient to read. Hour 49
  // folds the OOP cast (oopActions, e.g. Ares) in front of that hour's own actions.
  const OOP_HOUR = 49;
  const N = (plan.hours && plan.hours.length) || 48;
  for (let h = 1; h <= N; h++) {
    let acts = (plan.hours && plan.hours[h - 1]) || [];
    if (h === OOP_HOUR) acts = [...(Array.isArray(plan.oopActions) ? plan.oopActions : []), ...acts];
    if (!acts.length) continue;
    // rows[h] is hour h's post-action state (A_H) — its costs price this hour and its `enter`
    // field carries the entering wallet renderHour re-gates spells/claims from.
    const entry = rows[h];
    if (!entry || !entry.costs) continue;
    emit(h, renderHour(acts, entry, { unitName, home }));
  }
  if (N > OOP_HOUR) {
    warnings.push("Hours 50+ are post-OOP planning — included for reference but ignored by the in-game protection import (it applies hours 1–49).");
  }

  return { text: out.join("\n") + "\n", warnings, hours: out.filter((l) => l.startsWith("======")).length };
}

// Render one hour's queued actions into game-formatted lines, MERGING consecutive runs
// of the same action type into one line (the way the game logs a single multi-item
// submission). Merging only consecutive runs preserves the user's ordering across type
// boundaries, so rezone-before-construct / release-before-explore stay valid on import.
function renderHour(acts, entry, ctx) {
  const c = entry.costs;
  // The row is the POST-instant-action state (A_H); to re-gate spells on the mana you HAD and
  // skip already-claimed dailies, read the entering wallet the engine stamps on the row as
  // `enter` (mana / daily flags / peasants), falling back to the row itself for old traces.
  const en = entry.enter || entry;
  const lines = [];
  let manaLeft = en.mana || 0; // self-spells are mana-gated (skipped if short)
  let claimedPlat = !!en.dailyPlatinum, claimedLand = !!en.dailyLand;
  let pend = null;
  const flush = () => { if (pend) { const l = renderMerged(pend, c, ctx); if (l) lines.push(l); pend = null; } };

  for (const a of acts) {
    if (MERGE.has(a.type)) {
      if (!pend || pend.type !== a.type) { flush(); pend = { type: a.type, items: new Map(), order: [], netLand: new Map(), acres: 0 }; }
      accMerged(pend, a);
      continue;
    }
    flush();
    if (a.type === "spell") {
      const cost = (c.spell || {})[a.spell] || 0;
      if (cost <= manaLeft) { manaLeft -= cost; lines.push(`Your wizards successfully cast ${spellName(a.spell)} at a cost of ${ig(cost)} mana.`); }
    } else if (a.type === "claim_platinum") {
      if (!claimedPlat) { claimedPlat = true; lines.push(`You have been awarded with ${ig((en.peasants || 0) * 4)} platinum.`); }
    } else if (a.type === "claim_land") {
      if (!claimedLand) { claimedLand = true; lines.push(`You have been awarded with 20 ${landName(ctx.home)}.`); }
    } else if (a.type === "bank") {
      const src = (a.source || "").replace("resource_", ""), tgt = (a.target || "").replace("resource_", "");
      const recv = Math.floor((a.amount | 0) * (SELL[src] || 0) * (BUY[tgt] || 0));
      if ((a.amount | 0) > 0) lines.push(`${ig(a.amount)} ${src} have been traded for ${ig(recv)} ${tgt}.`);
    } else if (a.type === "draft_rate") {
      lines.push(`Draftrate changed to ${a.rate | 0}%.`);
    } else if (a.type === "improve") {
      const imp = Object.keys(a.data || {})[0] || "";
      if (imp && (a.amount | 0) > 0) lines.push(`You invested ${ig(a.amount)} ${a.resource} into ${imp}.`);
    }
    // a.type === "research": omitted — the protection importer has no research action.
  }
  flush();
  return lines;
}

// Accumulate one action into the pending merged group (summing duplicate keys, because
// the parser overwrites — not sums — a type repeated within a single line).
function accMerged(p, a) {
  const add = (key, n) => { if (!p.items.has(key)) p.order.push(key); p.items.set(key, (p.items.get(key) || 0) + n); };
  const n = a.n | 0;
  switch (a.type) {
    case "construct": add(a.building, n); break;
    case "destroy": add(a.building, n); break;
    case "explore": add(a.land || "plain", n); break;
    case "train": add(a.slot, n); break;
    case "release": add(a.unit === "draftees" ? "draftees" : a.slot, n); break;
    case "rezone":
      p.acres += n;
      p.netLand.set(a.to, (p.netLand.get(a.to) || 0) + n);
      p.netLand.set(a.from, (p.netLand.get(a.from) || 0) - n);
      break;
  }
}

function renderMerged(p, c, ctx) {
  const items = p.order.map((k) => [k, p.items.get(k)]).filter(([, n]) => n > 0);
  switch (p.type) {
    case "construct": {
      if (!items.length) return null;
      const total = items.reduce((s, [, n]) => s + n, 0);
      const list = items.map(([k, n]) => `${ig(n)} ${bldName(k)}`).join(", ");
      return `Construction of ${list} started at a cost of ${ig(total * c.constructPlat)} platinum and ${ig(total * c.constructLumber)} lumber.`;
    }
    case "destroy":
      if (!items.length) return null;
      return `Destruction of ${items.map(([k, n]) => `${ig(n)} ${bldName(k)}`).join(", ")} is complete.`;
    case "explore": {
      if (!items.length) return null;
      const total = items.reduce((s, [, n]) => s + n, 0);
      const list = items.map(([k, n]) => `${ig(n)} ${landName(k)}`).join(", ");
      return `Exploration for ${list} begun at a cost of ${ig(total * c.explorePlat)} platinum and ${ig(total * c.exploreDraftee)} draftees.`;
    }
    case "train": {
      if (!items.length) return null;
      const tot = { platinum: 0, ore: 0, mana: 0, lumber: 0, gems: 0 };
      let draft = 0;
      const list = items.map(([slot, n]) => {
        const t = (c.train || {})[slot] || {};
        for (const res in tot) tot[res] += (t[res] || 0) * n;
        draft += n;
        return `${ig(n)} ${ctx.unitName(slot)}`;
      }).join(", ");
      // platinum + ore always shown (game shows "0 ore"); mana/lumber/gems only when a
      // unit costs them; then draftees/spies/wizards. The cost text is cosmetic — the
      // importer reads only the unit list and recomputes the cost.
      let cost = `${ig(tot.platinum)} platinum, ${ig(tot.ore)} ore`;
      if (tot.mana) cost += `, ${ig(tot.mana)} mana`;
      if (tot.lumber) cost += `, ${ig(tot.lumber)} lumber`;
      if (tot.gems) cost += `, ${ig(tot.gems)} gems`;
      cost += `, ${ig(draft)} draftees, 0 spies, and 0 wizards`;
      return `Training of ${list} begun at a cost of ${cost}.`;
    }
    case "release":
      if (!items.length) return null;
      return `You successfully released ${items.map(([k, n]) => (k === "draftees" ? `${ig(n)} Draftees` : `${ig(n)} ${ctx.unitName(k)}`)).join(", ")}.`;
    case "rezone": {
      const deltas = LAND_ORDER.map((t) => [t, p.netLand.get(t) || 0]).filter(([, d]) => d !== 0);
      if (!deltas.length) return null;
      const list = deltas.map(([t, d]) => `${d > 0 ? ig(d) : "-" + ig(-d)} ${landName(t)}`).join(", ");
      return `Rezoning begun at a cost of ${ig(p.acres * c.rezonePlat)} platinum. The changes in land are as following: ${list}`;
    }
  }
  return null;
}

// Trigger a .txt download in the browser / Tauri webview.
export function downloadLog(filename, text) {
  const blob = new Blob([text], { type: "text/plain;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
