//! The per-tick transition, equivalent to round-50's precalculateTick + performTick.
//!
//! Single-pass model: resolve the queue entries arriving this tick (hours==1),
//! compute growth/production on that post-arrival state, apply them, handle
//! food/starvation and the prestige cap, and decrement the remaining queue.

use crate::calc;
use crate::rounding::rfloor;
use crate::state::{DominionState, QueueEntry};

/// Add an arriving queued amount to the matching field (keys are DB-column style,
/// e.g. "land_plain", "building_home", "military_spies").
fn add_resource(s: &mut DominionState, key: &str, amount: i64) {
    match key {
        "land_plain" => s.land_plain += amount,
        "land_mountain" => s.land_mountain += amount,
        "land_swamp" => s.land_swamp += amount,
        "land_cavern" => s.land_cavern += amount,
        "land_forest" => s.land_forest += amount,
        "land_hill" => s.land_hill += amount,
        "land_water" => s.land_water += amount,
        "building_home" => s.building_home += amount,
        "building_alchemy" => s.building_alchemy += amount,
        "building_farm" => s.building_farm += amount,
        "building_smithy" => s.building_smithy += amount,
        "building_masonry" => s.building_masonry += amount,
        "building_ore_mine" => s.building_ore_mine += amount,
        "building_gryphon_nest" => s.building_gryphon_nest += amount,
        "building_tower" => s.building_tower += amount,
        "building_wizard_guild" => s.building_wizard_guild += amount,
        "building_temple" => s.building_temple += amount,
        "building_diamond_mine" => s.building_diamond_mine += amount,
        "building_school" => s.building_school += amount,
        "building_lumberyard" => s.building_lumberyard += amount,
        "building_forest_haven" => s.building_forest_haven += amount,
        "building_factory" => s.building_factory += amount,
        "building_guard_tower" => s.building_guard_tower += amount,
        "building_shrine" => s.building_shrine += amount,
        "building_barracks" => s.building_barracks += amount,
        "building_dock" => s.building_dock += amount,
        "military_unit1" => s.military_unit1 += amount,
        "military_unit2" => s.military_unit2 += amount,
        "military_unit3" => s.military_unit3 += amount,
        "military_unit4" => s.military_unit4 += amount,
        "military_spies" => s.military_spies += amount,
        "military_assassins" => s.military_assassins += amount,
        "military_wizards" => s.military_wizards += amount,
        "military_archmages" => s.military_archmages += amount,
        _ => {}
    }
}

/// Advance one tick (one game hour). Pure: returns the next state.
pub fn tick(s: &DominionState) -> DominionState {
    // 1. Resolve arriving queue entries (hours==1) into a working state; decrement
    //    the rest. Arrived buildings become "built" (no longer constructing).
    let mut sc = s.clone();
    let mut next_queue = Vec::with_capacity(s.queue.len());
    for q in &s.queue {
        if q.hours == 1 {
            add_resource(&mut sc, &q.resource, q.amount);
        } else {
            let mut e = q.clone();
            e.hours -= 1;
            next_queue.push(e);
        }
    }
    sc.queue = next_queue;

    // 2. Compute growth & production on the post-arrival state.
    let peasant_g = calc::peasant_growth(&sc);
    let draftee_g = calc::draftee_growth(&sc);
    let plat = calc::platinum_production(&sc);
    let lumber = calc::lumber_net_change(&sc);
    let mana = calc::mana_net_change(&sc);
    let ore = calc::ore_production(&sc);
    let gems = calc::gem_production(&sc);
    let tech = calc::tech_production(&sc);
    let boats = calc::boat_production(&sc);
    let alchemy_forges = (sc.building_alchemy as f64
        * calc::spell_perk(&sc, "alchemy_improvement_forges_raw")) as i64;
    let food_net = calc::food_net_change(&sc);
    let starvation = if sc.resource_food + food_net < 0 {
        calc::starvation_casualties(&sc, sc.resource_food + food_net)
    } else {
        calc::Starvation::default()
    };
    let morale_g = calc::morale_gain(&sc);

    // 3. Apply.
    let mut ns = sc;
    ns.peasants += peasant_g;
    ns.military_draftees += draftee_g;
    ns.resource_platinum += plat;
    ns.resource_lumber += lumber;
    ns.resource_mana += mana;
    ns.resource_ore += ore;
    ns.resource_gems += gems;
    ns.resource_tech += tech;
    ns.resource_boats += boats;
    ns.improvement_forges += alchemy_forges;
    for (slot, amount) in calc::summons_unit_production(&ns) {
        ns.queue.push(QueueEntry {
            source: "training".into(),
            resource: format!("military_unit{slot}"),
            hours: 12,
            amount,
        });
    }
    for (slot, amount) in calc::spell_building_unit_production(&mut ns) {
        ns.queue.push(QueueEntry {
            source: "training".into(),
            resource: format!("military_unit{slot}"),
            hours: 12,
            amount,
        });
    }

    if ns.resource_food + food_net < 0 {
        ns.peasants -= starvation.peasants;
        ns.military_unit1 -= starvation.unit1;
        ns.military_unit2 -= starvation.unit2;
        ns.military_unit3 -= starvation.unit3;
        ns.military_unit4 -= starvation.unit4;
        ns.military_draftees -= starvation.draftees;
        ns.resource_food = 0;
    } else {
        ns.resource_food += food_net;
    }
    apply_active_spell_tick_effects(&mut ns);

    // 4. Prestige capped at max(total_land, 250).
    let cap = ns.total_land().max(250);
    if ns.prestige > cap {
        ns.prestige = cap;
    }

    // Morale regen (Dominion::getMoraleGain).
    ns.morale += morale_g;

    // Track highest land achieved (drives tech cost).
    let tl = ns.total_land();
    if tl > ns.highest_land_achieved {
        ns.highest_land_achieved = tl;
    }

    // Spell durations decrement; expired spells drop off.
    for sp in ns.spells.iter_mut() {
        sp.duration -= 1;
    }
    ns.spells.retain(|sp| sp.duration > 0);

    ns
}

fn apply_active_spell_tick_effects(s: &mut DominionState) {
    // Death and Decay: each active tick converts 0.5% of current peasants into
    // Undead unit1. PHP queues these with QueueService's default 12h, not the
    // normal 9h specialist training delay, and charges no draftees/resources.
    let convert_pct = calc::spell_perk(s, "convert_peasants_to_self_military_unit1");
    if convert_pct > 0.0 && s.peasants > 0 {
        let converted = rfloor(s.peasants as f64 * convert_pct / 100.0)
            .max(0)
            .min(s.peasants);
        if converted > 0 {
            s.peasants -= converted;
            s.queue.push(QueueEntry {
                source: "training".into(),
                resource: "military_unit1".into(),
                hours: 12,
                amount: converted,
            });
        }
    }
}
