// OVERTURE — Tauri backend.
//
// This layer is a *thin adapter*. It imports the bit-exact `engine` crate as a
// read-only dependency and translates between the app's own plan/row JSON shapes
// and the engine's public API. It must NEVER reimplement or alter game logic:
// every number it returns comes straight from `engine::plan::run` + `engine::calc::*`
// (the single source of truth, validated against the PHP oracle). If a value looks
// wrong, the fix belongs in the engine + its golden vectors, not here.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// The per-hour row is one large `json!({...})`; the added caps/employment keys push
// the macro past serde_json's default 128-deep expansion. Lift the limit for this crate.
#![recursion_limit = "256"]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};
use tauri::Manager; // path resolver (app.path()) for the per-user storage dir

use engine::rounding::{rceil, round_int};
use engine::state::DominionState;
use engine::{calc, config, data, plan};

// ---------------------------------------------------------------------------
// app plan  ->  engine Scenario
// ---------------------------------------------------------------------------

/// The race's home land type (what `explore`/`claim_land` produce). Data-driven;
/// falls back to plain for a race that has not declared one.
fn home_land_type(race: &str) -> String {
    data::get()
        .races
        .get(race)
        .map(|r| r.home_land_type.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "plain".to_string())
}

// `reshape_action` + `build_scenario` now live in engine::plan
// (scenario_value_from_overture_plan / reshape_overture_action) so the desktop
// importer and the headless replay path build byte-identical scenarios.

// ---------------------------------------------------------------------------
// engine DominionState  ->  app row JSON
// ---------------------------------------------------------------------------

/// Land queued from exploration (the "incoming" acres not yet on-hand).
fn incoming_land(s: &DominionState) -> i64 {
    s.queue
        .iter()
        .filter(|q| q.source == "exploration")
        .map(|q| q.amount)
        .sum()
}

/// Trained defensive power, DRAFTEES EXCLUDED (per spec: a race spell negates
/// them). On-hand units only — training queue is excluded, matching
/// `calc::defensive_power_raw`'s semantics, minus the draftee term.
fn trained_raw(s: &DominionState) -> f64 {
    // Per-unit defense incl. land/building perks + flat pairing bonus (race-generic,
    // matching calc::defensive_power_raw minus the draftee term).
    s.military_unit1 as f64 * calc::unit_defense_modified(s, 1)
        + s.military_unit2 as f64 * calc::unit_defense_modified(s, 2)
        + s.military_unit3 as f64 * calc::unit_defense_modified(s, 3)
        + s.military_unit4 as f64 * calc::unit_defense_modified(s, 4)
        + calc::pairing_defense_bonus(s)
}

/// (name, count) for all 19 buildings — single source for the maps below.
fn buildings_list(s: &DominionState) -> [(&'static str, i64); 19] {
    [
        ("home", s.building_home),
        ("alchemy", s.building_alchemy),
        ("farm", s.building_farm),
        ("smithy", s.building_smithy),
        ("masonry", s.building_masonry),
        ("tower", s.building_tower),
        ("temple", s.building_temple),
        ("guard_tower", s.building_guard_tower),
        ("ore_mine", s.building_ore_mine),
        ("lumberyard", s.building_lumberyard),
        ("wizard_guild", s.building_wizard_guild),
        ("gryphon_nest", s.building_gryphon_nest),
        ("diamond_mine", s.building_diamond_mine),
        ("school", s.building_school),
        ("forest_haven", s.building_forest_haven),
        ("factory", s.building_factory),
        ("shrine", s.building_shrine),
        ("barracks", s.building_barracks),
        ("dock", s.building_dock),
    ]
}

fn buildings_json(s: &DominionState) -> Value {
    let mut m = Map::new();
    for (name, count) in buildings_list(s) {
        m.insert(name.to_string(), json!(count));
    }
    Value::Object(m)
}

/// Land type a building must sit on (round-50 canonical map).
fn building_land(b: &str) -> &'static str {
    match b {
        "tower" | "wizard_guild" | "temple" => "swamp",
        "ore_mine" | "gryphon_nest" => "mountain",
        "guard_tower" | "factory" | "shrine" | "barracks" => "hill",
        "lumberyard" | "forest_haven" => "forest",
        "diamond_mine" | "school" => "cavern",
        "dock" => "water",
        _ => "plain",
    }
}

fn building_land_for_home(b: &str, home: &str) -> String {
    if b == "home" {
        home.to_string()
    } else {
        building_land(b).to_string()
    }
}

const LAND_TYPES: [&str; 7] = [
    "plain", "swamp", "hill", "mountain", "forest", "cavern", "water",
];

fn land_of(s: &DominionState, t: &str) -> i64 {
    match t {
        "plain" => s.land_plain,
        "swamp" => s.land_swamp,
        "hill" => s.land_hill,
        "mountain" => s.land_mountain,
        "forest" => s.land_forest,
        "cavern" => s.land_cavern,
        "water" => s.land_water,
        _ => 0,
    }
}

/// The opening build places any building free + instant, but `plan::run` leaves the
/// engine's land all home-type (it never rezones for the opening). So we reconstruct
/// a constant per-type land offset — each opening building's acre is moved off the
/// home type onto its own land type — so the per-type land display + the editor's
/// build/rezone availability stay consistent with the auto-zoned model. The engine's
/// production / growth / defense are land-type-agnostic, so they're already correct.
fn opening_land_offset(_plan_in: &Value, _home: &str) -> HashMap<String, i64> {
    // No offset. The engine's apply_starting_buildings already places opening buildings
    // on their real land types (temple→swamp, ore_mine→mountain, …), so plan::run returns
    // correct per-type land. Adding an offset on top double-counts — it would show negative
    // plain and inflated swamp/hill/mountain in landBy/freeLandByType (and mis-gate the
    // editor's builds/rezones). land_recon therefore reads the engine land directly.
    HashMap::new()
}

fn land_recon(s: &DominionState, t: &str, off: &HashMap<String, i64>) -> i64 {
    land_of(s, t) + off.get(t).copied().unwrap_or(0)
}

/// Barren (buildable) land per type = land − built − constructing, the budget the
/// editor uses to block over-construction / over-rezoning.
fn free_land_by_type(s: &DominionState, off: &HashMap<String, i64>) -> Value {
    let mut m = Map::new();
    let home = home_land_type(&s.race);
    for t in LAND_TYPES {
        let built: i64 = buildings_list(s)
            .iter()
            .filter(|(n, _)| building_land_for_home(n, &home) == t)
            .map(|(_, c)| c)
            .sum();
        let constructing: i64 = s
            .queue
            .iter()
            .filter(|q| q.source == "construction")
            .filter(|q| {
                building_land_for_home(
                    q.resource.strip_prefix("building_").unwrap_or(&q.resource),
                    &home,
                ) == t
            })
            .map(|q| q.amount)
            .sum();
        m.insert(
            t.to_string(),
            json!(land_recon(s, t, off) - built - constructing),
        );
    }
    Value::Object(m)
}

/// Exact per-unit action costs at this state — the engine's own calculators. The row is
/// rendered from the POST-instant-action state (A_H), so these costs already reflect this
/// tick's claim_land (+20 acres ⇒ a higher land-scaled rezone/construct/explore cost): a
/// NEW action the editor prices off them is the next one in line, applied after everything
/// already queued this hour, so Σ(count × unit_cost) ≤ the remaining wallet is exact even
/// across a same-tick claim → rezone → build chain.
fn costs_json(s: &DominionState) -> Value {
    let land = s.total_land() as f64;
    // Per-unit training cost, data-driven for every race (mirrors plan.rs::unit_train_cost
    // + TrainingCalculator): every resource present in the race's unit data, scaled by the
    // smithy/elite cost multiplier — EXCEPT gnome ore, which is never reduced. Only nonzero
    // entries are emitted; keys are wallet resource names so the editor gates generically.
    let train = |slot: usize| {
        let mut t = Map::new();
        for (res, amount) in calc::unit_training_costs(s, slot) {
            t.insert(res.to_string(), json!(amount));
        }
        Value::Object(t)
    };
    // Spies & wizards: base 500 platinum × the spy/wizard cost multiplier + 1 draftee
    // (TrainingCalculator). Both use the spy multiplier in the engine (parity); for
    // perk-less races they coincide. Assassins/archmages aren't modeled by the engine
    // (their spy/wizard pairing cost isn't ported), so they're intentionally omitted.
    let spy_wizard = json!({ "platinum": rceil(500.0 * calc::spy_cost_multiplier(s)) });
    json!({
        "explorePlat": calc::explore_platinum_cost(s),
        "exploreDraftee": calc::explore_draftee_cost(s),
        "constructPlat": calc::construct_platinum_cost(s),
        "constructLumber": calc::construct_lumber_cost(s),
        "rezonePlat": calc::rezone_platinum_cost(s),
        "techCost": calc::tech_cost(s),
        "train": {
            "1": train(1), "2": train(2), "3": train(3), "4": train(4),
            "spies": spy_wizard, "wizards": spy_wizard,
        },
        "spell": spell_costs(&s.race, land, s.protection_finished),
    })
}

/// Mana cost of every self-spell THIS race can cast AT THIS ROW (data-driven: common spells +
/// the race's racial self-spells; mana = round(cost_mana × land)). Castability is context-aware
/// on the row's `protection_finished`, so an `invalid_protection` racial spell (Undead-rework's
/// Death and Decay, Dark-Elf's Spellwright's Calling) is absent during protection and present
/// post-OOP — which is exactly what gates the editor's Magic tab per-hour.
fn spell_costs(race: &str, land: f64, protection_finished: bool) -> Value {
    let mut m = Map::new();
    for (key, sp) in &data::get().spells {
        if data::spell_castable_in_context(key, race, protection_finished) {
            m.insert(key.clone(), json!(round_int(sp.cost_mana * land)));
        }
    }
    Value::Object(m)
}

/// Capped ratio-of-land buildings: how many more fit before the bonus saturates.
/// `capCount = floor(cap_ratio × land)`; a negative `count − capCount` ⇒ over-cap
/// (slots that add nothing). Cap ratios + max % come from the engine (single source).
fn caps_json(s: &DominionState) -> Value {
    let entry = |count: i64, cap_count: i64, cur: Option<f64>, max: Option<f64>| -> Value {
        json!({ "count": count, "capCount": cap_count, "curPct": cur, "maxPct": max })
    };
    json!({
        "guard_tower": entry(s.building_guard_tower, calc::guard_tower_cap_count(s),
            Some(calc::guard_tower_bonus(s) * 100.0), Some(config::GUARD_TOWER_DP_MAX * 100.0)),
        "smithy": entry(s.building_smithy, calc::smithy_cap_count(s),
            Some(calc::smithy_reduction(s) * 100.0), Some(config::SMITHY_DISCOUNT_MAX * 100.0)),
        "factory": entry(s.building_factory, calc::factory_cap_count(s),
            Some(calc::factory_reduction(s) * 100.0), Some(config::FACTORY_DISCOUNT_MAX * 100.0)),
        "school": entry(s.building_school, calc::school_cap_count(s), None, None),
        "gryphon_nest": entry(s.building_gryphon_nest, calc::gryphon_nest_cap_count(s),
            Some(calc::gryphon_nest_bonus(s) * 100.0), Some(config::GRYPHON_NEST_OP_MAX * 100.0)),
    })
}

/// Employment balance: current jobs/employed plus the (perk-adjusted) per-building
/// housing/jobs constants, so the app can prescribe how many homes/barracks vs job
/// buildings would balance work against housing.
fn employment_json(s: &DominionState) -> Value {
    json!({
        "jobs": calc::employment_jobs(s),
        "employed": calc::population_employed(s),
        "peasants": s.peasants,
        "maxPeasantPop": calc::max_peasant_population(s),
        "populationMilitary": calc::population_military(s),
        "jobsPerBuilding": calc::jobs_per_building(),
        "housingPerHome": calc::housing_per_home(s),
        "housingPerNonhome": calc::housing_per_nonhome(),
        "barracksMilitaryHousing": calc::barracks_military_housing(s),
    })
}

/// Map one engine state to the app's per-hour row (field names match `mock.js`,
/// so the engine and the preview mock are interchangeable behind `bridge.js`).
fn row_json(s: &DominionState, hour: i64, actions: Value, off: &HashMap<String, i64>) -> Value {
    let mult = calc::defensive_power_multiplier(s) * calc::morale_multiplier(s);
    let traw = trained_raw(s);
    // Offensive power (target-less base), symmetric with the trained-DP fields.
    let op_mult = calc::offensive_power_multiplier(s) * calc::morale_multiplier(s);
    let op_raw = calc::offensive_power_raw(s);
    let spells: Vec<Value> = s
        .spells
        .iter()
        .filter(|sp| sp.duration > 0)
        .map(|sp| json!({ "key": sp.key, "dur": sp.duration }))
        .collect();
    json!({
        "hour": hour,
        "rem": 48 - hour,
        "land": s.total_land(),
        "landBy": {
            "plain": land_recon(s, "plain", off), "swamp": land_recon(s, "swamp", off),
            "hill": land_recon(s, "hill", off), "mountain": land_recon(s, "mountain", off),
            "forest": land_recon(s, "forest", off), "cavern": land_recon(s, "cavern", off),
            "water": land_recon(s, "water", off),
        },
        "incoming": incoming_land(s),
        "peasants": s.peasants,
        "draftees": s.military_draftees,
        "maxPop": calc::max_population(s),
        "employed": calc::population_employed(s),
        "jobs": calc::employment_jobs(s),
        "platinum": s.resource_platinum,
        "food": s.resource_food,
        "lumber": s.resource_lumber,
        "ore": s.resource_ore,
        "mana": s.resource_mana,
        "gems": s.resource_gems,
        "boats": s.resource_boats,
        "platPerHr": calc::platinum_production(s),
        "foodNet": calc::food_net_change(s),
        "lumberPerHr": calc::lumber_production(s),
        "manaPerHr": calc::mana_production(s),
        "orePerHr": calc::ore_production(s),
        "gemPerHr": calc::gem_production(s),
        "techPerHr": calc::tech_production(s),
        "boatsPerHr": calc::boat_production(s),
        "trainedRaw": traw,
        "trainedModded": traw * mult,
        "mult": mult,
        "trainedOpRaw": op_raw,
        "trainedOpModded": op_raw * op_mult,
        "opMult": op_mult,
        "morale": s.morale,
        "tech": s.resource_tech,
        "barren": calc::total_barren_land(s),
        "freeLandByType": free_land_by_type(s, off),
        "dailyPlatinum": s.daily_platinum,
        "dailyLand": s.daily_land,
        "draftRate": s.draft_rate,
        "techs": s.techs.clone(),
        "costs": costs_json(s),
        "caps": caps_json(s),
        "employment": employment_json(s),
        "buildings": buildings_json(s),
        "military": {
            "u1": s.military_unit1, "u2": s.military_unit2,
            "u3": s.military_unit3, "u4": s.military_unit4,
            "draftees": s.military_draftees,
            "spies": s.military_spies, "assassins": s.military_assassins,
            "wizards": s.military_wizards, "archmages": s.military_archmages,
        },
        "spells": spells,
        "actions": actions,
    })
}

// ---------------------------------------------------------------------------
// commands
// ---------------------------------------------------------------------------

/// Run the bit-exact engine over the app's plan and return one row per hour
/// (0..=48) plus the out-of-protection `final` summary. Microsecond-cheap, so the
/// frontend re-invokes it on every edit for zero-latency live updates.
#[tauri::command]
fn simulate(plan: Value) -> Result<Value, String> {
    let race = plan
        .get("race")
        .and_then(Value::as_str)
        .unwrap_or("human")
        .to_string();
    let dp_target = plan.get("dpTarget").and_then(Value::as_f64).unwrap_or(0.0);

    let scenario_v = plan::scenario_value_from_overture_plan(&plan);
    let sc: plan::Scenario =
        serde_json::from_value(scenario_v).map_err(|e| format!("scenario build failed: {e}"))?;

    // Reject malformed opening builds at the import boundary: unknown building keys,
    // negative counts, or a total exceeding starting land (which would mint acres).
    // The engine also drops unknown keys defensively, but failing visibly here keeps a
    // corrupt/hand-edited save from silently simulating a different dominion.
    let start_land = calc::total_land(&plan::start_state(&sc));
    if let Some(err) = plan::opening_build_error(&sc.opening_build, start_land) {
        return Err(err);
    }
    // Reject illegal per-hour / OOP actions (negative or non-integer counts, unknown
    // buildings/land/race) — the action-stream companion to the opening check above. A
    // hand-edited import with a negative count would otherwise mint resources and corrupt
    // the queues. Resource OVERSPEND is intentionally NOT rejected here (it's the honest
    // negative-balance display the editor relies on).
    if let Some(err) = plan::overture_plan_error(&plan) {
        return Err(err);
    }

    // states = [created, opening_build, building_phase_done, protection ticks 1..P,
    // (OOP-boundary snapshot), post-OOP ticks]. state_idx(H) is the dominion ENTERING hour H
    // (the wallet you act FROM that hour, pre-that-hour's production — the game is act-then-tick;
    // see OracleEmitCommand). Entering hour H == end of hour H-1. For protection (H ≤ P) that's
    // states[BASE + H-1] (hour 0 and hour 1 both enter from building_phase_done = states[BASE]).
    // At the OOP boundary the engine inserts ONE extra snapshot (post-protection + the OOP cast,
    // e.g. Ares) — the headline OOP state, shown as row P+1 (hour 49) — so rows at/after OOP carry
    // a +1 offset (states[BASE + H]). Post-OOP hours (P+2..) are the economy continuing past OOP
    // (Phase 1: no combat). The displayed ROW is then the entering state with hour H's INSTANT
    // actions replayed on top (A_H) — see the row loop below — so instant effects land in their
    // own tick; only production defers to row H+1.
    let states = plan::run(&sc);
    if states.len() < 3 {
        return Err("engine produced too few states".into());
    }
    const BASE: usize = 2;
    let prot = sc.ticks.len();
    // The engine inserts an extra OOP-BOUNDARY snapshot only when there's an OOP cast or a
    // post-OOP window (`has_oop`). But hour P+1 (OOP) is ALWAYS surfaced as a row — the default
    // view is "through OOP" with NO post-OOP planning — because even without that extra snapshot
    // the OOP state already exists as the last protection tick's result (entering hour P+1 =
    // states.last()). So: with has_oop we show every state past BASE (incl. the boundary snapshot
    // + post-OOP rows); without it we extend exactly one row past protection to that OOP state.
    let has_oop = !sc.oop_actions.is_empty() || !sc.post_oop_ticks.is_empty();
    let last_hour = if has_oop {
        states.len().saturating_sub(BASE + 1)
    } else {
        (prot + 1).min(states.len().saturating_sub(BASE))
    };
    // Row H -> engine state index (entering hour H). The OOP-boundary snapshot (has_oop) adds a
    // +1 offset for hours past P; without it, hour P+1 maps to the last protection tick (which IS
    // entering hour P+1) via the normal BASE+H-1 mapping.
    let state_idx = |h: usize| -> usize {
        if has_oop && h > prot {
            BASE + h
        } else {
            BASE + h.saturating_sub(1)
        }
    };
    let hours_in = plan
        .get("hours")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let off = opening_land_offset(&plan, &home_land_type(&race));

    let mut rows = Vec::with_capacity(last_hour + 1);
    for h in 0..=last_hour {
        // Row H shows the dominion DURING hour H *after its instant actions* (A_H). states[idx]
        // is the entering wallet E_H (pre-action); we clone it and replay hour H's actions
        // through the engine's own apply_action, so the row reflects this tick's INSTANT effects
        // immediately — spell mana spent from the current pool, daily plat/land claimed, rezones,
        // and the platinum/lumber *payments* for queued builds/explores/training. Crucially the
        // land-scaled costs (rezone/construct/explore) are re-priced AFTER a same-tick claim_land
        // (apply_action processes actions in order at the current land size), so claim → rezone →
        // build all settle at the new, higher land size in one tick. Only PRODUCTION is deferred:
        // it lands at the tick, i.e. in row H+1 (this row's platPerHr is exactly that amount).
        // (This mirrors the engine's own in-tick order — apply_action×N THEN tick — snapshotting
        // the pre-tick point run() itself skips; plan.rs/golden are untouched.)
        let entering = &states[state_idx(h)];
        let mut disp = entering.clone();
        // Engine-shaped actions for hour H (already reshaped in build_scenario): protection hours
        // are sc.ticks[h-1]; the OOP hour (P+1) and post-OOP hours are sc.post_oop_ticks[h-P-1].
        let acts_h: &[Value] = if h == 0 {
            &[]
        } else if h <= prot {
            sc.ticks.get(h - 1).map(Vec::as_slice).unwrap_or(&[])
        } else {
            sc.post_oop_ticks
                .get(h - prot - 1)
                .map(Vec::as_slice)
                .unwrap_or(&[])
        };
        for a in acts_h {
            plan::apply_action(&mut disp, a);
        }
        // app-shaped actions ride on the row they're taken from (the queued-action display); none
        // beyond the planned hours (e.g. the trailing post-OOP end row).
        let actions = if h >= 1 {
            hours_in.get(h - 1).cloned().unwrap_or_else(|| json!([]))
        } else {
            json!([])
        };
        let mut row = row_json(&disp, h as i64, actions, &off);
        // The log exporter re-derives each hour's actions from the ENTERING wallet (E_H) — it
        // gates self-spells on the entering mana and skips already-claimed dailies — so it needs
        // the pre-action mana / daily-claim flags / peasants that the A_H display state no longer
        // carries. Everything else (ledger, inspector, charts, editor wallet, feasibility) reads
        // the A_H row directly.
        if let Some(obj) = row.as_object_mut() {
            obj.insert(
                "enter".into(),
                json!({
                    "mana": entering.resource_mana,
                    "dailyPlatinum": entering.daily_platinum,
                    "dailyLand": entering.daily_land,
                    "peasants": entering.peasants,
                }),
            );
        }
        rows.push(row);
    }

    // `final` = the OOP headline state (the moment you leave protection — hour P+1, always). With
    // an OOP cast / post-OOP window it's the OOP-boundary snapshot; without one it's the last
    // protection tick (= entering hour P+1). Either way it equals row P+1, so the headline and
    // the OOP ledger row agree.
    let oop_hour = prot + 1;
    let oop_idx = if has_oop { BASE + prot + 1 } else { states.len() - 1 };
    let last = row_json(&states[oop_idx], oop_hour as i64, json!([]), &off);
    let tmod = last
        .get("trainedModded")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let land = last.get("land").and_then(Value::as_i64).unwrap_or(0);
    let incoming = last.get("incoming").and_then(Value::as_i64).unwrap_or(0);
    let committed = land + incoming;
    // The explicit trained-DP target is the app's sole defensive threshold — at
    // protection land sizes it dominates the in-game leave-gate (10·land−3250), so
    // surfacing the gate is noise. The engine keeps `min_defense` (golden-validated)
    // for attacker/full-round sims; it is simply not used here. (See CLAUDE.md.)
    let feasible = tmod >= dp_target;

    let mut final_obj = last.as_object().cloned().unwrap();
    final_obj.insert("race".into(), json!(race));
    final_obj.insert("committed".into(), json!(committed));
    final_obj.insert("feasible".into(), json!(feasible));
    final_obj.insert("dpTarget".into(), json!(dp_target));
    final_obj.insert("targetShort".into(), json!((dp_target - tmod).max(0.0)));

    Ok(json!({ "rows": rows, "final": Value::Object(final_obj) }))
}

/// Race keys for the LIVE round-50 roster (21 races; Human first). Drives the
/// race picker. = source-`playable` AND not admin-disabled in data/round50.json.
#[tauri::command]
fn races() -> Vec<String> {
    // The previous filter here used a `!contains("rework") && !contains("legacy")`
    // string heuristic that was INVERTED for reworked races — it HID the live
    // `*-rework` races (e.g. undead-rework = Crypt Lords) and SHOWED the dead
    // classics (undead = Progeny, playable:false). See engine::data::round50_live_keys.
    let mut v = data::round50_live_keys();
    v.sort();
    v.sort_by_key(|k| (k != "human", k.clone())); // human pinned first
    v
}

/// Static, race-specific reference the editor needs for labels: trained unit
/// names/DP/base cost, the tech tree, and the building→land map. Pulled straight
/// from the engine's data layer.
#[tauri::command]
fn meta(race: String) -> Value {
    let d = data::get();
    let units: Vec<Value> = d
        .races
        .get(&race)
        .map(|r| {
            r.units
                .iter()
                .enumerate()
                .map(|(i, u)| {
                    json!({
                        "slot": i + 1,
                        "name": u.name,
                        "defense": u.power.get("defense").copied().unwrap_or(0.0),
                        "offense": u.power.get("offense").copied().unwrap_or(0.0),
                        "plat": u.cost.get("platinum").copied().unwrap_or(0),
                        "ore": u.cost.get("ore").copied().unwrap_or(0),
                        "kind": if i < 2 { "specialist" } else { "elite" },
                        // not_trainable units (e.g. Planewalker's summoned slots) are hidden
                        // from the Train tab — the game can't train them either.
                        "trainable": !u.perks.contains_key("not_trainable"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let mut techs: Vec<Value> = d
        .techs
        .iter()
        .map(|(k, t)| json!({ "key": k, "name": t.name, "x": t.x, "y": t.y, "perks": t.perks, "requires": t.requires }))
        .collect();
    techs.sort_by(|a, b| a["key"].as_str().cmp(&b["key"].as_str()));
    // Round-50 LIVE buildable set (BuildingHelper::getBuildingTypes). forest_haven is
    // commented out in round-50 → dead code → excluded. The app offers only what the
    // current round actually allows. (Keep this list in sync with the live ruleset.)
    let home = home_land_type(&race);
    let building_land: Map<String, Value> = [
        "home",
        "alchemy",
        "farm",
        "smithy",
        "masonry",
        "tower",
        "temple",
        "guard_tower",
        "ore_mine",
        "lumberyard",
        "wizard_guild",
        "gryphon_nest",
        "diamond_mine",
        "school",
        "factory",
        "shrine",
        "barracks",
        "dock",
    ]
    .iter()
    .map(|b| (b.to_string(), json!(building_land_for_home(b, &home))))
    .collect();
    // Self-spells THIS race can cast (common + racial), data-driven. We list the full
    // out-of-protection set (`spell_castable_in_context(.., true)`) so the Magic tab knows the
    // names/costs of `invalid_protection` racial spells (Undead-rework's Death and Decay,
    // Dark-Elf's Spellwright's Calling) too — the per-row `costs.spell` gate (which IS
    // protection-aware) then only offers them at post-OOP hours. `invalidProtection` flags those
    // so the editor can label them "out of protection only".
    let mut spells: Vec<Value> = d
        .spells
        .iter()
        .filter(|(k, _)| data::spell_castable_in_context(k, &race, true))
        .map(|(k, sp)| {
            json!({
                "key": k,
                "name": sp.name,
                "costMana": sp.cost_mana,
                "desc": spell_effect_desc(&sp.perks),
                "invalidProtection": sp.perks.get("invalid_protection").copied().unwrap_or(0.0) != 0.0,
            })
        })
        .collect();
    // Stable order: by mana cost then key (common low-cost economy spells first).
    spells.sort_by(|a, b| {
        a["costMana"]
            .as_f64()
            .partial_cmp(&b["costMana"].as_f64())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a["key"].as_str().cmp(&b["key"].as_str()))
    });
    // Which non-universal resource this race actually USES — drives per-race column visibility in the
    // app. Only ORE is conditional: it has NO universal sink (it's purely a unit-training input), so a
    // race whose units never cost ore has no use for it. platinum / food / lumber (construction) / mana
    // (spells) / gems (diamond mines, which everyone builds) all stay on. Data-driven from unit costs.
    let resources = json!({
        "ore": engine::race_resources::race_has_training_resource(&race, "ore"),
    });
    json!({ "units": units, "techs": techs, "buildingLand": building_land, "homeLand": home, "spells": spells, "resources": resources })
}

/// Short human label for a self-spell's protection-relevant effect (economy/defense),
/// derived from its perks; empty if it has no protection effect.
fn spell_effect_desc(perks: &HashMap<String, f64>) -> String {
    let label = |p: &str| -> Option<&'static str> {
        Some(match p {
            "platinum_production" => "platinum",
            "ore_production" => "ore",
            "lumber_production" => "lumber",
            "food_production" => "food",
            "mana_production" => "mana",
            "gem_production" => "gems",
            "population_growth" => "population growth",
            "defense" => "defense",
            "max_population" => "housing",
            _ => return None,
        })
    };
    let mut parts: Vec<String> = Vec::new();
    for (p, v) in perks {
        if let Some(name) = label(p) {
            let sign = if *v >= 0.0 { "+" } else { "" };
            parts.push(format!("{sign}{}% {name}", v));
        }
    }
    parts.sort();
    parts.join(" · ")
}

// ---------------------------------------------------------------------------
// build storage + autosave — a user-visible folder tree under <Documents>/OVERTURE
//
// `saves/`     named builds the user explicitly saves (one *.overture.json each).
// `autosaves/` a rolling ring buffer written automatically while editing (newest N kept),
//              read back on launch to offer "restore last session" / crash recovery.
//
// The on-disk format is the SAME {overture, savedAt, plan} wrapper the export/import file
// uses, so files are interchangeable. The core fns take a base `dir` (no Tauri/clock deps)
// so they can be unit-tested against a temp dir; the #[tauri::command] wrappers resolve the
// real per-user dir and stamp the time.
// ---------------------------------------------------------------------------

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Sanitize a user-supplied save name into a safe single-segment filename stem.
fn safe_stem(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim().to_string();
    if cleaned.is_empty() { "build".to_string() } else { cleaned }
}

fn save_payload(plan: &Value, stamp: u64) -> Value {
    json!({ "overture": 1, "savedAt": stamp, "plan": plan })
}

fn is_autosave_file(p: &Path) -> bool {
    p.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("autosave-") && n.ends_with(".overture.json"))
}

/// Write a named build into `dir`, returning its path. Creates `dir` if missing; overwrites
/// a same-named build (the name is the identity, so re-saving updates in place).
fn write_named_save(dir: &Path, name: &str, plan: &Value, stamp: u64) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.overture.json", safe_stem(name)));
    std::fs::write(&path, serde_json::to_vec_pretty(&save_payload(plan, stamp)).unwrap_or_default())?;
    Ok(path)
}

/// List every saved build in `dir` (newest first) with light metadata for the library UI.
fn list_named_saves(dir: &Path) -> Vec<Value> {
    let mut out: Vec<(u64, Value)> = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") || is_autosave_file(&path) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        let plan = v.get("plan").cloned().unwrap_or(Value::Null);
        let saved_at = v.get("savedAt").and_then(|s| s.as_u64()).unwrap_or(0);
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("build")
            .trim_end_matches(".overture.json")
            .to_string();
        out.push((
            saved_at,
            json!({
                "name": name,
                "path": path.to_string_lossy(),
                "savedAt": saved_at,
                "race": plan.get("race").and_then(|r| r.as_str()).unwrap_or(""),
                "dpTarget": plan.get("dpTarget").and_then(|d| d.as_i64()).unwrap_or(0),
            }),
        ));
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.into_iter().map(|(_, v)| v).collect()
}

/// Write a rolling autosave into `dir`, then prune to the newest `keep` files.
fn write_autosave(dir: &Path, plan: &Value, stamp: u64, keep: usize) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("autosave-{stamp:013}.overture.json"));
    std::fs::write(&path, serde_json::to_vec(&save_payload(plan, stamp)).unwrap_or_default())?;
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| is_autosave_file(p)).collect())
        .unwrap_or_default();
    files.sort(); // zero-padded stamp ⇒ lexicographic == chronological
    if files.len() > keep {
        for old in &files[..files.len() - keep] {
            let _ = std::fs::remove_file(old);
        }
    }
    Ok(path)
}

/// The most-recent autosave's payload ({overture, savedAt, plan}), or null if none.
fn read_latest_autosave(dir: &Path) -> Value {
    let mut files: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.flatten().map(|e| e.path()).filter(|p| is_autosave_file(p)).collect(),
        Err(_) => return Value::Null,
    };
    files.sort();
    let Some(latest) = files.last() else { return Value::Null };
    std::fs::read_to_string(latest)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .unwrap_or(Value::Null)
}

/// `<Documents>/OVERTURE` — the user-visible storage root (auto-created on first write).
fn overture_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let docs = app
        .path()
        .document_dir()
        .map_err(|e| format!("could not resolve the Documents directory: {e}"))?;
    Ok(docs.join("OVERTURE"))
}

#[tauri::command]
fn save_build(app: tauri::AppHandle, name: String, plan: Value) -> Result<String, String> {
    let dir = overture_root(&app)?.join("saves");
    write_named_save(&dir, &name, &plan, now_millis())
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| format!("save failed: {e}"))
}

#[tauri::command]
fn list_saves(app: tauri::AppHandle) -> Result<Value, String> {
    let dir = overture_root(&app)?.join("saves");
    Ok(Value::Array(list_named_saves(&dir)))
}

/// Resolve a save NAME to its file strictly under `<root>/saves`. `safe_stem` neutralizes
/// path separators and `..`, so a hostile name can never escape the saves directory — the
/// commands take a name, not a caller-supplied path.
fn save_path_for(app: &tauri::AppHandle, name: &str) -> Result<PathBuf, String> {
    Ok(overture_root(app)?
        .join("saves")
        .join(format!("{}.overture.json", safe_stem(name))))
}

#[tauri::command]
fn load_build(app: tauri::AppHandle, name: String) -> Result<Value, String> {
    let path = save_path_for(&app, &name)?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("read failed: {e}"))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse failed: {e}"))?;
    // the app applies a PLAN, not the {overture,savedAt,plan} wrapper — unwrap it.
    Ok(v.get("plan").cloned().unwrap_or(v))
}

#[tauri::command]
fn delete_save(app: tauri::AppHandle, name: String) -> Result<(), String> {
    // Resolve the name under OVERTURE/saves (never an arbitrary path) and delete it.
    // Missing file ⇒ already gone, treat as success (idempotent).
    let path = save_path_for(&app, &name)?;
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(&path).map_err(|e| format!("delete failed: {e}"))
}

#[tauri::command]
fn autosave(app: tauri::AppHandle, plan: Value) -> Result<(), String> {
    let dir = overture_root(&app)?.join("autosaves");
    write_autosave(&dir, &plan, now_millis(), 12)
        .map(|_| ())
        .map_err(|e| format!("autosave failed: {e}"))
}

#[tauri::command]
fn latest_autosave(app: tauri::AppHandle) -> Result<Value, String> {
    let dir = overture_root(&app)?.join("autosaves");
    Ok(read_latest_autosave(&dir))
}

/// Which optional features this build includes. The open-source build ships the
/// simulator only, so this always reports `swarm: false` — a stable probe the frontend
/// can read without invoking a command that isn't registered.
#[tauri::command]
fn capabilities() -> Value {
    json!({ "swarm": false })
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            simulate, races, meta, capabilities,
            save_build, list_saves, load_build, delete_save, autosave, latest_autosave
        ])
        .run(tauri::generate_context!())
        .expect("error while running OVERTURE");
}

#[cfg(test)]
mod save_tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("overture-savetest-{}-{}", tag, now_millis()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn save_sanitizes_name_and_round_trips() {
        let dir = tmp("saves");
        let plan = json!({ "race": "human", "dpTarget": 6000, "hours": [[]] });
        let p = write_named_save(&dir, "My Build! / r50", &plan, 1000).unwrap();
        assert!(p.exists());
        // path separators + punctuation collapse to underscores; stays a single file
        assert_eq!(p.file_name().unwrap().to_str().unwrap(), "My Build_ _ r50.overture.json");
        let saves = list_named_saves(&dir);
        assert_eq!(saves.len(), 1);
        assert_eq!(saves[0]["race"], "human");
        assert_eq!(saves[0]["dpTarget"], 6000);
        assert_eq!(saves[0]["savedAt"], 1000);
        // re-saving the same name overwrites in place (no duplicate)
        write_named_save(&dir, "My Build! / r50", &plan, 2000).unwrap();
        assert_eq!(list_named_saves(&dir).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn autosave_keeps_only_newest_and_reads_latest() {
        let dir = tmp("autosaves");
        let plan = json!({ "race": "merfolk", "hours": [] });
        for stamp in 1..=20u64 {
            write_autosave(&dir, &plan, stamp, 5).unwrap();
        }
        let count = std::fs::read_dir(&dir).unwrap().filter(|e| e.is_ok()).count();
        assert_eq!(count, 5, "ring buffer should keep exactly the newest 5");
        let latest = read_latest_autosave(&dir);
        assert_eq!(latest["savedAt"], 20);
        assert_eq!(latest["plan"]["race"], "merfolk");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn latest_autosave_is_null_when_empty() {
        assert!(read_latest_autosave(&tmp("none")).is_null());
    }

    #[test]
    fn list_saves_ignores_autosave_files() {
        let dir = tmp("mixed");
        write_named_save(&dir, "real", &json!({"race":"orc"}), 5).unwrap();
        write_autosave(&dir, &json!({"race":"orc"}), 6, 12).unwrap(); // same dir, should be skipped
        let saves = list_named_saves(&dir);
        assert_eq!(saves.len(), 1);
        assert_eq!(saves[0]["name"], "real");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn safe_stem_neutralizes_path_traversal() {
        // load_build/delete_save resolve `<root>/saves/{safe_stem(name)}.overture.json`, so the
        // ONLY thing standing between a caller-supplied name and an arbitrary file is safe_stem:
        // no separator or parent-dir token may survive — the stem is always one filename segment.
        for hostile in ["../../etc/passwd", "..", "/etc/shadow", "a/b/c", "..\\..\\win", "...."] {
            let s = safe_stem(hostile);
            assert!(!s.contains('/'), "{hostile:?} -> {s:?} leaked a forward slash");
            assert!(!s.contains('\\'), "{hostile:?} -> {s:?} leaked a backslash");
            assert_ne!(s, "..", "{hostile:?} produced a parent-dir token");
            assert!(!s.is_empty());
        }
        assert_eq!(safe_stem(""), "build");
        assert_eq!(safe_stem("   "), "build");
        // a normal name round-trips unchanged (so list→load by name is stable)
        assert_eq!(safe_stem("human build 1"), "human build 1");
    }
}
