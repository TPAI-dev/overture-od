//! Post-protection combat calculators — faithful ports of round-50
//! MilitaryCalculator / RangeCalculator. Pure functions over `DominionState` (and a
//! second land value for range). NOT used by the protection sim; this is the
//! attacker/full-round layer, validated against the oracle's golden vectors.

use crate::calc;
use crate::config::*;
use crate::rounding::round_int;
use crate::state::DominionState;

/// Reduction applied to a DEFENDER's DP multiplier by THIS dominion's temples when it
/// attacks: `min(1.35% × temples/land, 27%)` (MilitaryCalculator::getTempleReduction;
/// wonder `enemy_defense` term = 0 here).
pub fn temple_reduction(s: &DominionState) -> f64 {
    let land = calc::total_land(s).max(1) as f64;
    (TEMPLE_DP_REDUCTION_COEF * s.building_temple as f64 / land).min(TEMPLE_DP_REDUCTION_MAX)
}

/// Defensive power this dominion shows to an attacker whose temple reduction is
/// `attacker_temple_reduction`. Mirrors getDefensivePower(..., multiplierReduction):
/// raw × max(mult − reduction, 1) × morale, floored at the minimum-defense gate (the
/// non-attacking-forces path, i.e. land-based defenses apply).
pub fn defensive_power_vs(s: &DominionState, attacker_temple_reduction: f64) -> f64 {
    let dp = calc::defensive_power_raw(s)
        * calc::defensive_power_multiplier_reduced(s, attacker_temple_reduction)
        * calc::morale_multiplier(s);
    dp.max(calc::min_defense(s))
}

/// Offensive power per acre of land (getOffensivePowerRatio).
pub fn offensive_power_ratio(s: &DominionState) -> f64 {
    calc::offensive_power(s) / calc::total_land(s).max(1) as f64
}

/// Defensive power per acre of land (getDefensivePowerRatio).
pub fn defensive_power_ratio(s: &DominionState) -> f64 {
    calc::defensive_power(s) / calc::total_land(s).max(1) as f64
}

/// Relative size of `target` vs `attacker`, as a percentage (getDominionRange):
/// `target_land / attacker_land × 100`. 100 = same size.
pub fn dominion_range(attacker_land: i64, target_land: i64) -> f64 {
    target_land as f64 / attacker_land.max(1) as f64 * 100.0
}

/// Whether `attacker` may invade `target` at all (RangeCalculator::isInRange, no guard
/// membership → both modifiers = MINIMUM_RANGE 0.4). Symmetric land-ratio band.
pub fn in_range(attacker_land: i64, target_land: i64) -> bool {
    let (a, t) = (attacker_land as f64, target_land as f64);
    let m = MINIMUM_RANGE;
    t >= a * m && t <= a / m && a >= t * m && a <= t / m
}

// ----------------------------------------------------------------------------
// Invasion outcome (InvadeActionService). No-war, non-repeat base case (war bonuses
// and repeat-invasion penalties need realm/history the protection engine doesn't carry).
// ----------------------------------------------------------------------------

/// The defender's DP as the attacker actually faces it — reduced by the attacker's
/// temples (getDefensivePowerWithTemples). Draftees count (base case: attacker has no
/// `ignore_draftees` spell).
pub fn defensive_power_with_temples(target: &DominionState, attacker: &DominionState) -> f64 {
    defensive_power_vs(target, temple_reduction(attacker))
}

/// Offensive power AGAINST a specific target — like `offensive_power`, but including the
/// landRatio-dependent `offense_staggered_land_range` perk that the target-less version
/// omits (so it equals `offensive_power` for races without that perk). Assumes the whole
/// army is sent; the per-unit pairing/land/building/prestige perks already fold in.
pub fn offensive_power_combat(attacker: &DominionState, target: &DominionState) -> f64 {
    let land_ratio = calc::total_land(target) as f64 / calc::total_land(attacker).max(1) as f64;
    let raw: f64 = (1..=4)
        .map(|slot| {
            (calc::unit_offense_modified(attacker, slot)
                + calc::unit_offense_staggered(attacker, slot, land_ratio))
                * calc::military_slot_count(attacker, slot) as f64
        })
        .sum::<f64>()
        + calc::pairing_offense_bonus(attacker);
    raw * calc::offensive_power_multiplier(attacker) * calc::morale_multiplier(attacker)
}

/// Acres the target loses on a successful hit (MilitaryCalculator::getLandLost, no war):
/// a piecewise function of the land ratio × 0.75 × attacker land, floored at 10.
pub fn land_lost(attacker_land: i64, target_land: i64) -> i64 {
    let a = attacker_land as f64;
    let ratio = target_land as f64 / a.max(1.0);
    let base = if ratio < 0.55 {
        0.304 * ratio * ratio - 0.227 * ratio + 0.048
    } else if ratio < 0.75 {
        0.154 * ratio - 0.069
    } else {
        0.129 * ratio - 0.048
    };
    let acres = base * LAND_LOSS_MULTIPLIER * a; // war multiplier = 1
    crate::rounding::rfloor(acres).max(10)
}

/// Total acres the attacker GAINS = conquered + generated (bonus) land. Non-repeat:
/// generated = conquered × LAND_GEN_RATIO, so gained = conquered × (1 + LAND_GEN_RATIO).
pub fn land_gained(attacker_land: i64, target_land: i64) -> i64 {
    let conquered = land_lost(attacker_land, target_land);
    conquered + crate::rounding::round_int(conquered as f64 * LAND_GEN_RATIO)
}

/// Does `attacker` (sending all its offensive units) break `target`? OP > DP-with-temples.
pub fn invasion_successful(attacker: &DominionState, target: &DominionState) -> bool {
    offensive_power_combat(attacker, target) > defensive_power_with_temples(target, attacker)
}

/// Attacker overwhelmed: a failed hit where OP fell ≥ 20% short of DP (bonus attacker
/// casualties / reduced defender casualties). Never true on a success.
pub fn is_overwhelmed(attacker: &DominionState, target: &DominionState) -> bool {
    let op = offensive_power_combat(attacker, target);
    let dp = defensive_power_with_temples(target, attacker);
    if op > dp || dp <= 0.0 {
        return false;
    }
    (1.0 - op / dp) >= OVERWHELMED_PERCENTAGE
}

// ----------------------------------------------------------------------------
// Prestige & morale (PrestigeCalculator / InvadeActionService). Base case: no
// hero/wonder/war perks; morale-scaled gain.
// ----------------------------------------------------------------------------

/// Attacker prestige GAINED on a successful 75–119-range hit (PrestigeCalculator::
/// getPrestigeGain = round(raw × multiplier)).
pub fn prestige_gain(attacker: &DominionState, target: &DominionState) -> i64 {
    let al = calc::total_land(attacker).max(1) as f64;
    let tl = calc::total_land(target) as f64;
    let range = tl / al;
    let raw = (range * PRESTIGE_RANGE_MULTIPLIER + PRESTIGE_CHANGE_BASE).min(PRESTIGE_CAP)
        + (tl + PRESTIGE_LAND_BASE).max(0.0) / PRESTIGE_LAND_FACTOR;
    let mult = 1.0 - (100 - attacker.morale) as f64 / 100.0
        + (calc::race_perk(attacker, "prestige_gains") + calc::tp(attacker, "prestige_gains"))
            / 100.0;
    round_int(raw * mult)
}

/// Target prestige LOST on a successful hit (5% of its prestige, capped at the
/// attacker's gain). Returned negative.
pub fn prestige_loss(target: &DominionState, attacker_gain: i64) -> i64 {
    let loss =
        (target.prestige as f64 * PRESTIGE_LOSS_PERCENTAGE / 100.0).min(attacker_gain as f64);
    round_int(-loss)
}

/// Attacker prestige PENALTY on an overwhelmed / out-of-band failed hit: −5% of its
/// prestige, with a steeper scaling penalty for real targets below 60% range. Negative.
pub fn prestige_penalty(
    attacker: &DominionState,
    target: &DominionState,
    target_has_user: bool,
) -> i64 {
    let al = calc::total_land(attacker).max(1) as f64;
    let tl = calc::total_land(target) as f64;
    let range = tl / al;
    let mut loss = attacker.prestige as f64 * -(PRESTIGE_LOSS_PERCENTAGE / 100.0);
    if target_has_user && range < 0.60 {
        let scaling = 16.0 / (range * range);
        loss = -(attacker.prestige as f64).min((-loss).max(scaling));
    }
    round_int(loss)
}

// ----------------------------------------------------------------------------
// Casualties (InvadeActionService::handleOffensive/DefensiveCasualties). BASE CASE:
// no fixed/immortal/conversion unit perks and no recent invasions (modifier = 1), so
// the per-slot casualty multipliers are 1.0. `units` is the sent army [u1..u4].
// ----------------------------------------------------------------------------

/// Per-slot casualty multiplier (CasualtiesCalculator::get{Offensive,Defensive}-
/// CasualtiesMultiplierForUnitSlot), base engine: no spell/tech/wonder/hero terms, so
/// the dominion-wide multiplier is 1.0. Handles immortal (→ 0), immortal_vs_land_range
/// (offense only), the flat `casualties` / `casualties_{offense,defense}` reduction, and
/// the `reduce_combat_losses` pairing addition. `units` = the sent army (offense path).
fn casualty_multiplier(
    s: &DominionState,
    slot: usize,
    land_ratio: f64,
    units: &[i64; 4],
    offensive: bool,
) -> f64 {
    if calc::unit_perk_scalar(s, slot, "immortal") != 0.0 {
        return 0.0;
    }
    if offensive {
        let ivlr = calc::unit_perk_scalar(s, slot, "immortal_vs_land_range");
        if ivlr != 0.0 && land_ratio >= ivlr / 100.0 {
            return 0.0;
        }
    }
    // Flat reduction: `casualties` (both directions) takes precedence, else the
    // direction-specific perk. Value is negative (e.g. -25) → a positive reduction.
    let dir = if offensive {
        "casualties_offense"
    } else {
        "casualties_defense"
    };
    let cas = {
        let c = calc::unit_perk_scalar(s, slot, "casualties");
        if c != 0.0 {
            c
        } else {
            calc::unit_perk_scalar(s, slot, dir)
        }
    };
    let mut unit_bonus = -cas / 100.0;
    // reduce_combat_losses pairing (none in the active race set; faithful if added).
    if let Some(rcl) =
        (1..=4).find(|&sl| calc::unit_perk_scalar(s, sl, "reduce_combat_losses") != 0.0)
    {
        let (cnt, total) = if offensive {
            (units[rcl - 1], units.iter().sum::<i64>())
        } else {
            let total = (1..=4)
                .map(|sl| calc::military_slot_count(s, sl))
                .sum::<i64>()
                + s.military_draftees;
            (calc::military_slot_count(s, rcl), total)
        };
        if total > 0 {
            unit_bonus += (cnt as f64 / total as f64) / 2.0;
        }
    }
    1.0 * (1.0 - unit_bonus) // nonUnitBonusMultiplier (1.0) × unit-bonus factor
}

/// Attacker units lost [u1..u4]. Base 8.5%; on a SUCCESS the killed count scales with the
/// share of the force needed to break the target, on a FAILURE the whole sent amount is
/// exposed; then × the per-slot multiplier. `fixed_casualties` units bypass all of this.
pub fn offensive_casualties(
    attacker: &DominionState,
    target: &DominionState,
    units: [i64; 4],
) -> [i64; 4] {
    let op = offensive_power_combat(attacker, target);
    let dp = defensive_power_with_temples(target, attacker);
    offensive_casualties_given(attacker, target, units, op, dp)
}

/// `offensive_casualties` with OP and DP supplied by the caller. Same per-slot multiplier
/// path (immortal → 0, `immortal_vs_land_range`, `fixed_casualties` bypass, flat
/// `casualties_*` reductions, `reduce_combat_losses`), reading `attacker`'s race/unit perks
/// — only the success flag and the needed-force share come from `op`/`dp`. For the intel
/// layer, which knows the sent OP and the target's temple-adjusted DP from scouted
/// multipliers it can't fold back into a full `DominionState`. `attacker` still needs its
/// race, the sent per-slot counts (for `reduce_combat_losses`), and land (land-ratio).
pub fn offensive_casualties_given(
    attacker: &DominionState,
    target: &DominionState,
    units: [i64; 4],
    op: f64,
    dp: f64,
) -> [i64; 4] {
    let total_sent: i64 = units.iter().sum();
    let mut lost = [0i64; 4];
    if total_sent <= 0 {
        return lost;
    }
    let success = op > dp;
    let pct = CASUALTIES_OFFENSIVE_BASE_PCT / 100.0;
    let avg_op = op / total_sent as f64;
    let needed = (dp / avg_op).round();
    let land_ratio = calc::total_land(target) as f64 / calc::total_land(attacker).max(1) as f64;
    for slot in 1..=4 {
        let amount = units[slot - 1];
        if amount == 0 {
            continue;
        }
        let fixed = calc::unit_perk_scalar(attacker, slot, "fixed_casualties");
        if fixed != 0.0 {
            lost[slot - 1] = crate::rounding::rceil(amount as f64 * fixed / 100.0);
            continue;
        }
        let unit_count = if success {
            (needed * (amount as f64 / total_sent as f64)).round()
        } else {
            amount as f64
        };
        let base = crate::rounding::rceil(unit_count * pct);
        let mult = casualty_multiplier(attacker, slot, land_ratio, &units, true);
        lost[slot - 1] = if (mult - 1.0).abs() > 1e-12 {
            crate::rounding::rceil(base as f64 * mult)
        } else {
            base
        };
    }
    lost
}

/// Defender losses on a hit: (draftees_lost, [u1..u4] lost). Base 3.6% × clamp(landRatio,
/// 0.4, 1) × (OP/DP), capped 4.8%; on a failure it scales linearly from 0 (overwhelmed)
/// to base at OP==DP. Per-slot multiplier then applies (immortal → 0; floored 0.9% when
/// the unit takes any casualties). Overwhelmed → none.
pub fn defensive_casualties(attacker: &DominionState, target: &DominionState) -> (i64, [i64; 4]) {
    let op = offensive_power_combat(attacker, target);
    let dp = defensive_power_with_temples(target, attacker);
    defensive_casualties_given(attacker, target, op, dp)
}

/// `defensive_casualties` with OP and DP supplied by the caller — see
/// `offensive_casualties_given` for why the intel layer needs this. Per-slot multipliers
/// still read `target`'s race/unit perks (immortal defenders → 0, flat `casualties_defense`,
/// etc.); `op`/`dp` only drive overwhelmed/scale.
pub fn defensive_casualties_given(
    attacker: &DominionState,
    target: &DominionState,
    op: f64,
    dp: f64,
) -> (i64, [i64; 4]) {
    let mut lost = [0i64; 4];
    // overwhelmed: failed hit where OP fell ≥ 20% short (mirrors `is_overwhelmed`, on the
    // supplied op/dp so it can't diverge from the casualty scaling below).
    if dp > 0.0 && op <= dp && (1.0 - op / dp) >= OVERWHELMED_PERCENTAGE {
        return (0, lost);
    }
    let success = op > dp;
    let mut pct = CASUALTIES_DEFENSIVE_BASE_PCT / 100.0;
    let land_ratio = calc::total_land(target) as f64 / calc::total_land(attacker).max(1) as f64;
    if success {
        pct *= crate::rounding::clamp(land_ratio, 0.4, 1.0);
        pct *= op / dp;
        pct = pct.min(CASUALTIES_DEFENSIVE_MAX_PCT / 100.0);
    } else {
        // linear from 0% at overwhelmed (OP/DP = 0.8) to base at OP==DP, ×(1/0.2)=×5
        pct *= (op / dp - (1.0 - OVERWHELMED_PERCENTAGE)) * (1.0 / OVERWHELMED_PERCENTAGE);
    }
    let floor = CASUALTIES_DEFENSIVE_MIN_PCT / 100.0;
    // Draftees (null slot): no immortal/casualties perk; per-slot multiplier = 1 for the
    // active race set (no reduce_combat_losses). recent-invasion modifier = 1.
    let draftees_lost = crate::rounding::rfloor(target.military_draftees as f64 * pct.max(floor));
    for slot in 1..=4 {
        if calc::unit_defense(target, slot) == 0.0 {
            continue;
        }
        let m = casualty_multiplier(target, slot, land_ratio, &[0; 4], false);
        let final_pct = if m > 0.0 { (pct * m).max(floor) } else { 0.0 };
        lost[slot - 1] =
            crate::rounding::rfloor(calc::military_slot_count(target, slot) as f64 * final_pct);
    }
    (draftees_lost, lost)
}

/// Home defensive power AFTER sending `units` on offense — they leave home for ~12h, so
/// this is the attacker's counter-attack exposure. `defensive_power` on the home army.
pub fn defensive_power_after_send(attacker: &DominionState, units: [i64; 4]) -> f64 {
    let mut home = attacker.clone();
    home.military_unit1 -= units[0];
    home.military_unit2 -= units[1];
    home.military_unit3 -= units[2];
    home.military_unit4 -= units[3];
    calc::defensive_power(&home)
}

/// Population over the housing cap (max(0, population − max_population)). A dominion is
/// "overpopulated" when this is > 0 — e.g. a target whose max-pop dropped after losing
/// land in an invasion. Pure state metric (population & max_population are validated).
pub fn overpopulation_excess(s: &DominionState) -> i64 {
    (calc::population(s) - calc::max_population(s)).max(0)
}

/// Units the attacker CONVERTS from the breaking force on a successful hit
/// (InvadeActionService::handleConversions) — returns [→u1,→u2,→u3,→u4] gained. Base
/// case: no conversion-rate spell perks, no upgrade_casualties. Faithful to round-50's
/// hardcoded conversion-race list (note: spirit has the perk but does NOT convert).
pub fn conversions(attacker: &DominionState, target: &DominionState, units: [i64; 4]) -> [i64; 4] {
    let mut converted = [0i64; 4];
    if !invasion_successful(attacker, target) {
        return converted;
    }
    if !matches!(
        attacker.race.as_str(),
        "lycanthrope" | "undead" | "vampire" | "dark-elf-rework" | "undead-rework"
    ) {
        return converted;
    }
    let land_ratio =
        (calc::total_land(target) as f64 / calc::total_land(attacker).max(1) as f64).min(1.0);
    let conv_mult = 1.0; // + conversion_rate spell perks (0 base)
    let off_mod = calc::offensive_power_multiplier(attacker); // getOffensivePowerMultiplier (no morale)
    let mut target_dp = defensive_power_with_temples(target, attacker);

    // (sentSlot, convertIntoSlot, base_offense_power, conversionRate), highest rate first.
    let mut converters: Vec<(usize, usize, f64, f64)> = Vec::new();
    for slot in 1..=4 {
        if units[slot - 1] == 0 {
            continue;
        }
        if let Some(serde_json::Value::String(spec)) = calc::unit_perk(attacker, slot, "conversion")
        {
            let p: Vec<&str> = spec.split(',').collect();
            if p.len() < 2 {
                continue;
            }
            let convert_slot: usize = p[0].trim().parse().unwrap_or(0);
            let divisor: f64 = p[1].trim().parse().unwrap_or(0.0);
            if convert_slot < 1 || convert_slot > 4 || divisor == 0.0 {
                continue;
            }
            converters.push((
                slot,
                convert_slot,
                calc::unit_offense(attacker, slot),
                1.0 / divisor,
            ));
        }
    }
    converters.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));

    for (slot, convert_slot, power, rate) in converters {
        if target_dp <= 0.0 || power * off_mod <= 0.0 {
            continue;
        }
        let needed = crate::rounding::rceil(target_dp / (power * off_mod));
        let converting = needed.min(units[slot - 1]);
        target_dp -= converting as f64 * power * off_mod;
        let mut converts = converting as f64 * rate * conv_mult;
        if land_ratio < 0.75 {
            converts *= 1.25 * land_ratio * land_ratio; // bottom-feeding penalty
        }
        converted[convert_slot - 1] += crate::rounding::rfloor(converts);
    }
    converted
}

/// Morale the attacker loses for an invasion (InvadeActionService::handleMoraleChanges):
/// 5, increased for hitting weak targets (< 75% range). `range_pct` is the % range.
pub fn morale_cost(range_pct: f64) -> i64 {
    let mut change = 5.0;
    if range_pct < 75.0 {
        let adj = ((range_pct / 100.0 - 0.4) * 100.0 / 7.0) - 5.0;
        change -= crate::rounding::php_round(adj, 0).max(-5.0);
    }
    change as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_land(plain: i64) -> DominionState {
        let mut s = DominionState::default();
        s.land_plain = plain;
        s
    }

    #[test]
    fn temple_reduction_caps_at_27pct() {
        let mut s = with_land(1000);
        s.building_temple = 100; // 100/1000 = 10% → 1.35×10 = 13.5%
        assert!((temple_reduction(&s) - 0.135).abs() < 1e-9);
        s.building_temple = 300; // 30% → 1.35×30 = 40.5% → capped 27%
        assert!((temple_reduction(&s) - TEMPLE_DP_REDUCTION_MAX).abs() < 1e-9);
    }

    #[test]
    fn range_formula_and_band() {
        // target 800 vs attacker 1000 → 80%
        assert!((dominion_range(1000, 800) - 80.0).abs() < 1e-9);
        // MINIMUM_RANGE 0.4: in-range band is [40%, 250%]
        assert!(in_range(1000, 400)); // exactly 40%
        assert!(in_range(1000, 2500)); // exactly 250%
        assert!(!in_range(1000, 399)); // below band
        assert!(!in_range(1000, 2501)); // above band
    }

    #[test]
    fn morale_cost_base_and_weak_target() {
        assert_eq!(morale_cost(80.0), 5); // ≥75% range → flat 5
        assert_eq!(morale_cost(100.0), 5);
        // 60% range: adj = ((0.6−0.4)·100/7) − 5 = −2.143 → round −2 → 5 − (−2) = 7
        assert_eq!(morale_cost(60.0), 7);
    }

    #[test]
    fn dp_after_send_drops_home_defense() {
        let mut s = with_land(350);
        s.race = "human".to_string();
        s.military_unit2 = 1000; // archers stay home
        s.military_unit4 = 1000; // cavalry sent on offense
        let full = calc::defensive_power(&s);
        let after = defensive_power_after_send(&s, [0, 0, 0, 1000]);
        assert!(
            after < full,
            "sending units must lower home DP: {after} < {full}"
        );
    }

    #[test]
    fn overpopulation_excess_detects_overcrowding() {
        let mut s = with_land(350);
        s.race = "human".to_string();
        assert_eq!(overpopulation_excess(&s), 0); // empty dominion → no overpop
        s.military_unit1 = 5000; // far over the housing cap
        assert!(overpopulation_excess(&s) > 0);
    }
}
