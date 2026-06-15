//! Calculators ported from round-50 (PopulationCalculator, LandCalculator, ...).
//! Race, tech, and self-spell terms are data-driven where they can affect
//! protection-period play.

use crate::config::*;
use crate::data;
use crate::rounding::{clamp, php_round, rceil, rfloor, round_int};
use crate::state::DominionState;
use std::sync::OnceLock;

const EXPLORE_COST_TABLE_MAX_LAND: usize = 100_000;

// ----------------------------------------------------------------------------
// Land
// ----------------------------------------------------------------------------

pub fn total_land(s: &DominionState) -> i64 {
    s.total_land()
}

pub fn construction_queue_total(s: &DominionState) -> i64 {
    s.queue
        .iter()
        .filter(|q| q.source == "construction")
        .map(|q| q.amount)
        .sum()
}

pub fn training_queue_total(s: &DominionState) -> i64 {
    s.queue
        .iter()
        .filter(|q| q.source == "training")
        .map(|q| q.amount)
        .sum()
}

/// getTotalBarrenLand = total land - built buildings - constructing buildings.
pub fn total_barren_land(s: &DominionState) -> i64 {
    total_land(s) - s.total_buildings() - construction_queue_total(s)
}

// ----------------------------------------------------------------------------
// Improvements (ImprovementCalculator::getImprovementMultiplierBonus)
//   bonus = max * (1 - e^(-points / (coeff*land + 15000))) * masonry_efficiency
// ----------------------------------------------------------------------------

pub fn improvement_efficiency(s: &DominionState) -> f64 {
    1.0 + (s.building_masonry as f64 * 2.75) / total_land(s) as f64
}

pub fn improvement_bonus(s: &DominionState, points: i64, max: f64, coeff: f64) -> f64 {
    if points <= 0 {
        return 0.0;
    }
    let land = total_land(s) as f64;
    let base = max * (1.0 - (-(points as f64) / (coeff * land + 15000.0)).exp());
    php_round(base * improvement_efficiency(s), 4)
}

pub fn keep_bonus(s: &DominionState) -> f64 {
    improvement_bonus(s, s.improvement_keep, 0.25, 4000.0)
}

/// ImproveActionService::getInvestmentMultiplier, excluding hero/wonder terms
/// that are not represented in this simulator state. Race and tech terms are
/// data-driven and matter in full-round explorer continuations.
pub fn investment_multiplier(s: &DominionState, resource: &str, improvement_type: &str) -> f64 {
    1.0 + race_perk(s, "invest_bonus") / 100.0
        + race_perk(s, &format!("invest_bonus_{resource}")) / 100.0
        + tp(s, &format!("invest_bonus_{improvement_type}")) / 100.0
}

// ----------------------------------------------------------------------------
// Prestige (PrestigeCalculator::getPrestigeMultiplier)
//   validated against the oracle: prestige 250 -> 0.025 multiplier
// ----------------------------------------------------------------------------

pub fn prestige_multiplier(s: &DominionState) -> f64 {
    s.prestige as f64 / 10000.0
}

// ----------------------------------------------------------------------------
// Spells. Same-key spell perks do not stack inside a category; resolveSpellPerk
// applies the best same-category value and sums category results.
// ----------------------------------------------------------------------------

pub fn spell_perk(s: &DominionState, perk: &str) -> f64 {
    let active = s
        .spells
        .iter()
        .filter(|sp| sp.duration > 0)
        .map(|sp| sp.key.as_str());
    data::resolved_spell_perk(active, perk)
}

#[cfg(test)]
mod spell_perk_tests {
    use super::*;
    use crate::state::ActiveSpell;

    fn state_with_spells(keys: &[&str]) -> DominionState {
        let mut s = DominionState::default();
        s.spells = keys
            .iter()
            .map(|key| ActiveSpell {
                key: (*key).to_string(),
                duration: 12,
            })
            .collect();
        s
    }

    #[test]
    fn same_category_positive_spell_perks_do_not_stack() {
        assert_eq!(
            spell_perk(&state_with_spells(&["ares_call", "howling"]), "defense"),
            10.0
        );
        assert_eq!(
            spell_perk(
                &state_with_spells(&["mining_strength", "miners_sight"]),
                "ore_production"
            ),
            20.0
        );
        assert_eq!(
            spell_perk(
                &state_with_spells(&["gaias_watch", "gaias_blessing"]),
                "food_production"
            ),
            20.0
        );
        assert_eq!(
            spell_perk(
                &state_with_spells(&["midas_touch", "alchemist_frost"]),
                "platinum_production"
            ),
            15.0
        );
        assert_eq!(
            spell_perk(
                &state_with_spells(&[
                    "bloodrage",
                    "crusade",
                    "howling",
                    "killing_rage",
                    "warsong",
                    "nightfall",
                ]),
                "offense"
            ),
            10.0
        );
    }

    #[test]
    fn same_category_duration_zero_spell_perks_are_ignored() {
        let mut s = state_with_spells(&["ares_call", "howling"]);
        s.spells[1].duration = 0;
        assert_eq!(spell_perk(&s, "defense"), 10.0);
    }

    #[test]
    fn different_spell_categories_still_resolve_separately() {
        assert_eq!(
            spell_perk(
                &state_with_spells(&["energy_mirror", "rejuvenation"]),
                "enemy_spell_damage"
            ),
            -90.0
        );
    }
}

/// Research-point cost of the next tech (TechCalculator::getTechCost).
pub fn tech_cost(s: &DominionState) -> i64 {
    let raw = 2.5 * s.highest_land_achieved as f64 + 50.0 * s.techs.len() as f64;
    round_int(raw).max(3750) // * mult (1.0 for Human)
}

/// Tech perk total for a key across researched techs (data-driven).
pub fn tp(s: &DominionState, perk: &str) -> f64 {
    data::tech_perk(&s.techs, perk)
}

/// Racial perk value (data-driven; percentage points, 0 if the race lacks it).
pub fn race_perk(s: &DominionState, perk: &str) -> f64 {
    data::get()
        .races
        .get(&s.race)
        .and_then(|r| r.perks.get(perk))
        .copied()
        .unwrap_or(0.0)
}

/// Defense power of trained unit slot 1..=4 for this race (data-driven). Human:
/// Spearman 0, Archer 3, Knight 6, Cavalry 3 — identical to the prior hardcode.
pub fn unit_defense(s: &DominionState, slot: usize) -> f64 {
    data::get()
        .races
        .get(&s.race)
        .and_then(|r| r.units.get(slot.saturating_sub(1)))
        .and_then(|u| u.power.get("defense"))
        .copied()
        .unwrap_or(0.0)
}

/// Offense power of trained unit slot 1..=4 for this race (data-driven).
pub fn unit_offense(s: &DominionState, slot: usize) -> f64 {
    data::get()
        .races
        .get(&s.race)
        .and_then(|r| r.units.get(slot.saturating_sub(1)))
        .and_then(|u| u.power.get("offense"))
        .copied()
        .unwrap_or(0.0)
}

/// Platinum/ore training cost of unit slot 1..=4 for this race (data-driven).
pub fn unit_cost(s: &DominionState, slot: usize, resource: &str) -> i64 {
    data::get()
        .races
        .get(&s.race)
        .and_then(|r| r.units.get(slot.saturating_sub(1)))
        .and_then(|u| u.cost.get(resource))
        .copied()
        .unwrap_or(0)
}

/// Does unit slot 1..=4 need a boat to be sent on invasion (data-driven; default
/// true). Flying/amphibious units (`need_boat: false`) are exempt. The demon slot-4
/// flying-spell special case is intentionally omitted (no spell-state in the QC
/// send model); it would only make demon offense MORE deliverable, so omitting it
/// is conservative.
pub fn unit_need_boat(s: &DominionState, slot: usize) -> bool {
    data::get()
        .races
        .get(&s.race)
        .and_then(|r| r.units.get(slot.saturating_sub(1)))
        .map(|u| u.need_boat)
        .unwrap_or(true)
}

/// Units carried per boat = base 30 + additive race/tech `boat_capacity` perks
/// (MilitaryCalculator::getBoatCapacity). undead-rework: 30 + 20 = 50.
pub fn boat_capacity(s: &DominionState) -> i64 {
    (UNITS_PER_BOAT + race_perk(s, "boat_capacity") + tp(s, "boat_capacity")).max(1.0) as i64
}

// ----------------------------------------------------------------------------
// Unit perks (round-50 MilitaryCalculator + ProductionCalculator). Data-driven;
// perk values are scalars (e.g. 0.5) or comma-strings ("type,ratio,max" /
// "slot,numRequired,amount"). Only the perks that act DURING protection — defense
// and resource production — are interpreted; combat/ops/invasion perks are inert
// pre-OOP and ignored.
// ----------------------------------------------------------------------------

/// Raw perk value for unit slot 1..=4, or None (data-driven, &'static).
pub fn unit_perk(s: &DominionState, slot: usize, key: &str) -> Option<&'static serde_json::Value> {
    data::get()
        .races
        .get(&s.race)
        .and_then(|r| r.units.get(slot.saturating_sub(1)))
        .and_then(|u| u.perks.get(key))
}

/// Is this unit slot trainable? (`not_trainable` perk → false.)
pub fn unit_trainable(s: &DominionState, slot: usize) -> bool {
    unit_perk(s, slot, "not_trainable").is_none()
}

/// Scalar value of a unit-slot perk (e.g. `casualties` = -25, `immortal` = 1), or 0 if
/// absent. For the comma/semicolon-string perks use `unit_perk` + parse instead.
pub fn unit_perk_scalar(s: &DominionState, slot: usize, key: &str) -> f64 {
    unit_perk(s, slot, key).map(perk_scalar).unwrap_or(0.0)
}

fn perk_parts(v: &serde_json::Value) -> Vec<String> {
    match v {
        serde_json::Value::String(s) => s.split(',').map(|x| x.trim().to_string()).collect(),
        _ => Vec::new(),
    }
}
fn perk_scalar(v: &serde_json::Value) -> f64 {
    match v {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}
fn military_count(s: &DominionState, slot: usize) -> i64 {
    match slot {
        1 => s.military_unit1,
        2 => s.military_unit2,
        3 => s.military_unit3,
        4 => s.military_unit4,
        _ => 0,
    }
}

pub fn military_slot_count(s: &DominionState, slot: usize) -> i64 {
    military_count(s, slot)
}

pub fn training_queue_by_slot(s: &DominionState, slot: usize) -> i64 {
    let resource = format!("military_unit{slot}");
    s.queue
        .iter()
        .filter(|q| q.source == "training" && q.resource == resource)
        .map(|q| q.amount)
        .sum()
}
fn land_by_type(s: &DominionState, t: &str) -> i64 {
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
fn building_by_type(s: &DominionState, b: &str) -> i64 {
    match b {
        "home" => s.building_home,
        "alchemy" => s.building_alchemy,
        "farm" => s.building_farm,
        "smithy" => s.building_smithy,
        "masonry" => s.building_masonry,
        "ore_mine" => s.building_ore_mine,
        "gryphon_nest" => s.building_gryphon_nest,
        "tower" => s.building_tower,
        "wizard_guild" => s.building_wizard_guild,
        "temple" => s.building_temple,
        "diamond_mine" => s.building_diamond_mine,
        "school" => s.building_school,
        "lumberyard" => s.building_lumberyard,
        "forest_haven" => s.building_forest_haven,
        "factory" => s.building_factory,
        "guard_tower" => s.building_guard_tower,
        "shrine" => s.building_shrine,
        "barracks" => s.building_barracks,
        "dock" => s.building_dock,
        _ => 0,
    }
}

/// `X_from_land`/`X_from_building` per-unit bonus = min((count/total·100)/ratio, max).
fn unit_power_from_ratio_perk(s: &DominionState, slot: usize, key: &str, by_building: bool) -> f64 {
    let Some(v) = unit_perk(s, slot, key) else {
        return 0.0;
    };
    let p = perk_parts(v);
    if p.len() < 3 {
        return 0.0;
    }
    let ratio: f64 = p[1].parse().unwrap_or(1.0);
    let max: f64 = p[2].parse().unwrap_or(0.0);
    if ratio == 0.0 {
        return 0.0;
    }
    let count = if by_building {
        building_by_type(s, &p[0])
    } else {
        land_by_type(s, &p[0])
    };
    (count as f64 / total_land(s).max(1) as f64 * 100.0 / ratio).min(max)
}

/// Defense of unit slot 1..=4 INCLUDING per-unit land/building perks (NOT pairing —
/// that is a flat total, see `pairing_defense_bonus`). Mirrors getUnitPowerWithPerks
/// for the protection-relevant perks (defense_from_land / defense_from_building).
pub fn unit_defense_modified(s: &DominionState, slot: usize) -> f64 {
    unit_defense(s, slot)
        + unit_power_from_ratio_perk(s, slot, "defense_from_land", false)
        + unit_power_from_ratio_perk(s, slot, "defense_from_building", true)
}

/// Offense of unit slot 1..=4 INCLUDING per-unit land/building offense perks + the
/// prestige perk (NOT pairing — see `pairing_offense_bonus`; NOT the landRatio-dependent
/// staggered/vs-race perks — those belong to the combat OP). Offense analog of
/// `unit_defense_modified`.
pub fn unit_offense_modified(s: &DominionState, slot: usize) -> f64 {
    unit_offense(s, slot)
        + unit_power_from_ratio_perk(s, slot, "offense_from_land", false)
        + unit_power_from_ratio_perk(s, slot, "offense_from_building", true)
        + unit_power_from_prestige(s, slot, "offense")
        + unit_power_from_raw_wizard_ratio(s, slot, "offense")
        + unit_power_from_spell_perk(s, slot, "offense")
}

fn unit_power_from_spell_perk(s: &DominionState, slot: usize, power_type: &str) -> f64 {
    // Special case from MilitaryCalculator::getUnitPowerFromSpellPerk:
    // Demon Infernal Command gives slot 1 +0.5 offense while active.
    if s.race == "demon" && slot == 1 && power_type == "offense" {
        return spell_perk(s, "offense_unit1");
    }
    let Some(v) = unit_perk(s, slot, &format!("{power_type}_from_spell")) else {
        return 0.0;
    };
    let p = perk_parts(v);
    if p.len() < 2 {
        return 0.0;
    }
    if s.spells.iter().any(|sp| sp.key == p[0] && sp.duration > 0) {
        p[1].parse().unwrap_or(0.0)
    } else {
        0.0
    }
}

/// Raw wizard ratio = (wizards + 2×archmages + counts_as_wizard units) / land.
pub fn raw_wizard_ratio(s: &DominionState) -> f64 {
    let mut wizards = s.military_wizards as f64 + s.military_archmages as f64 * 2.0;
    for slot in 1..=4 {
        let frac = unit_perk_scalar(s, slot, "counts_as_wizard");
        if frac != 0.0 {
            wizards += rfloor(military_count(s, slot) as f64 * frac) as f64;
        }
    }
    wizards / total_land(s).max(1) as f64
}

/// `<power>_raw_wizard_ratio` perk = min(rawWizardRatio × ratio, max). Target-less.
fn unit_power_from_raw_wizard_ratio(s: &DominionState, slot: usize, power_type: &str) -> f64 {
    let Some(v) = unit_perk(s, slot, &format!("{power_type}_raw_wizard_ratio")) else {
        return 0.0;
    };
    let p = perk_parts(v); // "1,3" → [ratio, max]
    if p.len() < 2 {
        return 0.0;
    }
    let ratio: f64 = p[0].parse().unwrap_or(0.0);
    let max: f64 = p[1].parse().unwrap_or(0.0);
    (raw_wizard_ratio(s) * ratio).min(max)
}

/// `offense_staggered_land_range` perk: a per-unit OP bonus that switches on once the
/// land ratio (target/attacker) reaches a threshold. Format: comma-separated
/// `range;power` entries; the highest threshold ≤ landRatio wins (mirrors the source's
/// overwrite-on-match loop). landRatio-dependent → combat OP only, not the base power.
pub fn unit_offense_staggered(s: &DominionState, slot: usize, land_ratio: f64) -> f64 {
    let Some(serde_json::Value::String(spec)) = unit_perk(s, slot, "offense_staggered_land_range")
    else {
        return 0.0;
    };
    let mut power = 0.0;
    for entry in spec.split(',') {
        let parts: Vec<&str> = entry.split(';').collect();
        if parts.len() < 2 {
            continue;
        }
        let range = parts[0].trim().parse::<f64>().unwrap_or(0.0) / 100.0;
        if range > land_ratio {
            continue;
        }
        power = parts[1].trim().parse::<f64>().unwrap_or(0.0);
    }
    power
}

/// `<power>_from_prestige` perk = min(prestige / amount, max), added to unit power.
/// Target-less (prestige isn't relative to a target), so it folds into the base power.
fn unit_power_from_prestige(s: &DominionState, slot: usize, power_type: &str) -> f64 {
    let Some(v) = unit_perk(s, slot, &format!("{power_type}_from_prestige")) else {
        return 0.0;
    };
    let p = perk_parts(v); // e.g. "300,3" → [amount, max]
    if p.len() < 2 {
        return 0.0;
    }
    let amount: f64 = p[0].parse().unwrap_or(0.0);
    let max: f64 = p[1].parse().unwrap_or(0.0);
    if amount == 0.0 {
        return 0.0;
    }
    (s.prestige as f64 / amount).min(max)
}

/// Total defense from `defense_from_pairing` across all slots (flat, added once to
/// raw DP). Per slot: min(count[slot], floor(count[paired]/numRequired)) × amount.
pub fn pairing_defense_bonus(s: &DominionState) -> f64 {
    let mut bonus = 0.0;
    for slot in 1..=4 {
        let Some(v) = unit_perk(s, slot, "defense_from_pairing") else {
            continue;
        };
        let p = perk_parts(v);
        if p.len() < 3 {
            continue;
        }
        let paired: usize = p[0].parse().unwrap_or(0);
        let num_req: i64 = p[1].parse().unwrap_or(1);
        let amount: f64 = p[2].parse().unwrap_or(0.0);
        if num_req <= 0 {
            continue;
        }
        let paired_avail = rfloor(military_count(s, paired) as f64 / num_req as f64);
        let number_paired = military_count(s, slot).min(paired_avail);
        bonus += number_paired as f64 * amount;
    }
    bonus
}

/// Total offense from `offense_from_pairing` across all slots (flat, added once to raw
/// OP). Offense analog of `pairing_defense_bonus`.
pub fn pairing_offense_bonus(s: &DominionState) -> f64 {
    let mut bonus = 0.0;
    for slot in 1..=4 {
        let Some(v) = unit_perk(s, slot, "offense_from_pairing") else {
            continue;
        };
        let p = perk_parts(v);
        if p.len() < 3 {
            continue;
        }
        let paired: usize = p[0].parse().unwrap_or(0);
        let num_req: i64 = p[1].parse().unwrap_or(1);
        let amount: f64 = p[2].parse().unwrap_or(0.0);
        if num_req <= 0 {
            continue;
        }
        let paired_avail = rfloor(military_count(s, paired) as f64 / num_req as f64);
        let number_paired = military_count(s, slot).min(paired_avail);
        bonus += number_paired as f64 * amount;
    }
    bonus
}

/// Race-driven hourly unit production (`summons_unit` unit perk). Produced units
/// enter the normal 12-hour training queue, matching TickService::performRaceUnitProduction.
pub fn summons_unit_production(s: &DominionState) -> Vec<(usize, i64)> {
    let mut out = Vec::new();
    for source_slot in 1..=4 {
        let Some(v) = unit_perk(s, source_slot, "summons_unit") else {
            continue;
        };
        let p = perk_parts(v);
        if p.len() < 3 {
            continue;
        }
        let target_slot: usize = p[0].parse().unwrap_or(0);
        let per_source: i64 = p[1].parse().unwrap_or(0);
        let cap_per_source: i64 = p[2].parse().unwrap_or(0);
        if !(1..=4).contains(&target_slot) || per_source <= 0 || cap_per_source <= 0 {
            continue;
        }
        let source_home = military_count(s, source_slot);
        let ideal = source_home / per_source;
        if ideal <= 0 {
            continue;
        }
        let source_total = source_home + training_queue_by_slot(s, source_slot);
        let target_existing =
            military_count(s, target_slot) + training_queue_by_slot(s, target_slot);
        let cap_remaining = (cap_per_source * source_total - target_existing).max(0);
        let produced = ideal.min(cap_remaining);
        if produced > 0 {
            out.push((target_slot, produced));
        }
    }
    out
}

/// Active-spell building production that creates military queue entries.
/// Spellwright's Calling: each wizard guild produces 0.05 Dark Elf rework unit3
/// (Adept) per tick. Mirrors PHP `TickService::performTick` — the sub-unit
/// remainder is carried in `racial_value` across ticks (so wizard-guild counts
/// not divisible by 20 still produce, instead of being floored to 0 each tick):
///   $unitsProduced = building_wizard_guild * value + racial_value;
///   queue rfloor($unitsProduced); racial_value = fmod($unitsProduced, 1);
pub fn spell_building_unit_production(s: &mut DominionState) -> Vec<(usize, i64)> {
    let mut out = Vec::new();
    let unit3_per_wizard_guild = spell_perk(s, "wizard_guilds_produce_military_unit3");
    if unit3_per_wizard_guild > 0.0 {
        let units_produced =
            s.building_wizard_guild as f64 * unit3_per_wizard_guild + s.racial_value;
        let amount = rfloor(units_produced);
        s.racial_value = units_produced % 1.0; // fmod(units_produced, 1): keep the remainder
        if amount > 0 {
            out.push((3, amount));
        }
    }
    out
}

/// getUnitPerkProductionBonus: Σ over slots of (scalar | min(landPct/ratio,max)) × count.
pub fn unit_perk_production_bonus(s: &DominionState, perk: &str) -> f64 {
    let mut bonus = 0.0;
    for slot in 1..=4 {
        let Some(v) = unit_perk(s, slot, perk) else {
            continue;
        };
        let per_unit = if matches!(v, serde_json::Value::String(_)) {
            let p = perk_parts(v);
            if p.len() < 3 {
                0.0
            } else {
                let ratio: f64 = p[1].parse().unwrap_or(1.0);
                let max: f64 = p[2].parse().unwrap_or(0.0);
                if ratio == 0.0 {
                    0.0
                } else {
                    (land_by_type(s, &p[0]) as f64 / total_land(s).max(1) as f64 * 100.0 / ratio)
                        .min(max)
                }
            }
        } else {
            perk_scalar(v)
        };
        bonus += per_unit * military_count(s, slot) as f64;
    }
    bonus
}

// ----------------------------------------------------------------------------
// Population & housing (PopulationCalculator)
// ----------------------------------------------------------------------------

pub fn max_population_raw(s: &DominionState) -> i64 {
    let home = s.building_home;
    let barracks = s.building_barracks;
    let non_home = s.total_buildings() - home - barracks;
    // housingPerHome = 30 + race home_housing perk (e.g. Planewalker -5).
    let housing_per_home = HOUSING_HOME + round_int(race_perk(s, "home_housing"));
    let housing_per_barren = HOUSING_BARREN
        + round_int(race_perk(s, "extra_barren_max_population"))
        + round_int(tp(s, "extra_barren_max_population"));
    home * housing_per_home
        + non_home * HOUSING_NONHOME
        + barracks * HOUSING_BARRACKS
        + construction_queue_total(s) * HOUSING_CONSTRUCTING
        + total_barren_land(s) * housing_per_barren
}

pub fn max_population_multiplier(s: &DominionState) -> f64 {
    // race max_population perk (data-driven; 0 for Human) + keep + tech.
    let multiplier = 1.0
        + keep_bonus(s)
        + race_perk(s, "max_population") / 100.0
        + tp(s, "max_population") / 100.0;
    multiplier * (1.0 + prestige_multiplier(s))
}

pub fn max_population_military_bonus(s: &DominionState) -> i64 {
    // troopsPerBarracks = 36 plus race/tech perks. Unit-housing perks are
    // omitted until covered by a golden vector.
    let units = population_military(s) - s.military_draftees;
    let troops_per_barracks = BARRACKS_MILITARY_HOUSING
        + round_int(race_perk(s, "barracks_housing"))
        + round_int(tp(s, "barracks_housing"));
    let cap = round_int(s.building_barracks as f64 * troops_per_barracks as f64);
    units.min(cap)
}

pub fn max_population(s: &DominionState) -> i64 {
    round_int(
        max_population_raw(s) as f64 * max_population_multiplier(s)
            + max_population_military_bonus(s) as f64,
    )
}

pub fn max_peasant_population(s: &DominionState) -> i64 {
    max_population(s) - population_military(s)
}

pub fn population_military(s: &DominionState) -> i64 {
    // NOTE: getTotalUnitsForSlot(1..4) semantics (whether it includes per-slot
    // training) to be confirmed when porting military training; for Human
    // baseline + specialist training this matches (military_unitN on-hand +
    // whole training queue).
    s.military_draftees
        + s.military_unit1
        + s.military_unit2
        + s.military_unit3
        + s.military_unit4
        + s.military_spies
        + s.military_assassins
        + s.military_wizards
        + s.military_archmages
        + training_queue_total(s)
}

pub fn population(s: &DominionState) -> i64 {
    s.peasants + population_military(s)
}

pub fn military_percentage(s: &DominionState) -> f64 {
    let pop = population(s);
    if pop == 0 {
        0.0
    } else {
        population_military(s) as f64 / pop as f64 * 100.0
    }
}

pub fn draftee_growth(s: &DominionState) -> i64 {
    if military_percentage(s) < s.draft_rate as f64 {
        round_int(s.peasants as f64 * DRAFTEE_GROWTH_RATE)
    } else {
        0
    }
}

pub fn population_birth_multiplier(s: &DominionState) -> f64 {
    if s.resource_food == 0 {
        return 0.0;
    }
    // temples (uncapped, temples/land*6) + race/tech/spell population_growth perks.
    let temple_bonus = (s.building_temple as f64 / total_land(s) as f64) * 6.0;
    1.0 + temple_bonus
        + race_perk(s, "population_growth") / 100.0
        + spell_perk(s, "population_growth") / 100.0
        + tp(s, "population_growth") / 100.0
}

pub fn population_birth(s: &DominionState) -> i64 {
    let raw = (s.peasants - draftee_growth(s)) as f64 * PEASANT_GROWTH_RATE;
    round_int(raw * population_birth_multiplier(s))
}

/// Net peasant change for the tick (can be negative). Mirrors
/// PopulationCalculator::getPopulationPeasantGrowth exactly.
pub fn peasant_growth(s: &DominionState) -> i64 {
    let dg = draftee_growth(s);
    let mut max_death = round_int(-MAX_PEASANT_DEATH_RATE * s.peasants as f64 - dg as f64);
    if max_death > -50 {
        max_death = (-50_i64).max(-s.peasants);
    }
    let room = max_population(s) - population(s) - dg;
    let change = population_birth(s) - dg;
    let max_change = room.min(change);
    max_death.max(max_change)
}

// ----------------------------------------------------------------------------
// Employment (PopulationCalculator)
// ----------------------------------------------------------------------------

/// Jobs: 20 per building except home and barracks.
pub fn employment_jobs(s: &DominionState) -> i64 {
    let employing = s.total_buildings() - s.building_home - s.building_barracks;
    JOBS_PER_BUILDING * employing // * (1 + wonder employment = 0)
}

pub fn population_employed(s: &DominionState) -> i64 {
    employment_jobs(s).min(s.peasants)
}

// ----------------------------------------------------------------------------
// Defense floor (MilitaryCalculator::getMinimumDefense)
// ----------------------------------------------------------------------------

pub fn min_defense(s: &DominionState) -> f64 {
    minimum_defense(total_land(s))
}

// ----------------------------------------------------------------------------
// Production (ProductionCalculator). Human + perk-less under protection, so
// race/tech/wonder/spell/hero/guard terms are 0/1; improvement terms included.
// Production = rfloor(raw * multiplier). Some multipliers are TODO-validated
// against the golden vectors (noted inline).
// ----------------------------------------------------------------------------

pub fn science_bonus(s: &DominionState) -> f64 {
    improvement_bonus(s, s.improvement_science, 0.20, 4000.0)
}
/// Harbor improvement (food + boat production). max 60%, coeff 5000.
pub fn harbor_bonus(s: &DominionState) -> f64 {
    improvement_bonus(s, s.improvement_harbor, 0.60, 5000.0)
}
/// Forges improvement (offensive power — unused by a protected explorer). max 30%, coeff 7500.
pub fn forges_bonus(s: &DominionState) -> f64 {
    improvement_bonus(s, s.improvement_forges, 0.30, 7500.0)
}
/// Spires improvement (wizard/spy power — unused by a protected explorer). max 60%, coeff 5000.
pub fn spires_bonus(s: &DominionState) -> f64 {
    improvement_bonus(s, s.improvement_spires, 0.60, 5000.0)
}

pub fn platinum_production(s: &DominionState) -> i64 {
    let platinum_per_alchemy = PLAT_PER_ALCHEMY + spell_perk(s, "platinum_production_raw");
    let raw = population_employed(s) as f64 * PEASANT_TAX
        + s.building_alchemy as f64 * platinum_per_alchemy;
    // race (data-driven) + science improvement + midas spell + tech; guard tax = 0.
    let mut bonus = science_bonus(s)
        + race_perk(s, "platinum_production") / 100.0
        + spell_perk(s, "platinum_production") / 100.0
        + tp(s, "platinum_production") / 100.0;
    if bonus > PLAT_PRODUCTION_MAX_BONUS {
        bonus = PLAT_PRODUCTION_MAX_BONUS;
    }
    rfloor(raw * (1.0 + bonus))
}

pub fn food_production_multiplier(s: &DominionState) -> f64 {
    // Racial food_production perk (data-driven; Human +5%). + prestige bonus
    // (getPrestigeMultiplier). tech/wonder/gaia/harbor/hero = 0 for Human.
    1.0 + race_perk(s, "food_production") / 100.0
        + prestige_multiplier(s)
        + spell_perk(s, "food_production") / 100.0
        + tp(s, "food_production") / 100.0
        + harbor_bonus(s)
}
pub fn food_production(s: &DominionState) -> i64 {
    let raw = s.building_farm as f64 * FOOD_PER_FARM + s.building_dock as f64 * FOOD_PER_DOCK;
    rfloor(raw * food_production_multiplier(s))
}
pub fn food_consumption(s: &DominionState) -> i64 {
    let m = 1.0 + (race_perk(s, "food_consumption") + tp(s, "food_consumption")) / 100.0;
    rfloor(population(s) as f64 * FOOD_CONSUMPTION_PER_CAPITA * m)
}
pub fn food_decay(s: &DominionState) -> i64 {
    // getFoodDecay uses round() (not rfloor): round(food * 1% * mult)
    let mult = 1.0 + (spell_perk(s, "food_decay") + tp(s, "food_decay")) / 100.0;
    round_int(s.resource_food as f64 * FOOD_DECAY * mult)
}
pub fn food_net_change(s: &DominionState) -> i64 {
    food_production(s) - food_consumption(s) - food_decay(s)
}

pub fn lumber_production(s: &DominionState) -> i64 {
    let raw = s.building_lumberyard as f64 * LUMBER_PER_LUMBERYARD
        + s.building_forest_haven as f64 * LUMBER_PER_FOREST_HAVEN;
    rfloor(
        raw * (1.0
            + (race_perk(s, "lumber_production")
                + spell_perk(s, "lumber_production")
                + tp(s, "lumber_production"))
                / 100.0),
    )
}
pub fn lumber_decay(s: &DominionState) -> i64 {
    // getLumberDecay uses round() (not rfloor)
    let mult = 1.0
        + (race_perk(s, "lumber_decay") + spell_perk(s, "lumber_decay") + tp(s, "lumber_decay"))
            / 100.0;
    round_int(s.resource_lumber as f64 * LUMBER_DECAY * mult)
}
pub fn lumber_net_change(s: &DominionState) -> i64 {
    lumber_production(s) - lumber_decay(s)
}

pub fn mana_production(s: &DominionState) -> i64 {
    let raw = s.building_tower as f64 * MANA_PER_TOWER
        + s.building_wizard_guild as f64
            * (MANA_PER_WIZARD_GUILD + spell_perk(s, "wizard_guild_mana_production_raw"));
    rfloor(raw * (1.0 + (race_perk(s, "mana_production") + tp(s, "mana_production")) / 100.0))
}
pub fn mana_decay(s: &DominionState) -> i64 {
    // getManaDecay uses round() (not rfloor)
    let mult = 1.0 + (spell_perk(s, "mana_decay") + tp(s, "mana_decay")) / 100.0;
    round_int(s.resource_mana as f64 * MANA_DECAY * mult)
}
pub fn mana_net_change(s: &DominionState) -> i64 {
    mana_production(s) - mana_decay(s)
}

pub fn ore_production(s: &DominionState) -> i64 {
    // raw = ore mines + unit-perk producers (e.g. Dwarf Miner 0.5/unit), THEN ×mult.
    let raw =
        s.building_ore_mine as f64 * ORE_PER_MINE + unit_perk_production_bonus(s, "ore_production");
    let mult = 1.0
        + (race_perk(s, "ore_production")
            + spell_perk(s, "ore_production")
            + tp(s, "ore_production"))
            / 100.0;
    rfloor(raw * mult)
}
pub fn gem_production(s: &DominionState) -> i64 {
    rfloor(
        s.building_diamond_mine as f64
            * GEMS_PER_DIAMOND_MINE
            * (1.0 + (race_perk(s, "gem_production") + tp(s, "gem_production")) / 100.0),
    )
}

pub fn tech_production(s: &DominionState) -> i64 {
    let schools = s.building_school as f64;
    if schools <= 0.0 {
        return 0;
    }
    let land = total_land(s) as f64;
    let pct = (schools / land).min(SCHOOL_MAX_LAND_RATIO);
    let effective = schools.min((land * SCHOOL_MAX_LAND_RATIO).floor()) * (1.0 - pct);
    rfloor(effective * (1.0 + race_perk(s, "tech_production") / 100.0))
}

pub fn boat_production(s: &DominionState) -> f64 {
    s.building_dock as f64 / 20.0
}

// ----------------------------------------------------------------------------
// Action cost calculators (Exploration/Construction/Rezoning) + morale.
// Human, perk-less; factory/smithy reductions included for generality (0 at start).
// ----------------------------------------------------------------------------

/// Net conquered land (drives the explored/conquered cost split). 0 under protection.
pub fn conquered_land(s: &DominionState) -> i64 {
    let (c, l) = (s.stat_total_land_conquered, s.stat_total_land_lost);
    if l >= c {
        0
    } else {
        c - l
    }
}

/// Explored-equivalent land for cost formulas: total_land - 250 (+conquered adj).
pub fn explored_land(s: &DominionState) -> i64 {
    let (c, l) = (s.stat_total_land_conquered, s.stat_total_land_lost);
    if l >= c {
        total_land(s) - 250 + (c - l).max(0)
    } else {
        total_land(s) - 250 - (c - l)
    }
}

pub fn factory_reduction(s: &DominionState) -> f64 {
    ((s.building_factory as f64 / total_land(s) as f64) * FACTORY_DISCOUNT_COEF)
        .min(FACTORY_DISCOUNT_MAX)
}

pub fn explore_platinum_cost(s: &DominionState) -> i64 {
    let p = explore_platinum_base(total_land(s));
    round_int(p * (1.0 + tp(s, "explore_platinum_cost") / 100.0))
}

fn explore_platinum_base(land: i64) -> f64 {
    let idx = land.max(0) as usize;
    if idx <= EXPLORE_COST_TABLE_MAX_LAND {
        return explore_cost_table()[idx];
    }
    explore_platinum_base_formula(land)
}

fn explore_cost_table() -> &'static [f64] {
    static TABLE: OnceLock<Vec<f64>> = OnceLock::new();
    TABLE
        .get_or_init(|| {
            (0..=EXPLORE_COST_TABLE_MAX_LAND)
                .map(|land| explore_platinum_base_formula(land as i64))
                .collect()
        })
        .as_slice()
}

fn explore_platinum_base_formula(land: i64) -> f64 {
    let land_f = land as f64;
    let mut p = 0.6 * land_f.powf(1.299);
    if land < 1520 {
        p += -0.001 * land_f * land_f + 1.91 * land_f - 593.0;
    }
    p
}

pub fn explore_draftee_cost(s: &DominionState) -> i64 {
    rfloor(total_land(s) as f64 / 150.0) + 3 + tp(s, "explore_draftee_cost") as i64
}

pub fn construct_platinum_cost(s: &DominionState) -> i64 {
    let raw = round_int(850.0 + conquered_land(s) as f64 + 1.25 * explored_land(s) as f64);
    // Race construction_cost perk applies to PLATINUM only (e.g. Firewalker −10%), not
    // lumber — getPlatinumCostMultiplier. Capped at −80% (0.20).
    rfloor(
        raw as f64
            * ((1.0 - factory_reduction(s))
                + race_perk(s, "construction_cost") / 100.0
                + tp(s, "construction_cost") / 100.0
                + tp(s, "construction_platinum_cost") / 100.0)
                .max(0.20),
    )
}

pub fn construct_lumber_cost(s: &DominionState) -> i64 {
    let raw = round_int(87.5 + conquered_land(s) as f64 / 4.25 + 0.285 * explored_land(s) as f64);
    rfloor(
        raw as f64
            * ((1.0 - factory_reduction(s)) + tp(s, "construction_lumber_cost") / 100.0).max(0.25),
    )
}

pub fn rezone_platinum_cost(s: &DominionState) -> i64 {
    let raw = round_int(250.0 + 0.6 * explored_land(s) as f64 + 0.2 * conquered_land(s) as f64);
    // Race rezone_cost perk (e.g. Wood Elf +10%) — RezoningCalculator::getCostMultiplier.
    rfloor(
        raw as f64
            * ((1.0 - factory_reduction(s))
                + race_perk(s, "rezone_cost") / 100.0
                + tp(s, "rezone_cost") / 100.0
                + spell_perk(s, "rezone_cost") / 100.0),
    )
}

/// Smithy training-cost discount (−2%/% of land owned, capped at −36%).
pub fn smithy_reduction(s: &DominionState) -> f64 {
    ((s.building_smithy as f64 / total_land(s) as f64) * SMITHY_DISCOUNT_COEF)
        .min(SMITHY_DISCOUNT_MAX)
}

/// Specialist/elite training cost multiplier (smithies -2%/% owned, cap -36%).
pub fn specialist_elite_multiplier(s: &DominionState) -> f64 {
    1.0 - smithy_reduction(s) + tp(s, "military_cost") / 100.0
}

pub const TRAINING_RESOURCES: [&str; 5] = ["platinum", "ore", "mana", "lumber", "gems"];

/// Per-unit specialist/elite training costs for race unit slot 1..=4.
/// Resource names are bare engine resources (`platinum`, `ore`, ...); callers that
/// work with wallet keys should prefix them with `resource_`.
pub fn unit_training_costs(s: &DominionState, slot: usize) -> Vec<(&'static str, i64)> {
    let m = specialist_elite_multiplier(s);
    let mut costs = Vec::new();
    for res in TRAINING_RESOURCES {
        let base = unit_cost(s, slot, res);
        if base <= 0 {
            continue;
        }
        let amount = if res == "ore" && s.race == "gnome" {
            base
        } else {
            rceil(base as f64 * m)
        };
        costs.push((res, amount));
    }
    costs
}

/// Spy/wizard training cost multiplier. TODO: confirm getSpyCostMultiplier; 1.0 at start.
pub fn spy_cost_multiplier(_s: &DominionState) -> f64 {
    1.0
}

// Starvation (CasualtiesCalculator::getStarvationCasualtiesByUnitType): when
// food < 0, casualties = min(|deficit|, round(0.02*pop)); 50% peasants, remainder
// across unit1-4 + draftees proportionally, leftover back to peasants.
#[derive(Default, Debug)]
pub struct Starvation {
    pub peasants: i64,
    pub unit1: i64,
    pub unit2: i64,
    pub unit3: i64,
    pub unit4: i64,
    pub draftees: i64,
}

pub fn starvation_casualties(s: &DominionState, deficit: i64) -> Starvation {
    let mut c = Starvation::default();
    if deficit >= 0 {
        return c;
    }
    let total = deficit.unsigned_abs() as i64;
    let total = total.min(round_int(population(s) as f64 * 0.02));
    if total <= 0 {
        return c;
    }
    let total_mil = s.military_draftees
        + s.military_unit1
        + s.military_unit2
        + s.military_unit3
        + s.military_unit4;
    c.peasants = ((total as f64 * 0.5) as i64).min(s.peasants);
    let mut remaining = total - c.peasants;
    let military_casualties = remaining;
    if total_mil > 0 {
        for (slot, count) in [
            (1, s.military_unit1),
            (2, s.military_unit2),
            (3, s.military_unit3),
            (4, s.military_unit4),
            (0, s.military_draftees),
        ] {
            if remaining <= 0 {
                break;
            }
            if count == 0 {
                continue;
            }
            let lost =
                rfloor(military_casualties as f64 * count as f64 / total_mil as f64).min(count);
            match slot {
                1 => c.unit1 = lost,
                2 => c.unit2 = lost,
                3 => c.unit3 = lost,
                4 => c.unit4 = lost,
                _ => c.draftees = lost,
            }
            remaining -= lost;
        }
    }
    if remaining > 0 {
        c.peasants = (c.peasants + remaining).min(s.peasants);
    }
    c
}

pub fn morale_gain(s: &DominionState) -> i64 {
    let g = if s.morale < 80 { 6 } else { 3 };
    g.min(100 - s.morale)
}

// ----------------------------------------------------------------------------
// Defensive power (MilitaryCalculator::getDefensivePower)
//   max( rawDP * multiplier * moraleMult, minDefense )
// rawDP uses ON-HAND units only (training queue excluded). Human unit defense:
// spearman 0, archer 3, knight 6, cavalry 3; draftee 1 each.
// ----------------------------------------------------------------------------

pub fn defensive_power_raw(s: &DominionState) -> f64 {
    // Per-race unit defense incl. land/building perks; flat pairing bonus added once;
    // draftees contribute 1 each. (MilitaryCalculator::getDefensivePowerRaw)
    s.military_unit1 as f64 * unit_defense_modified(s, 1)
        + s.military_unit2 as f64 * unit_defense_modified(s, 2)
        + s.military_unit3 as f64 * unit_defense_modified(s, 3)
        + s.military_unit4 as f64 * unit_defense_modified(s, 4)
        + pairing_defense_bonus(s)
        + s.military_draftees as f64
}

pub fn guard_tower_bonus(s: &DominionState) -> f64 {
    (GUARD_TOWER_DP_COEF * s.building_guard_tower as f64 / total_land(s) as f64)
        .min(GUARD_TOWER_DP_MAX)
}

/// Gryphon-nest OP modifier — offense analog of `guard_tower_bonus` (same 1.6/32% shape).
pub fn gryphon_nest_bonus(s: &DominionState) -> f64 {
    (GRYPHON_NEST_OP_COEF * s.building_gryphon_nest as f64 / total_land(s) as f64)
        .min(GRYPHON_NEST_OP_MAX)
}

pub fn walls_bonus(s: &DominionState) -> f64 {
    improvement_bonus(s, s.improvement_walls, 0.30, 7500.0)
}

pub fn defensive_power_multiplier(s: &DominionState) -> f64 {
    defensive_power_multiplier_reduced(s, 0.0)
}

/// DP multiplier with an external `reduction` subtracted before the floor — this is how
/// an attacker's temples cut the defender's modifier (getDefensivePowerMultiplier's
/// `multiplierReduction`). Floored at 1.0, exactly as the source.
pub fn defensive_power_multiplier_reduced(s: &DominionState, reduction: f64) -> f64 {
    // guard towers + walls + Ares + racial/tech defense perks (0 for Human).
    (1.0 + guard_tower_bonus(s)
        + walls_bonus(s)
        + (race_perk(s, "defense") + spell_perk(s, "defense") + tp(s, "defense")) / 100.0
        - reduction)
        .max(1.0)
}

pub fn morale_multiplier(s: &DominionState) -> f64 {
    clamp(0.9 + s.morale as f64 / 1000.0, 0.9, 1.0)
}

pub fn defensive_power(s: &DominionState) -> f64 {
    let dp = defensive_power_raw(s) * defensive_power_multiplier(s) * morale_multiplier(s);
    dp.max(min_defense(s))
}

// ----------------------------------------------------------------------------
// Standing defense (attacker-floor reference) — ANALYTIC, not a game number.
// "What DP stays home when the OFFENSIVE army is sent on attack." Counts every
// DEFENSE-DOMINANT unit (base defense >= base offense) — INCLUDING defensive elites like
// the Human Knight (2 off / 6 def), the primary Human defensive troop — and EXCLUDES the
// offense-DOMINANT units that get sent (the offensive elite/specialist, e.g. Human
// Cavalry 6/3 and Spearman 3/0) plus draftees. Having some OP does NOT make a unit
// offensive — being offense-DOMINANT does. The pairing bonus is dropped (it can be
// contingent on a sent unit being home). Built from golden-validated primitives — checked
// by a Rust unit test, not an oracle vector (the PHP game has no "DP excluding your own
// offensive elite" quantity).
// ----------------------------------------------------------------------------

/// Raw DP from DEFENSE-DOMINANT units (base defense >= base offense); the offensive
/// elite/specialist, draftees, and the pairing bonus are excluded. Mirrors the per-unit
/// term of `defensive_power_raw`.
pub fn standing_defense_raw(s: &DominionState) -> f64 {
    let mut dp = 0.0;
    for slot in 1..=4 {
        // Defense-dominant units stay home; offense-dominant units are sent on attack.
        if unit_defense(s, slot) >= unit_offense(s, slot) {
            let count = match slot {
                1 => s.military_unit1,
                2 => s.military_unit2,
                3 => s.military_unit3,
                _ => s.military_unit4,
            };
            dp += count as f64 * unit_defense_modified(s, slot);
        }
    }
    dp
}

/// Modded standing defense = `standing_defense_raw` × DP multiplier × morale — the same
/// modifier stack as `defensive_power`, without the `min_defense` floor (the converter's
/// DP schedule is the floor here, checked separately).
pub fn standing_defense_modded(s: &DominionState) -> f64 {
    standing_defense_raw(s) * defensive_power_multiplier(s) * morale_multiplier(s)
}

// ----------------------------------------------------------------------------
// Offensive power (MilitaryCalculator::getOffensivePower*). Target-LESS base: the
// range / land-ratio / vs-race terms (which need a target) are omitted, and
// wonder/hero terms are 0 in protection. OP = raw × multiplier × morale (morale
// multiplies OP and DP alike, per source). Draftees do NOT add offense.
// ----------------------------------------------------------------------------

pub fn offensive_power_raw(s: &DominionState) -> f64 {
    s.military_unit1 as f64 * unit_offense_modified(s, 1)
        + s.military_unit2 as f64 * unit_offense_modified(s, 2)
        + s.military_unit3 as f64 * unit_offense_modified(s, 3)
        + s.military_unit4 as f64 * unit_offense_modified(s, 4)
        + pairing_offense_bonus(s)
}

pub fn offensive_power_multiplier(s: &DominionState) -> f64 {
    // gryphon nests + forges improvement + racial/tech/self-spell offense perks +
    // prestige (prestige/10000). Wonder + hero offense = 0 (none in protection).
    1.0 + gryphon_nest_bonus(s)
        + forges_bonus(s)
        + (race_perk(s, "offense") + spell_perk(s, "offense") + tp(s, "offense")) / 100.0
        + favorable_terrain_bonus(s)
        + prestige_multiplier(s)
}

fn favorable_terrain_bonus(s: &DominionState) -> f64 {
    if spell_perk(s, "offense_from_barren_land") == 0.0 {
        return 0.0;
    }
    // PHP: +1% OP per 1% barren land, capped at the spell perk value (10%).
    let barren_ratio = total_barren_land(s).max(0) as f64 / total_land(s).max(1) as f64;
    barren_ratio.min(spell_perk(s, "offense_from_barren_land") / 100.0)
}

pub fn offensive_power(s: &DominionState) -> f64 {
    offensive_power_raw(s) * offensive_power_multiplier(s) * morale_multiplier(s)
}

// ----------------------------------------------------------------------------
// Striking offense (attacker-objective reference) — the symmetric twin of
// `standing_defense`. OP of the army that gets SENT: OFFENSE-DOMINANT units only (base
// offense > base defense, e.g. Human Cavalry 6/3 and Spearman 3/0). Home defenders
// (defense-dominant, incl. the Knight 2/6) are NOT counted — they stay home, so their OP
// is not deployed; counting it would double-count a unit as both home DP and sent OP.
// Together, `standing_defense` and `striking_offense` partition the 4 slots exactly. The
// pairing bonus is dropped, mirroring standing_defense.
// ----------------------------------------------------------------------------

/// Raw OP from OFFENSE-DOMINANT units (base offense > base defense); home defenders,
/// draftees, and the pairing bonus are excluded. Mirrors the per-unit term of
/// `offensive_power_raw`.
pub fn striking_offense_raw(s: &DominionState) -> f64 {
    let mut op = 0.0;
    for slot in 1..=4 {
        // Offense-dominant units are sent on attack; defense-dominant units stay home.
        if unit_offense(s, slot) > unit_defense(s, slot) {
            let count = match slot {
                1 => s.military_unit1,
                2 => s.military_unit2,
                3 => s.military_unit3,
                _ => s.military_unit4,
            };
            op += count as f64 * unit_offense_modified(s, slot);
        }
    }
    op
}

/// Modded striking offense = `striking_offense_raw` × OP multiplier × morale — the same
/// modifier stack as `offensive_power` (gryphon nests, forges, racial/tech/spell offense,
/// prestige, morale).
pub fn striking_offense_modded(s: &DominionState) -> f64 {
    striking_offense_raw(s) * offensive_power_multiplier(s) * morale_multiplier(s)
}

/// Striking offense actually DELIVERABLE given the dominion's boats. Boat-needing
/// offense units are capped by `boats × boat_capacity`; boat-exempt offense
/// (flying/amphibious) is always deliverable. Returns the modded OP of the carriable
/// army — i.e. the OP you can put on a target, not merely what you trained. The QC
/// objective gates on this so docks/boats are a real cost, not a free OP number.
pub fn striking_offense_sendable(s: &DominionState) -> f64 {
    let carry_units = (s.resource_boats.max(0.0) * boat_capacity(s) as f64).floor();
    let mut raw_no_boat = 0.0;
    let mut raw_boat = 0.0;
    let mut units_boat = 0.0;
    for slot in 1..=4 {
        // Same rule as striking_offense_raw: only offense-dominant units are sent.
        if unit_offense(s, slot) <= unit_defense(s, slot) {
            continue;
        }
        let count = match slot {
            1 => s.military_unit1,
            2 => s.military_unit2,
            3 => s.military_unit3,
            _ => s.military_unit4,
        } as f64;
        if count <= 0.0 {
            continue;
        }
        let raw = count * unit_offense_modified(s, slot);
        if unit_need_boat(s, slot) {
            raw_boat += raw;
            units_boat += count;
        } else {
            raw_no_boat += raw;
        }
    }
    let carry_frac = if units_boat > 0.0 {
        (carry_units / units_boat).clamp(0.0, 1.0)
    } else {
        1.0
    };
    (raw_no_boat + raw_boat * carry_frac) * offensive_power_multiplier(s) * morale_multiplier(s)
}

// ----------------------------------------------------------------------------
// App observability helpers — NOT game mechanics. Derived views the studio UI reads
// so it can render "N buildings from cap" and the employment balance without
// re-hardcoding any constants. Cap counts come straight from the engine so the
// desktop app and the browser mock agree to the building.
// ----------------------------------------------------------------------------

/// Smallest building count at which a `min(COEF·count/land, MAX)` bonus saturates —
/// the count the "N to cap" readout targets. The `MAX/COEF` boundary lands on an exact
/// integer for round land sizes (e.g. 1.6/0.32 → land/5); the −1e-6 before ceil absorbs
/// float fuzz so a count sitting exactly on the boundary reads "at cap", not "1 over".
fn cap_count(land: i64, coef: f64, max: f64) -> i64 {
    if land <= 0 || coef <= 0.0 {
        return 0;
    }
    ((max / coef) * land as f64 - 1e-6).ceil() as i64
}

/// Guard towers past this count add no DP (bonus capped at 32%).
pub fn guard_tower_cap_count(s: &DominionState) -> i64 {
    cap_count(total_land(s), GUARD_TOWER_DP_COEF, GUARD_TOWER_DP_MAX)
}
/// Smithies past this count add no training discount (capped at 36%).
pub fn smithy_cap_count(s: &DominionState) -> i64 {
    cap_count(total_land(s), SMITHY_DISCOUNT_COEF, SMITHY_DISCOUNT_MAX)
}
/// Factories past this count add no build/rezone discount (capped at 50%).
pub fn factory_cap_count(s: &DominionState) -> i64 {
    cap_count(total_land(s), FACTORY_DISCOUNT_COEF, FACTORY_DISCOUNT_MAX)
}
/// Schools past `floor(land·0.5)` add no tech (effective count is capped there).
pub fn school_cap_count(s: &DominionState) -> i64 {
    (SCHOOL_MAX_LAND_RATIO * total_land(s) as f64).floor() as i64
}
/// Gryphon nests past this count add no OP (bonus capped at 32%) — same as guard towers.
pub fn gryphon_nest_cap_count(s: &DominionState) -> i64 {
    cap_count(total_land(s), GRYPHON_NEST_OP_COEF, GRYPHON_NEST_OP_MAX)
}

/// Peasants employed per job building (every building except home/barracks).
pub fn jobs_per_building() -> i64 {
    JOBS_PER_BUILDING
}
/// Peasant housing per home (base + race home_housing perk), as used by max_population_raw.
pub fn housing_per_home(s: &DominionState) -> i64 {
    HOUSING_HOME + round_int(race_perk(s, "home_housing"))
}
/// Peasant housing per non-home, non-barracks building.
pub fn housing_per_nonhome() -> i64 {
    HOUSING_NONHOME
}
/// Military housed per barracks (base + race/tech perks) — these troops vacate the
/// shared population cap, freeing room for peasants.
pub fn barracks_military_housing(s: &DominionState) -> i64 {
    BARRACKS_MILITARY_HOUSING
        + round_int(race_perk(s, "barracks_housing"))
        + round_int(tp(s, "barracks_housing"))
}

#[cfg(test)]
mod cap_tests {
    use super::*;

    fn with_land(n: i64) -> DominionState {
        let mut s = DominionState::default();
        s.land_plain = n;
        s
    }

    /// The reported cap count is exactly where the bonus saturates: at the cap the bonus
    /// equals its max, one fewer is below max, and one more adds nothing. Checked at an
    /// exact-boundary land size (1000) and an off-boundary one (333), since float fuzz in
    /// MAX/COEF could otherwise shift the floor/ceil by one. This is the "N to cap"
    /// contract both the desktop app and the browser mock rely on.
    #[test]
    fn cap_counts_sit_exactly_on_bonus_saturation() {
        for &total in &[1000_i64, 333] {
            // guard tower (DP bonus)
            let mut s = with_land(total);
            let cap = guard_tower_cap_count(&s);
            s.building_guard_tower = cap;
            assert!(
                (guard_tower_bonus(&s) - GUARD_TOWER_DP_MAX).abs() < 1e-9,
                "gt@{total}: at cap → max"
            );
            s.building_guard_tower = cap + 50;
            assert!(
                (guard_tower_bonus(&s) - GUARD_TOWER_DP_MAX).abs() < 1e-9,
                "gt@{total}: over cap → still max"
            );
            s.building_guard_tower = cap - 1;
            assert!(
                guard_tower_bonus(&s) < GUARD_TOWER_DP_MAX,
                "gt@{total}: below cap → under max"
            );

            // smithy (training discount)
            let mut s = with_land(total);
            let cap = smithy_cap_count(&s);
            s.building_smithy = cap;
            assert!(
                (smithy_reduction(&s) - SMITHY_DISCOUNT_MAX).abs() < 1e-9,
                "smithy@{total}: at cap → max"
            );
            s.building_smithy = cap - 1;
            assert!(
                smithy_reduction(&s) < SMITHY_DISCOUNT_MAX,
                "smithy@{total}: below cap → under max"
            );

            // factory (build/rezone discount)
            let mut s = with_land(total);
            let cap = factory_cap_count(&s);
            s.building_factory = cap;
            assert!(
                (factory_reduction(&s) - FACTORY_DISCOUNT_MAX).abs() < 1e-9,
                "factory@{total}: at cap → max"
            );
            s.building_factory = cap - 1;
            assert!(
                factory_reduction(&s) < FACTORY_DISCOUNT_MAX,
                "factory@{total}: below cap → under max"
            );
        }
    }
}

#[cfg(test)]
mod standing_defense_tests {
    use super::*;

    fn set_slot(s: &mut DominionState, slot: usize, n: i64) {
        match slot {
            1 => s.military_unit1 = n,
            2 => s.military_unit2 = n,
            3 => s.military_unit3 = n,
            _ => s.military_unit4 = n,
        }
    }

    /// Standing defense (the converter/attacker floor) counts DEFENSE-DOMINANT units
    /// (base defense >= base offense) and never draftees or offense-DOMINANT units.
    /// Race-agnostic: each slot's role is read from the data. For Human the Archer (0/3)
    /// and Knight (2/6, the primary defensive troop) count; the offensive elite Cavalry
    /// (6/3) and offensive specialist Spearman (3/0) are excluded. Having OP does not make
    /// a unit offensive — being offense-dominant does.
    #[test]
    fn standing_defense_excludes_offense_and_draftees() {
        let mut s = crate::config::start_state("advanced", 0, "human");
        for slot in 1..=4 {
            set_slot(&mut s, slot, 0);
        }
        s.military_draftees = 0;

        // Draftees never contribute (unlike defensive_power_raw, which adds 1 each).
        let base = standing_defense_raw(&s);
        s.military_draftees = 5000;
        assert_eq!(
            standing_defense_raw(&s),
            base,
            "draftees must not count toward standing defense"
        );
        s.military_draftees = 0;

        // Per-slot: offense-bearing slots add nothing; pure-defender slots add exactly
        // count * unit_defense_modified.
        let (mut saw_defender, mut saw_offense) = (false, false);
        for slot in 1..=4 {
            for s2 in 1..=4 {
                set_slot(&mut s, s2, 0);
            }
            let before = standing_defense_raw(&s);
            set_slot(&mut s, slot, 100);
            let delta = standing_defense_raw(&s) - before;
            if unit_offense(&s, slot) > unit_defense(&s, slot) {
                saw_offense = true;
                assert_eq!(
                    delta, 0.0,
                    "offense-dominant slot {slot} (the sent army) must not add standing defense"
                );
            } else {
                saw_defender = true;
                let expected = 100.0 * unit_defense_modified(&s, slot);
                assert!(
                    (delta - expected).abs() < 1e-9,
                    "pure-defender slot {slot}: delta {delta} != expected {expected}"
                );
            }
        }
        assert!(
            saw_defender,
            "human should have defense-dominant slots (Archer, Knight)"
        );
        assert!(
            saw_offense,
            "human should have offense-dominant slots (Spearman, Cavalry)"
        );

        // standing_defense_modded applies the same multiplier stack as defensive_power.
        for slot in 1..=4 {
            set_slot(&mut s, slot, 50);
        }
        let expected_mod =
            standing_defense_raw(&s) * defensive_power_multiplier(&s) * morale_multiplier(&s);
        assert!((standing_defense_modded(&s) - expected_mod).abs() < 1e-9);
    }

    /// Striking offense is the symmetric twin: it counts OFFENSE-DOMINANT units only
    /// (Spearman 3/0, Cavalry 6/3) and never home defenders (Archer 0/3, Knight 2/6) or
    /// draftees. The Knight check is the key one — it has 2 OP but must NOT count as
    /// strike, because it stays home as DP ("OP does not make a unit offensive").
    #[test]
    fn striking_offense_excludes_home_defenders_and_draftees() {
        let mut s = crate::config::start_state("advanced", 0, "human");
        for slot in 1..=4 {
            set_slot(&mut s, slot, 0);
        }
        s.military_draftees = 0;

        // Draftees never strike.
        let base = striking_offense_raw(&s);
        s.military_draftees = 5000;
        assert_eq!(
            striking_offense_raw(&s),
            base,
            "draftees must not count toward striking offense"
        );
        s.military_draftees = 0;

        for slot in 1..=4 {
            for s2 in 1..=4 {
                set_slot(&mut s, s2, 0);
            }
            let before = striking_offense_raw(&s);
            set_slot(&mut s, slot, 100);
            let delta = striking_offense_raw(&s) - before;
            if unit_offense(&s, slot) > unit_defense(&s, slot) {
                let expected = 100.0 * unit_offense_modified(&s, slot);
                assert!(
                    (delta - expected).abs() < 1e-9,
                    "offense-dominant slot {slot}: delta {delta} != expected {expected}"
                );
            } else {
                assert_eq!(
                    delta, 0.0,
                    "home defender slot {slot} must not add striking offense (even with OP)"
                );
            }
        }

        // No double-counting: every slot is in exactly one of {standing_defense, striking}.
        for slot in 1..=4 {
            let home = unit_defense(&s, slot) >= unit_offense(&s, slot);
            let sent = unit_offense(&s, slot) > unit_defense(&s, slot);
            assert!(
                home ^ sent,
                "slot {slot} must be exactly one of home-defender / strike"
            );
        }
    }
}

#[cfg(test)]
mod boat_tests {
    use super::*;

    #[test]
    fn boat_capacity_includes_race_perk() {
        let human = crate::config::start_state("advanced", 0, "human");
        assert_eq!(boat_capacity(&human), 30, "base units-per-boat is 30");
        let undead = crate::config::start_state("advanced", 0, "undead-rework");
        assert_eq!(
            boat_capacity(&undead),
            50,
            "undead-rework boat_capacity perk (+20) -> 50 units/boat"
        );
    }

    #[test]
    fn undead_units_need_boats() {
        let undead = crate::config::start_state("advanced", 0, "undead-rework");
        for slot in 1..=4 {
            assert!(
                unit_need_boat(&undead, slot),
                "all undead-rework units need boats (slot {slot})"
            );
        }
        // Sanity: races with flying/amphibious offense have boat-exempt units.
        let demon = crate::config::start_state("advanced", 0, "demon");
        assert!(
            !unit_need_boat(&demon, 1),
            "demon Infernal Imp is boat-exempt"
        );
        let merfolk = crate::config::start_state("advanced", 0, "merfolk");
        assert!(!unit_need_boat(&merfolk, 1), "merfolk Merman is amphibious");
    }

    #[test]
    fn sendable_offense_is_gated_by_boats() {
        let mut s = crate::config::start_state("advanced", 0, "undead-rework");
        for slot in 1..=4 {
            match slot {
                1 => s.military_unit1 = 0,
                2 => s.military_unit2 = 0,
                3 => s.military_unit3 = 0,
                _ => s.military_unit4 = 0,
            }
        }
        s.military_draftees = 0;
        s.military_unit1 = 1000; // Zombie: offense-dominant (3/0), needs boats

        // Boats far in excess of need -> the full trained striking OP is deliverable.
        s.resource_boats = 100_000.0;
        let full = striking_offense_sendable(&s);
        let modded = striking_offense_modded(&s);
        assert!(full > 0.0);
        assert!(
            (full - modded).abs() < 1e-6,
            "with surplus boats, sendable == modded striking ({full} vs {modded})"
        );

        // 10 boats × 50/boat = 500 units carriable of 1000 trained -> half deliverable.
        s.resource_boats = 10.0;
        let half = striking_offense_sendable(&s);
        assert!(
            (half - full * 0.5).abs() < full * 1e-3,
            "10 boats carry 500/1000 zombies -> ~half OP ({half} vs {})",
            full * 0.5
        );

        // No boats -> no boat-needing offense can be sent.
        s.resource_boats = 0.0;
        assert_eq!(
            striking_offense_sendable(&s),
            0.0,
            "no boats -> 0 sendable OP"
        );
    }
}
