//! Two-dominion combat golden vectors (from the PHP oracle's invade emitter). Builds an
//! attacker + target from the scenario's attribute blocks and asserts the engine's
//! combat.rs reproduces the oracle's MilitaryCalculator/RangeCalculator outputs.

use engine::state::DominionState;
use engine::{calc, combat};
use serde_json::Value;
use std::fs;

/// Build a DominionState from a combat-scenario block (race + land/buildings/military/
/// draftees/improvements/prestige/morale). Mirrors the oracle's invade dominion-builder.
fn build(block: &Value) -> DominionState {
    let mut s = DominionState::default();
    s.race = block["race"].as_str().unwrap_or("human").to_lowercase();
    let n = |m: &Value, group: &str, key: &str| -> i64 {
        m.get(group)
            .and_then(|g| g.get(key))
            .and_then(Value::as_i64)
            .unwrap_or(0)
    };
    s.land_plain = n(block, "land", "plain");
    s.land_mountain = n(block, "land", "mountain");
    s.land_swamp = n(block, "land", "swamp");
    s.land_cavern = n(block, "land", "cavern");
    s.land_forest = n(block, "land", "forest");
    s.land_hill = n(block, "land", "hill");
    s.land_water = n(block, "land", "water");
    // buildings
    s.building_home = n(block, "buildings", "home");
    s.building_alchemy = n(block, "buildings", "alchemy");
    s.building_farm = n(block, "buildings", "farm");
    s.building_smithy = n(block, "buildings", "smithy");
    s.building_masonry = n(block, "buildings", "masonry");
    s.building_ore_mine = n(block, "buildings", "ore_mine");
    s.building_gryphon_nest = n(block, "buildings", "gryphon_nest");
    s.building_tower = n(block, "buildings", "tower");
    s.building_wizard_guild = n(block, "buildings", "wizard_guild");
    s.building_temple = n(block, "buildings", "temple");
    s.building_diamond_mine = n(block, "buildings", "diamond_mine");
    s.building_school = n(block, "buildings", "school");
    s.building_lumberyard = n(block, "buildings", "lumberyard");
    s.building_forest_haven = n(block, "buildings", "forest_haven");
    s.building_factory = n(block, "buildings", "factory");
    s.building_guard_tower = n(block, "buildings", "guard_tower");
    s.building_shrine = n(block, "buildings", "shrine");
    s.building_barracks = n(block, "buildings", "barracks");
    s.building_dock = n(block, "buildings", "dock");
    // military
    s.military_unit1 = n(block, "military", "unit1");
    s.military_unit2 = n(block, "military", "unit2");
    s.military_unit3 = n(block, "military", "unit3");
    s.military_unit4 = n(block, "military", "unit4");
    s.military_draftees = block.get("draftees").and_then(Value::as_i64).unwrap_or(0);
    s.military_wizards = block.get("wizards").and_then(Value::as_i64).unwrap_or(0);
    s.military_archmages = block.get("archmages").and_then(Value::as_i64).unwrap_or(0);
    // improvements
    s.improvement_science = n(block, "improvements", "science");
    s.improvement_keep = n(block, "improvements", "keep");
    s.improvement_spires = n(block, "improvements", "spires");
    s.improvement_forges = n(block, "improvements", "forges");
    s.improvement_walls = n(block, "improvements", "walls");
    s.improvement_harbor = n(block, "improvements", "harbor");
    s.prestige = block.get("prestige").and_then(Value::as_i64).unwrap_or(250);
    s.morale = block.get("morale").and_then(Value::as_i64).unwrap_or(100);
    s
}

fn check(file: &str) {
    let path = format!("tests/golden/combat/{file}");
    let txt = fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing combat vector {path}"));
    let v: Value = serde_json::from_str(&txt).unwrap();
    let attacker = build(&v["attacker"]);
    let target = build(&v["target"]);
    let c = &v["computed"];
    let (al, tl) = (calc::total_land(&attacker), calc::total_land(&target));

    assert_eq!(
        combat::dominion_range(al, tl),
        c["range"].as_f64().unwrap(),
        "{file}: range"
    );
    assert_eq!(
        combat::in_range(al, tl),
        c["in_range"].as_bool().unwrap(),
        "{file}: in_range"
    );

    let g_op = c["op"].as_f64().unwrap();
    // combat OP (unitsSent = all) — includes the landRatio-dependent staggered perk.
    let e_op = combat::offensive_power_combat(&attacker, &target);
    assert!(
        (e_op - g_op).abs() < 0.5,
        "{file}: op engine={e_op} golden={g_op}"
    );

    let g_tr = c["temple_reduction"].as_f64().unwrap();
    assert!(
        (combat::temple_reduction(&attacker) - g_tr).abs() < 1e-9,
        "{file}: temple_reduction"
    );

    let g_dp = c["dp_with_temples"].as_f64().unwrap();
    let e_dp = combat::defensive_power_with_temples(&target, &attacker);
    assert!(
        (e_dp - g_dp).abs() < 0.5,
        "{file}: dp_with_temples engine={e_dp} golden={g_dp}"
    );

    assert_eq!(
        combat::land_lost(al, tl),
        c["land_lost"].as_i64().unwrap(),
        "{file}: land_lost"
    );
    assert_eq!(
        combat::invasion_successful(&attacker, &target),
        c["success"].as_bool().unwrap(),
        "{file}: success"
    );

    if let Some(g) = c.get("prestige_gain").and_then(Value::as_i64) {
        assert_eq!(
            combat::prestige_gain(&attacker, &target),
            g,
            "{file}: prestige_gain"
        );
        assert_eq!(
            combat::prestige_loss(&target, g),
            c["prestige_loss"].as_i64().unwrap(),
            "{file}: prestige_loss"
        );
    }
    if let Some(p) = c.get("prestige_penalty").and_then(Value::as_i64) {
        // the oracle's factory target carries a user_id → real-target scaling applies
        assert_eq!(
            combat::prestige_penalty(&attacker, &target, true),
            p,
            "{file}: prestige_penalty"
        );
    }

    // Casualties (real protected handlers driven via reflection in the oracle).
    let units = [
        v["unitsSent"]["1"].as_i64().unwrap_or(0),
        v["unitsSent"]["2"].as_i64().unwrap_or(0),
        v["unitsSent"]["3"].as_i64().unwrap_or(0),
        v["unitsSent"]["4"].as_i64().unwrap_or(0),
    ];
    if let Some(aul) = c.get("attacker_units_lost") {
        let e = combat::offensive_casualties(&attacker, &target, units);
        // the supplied-OP/DP variant the intel layer calls must match the wrapper exactly
        // (fed the wrapper's own op/dp), across every golden vector.
        let op = combat::offensive_power_combat(&attacker, &target);
        let dp = combat::defensive_power_with_temples(&target, &attacker);
        assert_eq!(
            combat::offensive_casualties_given(&attacker, &target, units, op, dp),
            e,
            "{file}: offensive_casualties_given != offensive_casualties"
        );
        for slot in 1..=4 {
            let g = aul
                .get(slot.to_string())
                .and_then(Value::as_i64)
                .unwrap_or(0);
            assert_eq!(
                e[slot - 1],
                g,
                "{file}: attacker u{slot} lost engine={} golden={g}",
                e[slot - 1]
            );
        }
    }
    if let Some(dul) = c.get("defender_units_lost") {
        let (draftees, e) = combat::defensive_casualties(&attacker, &target);
        let op = combat::offensive_power_combat(&attacker, &target);
        let dp = combat::defensive_power_with_temples(&target, &attacker);
        assert_eq!(
            combat::defensive_casualties_given(&attacker, &target, op, dp),
            (draftees, e),
            "{file}: defensive_casualties_given != defensive_casualties"
        );
        let gd = dul.get("draftees").and_then(Value::as_i64).unwrap_or(0);
        assert_eq!(
            draftees, gd,
            "{file}: defender draftees lost engine={draftees} golden={gd}"
        );
        for slot in 1..=4 {
            let g = dul
                .get(slot.to_string())
                .and_then(Value::as_i64)
                .unwrap_or(0);
            assert_eq!(
                e[slot - 1],
                g,
                "{file}: defender u{slot} lost engine={} golden={g}",
                e[slot - 1]
            );
        }
    }
    if let Some(cu) = c.get("converted_units") {
        let e = combat::conversions(&attacker, &target, units);
        for slot in 1..=4 {
            let g = cu
                .get(slot.to_string())
                .and_then(Value::as_i64)
                .unwrap_or(0);
            assert_eq!(
                e[slot - 1],
                g,
                "{file}: converted u{slot} engine={} golden={g}",
                e[slot - 1]
            );
        }
    }
}

#[test]
fn combat_basic() {
    check("combat_basic.json");
}

#[test]
fn combat_fail() {
    check("combat_fail.json"); // OP < DP, not overwhelmed → failure-path casualties
}

#[test]
fn combat_overwhelmed() {
    check("combat_overwhelmed.json"); // OP ≪ DP → overwhelmed, defender takes 0 casualties
}

// Race casualty-perk coverage (per-slot multipliers), validated against the real handlers.
#[test]
fn combat_human_knights() {
    check("combat_human_knights.json"); // `casualties` -25 on Knights, both offense + defense
}
#[test]
fn combat_undead_immortal() {
    check("combat_undead_immortal.json"); // immortal Vampires take 0 defensive casualties
}
#[test]
fn combat_gnome_fixed() {
    check("combat_gnome_fixed.json"); // fixed_casualties (50%) bypasses the normal formula
}
#[test]
fn combat_firewalker_off() {
    check("combat_firewalker_off.json"); // casualties_offense -50 halves attacker losses
}
#[test]
fn combat_icekin_def() {
    check("combat_icekin_def.json"); // casualties_defense -50 halves defender losses
}
#[test]
fn combat_orc_prestige() {
    check("combat_orc_prestige.json"); // offense_from_prestige adds OP (target-less perk)
}
#[test]
fn combat_darkelf_on() {
    // range ≥75%: staggered_land_range adds +0.5 OP/unit AND immortal_vs_land_range
    // makes the attackers take zero offensive casualties.
    check("combat_darkelf_on.json");
}
#[test]
fn combat_darkelf_off() {
    check("combat_darkelf_off.json"); // range <75%: both perks switch off → normal OP + casualties
}
// Conversion (enemy casualties → attacker units) for the conversion races, + the
// faithfulness detail that spirit has the perk yet does NOT convert (race-list guard).
#[test]
fn combat_lycan_convert() {
    check("combat_lycan_convert.json");
}
#[test]
fn combat_undead_convert() {
    check("combat_undead_convert.json");
}
#[test]
fn combat_vampire_convert() {
    check("combat_vampire_convert.json");
}
#[test]
fn combat_spirit_noconvert() {
    check("combat_spirit_noconvert.json"); // has the perk, but not on the convert list → 0
}
#[test]
fn combat_icekin_wizard() {
    check("combat_icekin_wizard.json"); // offense_raw_wizard_ratio: +min(wizards/land, max) OP
}
