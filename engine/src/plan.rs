//! Scenario runner — mirrors the PHP oracle's emit flow:
//! create start state -> place opening build -> building-phase tick -> for each
//! tick: apply actions then advance one protection tick. Produces one state
//! snapshot per oracle step so output can be diffed against the golden vectors.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use crate::calc;
use crate::config;
use crate::data;
use crate::rounding::rceil;
use crate::state::{DominionState, QueueEntry};

#[derive(Deserialize, Debug, Default, Clone)]
pub struct Scenario {
    pub race: String,
    #[serde(rename = "protectionType", default)]
    pub protection_type: String,
    #[serde(rename = "openingBuild", default)]
    pub opening_build: HashMap<String, i64>,
    #[serde(default)]
    pub ticks: Vec<Vec<Value>>,
    #[serde(rename = "oopActions", default)]
    pub oop_actions: Vec<Value>,
    /// Post-OOP economic hours (49..N), each a list of actions, run with `post_oop_tick`
    /// after the OOP boundary (Phase 1: economy only, no combat). Default-empty ⇒ every
    /// existing caller (app / feasibility / golden JSON) is byte-identical.
    #[serde(rename = "postOopTicks", default)]
    pub post_oop_ticks: Vec<Vec<Value>>,
    #[serde(rename = "daysLate", default)]
    pub days_late: i64,
}

pub fn start_state(sc: &Scenario) -> DominionState {
    config::start_state(&sc.protection_type, sc.days_late, &sc.race)
}

/// Run a scenario, returning one snapshot per oracle step.
pub fn run(sc: &Scenario) -> Vec<DominionState> {
    let mut s = start_state(sc);
    let mut steps = Vec::new();

    steps.push(s.clone()); // "created"

    apply_starting_buildings(&mut s, &sc.opening_build);
    steps.push(s.clone()); // "opening_build"

    // Building-phase tick: decrement only, no production (per getTickDominion).
    if s.protection_ticks_remaining > s.protection_ticks {
        s.protection_ticks_remaining -= 1;
    }
    steps.push(s.clone()); // "building_phase_done"

    for actions in &sc.ticks {
        for a in actions {
            apply_action(&mut s, a);
        }
        protection_tick(&mut s);
        steps.push(s.clone());
    }

    // OOP boundary: the moment you leave protection — apply the OOP cast (e.g. Ares) and
    // snapshot. This is the OOP headline / row-49 state. Pushed when there are OOP actions
    // OR a post-OOP window to follow; with no post-OOP window the condition reduces to the
    // old `!oop_actions.is_empty()`, so existing callers are byte-identical.
    if !sc.oop_actions.is_empty() || !sc.post_oop_ticks.is_empty() {
        // Out of protection from here on. The oracle flips protection_finished at the final
        // protection tick (remaining→0) for advanced dominions; we set it at the boundary,
        // which is observationally identical (protection_finished is cast/display-only — never
        // a compared golden FIELD, and calc/tick never read it). This gates `invalid_protection`
        // racial spells: refused under protection, castable now (e.g. Undead Death and Decay).
        s.protection_finished = true;
        for a in &sc.oop_actions {
            apply_action(&mut s, a);
        }
        steps.push(s.clone());
    }

    // Post-OOP economic hours (49..N): the same per-hour economy as protection (no combat
    // this phase), via `post_oop_tick`. Inert for every existing caller (default-empty).
    for actions in &sc.post_oop_ticks {
        for a in actions {
            apply_action(&mut s, a);
        }
        post_oop_tick(&mut s);
        steps.push(s.clone());
    }
    steps
}

// ---------------------------------------------------------------------------
// OVERTURE plan { race, opening, hours, oopActions }  ->  engine Scenario.
// Single source of truth shared by the desktop importer (app/src-tauri/src/main.rs)
// and the headless replay path, so both build byte-identical scenarios.
// ---------------------------------------------------------------------------

fn one_kv(key: String, val: Value) -> Value {
    let mut m = serde_json::Map::new();
    m.insert(key, val);
    Value::Object(m)
}

fn overture_home_land_type(race: &str) -> String {
    data::get()
        .races
        .get(race)
        .map(|r| r.home_land_type.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "plain".to_string())
}

/// Reshape one OVERTURE shorthand action (the editor/import form) into the engine
/// `apply_action` shape. Mirrors the desktop importer's `reshape_action`.
pub fn reshape_overture_action(a: &Value, home: &str) -> Value {
    use serde_json::json;
    let t = a.get("type").and_then(Value::as_str).unwrap_or("");
    let n = a.get("n").and_then(Value::as_i64).unwrap_or(0);
    match t {
        "construct" => {
            let b = a.get("building").and_then(Value::as_str).unwrap_or("home");
            json!({ "type": "construct", "data": one_kv(format!("building_{b}"), json!(n)) })
        }
        "rezone" => {
            let from = a.get("from").and_then(Value::as_str).unwrap_or("plain");
            let to = a.get("to").and_then(Value::as_str).unwrap_or("plain");
            let mut top = serde_json::Map::new();
            top.insert("type".into(), json!("rezone"));
            top.insert("remove".into(), one_kv(from.to_string(), json!(n)));
            top.insert("add".into(), one_kv(to.to_string(), json!(n)));
            Value::Object(top)
        }
        "explore" => {
            let lt = a
                .get("land")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or(home);
            json!({ "type": "explore", "data": one_kv(format!("land_{lt}"), json!(n)) })
        }
        "train" => {
            if let Some(kind) = a.get("slot").and_then(Value::as_str) {
                json!({ "type": "train", "data": one_kv(format!("military_{kind}"), json!(n)) })
            } else {
                let slot = a.get("slot").and_then(Value::as_i64).unwrap_or(2);
                json!({ "type": "train", "data": one_kv(format!("military_unit{slot}"), json!(n)) })
            }
        }
        "destroy" => {
            let b = a.get("building").and_then(Value::as_str).unwrap_or("home");
            json!({ "type": "destroy", "data": one_kv(format!("building_{b}"), json!(n)) })
        }
        "release" => {
            let key = if let Some(u) = a.get("unit").and_then(Value::as_str) {
                if u == "draftees" {
                    "draftees".to_string()
                } else {
                    format!("military_{u}")
                }
            } else if let Some(slot) = a.get("slot").and_then(Value::as_i64) {
                format!("military_unit{slot}")
            } else {
                "draftees".to_string()
            };
            json!({ "type": "release", "data": one_kv(key, json!(n)) })
        }
        // spell / bank / claim_platinum / claim_land / draft_rate / improve /
        // research are already in engine shape.
        _ => a.clone(),
    }
}

/// Build an engine `Scenario` JSON from an OVERTURE plan. `hours[0..48)` are the
/// protection ticks; `hours[48..]` are the post-OOP economy hours; `oopActions`
/// is the OOP-boundary cast applied between them.
pub fn scenario_value_from_overture_plan(plan_in: &Value) -> Value {
    use serde_json::json;
    let race = plan_in
        .get("race")
        .and_then(Value::as_str)
        .unwrap_or("human")
        .to_string();
    let home = overture_home_land_type(&race);
    let opening = plan_in.get("opening").cloned().unwrap_or_else(|| json!({}));
    let hours = plan_in
        .get("hours")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let all_hours: Vec<Value> = hours
        .iter()
        .map(|hour| {
            let acts = hour.as_array().cloned().unwrap_or_default();
            Value::Array(acts.iter().map(|a| reshape_overture_action(a, &home)).collect())
        })
        .collect();
    const PROTECTION_HOURS: usize = 48;
    let split = all_hours.len().min(PROTECTION_HOURS);
    let ticks: Vec<Value> = all_hours[..split].to_vec();
    let post_oop_ticks: Vec<Value> =
        all_hours.get(split..).map(<[_]>::to_vec).unwrap_or_default();
    let oop_actions: Vec<Value> = plan_in
        .get("oopActions")
        .or_else(|| plan_in.get("oop_actions"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|a| reshape_overture_action(a, &home))
        .collect();
    json!({
        "race": race,
        "protectionType": "advanced",
        "openingBuild": opening,
        "ticks": Value::Array(ticks),
        "oopActions": Value::Array(oop_actions),
        "postOopTicks": Value::Array(post_oop_ticks),
        "daysLate": plan_in.get("daysLate").and_then(Value::as_i64).unwrap_or(0),
    })
}

fn protection_tick(s: &mut DominionState) {
    s.protection_ticks_remaining -= 1;
    if s.protection_ticks_remaining == 0
        || (s.protection_ticks_remaining == 24 && s.protection_type != "quick")
    {
        s.daily_platinum = false;
        s.daily_land = false;
    }
    *s = crate::tick::tick(s);
}

/// One post-protection hour (Phase 1: economy only, no combat). Mirrors `protection_tick`
/// but for the post-OOP clock: the real game keeps resetting the daily plat/land bonus on
/// the round-start wall-clock hour, which is the arithmetic continuation of the protection
/// resets — protection lands them at game-hours 24 and 48 (`protection_ticks_remaining` 24
/// and 0), so post-OOP they continue at 72 and 96, i.e. `remaining % 24 == 0`. The counter
/// decrements past 0 here purely as that 24h clock (matches the oracle, which never clamps
/// it post-protection). `tick::tick` has no protection gating, so the economy continues
/// oracle-exactly. `protection_tick` is intentionally left untouched (golden invariance).
fn post_oop_tick(s: &mut DominionState) {
    s.protection_finished = true;
    s.protection_ticks_remaining -= 1; // intentionally unclamped — the 24h daily-bonus clock
    if s.protection_ticks_remaining % 24 == 0 {
        s.daily_platinum = false;
        s.daily_land = false;
    }
    *s = crate::tick::tick(s);
}

pub fn apply_action(s: &mut DominionState, a: &Value) {
    match a["type"].as_str().unwrap_or("") {
        "explore" => {
            let plat_per = calc::explore_platinum_cost(s);
            let draftee_per = calc::explore_draftee_cost(s);
            let mut total = 0i64;
            if let Some(data) = a["data"].as_object() {
                for (k, v) in data {
                    let n = v.as_i64().unwrap_or(0);
                    total += n;
                    s.queue.push(QueueEntry {
                        source: "exploration".into(),
                        resource: k.clone(),
                        hours: 12,
                        amount: n,
                    });
                }
            }
            s.resource_platinum -= plat_per * total;
            s.military_draftees -= draftee_per * total;
            let drop = (1).max(crate::rounding::rfloor((total as f64 + 2.0) / 3.0));
            s.morale -= drop.min(s.morale);
        }
        "construct" => {
            let plat_per = calc::construct_platinum_cost(s);
            let lumber_per = calc::construct_lumber_cost(s);
            let mut total = 0i64;
            if let Some(data) = a["data"].as_object() {
                for (k, v) in data {
                    let n = v.as_i64().unwrap_or(0);
                    total += n;
                    s.queue.push(QueueEntry {
                        source: "construction".into(),
                        resource: k.clone(),
                        hours: 12,
                        amount: n,
                    });
                }
            }
            s.resource_platinum -= plat_per * total;
            s.resource_lumber -= lumber_per * total;
        }
        "rezone" => {
            let plat_per = calc::rezone_platinum_cost(s);
            let mut total = 0i64;
            if let Some(remove) = a["remove"].as_object() {
                for (k, v) in remove {
                    let n = v.as_i64().unwrap_or(0);
                    total += n;
                    add_land(s, k, -n);
                }
            }
            if let Some(add) = a["add"].as_object() {
                for (k, v) in add {
                    add_land(s, k, v.as_i64().unwrap_or(0));
                }
            }
            s.resource_platinum -= plat_per * total;
        }
        "train" => {
            if let Some(data) = a["data"].as_object() {
                for (k, v) in data {
                    let n = v.as_i64().unwrap_or(0);
                    let unit = k.strip_prefix("military_").unwrap_or(k);
                    // not_trainable units (e.g. Planewalker's summoned slots) can't be trained.
                    if let Some(slot) = unit
                        .strip_prefix("unit")
                        .and_then(|d| d.parse::<usize>().ok())
                    {
                        if !calc::unit_trainable(s, slot) {
                            continue;
                        }
                    }
                    let (costs, draftees, hours) = unit_train_cost(s, unit);
                    for &(res, per) in &costs {
                        add_resource_bare(s, res, -per * n);
                    }
                    s.military_draftees -= draftees * n;
                    s.queue.push(QueueEntry {
                        source: "training".into(),
                        resource: k.clone(),
                        hours,
                        amount: n,
                    });
                }
            }
        }
        "spell" => {
            if let Some(key) = a["spell"].as_str() {
                // LIVE self-spells this race may cast right now (data-driven: common
                // Harmony/Midas/… + racial Miner's Sight / Alchemist Frost / Howling). Casting
                // is context-aware: `invalid_protection` spells (Undead-rework's Death and Decay,
                // Dark-Elf's Spellwright's Calling) are refused UNDER protection but become
                // castable once `protection_finished` (post-OOP). Mirrors SpellActionService.
                if data::spell_castable_in_context(key, &s.race, s.protection_finished) {
                    let land = s.total_land();
                    let mana_cost =
                        crate::rounding::round_int(data::spell_cost_mana(key) * land as f64);
                    if s.resource_mana >= mana_cost {
                        s.resource_mana -= mana_cost;
                        let dur = data::spell_duration(key);
                        if let Some(sp) = s.spells.iter_mut().find(|sp| sp.key == key) {
                            sp.duration = dur;
                        } else {
                            s.spells.push(crate::state::ActiveSpell {
                                key: key.to_string(),
                                duration: dur,
                            });
                        }
                    }
                }
            }
        }
        "improve" => {
            let resource = a["resource"].as_str().unwrap_or("");
            let worth = match resource {
                "platinum" => 1,
                "lumber" | "ore" => 2,
                "gems" => 12,
                _ => 0,
            };
            let have = match resource {
                "platinum" => s.resource_platinum,
                "lumber" => s.resource_lumber,
                "ore" => s.resource_ore,
                "gems" => s.resource_gems,
                _ => 0,
            };
            if worth > 0 {
                if let Some(data) = a["data"].as_object() {
                    let total: i64 = data.values().filter_map(|v| v.as_i64()).sum();
                    if total > 0 && total <= have {
                        for (typ, v) in data {
                            let amount = v.as_i64().unwrap_or(0);
                            let mult = calc::investment_multiplier(s, resource, typ);
                            let points =
                                crate::rounding::rfloor(amount as f64 * worth as f64 * mult);
                            add_improvement(s, typ, points);
                        }
                        match resource {
                            "platinum" => s.resource_platinum -= total,
                            "lumber" => s.resource_lumber -= total,
                            "ore" => s.resource_ore -= total,
                            "gems" => s.resource_gems -= total,
                            _ => {}
                        }
                    }
                }
            }
        }
        "bank" => {
            let source = a["source"].as_str().unwrap_or("");
            let target = a["target"].as_str().unwrap_or("");
            let amount = a["amount"].as_i64().unwrap_or(0);
            let amt = amount.min(resource_get(s, source)).max(0);
            if amt > 0 {
                let gained =
                    crate::rounding::rfloor(amt as f64 * bank_sell(source) * bank_buy(target));
                resource_set(s, source, resource_get(s, source) - amt);
                resource_set(s, target, resource_get(s, target) + gained);
            }
        }
        "draft_rate" => {
            if let Some(r) = a["rate"].as_i64() {
                s.draft_rate = r;
            }
        }
        // Raze buildings -> barren land. Instant, free (refund/discount only with
        // destruction techs/heroes = none here); 15%-DP-loss cap is skipped under
        // protection. DestroyActionService.php.
        "destroy" => {
            if let Some(data) = a["data"].as_object() {
                for (k, v) in data {
                    let b = k.strip_prefix("building_").unwrap_or(k);
                    add_building(s, b, -v.as_i64().unwrap_or(0));
                }
            }
        }
        // Release troops: units -> draftees, draftees -> peasants. Instant, free;
        // 15%-DP-loss cap skipped under protection. ReleaseActionService.php.
        "release" => {
            if let Some(data) = a["data"].as_object() {
                for (k, v) in data {
                    let n = v.as_i64().unwrap_or(0);
                    let unit = k.strip_prefix("military_").unwrap_or(k);
                    if unit == "draftees" {
                        s.military_draftees -= n;
                        s.peasants += n;
                    } else {
                        add_military(s, unit, -n);
                        s.military_draftees += n;
                    }
                }
            }
        }
        "research" => {
            if let Some(key) = a["tech"].as_str() {
                let cost = crate::calc::tech_cost(s);
                let already = s.techs.iter().any(|t| t == key);
                if !already
                    && s.resource_tech >= cost
                    && crate::data::tech_prereqs_met(key, &s.techs)
                {
                    s.resource_tech -= cost;
                    s.techs.push(key.to_string());
                }
            }
        }
        "claim_platinum" => {
            s.resource_platinum += s.peasants * 4;
            s.resource_tech += 350;
            s.daily_platinum = true;
        }
        "claim_land" => {
            let home = home_land(s); // race's home land type (DailyBonusesActionService)
            add_land(s, &home, 20);
            s.daily_land = true;
        }
        other => {
            eprintln!("apply_action: unhandled {other}");
        }
    }
}

/// Per-unit training cost for ANY race (data-driven). Returns (resource costs as
/// (name, per-unit amount), draftees, queue_hours). Mirrors round-50
/// TrainingCalculator::getTrainingCostsPerUnit + TrainActionService:
///   • each resource present in the race's unit data (platinum/ore/mana/lumber/gems)
///     is scaled by the specialist/elite cost multiplier (smithy reduction; the
///     elite-only military_cost spell perk is 0 under protection, so proficiency
///     doesn't matter here) — EXCEPT gnome ore, which is never reduced;
///   • +1 draftee per unit;
///   • train time is SLOT-based, not proficiency-based: slots 1–2 → 9h, 3–4 → 12h
///     (TrainActionService hardcodes military_unit1/2 to the 9-hour bucket).
/// For Human this is identical to the prior hard-coded human_unit_cost.
fn unit_train_cost(s: &DominionState, unit: &str) -> (Vec<(&'static str, i64)>, i64, i64) {
    if let Some(slot) = unit
        .strip_prefix("unit")
        .and_then(|d| d.parse::<usize>().ok())
    {
        let hours = if slot <= 2 { 9 } else { 12 };
        return (calc::unit_training_costs(s, slot), 1, hours);
    }
    // spies / wizards: base 500 platinum × ops cost multiplier, +1 draftee, 12h.
    // (Not exposed in the app's train tab, but kept for parity.)
    let sp = calc::spy_cost_multiplier(s);
    match unit {
        "spies" | "wizards" => (vec![("platinum", rceil(500.0 * sp))], 1, 12),
        _ => (vec![], 0, 12),
    }
}

/// Add `delta` to a bare-named resource (platinum/ore/mana/lumber/gems).
fn add_resource_bare(s: &mut DominionState, res: &str, delta: i64) {
    match res {
        "platinum" => s.resource_platinum += delta,
        "ore" => s.resource_ore += delta,
        "mana" => s.resource_mana += delta,
        "lumber" => s.resource_lumber += delta,
        "gems" => s.resource_gems += delta,
        _ => {}
    }
}

fn bank_sell(k: &str) -> f64 {
    match k {
        "resource_platinum" | "resource_lumber" | "resource_ore" => 0.5,
        "resource_gems" => 2.0,
        _ => 0.0,
    }
}
fn bank_buy(k: &str) -> f64 {
    match k {
        "resource_platinum" | "resource_lumber" | "resource_ore" => 1.0,
        "resource_food" => 0.5,
        _ => 0.0,
    }
}
fn resource_get(s: &DominionState, k: &str) -> i64 {
    match k {
        "resource_platinum" => s.resource_platinum,
        "resource_lumber" => s.resource_lumber,
        "resource_ore" => s.resource_ore,
        "resource_gems" => s.resource_gems,
        "resource_food" => s.resource_food,
        "resource_mana" => s.resource_mana,
        _ => 0,
    }
}
fn resource_set(s: &mut DominionState, k: &str, v: i64) {
    match k {
        "resource_platinum" => s.resource_platinum = v,
        "resource_lumber" => s.resource_lumber = v,
        "resource_ore" => s.resource_ore = v,
        "resource_gems" => s.resource_gems = v,
        "resource_food" => s.resource_food = v,
        "resource_mana" => s.resource_mana = v,
        _ => {}
    }
}

fn add_improvement(s: &mut DominionState, typ: &str, points: i64) {
    match typ {
        "science" => s.improvement_science += points,
        "keep" => s.improvement_keep += points,
        "spires" => s.improvement_spires += points,
        "forges" => s.improvement_forges += points,
        "walls" => s.improvement_walls += points,
        "harbor" => s.improvement_harbor += points,
        _ => {}
    }
}

fn add_military(s: &mut DominionState, unit: &str, n: i64) {
    match unit {
        "unit1" => s.military_unit1 += n,
        "unit2" => s.military_unit2 += n,
        "unit3" => s.military_unit3 += n,
        "unit4" => s.military_unit4 += n,
        "spies" => s.military_spies += n,
        "assassins" => s.military_assassins += n,
        "wizards" => s.military_wizards += n,
        "archmages" => s.military_archmages += n,
        "draftees" => s.military_draftees += n,
        _ => {}
    }
}

fn add_land(s: &mut DominionState, t: &str, n: i64) {
    match t {
        "plain" => s.land_plain += n,
        "mountain" => s.land_mountain += n,
        "swamp" => s.land_swamp += n,
        "cavern" => s.land_cavern += n,
        "forest" => s.land_forest += n,
        "hill" => s.land_hill += n,
        "water" => s.land_water += n,
        _ => {}
    }
}

fn clear_land(s: &mut DominionState) {
    s.land_plain = 0;
    s.land_mountain = 0;
    s.land_swamp = 0;
    s.land_cavern = 0;
    s.land_forest = 0;
    s.land_hill = 0;
    s.land_water = 0;
}

fn home_land(s: &DominionState) -> String {
    data::get()
        .races
        .get(&s.race)
        .map(|race| race.home_land_type.clone())
        .filter(|land| !land.is_empty())
        .unwrap_or_else(|| "plain".to_string())
}

fn building_land(s: &DominionState, b: &str) -> String {
    if b == "home" {
        return home_land(s);
    }
    match b {
        "alchemy" | "farm" | "smithy" | "masonry" => "plain",
        "tower" | "wizard_guild" | "temple" => "swamp",
        "ore_mine" | "gryphon_nest" => "mountain",
        "guard_tower" | "factory" | "shrine" | "barracks" => "hill",
        "lumberyard" | "forest_haven" => "forest",
        "diamond_mine" | "school" => "cavern",
        "dock" => "water",
        _ => "plain",
    }
    .to_string()
}

/// The building types the engine models (round-50 roster). Keys may appear with or
/// without a `building_` prefix in opening builds; both forms are accepted. Used to
/// validate imported openings and to harden `apply_starting_buildings` against unknown
/// keys — which would otherwise mint barren land with no building behind it.
pub const KNOWN_BUILDINGS: [&str; 19] = [
    "home",
    "alchemy",
    "farm",
    "smithy",
    "masonry",
    "ore_mine",
    "gryphon_nest",
    "tower",
    "wizard_guild",
    "temple",
    "diamond_mine",
    "school",
    "lumberyard",
    "forest_haven",
    "factory",
    "guard_tower",
    "shrine",
    "barracks",
    "dock",
];

/// True if `b` (with or without a `building_` prefix) is a building the engine models.
pub fn is_known_building(b: &str) -> bool {
    KNOWN_BUILDINGS.contains(&b.trim_start_matches("building_"))
}

/// Validate an imported opening build against the dominion's starting land. Returns
/// `Some(message)` describing the first problem found, or `None` if the opening is
/// legal. Rejects unknown building keys and negative counts, and rejects a total that
/// exceeds `start_land` (which would conjure land out of thin air). A total *under*
/// `start_land` is legal — the engine auto-fills the remainder with homes.
pub fn opening_build_error(opening: &HashMap<String, i64>, start_land: i64) -> Option<String> {
    let mut total = 0i64;
    for (building, count) in opening {
        if !is_known_building(building) {
            return Some(format!("unknown building in opening build: \"{building}\""));
        }
        if *count < 0 {
            return Some(format!("negative count for {building} in opening build: {count}"));
        }
        total += *count;
    }
    if total > start_land {
        return Some(format!(
            "opening build totals {total} acres but the dominion starts with only {start_land}"
        ));
    }
    None
}

fn apply_starting_buildings(s: &mut DominionState, opening: &HashMap<String, i64>) {
    let total_land = calc::total_land(s);
    // Drop unknown keys before summing: an unknown key would otherwise inflate `specified`
    // (under-filling homes) AND mint barren land in the loop below. The import boundary
    // already rejects them (`opening_build_error`); this is defense-in-depth for callers
    // that bypass it. Unknown acreage falls through to the home fill.
    let mut counts = opening
        .iter()
        .filter(|(building, _)| is_known_building(building))
        .map(|(building, count)| (building.trim_start_matches("building_").to_string(), *count))
        .collect::<HashMap<_, _>>();
    let specified = counts.values().copied().sum::<i64>();
    if specified < total_land {
        *counts.entry("home".to_string()).or_default() += total_land - specified;
    }

    clear_land(s);
    for (building, count) in counts {
        let n = count.max(0);
        if n == 0 {
            continue;
        }
        add_building(s, &building, n);
        add_land(s, &building_land(s, &building), n);
    }
    s.peasants = calc::max_peasant_population(s);
}

fn add_building(s: &mut DominionState, b: &str, c: i64) {
    match b {
        "home" => s.building_home += c,
        "alchemy" => s.building_alchemy += c,
        "farm" => s.building_farm += c,
        "smithy" => s.building_smithy += c,
        "masonry" => s.building_masonry += c,
        "ore_mine" => s.building_ore_mine += c,
        "gryphon_nest" => s.building_gryphon_nest += c,
        "tower" => s.building_tower += c,
        "wizard_guild" => s.building_wizard_guild += c,
        "temple" => s.building_temple += c,
        "diamond_mine" => s.building_diamond_mine += c,
        "school" => s.building_school += c,
        "lumberyard" => s.building_lumberyard += c,
        "forest_haven" => s.building_forest_haven += c,
        "factory" => s.building_factory += c,
        "guard_tower" => s.building_guard_tower += c,
        "shrine" => s.building_shrine += c,
        "barracks" => s.building_barracks += c,
        "dock" => s.building_dock += c,
        _ => {}
    }
}
