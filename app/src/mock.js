// mock.js — a reactive stand-in for the bit-exact Rust engine, used only when the
// Tauri bridge is absent (browser preview). It is intentionally NOT bit-exact, but
// it now MIRRORS THE ENGINE'S CONTRACT: it does not silently clamp spends, so an
// unaffordable action shows as a negative balance exactly as the engine would. It
// also emits the same per-hour cost table + budget fields the editor enforces on.
// The real backend (src-tauri) calls the untouched engine crate instead.

import { TECHS } from "./techdata.js"; // preview-only tech graph (real app gets it from the engine)

const HOUSING = { home: 30, nonHome: 15, constructing: 15, barren: 5 };
// canonical building → land-type map (matches the engine's building_land map)
const BUILDING_LAND = {
  home: "plain", alchemy: "plain", farm: "plain", smithy: "plain", masonry: "plain",
  tower: "swamp", wizard_guild: "swamp", temple: "swamp",
  ore_mine: "mountain", gryphon_nest: "mountain",
  guard_tower: "hill", factory: "hill", shrine: "hill", barracks: "hill",
  lumberyard: "forest", forest_haven: "forest",
  diamond_mine: "cavern", school: "cavern", dock: "water",
};
const PLAIN_OK = ["home", "alchemy", "farm", "smithy", "masonry"];
const LAND_TYPES = ["plain", "swamp", "hill", "mountain", "forest", "cavern", "water"];
const OOP_HOUR = 49; // hour 49 = out of protection; the economy continues past it (Phase 1)
// Human units: [defense, basePlat, baseOre, trainHours]
const UNIT = {
  1: { off: 3, def: 0, plat: 315, ore: 25, h: 9, name: "Spearman", kind: "specialist" },
  2: { off: 0, def: 3, plat: 275, ore: 15, h: 9, name: "Archer", kind: "specialist" },
  3: { off: 2, def: 6, plat: 1040, ore: 75, h: 12, name: "Knight", kind: "elite" },
  4: { off: 6, def: 3, plat: 1280, ore: 100, h: 12, name: "Cavalry", kind: "elite" },
};
const SPELL_MANA = { midas_touch: 2.5, harmony: 2.5, ares_call: 2.5, gaias_watch: 2.0, mining_strength: 2.0 };
const r = (x) => Math.round(x);
const rceil = (x) => Math.ceil(x - 1e-9);

// Live round-50 races whose units cost NO ore (so the ore column/inputs hide). PREVIEW-ONLY mirror of
// the engine's data-driven `race_has_training_resource` (the real backend computes it from unit costs),
// kept so the browser preview hides ore per race exactly like the desktop app. Only ORE is per-race;
// platinum/food/lumber/mana/gems all stay on for everyone (gems = diamond mines, which everyone builds).
const NO_ORE = new Set(["demon", "firewalker", "lizardfolk", "merfolk", "orc", "spirit-rework", "sylvan", "undead-rework", "vampire", "wood-elf-rework"]);

// Static reference the editor uses for labels (Human stats; the real backend's `meta` command is
// data-driven for every race). The `resources` flag IS race-aware (see NO_ORE/GEMS_RACES) so the
// preview's per-race resource hiding matches the desktop app.
export function meta(race) {
  const buildingLand = { ...BUILDING_LAND };
  delete buildingLand.forest_haven; // round-50: forest_haven is dead code, not buildable
  return {
    units: [1, 2, 3, 4].map((s) => ({ slot: s, name: UNIT[s].name, defense: UNIT[s].def, offense: UNIT[s].off, plat: UNIT[s].plat, ore: UNIT[s].ore, kind: UNIT[s].kind, trainable: true })),
    techs: TECHS,
    buildingLand,
    resources: { ore: !NO_ORE.has(race) },
    homeLand: "plain", // Human preview; the real backend's meta is data-driven per race
    // Common self-spells (Human preview); the real backend adds each race's racial spells.
    spells: [
      { key: "harmony", name: "Harmony", costMana: 2.5, desc: "+50% population growth" },
      { key: "midas_touch", name: "Midas Touch", costMana: 2.5, desc: "+10% platinum" },
      { key: "ares_call", name: "Ares' Call", costMana: 2.5, desc: "+10% defense" },
      { key: "gaias_watch", name: "Gaia's Watch", costMana: 2.0, desc: "+10% food" },
      { key: "mining_strength", name: "Mining Strength", costMana: 2.0, desc: "+10% ore" },
    ],
  };
}

export function simulate(plan) {
  const land = { plain: 350, mountain: 0, swamp: 0, hill: 0, forest: 0, cavern: 0, water: 0 };
  const b = Object.fromEntries(Object.keys(BUILDING_LAND).map((k) => [k, 0]));
  const mil = { u1: 0, u2: 0, u3: 0, u4: 0, spies: 0, assassins: 0, wizards: 0, archmages: 0 };
  // opening build: place ANY building free + instant, auto-zoning its land
  // (non-plain buildings rezone plain → their type at no cost), capped at 350 acres.
  let placed = 0;
  for (const [k, n] of Object.entries(plan.opening || {})) {
    if (b[k] == null) continue;
    const v = Math.max(0, Math.min(n | 0, 350 - placed));
    if (v <= 0) continue;
    b[k] += v; placed += v;
    const lt = BUILDING_LAND[k];
    if (lt !== "plain") { land[lt] += v; land.plain -= v; }
  }
  let peasants = 1000, draftees = 300;
  let plat = 120000, food = 15000, lumber = 15000, ore = 0, mana = 0, gems = 0, tech = 0, boats = 0;
  let morale = 100, draftRate = 90;
  const techs = [];
  const spells = {};            // key -> remaining hours
  const queue = [];             // {arrive, kind, ...}
  const rows = [];
  let dailyPlat = false, dailyLand = false;

  const totalLand = () => LAND_TYPES.reduce((a, t) => a + land[t], 0);
  const totalB = () => Object.values(b).reduce((a, c) => a + c, 0);
  const builtOn = (lt) => Object.entries(b).filter(([k]) => BUILDING_LAND[k] === lt).reduce((a, [, n]) => a + n, 0);
  const constructingTotal = () => queue.filter((q) => q.kind === "build").reduce((a, q) => a + q.n, 0);
  const constructingOn = (lt) => queue.filter((q) => q.kind === "build" && BUILDING_LAND[q.building] === lt).reduce((a, q) => a + q.n, 0);
  const barren = () => totalLand() - totalB() - constructingTotal();
  const jobs = () => (totalB() - b.home - b.barracks) * 20;
  const maxPop = () => r((b.home * HOUSING.home + (totalB() - b.home) * HOUSING.nonHome + constructingTotal() * HOUSING.constructing + barren() * HOUSING.barren) * (1 + 250 / 10000));
  const employed = () => Math.min(jobs(), peasants);
  const smithyMult = () => 1 - Math.min((b.smithy / Math.max(1, totalLand())) * 2, 0.36);
  const gtBonus = () => Math.min(1.6 * b.guard_tower / Math.max(1, totalLand()), 0.32);
  const moraleMult = () => Math.min(1, Math.max(0.9, 0.9 + morale / 1000));
  const mult = () => (1 + gtBonus() + (spells.ares_call > 0 ? 0.1 : 0)) * moraleMult();
  const trainedRaw = () => mil.u1 * UNIT[1].def + mil.u2 * UNIT[2].def + mil.u3 * UNIT[3].def + mil.u4 * UNIT[4].def;
  const gryphonBonus = () => Math.min(1.6 * b.gryphon_nest / Math.max(1, totalLand()), 0.32);
  // preview OP multiplier: gryphon nests × morale (forges/prestige/racial offense omitted in the mock)
  const opMult = () => (1 + gryphonBonus()) * moraleMult();
  const trainedOpRaw = () => mil.u1 * UNIT[1].off + mil.u2 * UNIT[2].off + mil.u3 * UNIT[3].off + mil.u4 * UNIT[4].off;
  const exploreDraftee = () => Math.floor(totalLand() / 150) + 3;
  const explorePlat = () => r(0.6 * Math.pow(totalLand(), 1.299) + (totalLand() < 1520 ? -0.001 * totalLand() ** 2 + 1.91 * totalLand() - 593 : 0));
  const constructPlat = () => r(850 + 1.25 * (totalLand() - 250));
  const constructLumber = () => r(87.5 + 0.285 * (totalLand() - 250));
  const rezonePlat = () => r(250 + 0.6 * (totalLand() - 250));
  const techCost = () => Math.max(3750, r(2.5 * totalLand() + 50 * techs.length));
  const incoming = () => queue.filter((q) => q.kind === "land").reduce((a, q) => a + q.n, 0);
  const techPerHr = () => { const s = b.school; if (s <= 0) return 0; const land = totalLand(); const pct = Math.min(s / land, 0.5); return Math.floor(Math.min(s, Math.floor(land * 0.5)) * (1 - pct)); };

  function costs() {
    const train = {};
    // Human-only preview: train cost keyed by wallet resource name (matches the engine's
    // data-driven shape so the shared editor/log code works in the browser too).
    for (const s of [1, 2, 3, 4]) train[s] = { platinum: rceil(UNIT[s].plat * smithyMult()), ore: rceil(UNIT[s].ore * smithyMult()) };
    // spies/wizards: base 500 platinum (+1 draftee); preview ignores the cost multiplier.
    train.spies = { platinum: 500 }; train.wizards = { platinum: 500 };
    const spell = {};
    for (const k in SPELL_MANA) spell[k] = r(SPELL_MANA[k] * totalLand());
    return { explorePlat: explorePlat(), exploreDraftee: exploreDraftee(), constructPlat: constructPlat(), constructLumber: constructLumber(), rezonePlat: rezonePlat(), techCost: techCost(), train, spell };
  }

  // Mock parity for the engine's caps/employment emits (approximate, preview-only).
  // capCount mirrors engine calc::cap_count — ceil(max/coef·land − ε) so the boundary
  // reads "at cap", not "1 over"; school uses floor(land·0.5).
  function capsOf() {
    const land = Math.max(1, totalLand());
    const capCount = (coef, max) => Math.ceil((max / coef) * land - 1e-6);
    const e = (count, cap, cur, max) => ({ count, capCount: cap, curPct: cur, maxPct: max });
    return {
      guard_tower: e(b.guard_tower, capCount(1.6, 0.32), Math.min((b.guard_tower / land) * 1.6, 0.32) * 100, 32),
      smithy: e(b.smithy, capCount(2, 0.36), Math.min((b.smithy / land) * 2, 0.36) * 100, 36),
      factory: e(b.factory, capCount(5, 0.5), Math.min((b.factory / land) * 5, 0.5) * 100, 50),
      school: e(b.school, Math.floor(0.5 * land), null, null),
      gryphon_nest: e(b.gryphon_nest, capCount(1.6, 0.32), Math.min((b.gryphon_nest / land) * 1.6, 0.32) * 100, 32),
    };
  }
  function employmentOf() {
    const popMil = draftees + mil.u1 + mil.u2 + mil.u3 + mil.u4;
    return {
      jobs: jobs(), employed: employed(), peasants,
      maxPeasantPop: maxPop() - popMil, populationMilitary: popMil,
      jobsPerBuilding: 20, housingPerHome: 30, housingPerNonhome: 15, barracksMilitaryHousing: 36,
    };
  }

  function snapshot(hour) {
    const platHr = (r(b.alchemy * 45 + employed() * 2.7) * (spells.midas_touch > 0 ? 1.1 : 1)) | 0;
    const foodGross = r(b.farm * 80 * (1 + (spells.gaias_watch > 0 ? 0.1 : 0)));
    const foodNet = foodGross - r((peasants + draftees + mil.u1 + mil.u2 + mil.u3 + mil.u4) * 0.25) - r(food * 0.01);
    const freeLandByType = Object.fromEntries(LAND_TYPES.map((t) => [t, land[t] - builtOn(t) - constructingOn(t)]));
    return {
      hour, rem: 48 - hour,
      land: totalLand(), landBy: { ...land }, incoming: incoming(), barren: barren(), freeLandByType,
      peasants, draftees, maxPop: maxPop(), employed: employed(), jobs: jobs(),
      platinum: plat, food, lumber, ore, mana, gems, tech, boats,
      platPerHr: r(platHr), foodNet, lumberPerHr: b.lumberyard * 50 + b.forest_haven * 25, manaPerHr: b.tower * 25 + b.wizard_guild * 5, orePerHr: b.ore_mine * 60 * (spells.mining_strength > 0 ? 1.1 : 1),
      gemPerHr: b.diamond_mine * 15, techPerHr: techPerHr(), boatsPerHr: b.dock / 20,
      trainedRaw: trainedRaw(), trainedModded: trainedRaw() * mult(), mult: mult(),
      trainedOpRaw: trainedOpRaw(), trainedOpModded: trainedOpRaw() * opMult(), opMult: opMult(),
      morale, draftRate,
      dailyPlatinum: dailyPlat, dailyLand: dailyLand, techs: [...techs],
      costs: costs(),
      caps: capsOf(), employment: employmentOf(),
      buildings: { ...b }, military: { ...mil, draftees },
      spells: Object.entries(spells).filter(([, d]) => d > 0).map(([k, d]) => ({ key: k, dur: d })),
      actions: (plan.hours && plan.hours[hour - 1]) || [],
    };
  }

  { const r0 = snapshot(0); r0.enter = { mana, dailyPlatinum: dailyPlat, dailyLand: dailyLand, peasants }; rows.push(r0); }

  const HOURS = (plan.hours || []).length || 48; // protection (48) + post-OOP planning window
  for (let h = 1; h <= HOURS; h++) {
    // Daily plat/land bonus resets every game-day (hours 1, 25, 49, 73, 97 …) — continues
    // past OOP, mirroring the engine's post_oop_tick (preview-approximate).
    if (h % 24 === 1) { dailyPlat = false; dailyLand = false; }
    const acts = (plan.hours && plan.hours[h - 1]) || [];
    // Capture the ENTERING wallet (E_H) the log exporter re-gates from, BEFORE this hour's
    // instant actions mutate the pools (mana / daily-claim flags / peasants).
    const enter = { mana, dailyPlatinum: dailyPlat, dailyLand: dailyLand, peasants };
    // Instant actions FIRST — they affect THIS tick's balances (spell mana spent from the
    // current pool, daily claims, rezones, queued-build/explore/train payments). costs() is
    // recomputed per action, so a same-tick claim_land escalates the rezone/build after it.
    for (const a of acts) {
      const c = costs();
      if (a.type === "claim_platinum") { if (!dailyPlat) { plat += peasants * 4; tech += 350; dailyPlat = true; } }
      else if (a.type === "claim_land") { if (!dailyLand) { land.plain += 20; dailyLand = true; } }
      else if (a.type === "rezone") {
        const n = a.n | 0;
        land[a.from] -= n; land[a.to] += n; plat -= c.rezonePlat * n;     // no clamp → can go negative
      } else if (a.type === "construct") {
        const n = a.n | 0;
        plat -= c.constructPlat * n; lumber -= c.constructLumber * n;
        queue.push({ arrive: h + 12, kind: "build", building: a.building, n });
      } else if (a.type === "explore") {
        const n = a.n | 0, lt = LAND_TYPES.includes(a.land) ? a.land : "plain";
        plat -= c.explorePlat * n; draftees -= c.exploreDraftee * n;
        queue.push({ arrive: h + 12, kind: "land", land: lt, n });
        morale = Math.max(0, morale - Math.max(1, Math.floor((n + 2) / 3)));
      } else if (a.type === "train") {
        const n = a.n | 0, t = c.train[a.slot] || {};
        if (a.slot === "spies" || a.slot === "wizards") {
          plat -= (t.platinum || 0) * n; draftees -= n;
          queue.push({ arrive: h + 12, kind: "espionage", unit: a.slot, n });
        } else {
          const u = UNIT[a.slot]; if (!u) continue;
          plat -= (t.platinum || 0) * n; ore -= (t.ore || 0) * n; draftees -= n;
          queue.push({ arrive: h + u.h, kind: "unit", slot: a.slot, n });
        }
      } else if (a.type === "spell") {
        const cost = c.spell[a.spell] || 0;
        if (mana >= cost) { mana -= cost; spells[a.spell] = 12; }          // engine gates spells on mana
      } else if (a.type === "bank") {
        const src = (a.source || "").replace("resource_", ""), tgt = (a.target || "").replace("resource_", "");
        const pool = { platinum: plat, lumber, ore, gems, food, mana };
        const amt = Math.max(0, Math.min(a.amount | 0, pool[src] ?? 0));
        const sell = src === "gems" ? 2 : 0.5, buy = tgt === "food" ? 0.5 : 1;
        const gained = Math.floor(amt * sell * buy);
        if (src === "platinum") plat -= amt; else if (src === "lumber") lumber -= amt; else if (src === "ore") ore -= amt; else if (src === "gems") gems -= amt;
        if (tgt === "platinum") plat += gained; else if (tgt === "lumber") lumber += gained; else if (tgt === "ore") ore += gained; else if (tgt === "food") food += gained;
      } else if (a.type === "destroy") {
        if (b[a.building] != null) b[a.building] = Math.max(0, b[a.building] - (a.n | 0));
      } else if (a.type === "release") {
        const n = a.n | 0;
        if (a.unit === "draftees") { draftees -= n; peasants += n; }
        else if (mil["u" + a.slot] != null) { mil["u" + a.slot] -= n; draftees += n; }
      } else if (a.type === "draft_rate") {
        draftRate = a.rate | 0;
      } else if (a.type === "improve") {
        const amt = a.amount | 0, res = a.resource;
        const pool = { platinum: plat, lumber, ore, gems };
        if ((pool[res] ?? 0) >= amt && amt > 0) {
          if (res === "platinum") plat -= amt; else if (res === "lumber") lumber -= amt; else if (res === "ore") ore -= amt; else if (res === "gems") gems -= amt;
        }
      } else if (a.type === "research") {
        if (!techs.includes(a.tech) && tech >= c.techCost) { tech -= c.techCost; techs.push(a.tech); }
      }
    }
    // Snapshot the POST-instant-action state (A_H): production has NOT landed yet, so it shows
    // in the NEXT row. Carries hour h's actions + the entering-wallet `enter` fields (for the log).
    { const row = snapshot(h); row.enter = enter; rows.push(row); }

    // resolve arrivals
    for (const q of queue.filter((q) => q.arrive === h)) {
      if (q.kind === "build") b[q.building] += q.n;
      else if (q.kind === "land") land[q.land || "plain"] += q.n;
      else if (q.kind === "unit") mil["u" + q.slot] += q.n;
      else if (q.kind === "espionage") mil[q.unit] += q.n;
    }
    for (let i = queue.length - 1; i >= 0; i--) if (queue[i].arrive === h) queue.splice(i, 1);

    // production
    plat += (r(b.alchemy * 45 + employed() * 2.7) * (spells.midas_touch > 0 ? 1.1 : 1)) | 0;
    lumber += b.lumberyard * 50 + b.forest_haven * 25 - r(lumber * 0.01);
    mana += b.tower * 25 + b.wizard_guild * 5 - r(mana * 0.02);
    ore += r(b.ore_mine * 60 * (spells.mining_strength > 0 ? 1.1 : 1));
    gems += b.diamond_mine * 15;
    tech += techPerHr();
    boats += b.dock / 20;
    food += r(b.farm * 80 * (1 + (spells.gaias_watch > 0 ? 0.1 : 0))) - r((peasants + draftees + mil.u1 + mil.u2 + mil.u3 + mil.u4) * 0.25) - r(food * 0.01);
    food = Math.max(0, food);

    // growth (temples drive births; draft-rate gates draftee growth)
    const totalPop = peasants + draftees + mil.u1 + mil.u2 + mil.u3 + mil.u4;
    const milPct = totalPop > 0 ? ((draftees + mil.u1 + mil.u2 + mil.u3 + mil.u4) / totalPop) * 100 : 0;
    const birthMult = food > 0 ? 1 + (b.temple / Math.max(1, totalLand())) * 6 + (spells.harmony > 0 ? 0.5 : 0) : 0;
    const dg = food > 0 && milPct < draftRate ? r(peasants * 0.01) : 0;
    const room = Math.max(0, maxPop() - totalPop - dg);
    const birth = food > 0 ? r((peasants - dg) * 0.03 * birthMult) : r(-0.05 * peasants);
    peasants += Math.max(-peasants, Math.min(room, birth - dg));
    draftees += dg;

    for (const k in spells) if (spells[k] > 0) spells[k]--;
    morale = Math.min(100, morale + (morale < 80 ? 6 : 3));
  }

  // Trailing post-OOP end row (entering hour HOURS+1 = end of hour HOURS), matching the engine.
  { const endRow = snapshot(HOURS + 1); endRow.enter = { mana, dailyPlatinum: dailyPlat, dailyLand: dailyLand, peasants }; rows.push(endRow); }

  // `final` = the OOP headline = the entering-hour-49 row (NOT the post-OOP end). (The mock
  // doesn't model the OOP Ares boost, so OOP DP here is approximate — NOT game-accurate.)
  const oop = rows[OOP_HOUR] || rows[rows.length - 1];
  const committed = oop.land + oop.incoming;
  const feasible = oop.trainedModded >= (plan.dpTarget || 0);
  const final = {
    ...oop, race: plan.race || "human", committed, feasible,
    dpTarget: plan.dpTarget || 0,
    targetShort: Math.max(0, (plan.dpTarget || 0) - oop.trainedModded),
  };
  return { rows, final };
}
