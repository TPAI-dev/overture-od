// mock.js — a reactive stand-in for the bit-exact Rust engine, used only when the
// Tauri bridge is absent (browser preview). It is intentionally NOT bit-exact, but
// it now MIRRORS THE ENGINE'S CONTRACT: it does not silently clamp spends, so an
// unaffordable action shows as a negative balance exactly as the engine would. It
// also emits the same per-hour cost table + budget fields the editor enforces on.
// The real backend (src-tauri) calls the untouched engine crate instead.

import { TECHS } from "./techdata.js"; // preview-only tech graph (real app gets it from the engine)
const TECH_BY_KEY = new Map(TECHS.map((tech) => [tech.key, tech]));

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
const OOP_HOUR = 49; // hour 49 = out of protection; events begin here
const RULESET = {
  id: "round51",
  round: 51,
  sourceTag: "1.51.0",
  sourceCommit: "35b977df6b47fd24636f920657b5c4edb46bbff7",
  productionOverrides: {
    arcaneInfusionPlatinum: "1400: developer-confirmed live production value on 2026-07-31; the 1.51.0 tag still contains the stale 1350 value",
    sylvanCentaurPlatinum: "950: developer-confirmed live production value on 2026-07-31; the 1.51.0 tag still contains the stale 970 value",
  },
};
// Human units: preview-only stats plus exact invasion return hours.
const UNIT = {
  1: { off: 3, def: 0, plat: 315, ore: 25, h: 9, ret: 12, name: "Spearman", kind: "specialist" },
  2: { off: 0, def: 3, plat: 275, ore: 15, h: 9, ret: 12, name: "Archer", kind: "specialist" },
  3: { off: 2, def: 6, plat: 1040, ore: 75, h: 12, ret: 12, name: "Knight", kind: "elite" },
  4: { off: 6, def: 3, plat: 1280, ore: 100, h: 12, ret: 9, name: "Cavalry", kind: "elite" },
};
const SPELL_MANA = { midas_touch: 2.5, harmony: 2.5, ares_call: 2.5, gaias_watch: 2.0, mining_strength: 2.0 };
const IMPROVEMENT_KEYS = ["science", "keep", "forges", "walls", "spires", "harbor"];
const INVESTMENT_WORTH = { platinum: 1, lumber: 2, ore: 2, gems: 12 };
const r = (x) => Math.round(x);
const rceil = (x) => Math.ceil(x - 1e-9);

// Browser-preview counterpart to combat::land_gained. The desktop app always
// calls the Rust command; this keeps the non-bit-exact design preview usable.
export function invasionLandGain(attackerLand, targetLand) {
  const attacker = Math.trunc(attackerLand || 0), target = Math.trunc(targetLand || 0);
  if (attacker <= 0) throw new Error("attacker land must be positive");
  if (target <= 0) throw new Error("target land must be positive");
  const ratio = target / attacker;
  if (ratio < 0.4 || ratio > 2.5) throw new Error(`target at ${target} acres is outside the legal 40%-250% invasion range`);
  const base = ratio < 0.55
    ? 0.304 * ratio * ratio - 0.227 * ratio + 0.048
    : ratio < 0.75 ? 0.154 * ratio - 0.069 : 0.129 * ratio - 0.048;
  const conquered = Math.max(Math.floor(Math.round(base * 0.75 * attacker * 1e10) / 1e10), 10);
  return { attackerLand: attacker, targetLand: target, rangePct: ratio * 100, conquered, generated: conquered, gained: conquered * 2 };
}

// Static reference the editor uses for labels (Human stats; the real backend's `meta` command is
// data-driven for every race). Ore stays visible for every race because it can
// always be invested into castle improvements.
export function meta(race) {
  const buildingLand = { ...BUILDING_LAND };
  delete buildingLand.forest_haven; // dead source entry, not buildable
  return {
    units: [1, 2, 3, 4].map((s) => ({ slot: s, name: UNIT[s].name, defense: UNIT[s].def, offense: UNIT[s].off, plat: UNIT[s].plat, ore: UNIT[s].ore, kind: UNIT[s].kind, returnHours: UNIT[s].ret, needBoat: true, trainable: true })),
    techs: TECHS,
    buildingLand,
    resources: { ore: true },
    boatCapacity: 30,
    homeLand: "plain", // Human preview; the real backend's meta is data-driven per race
    ruleset: { ...RULESET, productionOverrides: { ...RULESET.productionOverrides } },
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
  let morale = 100, prestige = 250, draftRate = 90, discountedLand = 0;
  const techs = [];
  const improvements = Object.fromEntries(IMPROVEMENT_KEYS.map((key) => [key, 0]));
  const baseBoatCapacity = meta(plan.race).boatCapacity;
  const boatCapacity = () => baseBoatCapacity + techs.reduce(
    (total, key) => total + (TECH_BY_KEY.get(key)?.perks?.boat_capacity || 0),
    0
  );
  const spells = {};            // key -> remaining hours
  const queue = [];             // {arrive, kind, ...}
  const rows = [];
  const eventsByHour = new Map();
  let dailyPlat = false, dailyLand = false;

  const totalLand = () => LAND_TYPES.reduce((a, t) => a + land[t], 0);
  const round4 = (value) => Math.round(value * 10000) / 10000;
  const improvementEfficiency = () => 1 + (b.masonry * 2.75) / Math.max(1, totalLand());
  const improvementBonus = (key, max, coeff) => improvements[key] <= 0 ? 0 : round4(max * (1 - Math.exp(-improvements[key] / (coeff * totalLand() + 15000))) * improvementEfficiency());
  const scienceBonus = () => improvementBonus("science", 0.20, 4000);
  const keepBonus = () => improvementBonus("keep", 0.25, 4000);
  const forgesBonus = () => improvementBonus("forges", 0.30, 7500);
  const wallsBonus = () => improvementBonus("walls", 0.30, 7500);
  const spiresBonus = () => improvementBonus("spires", 0.60, 5000);
  const harborBonus = () => improvementBonus("harbor", 0.60, 5000);
  const harborBoatBonus = () => improvements.harbor <= 0 ? 0 : round4(Math.min(0.50, 0.60 * (1 - Math.exp(-improvements.harbor / (5000 * totalLand() + 15000))) * 1.5));
  const investmentMultiplier = (resource, improvement) => 1
    + (plan.race === "goblin" && resource === "gems" ? 0.10 : 0)
    + techs.reduce((sum, key) => sum + ((TECH_BY_KEY.get(key)?.perks?.[`invest_bonus_${improvement}`] || 0) / 100), 0);
  const totalB = () => Object.values(b).reduce((a, c) => a + c, 0);
  const builtOn = (lt) => Object.entries(b).filter(([k]) => BUILDING_LAND[k] === lt).reduce((a, [, n]) => a + n, 0);
  const constructingTotal = () => queue.filter((q) => q.kind === "build").reduce((a, q) => a + q.n, 0);
  const constructingOn = (lt) => queue.filter((q) => q.kind === "build" && BUILDING_LAND[q.building] === lt).reduce((a, q) => a + q.n, 0);
  const barren = () => totalLand() - totalB() - constructingTotal();
  const jobs = () => (totalB() - b.home - b.barracks) * 20;
  const maxPop = () => r((b.home * HOUSING.home + (totalB() - b.home) * HOUSING.nonHome + constructingTotal() * HOUSING.constructing + barren() * HOUSING.barren) * (1 + prestige / 10000 + keepBonus()));
  const employed = () => Math.min(jobs(), peasants);
  const smithyMult = () => 1 - Math.min((b.smithy / Math.max(1, totalLand())) * 2, 0.36);
  const gtBonus = () => Math.min(1.6 * b.guard_tower / Math.max(1, totalLand()), 0.32);
  const moraleMult = () => Math.min(1, Math.max(0.9, 0.9 + morale / 1000));
  const mult = () => (1 + gtBonus() + wallsBonus() + (spells.ares_call > 0 ? 0.1 : 0)) * moraleMult();
  const trainedRaw = () => mil.u1 * UNIT[1].def + mil.u2 * UNIT[2].def + mil.u3 * UNIT[3].def + mil.u4 * UNIT[4].def;
  const gryphonBonus = () => Math.min(1.6 * b.gryphon_nest / Math.max(1, totalLand()), 0.32);
  // preview OP multiplier: gryphon nests + Forges × morale (other race/prestige channels omitted).
  const opMult = () => (1 + gryphonBonus() + forgesBonus()) * moraleMult();
  const trainedOpRaw = () => mil.u1 * UNIT[1].off + mil.u2 * UNIT[2].off + mil.u3 * UNIT[3].off + mil.u4 * UNIT[4].off;
  const exploreDraftee = () => Math.floor(totalLand() / 150) + 3;
  const explorePlat = () => r(0.6 * Math.pow(totalLand(), 1.299) + (totalLand() < 1520 ? -0.001 * totalLand() ** 2 + 1.91 * totalLand() - 593 : 0));
  const constructPlat = () => r(850 + 1.25 * (totalLand() - 250));
  const constructLumber = () => r(87.5 + 0.285 * (totalLand() - 250));
  const rezonePlat = () => r(250 + 0.6 * (totalLand() - 250));
  const techCost = () => Math.max(3750, r(2.5 * totalLand() + 50 * techs.length));
  const roundDay = () => Math.max(1, (plan.daysLate | 0) + 1 + Math.floor((Math.max(1, currentHour) - 1) / 24));
  const discountedLandMultiplier = () => Math.round(Math.min(0.50, Math.max(0.35, 1 - (0.0075 * (roundDay() + 42) - 0.0025))) * 10000) / 10000;
  const discountedConstructionCost = (perAcre, count) => {
    const n = Math.max(0, count | 0), discounted = Math.min(n, Math.max(0, discountedLand | 0));
    return perAcre * n - (discounted > 0 ? rceil(perAcre * discounted * (1 - discountedLandMultiplier())) : 0);
  };
  const incoming = () => queue.filter((q) => q.kind === "land").reduce((a, q) => a + q.n, 0);
  const incomingByType = () => Object.fromEntries(LAND_TYPES.map((t) => [t, queue.filter((q) => q.kind === "land" && (q.land || "plain") === t).reduce((a, q) => a + q.n, 0)]));
  const away = () => {
    const units = { u1: 0, u2: 0, u3: 0, u4: 0 };
    const returns = [];
    for (const q of queue.filter((q) => q.kind === "return")) {
      units["u" + q.slot] += q.n;
      returns.push({ slot: q.slot, hours: Math.max(0, q.arrive - currentHour), amount: q.n });
    }
    return { ...units, total: units.u1 + units.u2 + units.u3 + units.u4, returns };
  };
  let currentHour = 0;
  const techPerHr = () => { const s = b.school; if (s <= 0) return 0; const land = totalLand(); const pct = Math.min(s / land, 0.5); return Math.floor(Math.min(s, Math.floor(land * 0.5)) * (1 - pct)); };

  function costs() {
    const train = {};
    // Human-only preview: train cost keyed by wallet resource name (matches the engine's
    // data-driven shape so the shared editor/log code works in the browser too).
    for (const s of [1, 2, 3, 4]) {
      train[s] = { platinum: rceil(UNIT[s].plat * smithyMult()), draftees: 1 };
      if (UNIT[s].ore > 0) train[s].ore = rceil(UNIT[s].ore * smithyMult());
    }
    // Common-unit preview costs. The desktop backend supplies exact modifiers
    // and Arcane Infusion state; this keeps browser editing structurally faithful.
    train.spies = { platinum: 500, draftees: 1 };
    train.assassins = { platinum: 1000, spies: 1 };
    train.wizards = { platinum: 500, draftees: 1 };
    train.archmages = { platinum: 1000, wizards: 1 };
    const spell = {};
    for (const k in SPELL_MANA) spell[k] = r(SPELL_MANA[k] * totalLand());
    return {
      explorePlat: explorePlat(), exploreDraftee: exploreDraftee(),
      constructPlat: constructPlat(), constructLumber: constructLumber(),
      constructDiscountMultiplier: discountedLandMultiplier(),
      rezonePlat: rezonePlat(), techCost: techCost(), train, spell,
    };
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
    const aw = away();
    const popMil = draftees + mil.u1 + mil.u2 + mil.u3 + mil.u4 + aw.total;
    return {
      jobs: jobs(), employed: employed(), peasants,
      maxPeasantPop: maxPop() - popMil, populationMilitary: popMil,
      jobsPerBuilding: 20, housingPerHome: 30, housingPerNonhome: 15, barracksMilitaryHousing: 36,
    };
  }
  function improvementsOf() {
    const bonusFor = (key) => ({
      science: scienceBonus, keep: keepBonus, forges: forgesBonus,
      walls: wallsBonus, spires: spiresBonus, harbor: harborBonus,
    })[key]();
    return Object.fromEntries(IMPROVEMENT_KEYS.map((key) => [key, {
      points: improvements[key],
      bonusPct: bonusFor(key) * 100,
      secondaryBonusPct: key === "harbor" ? harborBoatBonus() * 100 : null,
      multipliers: Object.fromEntries(Object.keys(INVESTMENT_WORTH).map((resource) => [resource, investmentMultiplier(resource, key)])),
    }]));
  }

  function snapshot(hour) {
    const platHr = r((b.alchemy * 45 + employed() * 2.7) * (1 + scienceBonus() + (spells.midas_touch > 0 ? 0.1 : 0)));
    const foodGross = r(b.farm * 80 * (1 + harborBonus() + (spells.gaias_watch > 0 ? 0.1 : 0)));
    const aw = away();
    const popMil = draftees + mil.u1 + mil.u2 + mil.u3 + mil.u4 + aw.total;
    const population = peasants + popMil;
    const foodNet = foodGross - r(population * 0.25) - r(food * 0.01);
    const freeLandByType = Object.fromEntries(LAND_TYPES.map((t) => [t, land[t] - builtOn(t) - constructingOn(t)]));
    return {
      hour, roundDay: roundDay(), rem: 48 - hour,
      land: totalLand(), landBy: { ...land }, incoming: incoming(), incomingByType: incomingByType(), barren: barren(), freeLandByType,
      peasants, draftees, population, populationMilitary: popMil, maxPop: maxPop(), employed: employed(), jobs: jobs(),
      platinum: plat, food, lumber, ore, mana, gems, tech, boats, boatCapacity: boatCapacity(),
      platPerHr: r(platHr), foodNet, lumberPerHr: b.lumberyard * 50 + b.forest_haven * 25, manaPerHr: r((b.tower * 25 + b.wizard_guild * 5) * (1 + spiresBonus())), orePerHr: b.ore_mine * 60 * (spells.mining_strength > 0 ? 1.1 : 1),
      gemPerHr: b.diamond_mine * 15, techPerHr: techPerHr(), boatsPerHr: b.dock / 20 * (1 + harborBoatBonus()),
      trainedRaw: trainedRaw(), trainedModded: trainedRaw() * mult(), mult: mult(),
      trainedOpRaw: trainedOpRaw(), trainedOpModded: trainedOpRaw() * opMult(), opMult: opMult(),
      unitOffense: [1, 2, 3, 4].map((slot) => UNIT[slot].off),
      unitNeedBoat: [true, true, true, true],
      unitReturnHours: [1, 2, 3, 4].map((slot) => UNIT[slot].ret),
      morale, prestige, draftRate, discountedLand,
      dailyPlatinum: dailyPlat, dailyLand: dailyLand, techs: [...techs],
      costs: costs(),
      caps: capsOf(), employment: employmentOf(), improvements: improvementsOf(),
      buildings: { ...b }, military: { ...mil, draftees }, away: aw,
      unitNeedsBoat: [true, true, true, true],
      spells: Object.entries(spells).filter(([, d]) => d > 0).map(([k, d]) => ({ key: k, dur: d })),
      actions: (plan.hours && plan.hours[hour - 1]) || [],
      events: eventsByHour.get(hour) || [],
    };
  }

  { const r0 = snapshot(0); r0.enter = { mana, dailyPlatinum: dailyPlat, dailyLand: dailyLand, peasants, discountedLand }; rows.push(r0); }

  const eventEnd = Math.max(0, ...(plan.events || []).map((e) => Math.min(528, Math.max(0, e.hour | 0)) + (e.type === "invasion" ? 11 : 0)));
  const HOURS = Math.max((plan.hours || []).length || 48, eventEnd); // includes the +12 arrival row
  for (let h = 1; h <= HOURS; h++) {
    currentHour = h;
    // Daily plat/land bonus resets every game-day (hours 1, 25, 49, 73, 97 …) — continues
    // past OOP, mirroring the engine's post_oop_tick (preview-approximate).
    if (h % 24 === 1) { dailyPlat = false; dailyLand = false; }
    // Arrivals are part of the entering wallet for this hour.
    for (const q of queue.filter((q) => q.arrive === h)) {
      if (q.kind === "build") b[q.building] += q.n;
      else if (q.kind === "land") {
        land[q.land || "plain"] += q.n;
        if (q.discounted) discountedLand += q.n;
      }
      else if (q.kind === "unit") mil["u" + q.slot] += q.n;
      else if (q.kind === "return") mil["u" + q.slot] += q.n;
      else if (q.kind === "espionage") mil[q.unit] += q.n;
      else if (q.kind === "prestige") prestige += q.n;
      else if (q.kind === "boats") boats += q.n;
    }
    for (let i = queue.length - 1; i >= 0; i--) if (queue[i].arrive === h) queue.splice(i, 1);
    const acts = (plan.hours && plan.hours[h - 1]) || [];
    // Capture the ENTERING wallet (E_H) the log exporter re-gates from, BEFORE this hour's
    // instant actions mutate the pools (mana / daily-claim flags / peasants).
    const enter = { mana, dailyPlatinum: dailyPlat, dailyLand: dailyLand, peasants, discountedLand };
    const resolvedAll = []; // per-hour resolved invest-all amounts, in action order (for the row echo)
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
        plat -= discountedConstructionCost(c.constructPlat, n);
        lumber -= discountedConstructionCost(c.constructLumber, n);
        discountedLand = Math.max(0, discountedLand - n);
        queue.push({ arrive: h + 12, kind: "build", building: a.building, n });
      } else if (a.type === "explore") {
        const n = a.n | 0, lt = LAND_TYPES.includes(a.land) ? a.land : "plain";
        plat -= c.explorePlat * n; draftees -= c.exploreDraftee * n;
        queue.push({ arrive: h + 12, kind: "land", land: lt, n });
        morale = Math.max(0, morale - Math.max(1, Math.floor((n + 2) / 3)));
      } else if (a.type === "train") {
        const n = a.n | 0, t = c.train[a.slot] || {};
        plat -= (t.platinum || 0) * n;
        ore -= (t.ore || 0) * n;
        mana -= (t.mana || 0) * n;
        lumber -= (t.lumber || 0) * n;
        gems -= (t.gems || 0) * n;
        draftees -= (t.draftees || 0) * n;
        mil.spies -= (t.spies || 0) * n;
        mil.wizards -= (t.wizards || 0) * n;
        if (typeof a.slot === "string") {
          queue.push({ arrive: h + 12, kind: "espionage", unit: a.slot, n });
        } else {
          const u = UNIT[a.slot]; if (!u) continue;
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
        const res = a.resource, worth = INVESTMENT_WORTH[res];
        if (!worth) throw new Error("castle investments accept platinum, lumber, ore, or gems");
        if (a.all === true) {
          // invest-all (auto-invest rules): the entire current stock into one improvement,
          // resolved here at execution; the row echo below reports the resolved amount.
          const pool0 = { platinum: plat, lumber, ore, gems };
          const keys = Object.keys(a.data || {});
          if (keys.length !== 1 || !IMPROVEMENT_KEYS.includes(keys[0])) throw new Error("invest-all needs exactly one improvement");
          const amt0 = Math.max(0, Math.floor(pool0[res] ?? 0));
          resolvedAll.push(amt0);
          if (amt0 > 0) {
            improvements[keys[0]] += Math.floor(amt0 * worth * investmentMultiplier(res, keys[0]));
            if (res === "platinum") plat -= amt0; else if (res === "lumber") lumber -= amt0; else if (res === "ore") ore -= amt0; else if (res === "gems") gems -= amt0;
          }
          continue;
        }
        const entries = Object.entries(a.data || {});
        if (!entries.length || entries.some(([key, amount]) => !IMPROVEMENT_KEYS.includes(key) || !Number.isInteger(amount) || amount < 0)) {
          throw new Error("castle investment contains an invalid improvement allocation");
        }
        const amt = entries.reduce((sum, [, amount]) => sum + amount, 0);
        if (amt <= 0) throw new Error("castle investment must allocate a positive amount");
        if (a.amount != null && (a.amount | 0) !== amt) throw new Error("castle investment total does not match its allocations");
        const pool = { platinum: plat, lumber, ore, gems };
        if ((pool[res] ?? 0) >= amt && amt > 0) {
          for (const [key, amount] of entries) improvements[key] += Math.floor(amount * worth * investmentMultiplier(res, key));
          if (res === "platinum") plat -= amt; else if (res === "lumber") lumber -= amt; else if (res === "ore") ore -= amt; else if (res === "gems") gems -= amt;
        }
      } else if (a.type === "research") {
        if (!techs.includes(a.tech) && tech >= c.techCost) { tech -= c.techCost; techs.push(a.tech); }
      }
    }
    // Explicit scenario events are applied after same-hour actions/spells and
    // before the tick, matching the desktop engine ordering.
    const outcomes = [];
    for (const ev of (plan.events || []).filter((e) => (e.hour | 0) === h)) {
      if (h < OOP_HOUR) throw new Error("scenario events begin out of protection at hour 49");
      if (ev.type === "prestige") {
        const amount = ev.amount | 0;
        if (!amount) throw new Error("prestige event amount must be non-zero");
        if (prestige + amount < 0) throw new Error("prestige adjustment would reduce prestige below zero");
        prestige += amount;
        outcomes.push({ id: ev.id, type: "prestige", hour: h, prestige: amount });
        continue;
      }
      if (ev.type !== "invasion") throw new Error(`unknown scenario event: ${ev.type}`);
      const sent = [0, 1, 2, 3].map((i) => Math.max(0, (ev.sent && ev.sent[i]) | 0));
      for (let i = 0; i < 4; i++) if (sent[i] > (mil["u" + (i + 1)] || 0)) throw new Error(`only ${mil["u" + (i + 1)] || 0} slot ${i + 1} troops are home`);
      if (!sent.some((n) => n > 0)) throw new Error("invasion must send at least one troop");
      const targetLand = ev.targetLand | 0, targetDp = Math.max(0, +ev.targetDp || 0);
      const ratio = targetLand / Math.max(1, totalLand());
      if (ratio < 0.4 || ratio > 2.5) throw new Error("target is outside the legal invasion range");
      const boatUnits = sent.reduce((a, n) => a + n, 0);
      if (boatUnits > Math.floor(boats) * boatCapacity()) throw new Error("not enough boats to carry the requested invasion army");
      if (morale < 80) throw new Error(`invasion requires at least 80 morale; the dominion has ${morale}`);
      const op = sent.reduce((sum, n, i) => sum + n * UNIT[i + 1].off, 0) * opMult();
      if (op <= targetDp) throw new Error(`invasion would fail: ${r(op)} OP does not beat ${r(targetDp)} target DP`);
      const defenseMult = mult();
      const sentDp = sent.reduce((sum, n, i) => sum + n * UNIT[i + 1].def, 0) * defenseMult;
      const currentHomeDp = (draftees + trainedRaw()) * defenseMult;
      const returningDp = [1, 2, 3, 4].reduce((sum, slot) => sum + away()["u" + slot] * UNIT[slot].def, 0) * defenseMult;
      if (sentDp > 0 && currentHomeDp - sentDp < (currentHomeDp + returningDp) * 0.40) {
        throw new Error("invasion must leave enough defensive power at home (40% rule)");
      }
      const homeDp = (draftees + sent.reduce((sum, n, i) => sum + ((mil["u" + (i + 1)] || 0) - n) * UNIT[i + 1].def, 0)) * defenseMult;
      if (op > Math.ceil(homeDp * 1.25)) {
        throw new Error("invasion sends too much offense for the defensive power left home (5:4 rule)");
      }
      const totalSent = sent.reduce((a, n) => a + n, 0);
      const needed = Math.round(targetDp / Math.max(0.0001, op / totalSent));
      const calculated = sent.map((n) => n ? rceil(Math.round(needed * n / totalSent) * 0.085) : 0);
      const casualties = Array.isArray(ev.casualtiesOverride) ? ev.casualtiesOverride.map((n, i) => Math.max(0, Math.min(sent[i], n | 0))) : calculated;
      const survivors = sent.map((n, i) => n - casualties[i]);
      const returnHours = [1, 2, 3, 4].map((slot) => UNIT[slot].ret);
      for (let i = 0; i < 4; i++) {
        mil["u" + (i + 1)] -= sent[i];
        if (survivors[i] > 0) queue.push({ arrive: h + returnHours[i], kind: "return", slot: i + 1, n: survivors[i] });
      }
      for (const ret of [...new Set(returnHours)]) {
        const carried = sent.reduce((sum, n, i) => sum + (returnHours[i] === ret ? n : 0), 0);
        const sentBoats = Math.floor(carried / 30);
        if (sentBoats > 0) { boats -= sentBoats; queue.push({ arrive: h + ret, kind: "boats", n: sentBoats }); }
      }
      const byType = Object.fromEntries(LAND_TYPES.map((t) => [t, Math.max(0, (ev.landByType && ev.landByType[t]) | 0)]));
      const landTotal = Object.values(byType).reduce((a, n) => a + n, 0);
      for (const [lt, n] of Object.entries(byType)) if (n > 0) {
        queue.push({ arrive: h + 12, kind: "land", land: lt, n, discounted: ratio >= 0.75 });
      }
      const slowest = Math.max(...sent.map((n, i) => n > 0 ? returnHours[i] : 0));
      if ((ev.prestige | 0) > 0) queue.push({ arrive: h + slowest, kind: "prestige", n: ev.prestige | 0 });
      const moraleDelta = -5; morale = Math.max(0, morale + moraleDelta);
      outcomes.push({
        id: ev.id, type: "invasion", hour: h, sent, calculatedCasualties: calculated,
        casualties, survivors, converted: [0, 0, 0, 0], returnHours, landByType: byType, landTotal,
        landReturnHour: landTotal > 0 ? h + 12 : null, prestige: ev.prestige | 0,
        prestigeReturnHour: (ev.prestige | 0) > 0 ? h + slowest : null,
        op, targetDp, rangePct: ratio * 100, moraleDelta,
        populationFreed: casualties.reduce((a, n) => a + n, 0),
        manualOverride: Array.isArray(ev.casualtiesOverride), boatsSent: Math.floor(boatUnits / boatCapacity()),
        appliedEffects: [],
      });
    }
    if (outcomes.length) eventsByHour.set(h, outcomes);
    // Snapshot the POST-instant-action state (A_H): production has NOT landed yet, so it shows
    // in the NEXT row. Carries hour h's actions + the entering-wallet `enter` fields (for the log).
    {
      const row = snapshot(h); row.enter = enter;
      // Echo invest-all actions with their resolved amounts (a mapped copy — never mutate the plan).
      if (resolvedAll.length) {
        let k = 0;
        row.actions = row.actions.map((a) => {
          if (!(a.type === "improve" && a.all === true)) return a;
          const amt = resolvedAll[k++] ?? 0, key = Object.keys(a.data || {})[0];
          return { ...a, data: key ? { [key]: amt } : {}, amount: amt };
        });
      }
      rows.push(row);
    }

    // production
    plat += r((b.alchemy * 45 + employed() * 2.7) * (1 + scienceBonus() + (spells.midas_touch > 0 ? 0.1 : 0)));
    lumber += b.lumberyard * 50 + b.forest_haven * 25 - r(lumber * 0.01);
    mana += r((b.tower * 25 + b.wizard_guild * 5) * (1 + spiresBonus())) - r(mana * 0.02);
    ore += r(b.ore_mine * 60 * (spells.mining_strength > 0 ? 1.1 : 1));
    gems += b.diamond_mine * 15;
    tech += techPerHr();
    boats += b.dock / 20 * (1 + harborBoatBonus());
    food += r(b.farm * 80 * (1 + harborBonus() + (spells.gaias_watch > 0 ? 0.1 : 0))) - r((peasants + draftees + mil.u1 + mil.u2 + mil.u3 + mil.u4 + away().total) * 0.25) - r(food * 0.01);
    food = Math.max(0, food);

    // growth (temples drive births; draft-rate gates draftee growth)
    const totalPop = peasants + draftees + mil.u1 + mil.u2 + mil.u3 + mil.u4 + away().total;
    const milPct = totalPop > 0 ? ((draftees + mil.u1 + mil.u2 + mil.u3 + mil.u4 + away().total) / totalPop) * 100 : 0;
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
  currentHour = HOURS + 1;
  for (const q of queue.filter((q) => q.arrive === currentHour)) {
    if (q.kind === "build") b[q.building] += q.n;
    else if (q.kind === "land") {
      land[q.land || "plain"] += q.n;
      if (q.discounted) discountedLand += q.n;
    }
    else if (q.kind === "unit" || q.kind === "return") mil["u" + q.slot] += q.n;
    else if (q.kind === "espionage") mil[q.unit] += q.n;
    else if (q.kind === "prestige") prestige += q.n;
    else if (q.kind === "boats") boats += q.n;
  }
  for (let i = queue.length - 1; i >= 0; i--) if (queue[i].arrive === currentHour) queue.splice(i, 1);
  { const endRow = snapshot(currentHour); endRow.enter = { mana, dailyPlatinum: dailyPlat, dailyLand: dailyLand, peasants, discountedLand }; rows.push(endRow); }

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
  return {
    ruleset: { ...RULESET, productionOverrides: { ...RULESET.productionOverrides } },
    rows,
    final,
  };
}
