//! Bit-exact validation: run each scenario through the Rust engine and diff every
//! tracked field against the PHP oracle's golden vectors (engine/tests/golden/*.json).

use std::fs;

use engine::plan::{run, Scenario};

fn as_i64(v: &serde_json::Value) -> i64 {
    match v {
        serde_json::Value::Number(n) => n
            .as_i64()
            .unwrap_or_else(|| n.as_f64().unwrap_or(0.0) as i64),
        serde_json::Value::String(s) => s.parse::<f64>().unwrap_or(0.0) as i64,
        serde_json::Value::Bool(b) => *b as i64,
        _ => 0,
    }
}

const FIELDS: &[&str] = &[
    "resource_platinum",
    "resource_food",
    "resource_lumber",
    "resource_ore",
    "resource_mana",
    "resource_gems",
    "resource_tech",
    "peasants",
    "military_draftees",
    "military_unit1",
    "military_unit2",
    "military_unit3",
    "military_unit4",
    "military_spies",
    "military_assassins",
    "military_wizards",
    "military_archmages",
    "land_plain",
    "land_mountain",
    "land_swamp",
    "land_cavern",
    "land_forest",
    "land_hill",
    "land_water",
    "building_home",
    "building_alchemy",
    "building_farm",
    "building_smithy",
    "building_masonry",
    "building_lumberyard",
    "building_forest_haven",
    "building_ore_mine",
    "building_gryphon_nest",
    "building_tower",
    "building_wizard_guild",
    "building_temple",
    "building_diamond_mine",
    "building_school",
    "building_factory",
    "building_guard_tower",
    "building_shrine",
    "building_barracks",
    "building_dock",
    "morale",
    "prestige",
    "protection_ticks_remaining",
    "improvement_science",
    "improvement_walls",
    "draft_rate",
    "resource_gems",
    "resource_tech",
];

fn check(file: &str) {
    let path = format!("tests/golden/{file}");
    let txt = fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing golden file {path}"));
    let v: serde_json::Value = serde_json::from_str(&txt).unwrap();
    let sc: Scenario = serde_json::from_value(v["scenario"].clone()).unwrap();

    let steps = run(&sc);
    let golden = v["steps"].as_array().unwrap();
    assert_eq!(
        steps.len(),
        golden.len(),
        "{file}: step count engine {} vs golden {}",
        steps.len(),
        golden.len()
    );

    for (i, (es, gs)) in steps.iter().zip(golden).enumerate() {
        let ej = serde_json::to_value(es).unwrap();
        let gattrs = &gs["attrs"];
        for f in FIELDS {
            let e = as_i64(&ej[*f]);
            let g = as_i64(&gattrs[*f]);
            assert_eq!(
                e, g,
                "{file} step {i} ({}): field {f}: engine={e} golden={g}",
                gs["label"]
            );
        }
        // Defensive power (computed value from the oracle).
        let e_dp = engine::calc::defensive_power(es);
        let g_dp = gs["computed"]["dp"].as_f64().unwrap_or(0.0);
        assert!(
            (e_dp - g_dp).abs() < 0.5,
            "{file} step {i} ({}): dp engine={e_dp} golden={g_dp}",
            gs["label"]
        );
        // Offensive power (target-less base; the oracle's getOffensivePower($d)).
        // Guarded so only vectors that actually carry an `op` value are validated.
        if let Some(g_op) = gs["computed"]["op"].as_f64() {
            let e_op = engine::calc::offensive_power(es);
            assert!(
                (e_op - g_op).abs() < 0.5,
                "{file} step {i} ({}): op engine={e_op} golden={g_op}",
                gs["label"]
            );
        }
        // Combat layer (guarded — only validated on vectors re-emitted with these fields).
        if let Some(g) = gs["computed"]["temple_reduction"].as_f64() {
            let e = engine::combat::temple_reduction(es);
            assert!(
                (e - g).abs() < 1e-6,
                "{file} step {i} ({}): temple_reduction engine={e} golden={g}",
                gs["label"]
            );
        }
        if let Some(g) = gs["computed"]["op_ratio"].as_f64() {
            let e = engine::combat::offensive_power_ratio(es);
            assert!(
                (e - g).abs() < 1e-3,
                "{file} step {i} ({}): op_ratio engine={e} golden={g}",
                gs["label"]
            );
        }
        if let Some(g) = gs["computed"]["dp_ratio"].as_f64() {
            let e = engine::combat::defensive_power_ratio(es);
            assert!(
                (e - g).abs() < 1e-3,
                "{file} step {i} ({}): dp_ratio engine={e} golden={g}",
                gs["label"]
            );
        }
        // Networth (guarded — only validated on vectors re-emitted with `networth`).
        if let Some(g) = gs["computed"]["networth"].as_f64() {
            let e = engine::networth::dominion_networth(es) as f64;
            assert!(
                (e - g).abs() < 0.5,
                "{file} step {i} ({}): networth engine={e} golden={g}",
                gs["label"]
            );
        }
        // DP under this dominion's own temple reduction (validates the multiplierReduction path).
        if let Some(g) = gs["computed"]["dp_vs_self_temple"].as_f64() {
            let e = engine::combat::defensive_power_vs(es, engine::combat::temple_reduction(es));
            assert!(
                (e - g).abs() < 0.5,
                "{file} step {i} ({}): dp_vs_self_temple engine={e} golden={g}",
                gs["label"]
            );
        }
    }
}

#[test]
fn baseline() {
    check("baseline.json");
}

#[test]
fn daily_bonus() {
    check("daily_bonus.json");
}

#[test]
fn explore() {
    check("explore.json");
}

#[test]
fn construct() {
    check("construct.json");
}

#[test]
fn rezone() {
    check("rezone.json");
}

#[test]
fn train_spies() {
    check("train_spies.json");
}

#[test]
fn human_benchmark_r50() {
    check("human_benchmark_r50.json");
}

#[test]
fn human_build_6800() {
    check("human_build_6800.json");
}

#[test]
fn defense() {
    check("defense.json");
}

#[test]
fn spells() {
    check("spells.json");
}

#[test]
fn improvements() {
    check("improvements.json");
}

#[test]
fn bank() {
    check("bank.json");
}

#[test]
fn draft_rate() {
    check("draft_rate.json");
}

#[test]
fn late_start() {
    check("late_start.json");
}

#[test]
fn starvation() {
    check("starvation.json");
}

#[test]
fn destroy() {
    check("destroy.json");
}

#[test]
fn release() {
    check("release.json");
}

#[test]
fn harbor() {
    check("harbor.json");
}

// Offensive power: gryphon nests + forges improvement + prestige + trained offensive
// units (Spearmen), validating the full OP multiplier (the only term not exercised by
// the other vectors is the gryphon-nest bonus).
#[test]
fn gryphon_op() {
    check("gryphon_op.json");
}

#[test]
fn pop_test() {
    check("pop_test.json");
}

#[test]
fn pop_test_temple() {
    check("pop_test_temple.json");
}

// ---- Race fidelity: unit perks (defense_from_land/building/pairing, ore_production)
// + the gnome ore-cost exception. Each validates per-race resources AND computed DP. ----

#[test]
fn dwarf_ore() {
    check("dwarf_ore.json"); // ore_production unit perk (Miner +0.5 ore/unit)
}

#[test]
fn wood_elf_def() {
    check("wood_elf_def.json"); // defense_from_land (Mystic, forest)
}

#[test]
fn dark_elf_def() {
    check("dark_elf_def.json"); // defense_from_building (Adept, wizard_guild)
}

#[test]
fn kobold_pairing() {
    check("kobold_pairing.json"); // defense_from_pairing (Beast + Overlord)
}

#[test]
fn gnome_def() {
    check("gnome_def.json"); // defense_from_land (cap) + gnome un-reduced ore training cost
}

// ---- Racial self-spells (data-driven): mana cost, race gating, and perk effect. ----

#[test]
fn dwarf_minerssight() {
    check("dwarf_minerssight.json"); // Miner's Sight → +20% ore production
}

#[test]
fn icekin_frost() {
    check("icekin_frost.json"); // Alchemist Frost → +15% platinum production
}

#[test]
fn kobold_howling() {
    check("kobold_howling.json"); // Howling → +10% defense (multiplier)
}

// ---- Race-level cost / housing perks. ----

#[test]
fn firewalker_construct() {
    check("firewalker_construct.json"); // construction_cost −10% (platinum only)
}

#[test]
fn woodelf_rezone() {
    check("woodelf_rezone.json"); // rezone_cost +10%
}

#[test]
fn planewalker_homes() {
    check("planewalker_homes.json"); // home_housing −5 (starting peasants) + not_trainable slots
}

#[test]
fn firewalker_alchemistflame() {
    check("firewalker_alchemistflame.json"); // racial spell platinum_production_raw (+per-alchemy)
}

#[test]
fn gnome_school() {
    check("gnome_school.json"); // school research-point production × tech_production race perk
}
