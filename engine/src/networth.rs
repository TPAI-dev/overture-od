//! Single source of truth for OpenDominion **networth**, bit-exact to the round-50 PHP
//! game (`../OpenDominion-source/src/Calculators/NetworthCalculator.php`).
//!
//! Networth is purely **land + buildings + military** — it encodes NO resources
//! (platinum/food/lumber/ore/mana/gems/tech). That is exactly what makes it a usable
//! defense proxy for explorer / DP-estimation tooling.
//!
//! ```text
//! NW = 20 · totalLand            land_<type> only — barren counts; INCOMING (queues) does NOT
//!    +  5 · totalBuildings        constructed only — buildings still constructing do NOT
//!    +  Σ_slot NW_slot · count    count = home + returning-from-invasion; IN-TRAINING excluded
//!    +  5 · (spies + assassins + wizards + archmages)
//!    then round()                 (PHP round(), half away from zero)
//! ```
//!
//! Per-unit value (`NetworthCalculator::getUnitNetworth`):
//! - **slots 1 & 2 (specialists): flat 5** each, always.
//! - **slots 3 & 4 (elites): `round(2 · max(off, def), 2)`**, where off/def are
//!   `MilitaryCalculator::getUnitPowerWithPerks($dom, null, 1, $unit, …)` — i.e. base power
//!   **plus** the target-less perks (`*_from_land`, `*_from_building`, `*_from_prestige`,
//!   `*_raw_wizard_ratio`, `*_from_spell`, and `*_staggered_land_range` evaluated at
//!   `landRatio = 1`). Versus-race / versus-building perks contribute 0 (null target), and
//!   pairing perks are NOT summed here — **except** `kobold-rework`, which adds a hard-coded
//!   `+2/+2` to both off and def before the max (a NetworthCalculator special case).
//!
//! Consequence: elite networth is a **constant** for races without those perks (the
//! majority), but **drifts** with the dominion's own land mix / prestige / wizard ratio for
//! the six truly-dynamic round-50 elites (sylvan Dryad, icekin FrostMage, gnome Rockapult,
//! demon Succubus — defense-scaling; icekin Ice Elemental, orc Bone Breaker, wood-elf Druid
//! — offense-scaling). This module reuses the already golden-validated perk math in
//! [`crate::calc`] so the dynamic cases stay bit-exact rather than re-deriving them.

use crate::calc;
use crate::rounding::{php_round, round_int};
use crate::state::DominionState;

/// Flat networth of a single specialist / spy / wizard / building.
pub const NW_SPECIALIST: f64 = 5.0;
const NW_PER_ELITE_POINT: f64 = 2.0;
const NW_PER_LAND: f64 = 20.0;
const NW_PER_BUILDING: f64 = 5.0;
const NW_PER_CASTER: f64 = 5.0; // spy / assassin / wizard / archmage, each

/// Number of trained units of `slot` (1..=4) that count toward networth, mirroring
/// `MilitaryCalculator::getTotalUnitsForSlot`: **home + units returning from invasion**,
/// with the **training queue excluded**.
///
/// The protection engine never invades, so a returning ("invasion"-source) queue is absent
/// from those states and this reduces to the home `military_unitN` column — which is also
/// exactly right for an **explorer** (never attacked ⇒ no troops out). The invasion-queue
/// term is kept for fidelity with the full game / reconstructed attacker states.
pub fn total_units_for_slot(s: &DominionState, slot: usize) -> i64 {
    let resource = format!("military_unit{slot}");
    let returning: i64 = s
        .queue
        .iter()
        .filter(|q| {
            matches!(q.source.as_str(), "invasion" | "return" | "returning")
                && q.resource == resource
        })
        .map(|q| q.amount)
        .sum();
    calc::military_slot_count(s, slot) + returning
}

/// Per-unit networth of unit `slot` (1..=4), bit-exact to `getUnitNetworth`.
///
/// Slots 1 & 2 are a flat 5. Slots 3 & 4 are `round(2 · max(off, def), 2)` over the
/// perk-inclusive, target-less unit power (the `landRatio = 1` evaluation the game uses for
/// networth — which fully switches on any `staggered_land_range` perk).
pub fn unit_networth(s: &DominionState, slot: usize) -> f64 {
    if slot == 1 || slot == 2 {
        return NW_SPECIALIST;
    }

    // getUnitPowerWithPerks($dom, null, 1, $unit, …): base + target-less perks. The
    // staggered perk is evaluated at landRatio = 1 (any range ≤ 1 fires); defense has no
    // staggered perk on any round-50 race, so only the offense side adds it.
    let mut offense = calc::unit_offense_modified(s, slot) + calc::unit_offense_staggered(s, slot, 1.0);
    let mut defense = calc::unit_defense_modified(s, slot);

    // NetworthCalculator special case: kobold-rework pairs add +2/+2 before the max.
    if s.race == "kobold-rework" {
        offense += 2.0;
        defense += 2.0;
    }

    php_round(NW_PER_ELITE_POINT * offense.max(defense), 2)
}

/// Full networth decomposition for a dominion. The estimator subtracts the land/building/
/// caster terms to isolate the **military** networth it attributes to DP growth.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NetworthBreakdown {
    /// `20 · totalLand` (barren included, incoming excluded).
    pub land: f64,
    /// `5 · totalBuildings` (constructed only).
    pub buildings: f64,
    /// `5 · (spies + assassins + wizards + archmages)`.
    pub casters: f64,
    /// Per-slot total networth contribution `count · unit_networth(slot)`, index 0..=3 = slot 1..=4.
    pub units: [f64; 4],
    /// Unrounded sum of every component.
    pub total_unrounded: f64,
}

impl NetworthBreakdown {
    /// Total contributed by trained units across all four slots.
    pub fn units_total(&self) -> f64 {
        self.units.iter().sum()
    }
    /// Networth from the two specialist slots (1 & 2) — the slots that include defensive
    /// specialists (which DO add DP) but are NW-indistinguishable from spies/wizards.
    pub fn specialist_units(&self) -> f64 {
        self.units[0] + self.units[1]
    }
    /// Networth from the two elite slots (3 & 4).
    pub fn elite_units(&self) -> f64 {
        self.units[2] + self.units[3]
    }
}

/// Compute the full networth decomposition (every component, unrounded).
pub fn networth_breakdown(s: &DominionState) -> NetworthBreakdown {
    let mut b = NetworthBreakdown {
        land: NW_PER_LAND * s.total_land() as f64,
        buildings: NW_PER_BUILDING * s.total_buildings() as f64,
        casters: NW_PER_CASTER
            * (s.military_spies + s.military_assassins + s.military_wizards + s.military_archmages)
                as f64,
        ..Default::default()
    };
    for slot in 1..=4 {
        b.units[slot - 1] = total_units_for_slot(s, slot) as f64 * unit_networth(s, slot);
    }
    b.total_unrounded = b.land + b.buildings + b.casters + b.units_total();
    b
}

/// A dominion's networth, bit-exact to `NetworthCalculator::getDominionNetworth`
/// (final `round()` applied).
pub fn dominion_networth(s: &DominionState) -> i64 {
    round_int(networth_breakdown(s).total_unrounded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::DominionState;

    fn dom(race: &str) -> DominionState {
        let mut s = DominionState::default();
        s.race = race.to_string();
        s
    }

    // ---- Per-unit networth: static races (constants from the verified source table) ----

    #[test]
    fn unit_networth_specialists_always_five() {
        let s = dom("human");
        assert_eq!(unit_networth(&s, 1), 5.0);
        assert_eq!(unit_networth(&s, 2), 5.0);
        // undead-rework slot 2 (Necromancer 0/4) is a specialist → still flat 5, not 8.
        let u = dom("undead-rework");
        assert_eq!(unit_networth(&u, 2), 5.0);
    }

    #[test]
    fn unit_networth_static_elites() {
        let h = dom("human");
        assert_eq!(unit_networth(&h, 3), 12.0); // Knight 2/6 → 2·6
        assert_eq!(unit_networth(&h, 4), 12.0); // Cavalry 6/3 → 2·6
        let m = dom("merfolk");
        assert_eq!(unit_networth(&m, 3), 14.0); // Leviathan 0/7 → 2·7
        assert_eq!(unit_networth(&m, 4), 8.0); // Kraken 4/2 → 2·4
        let t = dom("troll");
        assert_eq!(unit_networth(&t, 3), 14.0); // Basher 7/7
        assert_eq!(unit_networth(&t, 4), 14.0); // Smasher 7/7
        let ur = dom("undead-rework");
        assert_eq!(unit_networth(&ur, 3), 12.0); // Crypt Lord 0/6
        assert_eq!(unit_networth(&ur, 4), 8.0); // Abomination 4/3 (immortal_from_pairing is NOT a power perk)
    }

    #[test]
    fn unit_networth_kobold_plus_two_special_case() {
        let k = dom("kobold-rework");
        // Taskmaster 3/3 → +2/+2 → 5/5 → 2·5 = 10; Overlord 3/2 → 5/4 → 2·5 = 10.
        assert_eq!(unit_networth(&k, 3), 10.0);
        assert_eq!(unit_networth(&k, 4), 10.0);
    }

    #[test]
    fn unit_networth_gnome_juggernaut_staggered_on_at_ratio_one() {
        // Juggernaut 6.5/3 + offense_staggered "85;0.5" → at landRatio=1 always +0.5 →
        // off 7.0 → 2·7 = 14 (constant at networth time).
        let g = dom("gnome");
        assert_eq!(unit_networth(&g, 4), 14.0);
    }

    // ---- Per-unit networth: dynamic elites drift with the dominion's own state ----

    #[test]
    fn unit_networth_sylvan_dryad_scales_with_forest() {
        // Dryad 0/3 + defense_from_land [forest, 20, 4] → def = 3 + min(forest%/20, 4).
        let mut s = dom("sylvan");
        s.land_plain = 100; // 0% forest → def 3 → 2·3 = 6
        assert_eq!(unit_networth(&s, 3), 6.0);

        let mut s = dom("sylvan");
        s.land_forest = 40;
        s.land_plain = 60; // 40% forest → +2 → def 5 → 10
        assert_eq!(unit_networth(&s, 3), 10.0);

        let mut s = dom("sylvan");
        s.land_forest = 100; // 100% forest → +min(5,4)=4 → def 7 → 14
        assert_eq!(unit_networth(&s, 3), 14.0);
    }

    #[test]
    fn unit_networth_orc_bonebreaker_scales_with_prestige() {
        // Bone Breaker 4/3 + offense_from_prestige [300, 3] → off = 4 + min(prestige/300, 3).
        let mut s = dom("orc");
        s.prestige = 0; // off 4 → 2·4 = 8
        assert_eq!(unit_networth(&s, 4), 8.0);
        s.prestige = 450; // off 5.5 → 2·5.5 = 11
        assert_eq!(unit_networth(&s, 4), 11.0);
        s.prestige = 900; // off 7 (cap) → 14
        assert_eq!(unit_networth(&s, 4), 14.0);
        s.prestige = 9000; // capped at +3 → still 14
        assert_eq!(unit_networth(&s, 4), 14.0);
    }

    #[test]
    fn unit_networth_icekin_ice_elemental_scales_with_wizard_ratio() {
        // Ice Elemental 4/2 + offense_raw_wizard_ratio [1, 3] → off = 4 + min(wizRatio, 3).
        let mut s = dom("icekin");
        s.land_plain = 100;
        s.military_wizards = 0; // ratio 0 → off 4 → 8
        assert_eq!(unit_networth(&s, 4), 8.0);
        s.military_wizards = 100; // ratio 1 → off 5 → 10
        assert_eq!(unit_networth(&s, 4), 10.0);
        s.military_wizards = 300; // ratio 3 → off 7 → 14
        assert_eq!(unit_networth(&s, 4), 14.0);
        s.military_wizards = 1000; // ratio capped at +3 → 14
        assert_eq!(unit_networth(&s, 4), 14.0);
    }

    // ---- Full dominion networth ----

    #[test]
    fn dominion_networth_human_full_components() {
        let mut s = dom("human");
        s.land_plain = 250; // 250 · 20 = 5000
        s.building_home = 90; // 90 · 5 = 450
        s.military_unit1 = 100; // 100 · 5  = 500
        s.military_unit2 = 100; // 100 · 5  = 500
        s.military_unit3 = 100; // 100 · 12 = 1200 (Knight)
        s.military_unit4 = 100; // 100 · 12 = 1200 (Cavalry)
        s.military_spies = 25; // 25 · 5 = 125
        s.military_wizards = 25; // 25 · 5 = 125
        // 5000 + 450 + 500 + 500 + 1200 + 1200 + 125 + 125 = 9100
        assert_eq!(dominion_networth(&s), 9100);
    }

    #[test]
    fn dominion_networth_excludes_training_queue_and_draftees() {
        let mut s = dom("human");
        s.land_plain = 100; // 2000
        s.military_unit2 = 50; // 250
        s.military_draftees = 9999; // draftees contribute 0 NW
        s.queue.push(crate::state::QueueEntry {
            source: "training".into(),
            resource: "military_unit2".into(),
            hours: 3,
            amount: 1000, // in-training → excluded
        });
        assert_eq!(dominion_networth(&s), 2250);
    }

    #[test]
    fn dominion_networth_counts_returning_invasion_units() {
        let mut s = dom("human");
        s.land_plain = 100; // 2000
        s.military_unit4 = 10; // 10 · 12 = 120 (home)
        s.queue.push(crate::state::QueueEntry {
            source: "invasion".into(),
            resource: "military_unit4".into(),
            hours: 2,
            amount: 5, // returning → counted, 5 · 12 = 60
        });
        assert_eq!(dominion_networth(&s), 2180);
    }

    #[test]
    fn breakdown_components_sum_to_total() {
        let mut s = dom("sylvan");
        s.land_forest = 80;
        s.land_plain = 20; // 100 land → 2000
        s.building_home = 50; // 250
        s.military_unit2 = 200; // 1000 (Sprite, defensive spec)
        s.military_unit3 = 30; // Dryad at 80% forest: def 3+4 = 7 → NW 14 → 420
        s.military_wizards = 40; // 200
        let b = networth_breakdown(&s);
        assert_eq!(b.land, 2000.0);
        assert_eq!(b.buildings, 250.0);
        assert_eq!(b.casters, 200.0);
        assert_eq!(b.units[1], 1000.0);
        assert_eq!(b.units[2], 420.0);
        assert_eq!(dominion_networth(&s), 3870);
        assert_eq!(round_int(b.total_unrounded), 3870);
    }
}
