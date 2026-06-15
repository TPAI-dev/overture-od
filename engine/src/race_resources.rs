//! Race-aware troop-resource planning helpers.
//!
//! The engine already charges every unit's exact resource costs during training.
//! These helpers identify which non-platinum
//! resources a race can need for troops, preserving those resources from surplus
//! sinks, and sizing producer buildings when a resource decays or cannot be
//! bank-bought.

use std::collections::BTreeSet;

use crate::calc;
use crate::config;
use crate::data;
use crate::rounding::rceil;
use crate::state::DominionState;

pub const NON_PLATINUM_TRAINING_RESOURCES: [&str; 4] = ["ore", "mana", "lumber", "gems"];

pub fn resource_wallet_key(resource: &str) -> String {
    if resource.starts_with("resource_") {
        resource.to_string()
    } else {
        format!("resource_{resource}")
    }
}

pub fn resource_get(state: &DominionState, resource: &str) -> i64 {
    match resource.strip_prefix("resource_").unwrap_or(resource) {
        "platinum" => state.resource_platinum,
        "food" => state.resource_food,
        "lumber" => state.resource_lumber,
        "ore" => state.resource_ore,
        "mana" => state.resource_mana,
        "gems" => state.resource_gems,
        "tech" => state.resource_tech,
        _ => 0,
    }
}

pub fn resource_set(state: &mut DominionState, resource: &str, value: i64) {
    match resource.strip_prefix("resource_").unwrap_or(resource) {
        "platinum" => state.resource_platinum = value,
        "food" => state.resource_food = value,
        "lumber" => state.resource_lumber = value,
        "ore" => state.resource_ore = value,
        "mana" => state.resource_mana = value,
        "gems" => state.resource_gems = value,
        "tech" => state.resource_tech = value,
        _ => {}
    }
}

pub fn building_count(state: &DominionState, building: &str) -> i64 {
    match building {
        "home" => state.building_home,
        "alchemy" => state.building_alchemy,
        "farm" => state.building_farm,
        "smithy" => state.building_smithy,
        "ore_mine" => state.building_ore_mine,
        "tower" => state.building_tower,
        "wizard_guild" => state.building_wizard_guild,
        "temple" => state.building_temple,
        "lumberyard" => state.building_lumberyard,
        "forest_haven" => state.building_forest_haven,
        "factory" => state.building_factory,
        "guard_tower" => state.building_guard_tower,
        "barracks" => state.building_barracks,
        "diamond_mine" => state.building_diamond_mine,
        "school" => state.building_school,
        _ => 0,
    }
}

pub fn add_building(state: &mut DominionState, building: &str, amount: i64) {
    if amount <= 0 {
        return;
    }
    match building {
        "home" => state.building_home += amount,
        "alchemy" => state.building_alchemy += amount,
        "farm" => state.building_farm += amount,
        "smithy" => state.building_smithy += amount,
        "ore_mine" => state.building_ore_mine += amount,
        "tower" => state.building_tower += amount,
        "wizard_guild" => state.building_wizard_guild += amount,
        "temple" => state.building_temple += amount,
        "lumberyard" => state.building_lumberyard += amount,
        "forest_haven" => state.building_forest_haven += amount,
        "factory" => state.building_factory += amount,
        "guard_tower" => state.building_guard_tower += amount,
        "barracks" => state.building_barracks += amount,
        "diamond_mine" => state.building_diamond_mine += amount,
        "school" => state.building_school += amount,
        _ => {}
    }
}

pub fn incoming_building(state: &DominionState, building: &str) -> i64 {
    let resource = format!("building_{building}");
    state
        .queue
        .iter()
        .filter(|q| q.source == "construction" && q.resource == resource)
        .map(|q| q.amount)
        .sum()
}

pub fn producer_building_for_resource(resource: &str) -> Option<&'static str> {
    match resource.strip_prefix("resource_").unwrap_or(resource) {
        "ore" => Some("ore_mine"),
        "mana" => Some("tower"),
        "lumber" => Some("lumberyard"),
        "gems" => Some("diamond_mine"),
        _ => None,
    }
}

pub fn trainable_resource_set(state: &DominionState) -> BTreeSet<&'static str> {
    let mut resources = BTreeSet::new();
    for slot in 1..=4 {
        if !calc::unit_trainable(state, slot) {
            continue;
        }
        for (resource, amount) in calc::unit_training_costs(state, slot) {
            if amount > 0 && resource != "platinum" {
                resources.insert(resource);
            }
        }
    }
    resources
}

pub fn race_has_training_resource(race: &str, resource: &str) -> bool {
    let Some(race_data) = data::get().races.get(race) else {
        return false;
    };
    race_data.units.iter().any(|unit| {
        !unit.perks.contains_key("not_trainable")
            && unit
                .cost
                .get(resource.strip_prefix("resource_").unwrap_or(resource))
                .copied()
                .unwrap_or(0)
                > 0
    })
}

pub fn training_resource_per_raw_defense(
    state: &DominionState,
    resource: &str,
    slots: &[usize],
) -> Option<f64> {
    slots
        .iter()
        .copied()
        .filter(|slot| calc::unit_trainable(state, *slot))
        .filter_map(|slot| {
            let defense = calc::unit_defense_modified(state, slot);
            if defense <= 0.0 {
                return None;
            }
            let cost = calc::unit_training_costs(state, slot)
                .into_iter()
                .find(|(res, _)| *res == resource.strip_prefix("resource_").unwrap_or(resource))
                .map(|(_, amount)| amount)
                .unwrap_or(0);
            (cost > 0).then_some(cost as f64 / defense)
        })
        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
}

pub fn training_resource_need_for_raw_defense(
    state: &DominionState,
    resource: &str,
    raw_needed: f64,
    slots: &[usize],
) -> i64 {
    if raw_needed <= 0.0 {
        return 0;
    }
    training_resource_per_raw_defense(state, resource, slots)
        .map(|per_raw| rceil(raw_needed * per_raw).max(0))
        .unwrap_or(0)
}

pub fn projection_net_change(state: &DominionState, resource: &str) -> i64 {
    match resource.strip_prefix("resource_").unwrap_or(resource) {
        "platinum" => calc::platinum_production(state),
        "food" => calc::food_net_change(state),
        "lumber" => calc::lumber_net_change(state),
        "ore" => calc::ore_production(state),
        "mana" => calc::mana_net_change(state),
        "gems" => calc::gem_production(state),
        _ => 0,
    }
}

pub fn projected_resource_after_hours(state: &DominionState, resource: &str, hours: i64) -> i64 {
    let mut projection = state.clone();
    let resource = resource.strip_prefix("resource_").unwrap_or(resource);
    for _ in 0..hours.max(0) {
        let next =
            resource_get(&projection, resource) + projection_net_change(&projection, resource);
        resource_set(&mut projection, resource, next.max(0));
    }
    resource_get(&projection, resource)
}

pub fn projected_resource_after_hours_with_extra_building(
    state: &DominionState,
    resource: &str,
    building: &str,
    extra_buildings: i64,
    hours: i64,
) -> i64 {
    let mut projection = state.clone();
    add_building(&mut projection, building, extra_buildings.max(0));
    projected_resource_after_hours(&projection, resource, hours)
}

/// Target built+incoming producer count needed to have `needed` of `resource`
/// available by `deadline_hours`, accounting for existing production and decay.
pub fn producer_target_for_resource_need(
    state: &DominionState,
    resource: &str,
    needed: i64,
    deadline_hours: i64,
) -> Option<i64> {
    let resource = resource.strip_prefix("resource_").unwrap_or(resource);
    let building = producer_building_for_resource(resource)?;
    let existing = building_count(state, building) + incoming_building(state, building);
    let needed = needed.max(0);
    if needed <= 0 {
        return Some(existing);
    }

    let deadline_hours = deadline_hours.max(1);
    let projected_existing = projected_resource_after_hours(state, resource, deadline_hours);
    let short = (needed - projected_existing).max(0);
    if short <= 0 {
        return Some(existing);
    }

    let productive_hours = (deadline_hours - config::CONSTRUCT_DELAY).max(1);
    let base = projected_resource_after_hours(state, resource, productive_hours);
    let plus_one = projected_resource_after_hours_with_extra_building(
        state,
        resource,
        building,
        1,
        productive_hours,
    );
    let marginal = (plus_one - base).max(1);
    Some(existing + rceil(short as f64 / marginal as f64).max(0))
}

pub fn producer_target_for_raw_defense_need(
    state: &DominionState,
    resource: &str,
    raw_needed: f64,
    slots: &[usize],
    deadline_hours: i64,
) -> Option<i64> {
    let needed = training_resource_need_for_raw_defense(state, resource, raw_needed, slots);
    producer_target_for_resource_need(state, resource, needed, deadline_hours)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_find_mana_gems_and_lumber_training_resources() {
        let undead = config::start_state("advanced", 0, "undead-rework");
        let dark_elf = config::start_state("advanced", 0, "dark-elf-rework");
        let orc = config::start_state("advanced", 0, "orc");

        assert!(trainable_resource_set(&undead).contains("mana"));
        assert!(trainable_resource_set(&dark_elf).contains("gems"));
        assert!(trainable_resource_set(&orc).contains("lumber"));
    }

    #[test]
    fn mana_projection_accounts_for_decay() {
        let mut s = config::start_state("advanced", 0, "human");
        s.resource_mana = 10_000;
        s.building_tower = 0;

        assert!(projected_resource_after_hours(&s, "mana", 12) < 10_000);
    }

    #[test]
    fn producer_target_sizes_mana_towers_for_training_need() {
        let mut s = config::start_state("advanced", 0, "undead-rework");
        s.protection_finished = true;
        s.protection_ticks_remaining = 0;
        s.resource_mana = 0;
        s.building_tower = 0;

        let target = producer_target_for_resource_need(&s, "mana", 20_000, 36).unwrap();
        assert!(target > 0);
    }
}
