//! Round-50 game constants and starting states. Every value is verified against
//! ../OpenDominion-source @ round-50 (see docs/SPEC.md for citations).

use crate::state::{DominionState, QueueEntry};

// --- protection ---
pub const ADVANCED_TICKS: i64 = 48;
pub const QUICK_TICKS: i64 = 36;

// --- production per building / per capita (ProductionCalculator) ---
pub const PEASANT_TAX: f64 = 2.7; // platinum per employed peasant
pub const PLAT_PER_ALCHEMY: f64 = 45.0;
pub const FOOD_PER_FARM: f64 = 80.0;
pub const FOOD_PER_DOCK: f64 = 40.0;
/// Base units carried per boat on invasion (MilitaryCalculator::UNITS_PER_BOAT).
/// Additive race/tech `boat_capacity` perks raise it (undead-rework +20 → 50).
pub const UNITS_PER_BOAT: f64 = 30.0;
/// Base attacker offensive casualty rate on a successful hit
/// (CASUALTIES_OFFENSIVE_BASE_PCT/100). Used by the QC sustained-throughput
/// objective to size the army the economy must keep replacing.
pub const OFFENSE_CASUALTY_RATE: f64 = 0.085;
pub const LUMBER_PER_LUMBERYARD: f64 = 50.0;
pub const LUMBER_PER_FOREST_HAVEN: f64 = 25.0;
pub const ORE_PER_MINE: f64 = 60.0;
pub const MANA_PER_TOWER: f64 = 25.0;
pub const MANA_PER_WIZARD_GUILD: f64 = 5.0;
pub const GEMS_PER_DIAMOND_MINE: f64 = 15.0;
pub const FOOD_CONSUMPTION_PER_CAPITA: f64 = 0.25;
pub const FOOD_DECAY: f64 = 0.01;
pub const LUMBER_DECAY: f64 = 0.01;
pub const MANA_DECAY: f64 = 0.02;
pub const PLAT_PRODUCTION_MAX_BONUS: f64 = 0.50; // +50% cap

// --- housing (PopulationCalculator) ---
pub const HOUSING_HOME: i64 = 30;
pub const HOUSING_NONHOME: i64 = 15;
pub const HOUSING_BARRACKS: i64 = 0;
pub const HOUSING_CONSTRUCTING: i64 = 15;
pub const HOUSING_BARREN: i64 = 5;
pub const BARRACKS_MILITARY_HOUSING: i64 = 36;

// --- jobs / employment ---
pub const JOBS_PER_BUILDING: i64 = 20;

// --- ratio-of-land capped building bonuses ---
// Each is `min(count/land * COEF, MAX)`, so the bonus saturates at count/land = MAX/COEF.
// Defined once here and shared by the bonus calculators + the app's "buildings from cap"
// readout, so the cap math is never re-hardcoded.
pub const GUARD_TOWER_DP_COEF: f64 = 1.6; // +DP modifier
pub const GUARD_TOWER_DP_MAX: f64 = 0.32;
pub const GRYPHON_NEST_OP_COEF: f64 = 1.6; // +OP modifier (same shape as guard towers)
pub const GRYPHON_NEST_OP_MAX: f64 = 0.32;

// --- combat: attacker's temples reduce the defender's DP multiplier (MilitaryCalculator) ---
pub const TEMPLE_DP_REDUCTION_COEF: f64 = 1.35; // -DP-multiplier per 1% of land as temples
pub const TEMPLE_DP_REDUCTION_MAX: f64 = 0.27;
// --- combat: realm range (RangeCalculator::MINIMUM_RANGE), no guard membership ---
pub const MINIMUM_RANGE: f64 = 0.4;
// --- combat: land conquered (MilitaryCalculator) + invasion outcome (InvadeActionService) ---
pub const LAND_LOSS_MULTIPLIER: f64 = 0.75; // MilitaryCalculator::LAND_LOSS_MULTIPLIER
pub const LAND_GEN_RATIO: f64 = 1.00; // bonus (generated) land per conquered acre
pub const OVERWHELMED_PERCENTAGE: f64 = 0.20; // attacker overwhelmed if (1 − OP/DP) ≥ 20%
                                              // --- combat: prestige (PrestigeCalculator) ---
pub const PRESTIGE_CAP: f64 = 70.0;
pub const PRESTIGE_RANGE_MULTIPLIER: f64 = 200.0;
pub const PRESTIGE_CHANGE_BASE: f64 = -115.0;
pub const PRESTIGE_LAND_FACTOR: f64 = 100.0;
pub const PRESTIGE_LAND_BASE: f64 = -750.0;
pub const PRESTIGE_LOSS_PERCENTAGE: f64 = 5.0;
// --- combat: casualties (InvadeActionService percentages) ---
pub const CASUALTIES_OFFENSIVE_BASE_PCT: f64 = 8.5;
pub const CASUALTIES_DEFENSIVE_BASE_PCT: f64 = 3.6;
pub const CASUALTIES_DEFENSIVE_MAX_PCT: f64 = 4.8;
pub const CASUALTIES_DEFENSIVE_MIN_PCT: f64 = 0.9;
pub const SMITHY_DISCOUNT_COEF: f64 = 2.0; // -military training cost
pub const SMITHY_DISCOUNT_MAX: f64 = 0.36;
pub const FACTORY_DISCOUNT_COEF: f64 = 5.0; // -construction/rezone cost
pub const FACTORY_DISCOUNT_MAX: f64 = 0.50;
pub const SCHOOL_MAX_LAND_RATIO: f64 = 0.5; // schools past land*0.5 add no tech

// --- population growth ---
pub const PEASANT_GROWTH_RATE: f64 = 0.03;
pub const MAX_PEASANT_DEATH_RATE: f64 = 0.05;
pub const DRAFTEE_GROWTH_RATE: f64 = 0.01;

// --- defense floor (MilitaryCalculator::getMinimumDefense) ---
pub fn minimum_defense(total_land: i64) -> f64 {
    (10.0 * total_land as f64 - 3250.0).max(750.0)
}

// --- queue delays (ticks) ---
pub const EXPLORE_DELAY: i64 = 12;
pub const CONSTRUCT_DELAY: i64 = 12;
pub const TRAIN_DELAY_DEFAULT: i64 = 12;
pub const TRAIN_DELAY_SPECIALIST: i64 = 9; // military_unit1 / military_unit2

/// Round-50 "advanced" starting state (the explorer mode). State at creation,
/// before the opening build is placed. protection_ticks_remaining starts at
/// ADVANCED_TICKS + 1 because tick #1 is the building phase.
pub fn advanced_start() -> DominionState {
    DominionState {
        race: "human".to_string(),
        resource_platinum: 120_000,
        resource_food: 15_000,
        resource_lumber: 15_000,
        peasants: 1_000,
        military_draftees: 300,
        land_plain: 350,
        prestige: 250,
        morale: 100,
        draft_rate: 90,
        wizard_strength: 100,
        highest_land_achieved: 350,
        protection_type: "advanced".to_string(),
        protection_ticks: ADVANCED_TICKS,
        protection_ticks_remaining: ADVANCED_TICKS + 1,
        ..Default::default()
    }
}

/// Starting state with optional late-start bonuses (registration on day >=2).
/// getLateStartAttributes grants per extra day: +10k plat, +1.5k food, +2.5k lumber,
/// +2.5k ore, +1k mana, +25k gems, +2.5k tech, +25 boats, +120 draftees.
pub fn start_state(protection_type: &str, days_late: i64, race: &str) -> DominionState {
    let mut s = if protection_type == "quick" {
        quick_start()
    } else {
        advanced_start()
    };
    if !race.is_empty() {
        s.race = race.to_ascii_lowercase();
    }
    let d = days_late.max(0);
    if d > 0 {
        s.resource_platinum += 10_000 * d;
        s.resource_food += 1_500 * d;
        s.resource_lumber += 2_500 * d;
        s.resource_ore += 2_500 * d;
        s.resource_mana += 1_000 * d;
        s.resource_gems += 25_000 * d;
        s.resource_tech += 2_500 * d;
        s.resource_boats += 25.0 * d as f64;
        s.military_draftees += 120 * d;
    }
    s
}

/// Round-50 "quick" starting state (500 plain, +60 plain incoming, 36 ticks).
pub fn quick_start() -> DominionState {
    DominionState {
        race: "human".to_string(),
        resource_platinum: 50_000,
        resource_food: 15_000,
        resource_lumber: 15_000,
        peasants: 1_000,
        military_draftees: 150,
        land_plain: 500,
        prestige: 250,
        morale: 100,
        draft_rate: 90,
        wizard_strength: 100,
        highest_land_achieved: 560,
        protection_type: "quick".to_string(),
        protection_ticks: QUICK_TICKS,
        protection_ticks_remaining: QUICK_TICKS + 1,
        queue: vec![QueueEntry {
            source: "exploration".to_string(),
            resource: "land_plain".to_string(),
            hours: 12,
            amount: 60,
        }],
        ..Default::default()
    }
}
