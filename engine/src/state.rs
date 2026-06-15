//! Dominion state — field names mirror the round-50 `dominions` DB columns so that
//! engine output can be diffed directly against the PHP oracle's golden vectors.

use serde::{Deserialize, Serialize};

/// One pending entry in the action queue (explore/construction/training/...).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct QueueEntry {
    pub source: String,   // "exploration" | "construction" | "training" | ...
    pub resource: String, // "land_plain" | "building_home" | "military_spies" | ...
    pub hours: i64,       // ticks remaining until it resolves
    pub amount: i64,
}

/// An active self-spell (e.g. "midas_touch") with remaining duration in ticks.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ActiveSpell {
    pub key: String,
    pub duration: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DominionState {
    // race key (lowercase, matches data/races/*.json "key"); drives unit defense
    // values, training costs, and racial production perks. Set at start.
    #[serde(default)]
    pub race: String,

    // --- resources ---
    pub resource_platinum: i64,
    pub resource_food: i64,
    pub resource_lumber: i64,
    pub resource_ore: i64,
    pub resource_mana: i64,
    pub resource_gems: i64,
    pub resource_tech: i64,
    pub resource_boats: f64,

    // --- population & military ---
    pub peasants: i64,
    pub military_draftees: i64,
    pub military_unit1: i64,
    pub military_unit2: i64,
    pub military_unit3: i64,
    pub military_unit4: i64,
    pub military_spies: i64,
    pub military_assassins: i64,
    pub military_wizards: i64,
    pub military_archmages: i64,

    // --- land (barren + built; per type) ---
    pub land_plain: i64,
    pub land_mountain: i64,
    pub land_swamp: i64,
    pub land_cavern: i64,
    pub land_forest: i64,
    pub land_hill: i64,
    pub land_water: i64,

    // --- buildings (19 types) ---
    pub building_home: i64,
    pub building_alchemy: i64,
    pub building_farm: i64,
    pub building_smithy: i64,
    pub building_masonry: i64,
    pub building_ore_mine: i64,
    pub building_gryphon_nest: i64,
    pub building_tower: i64,
    pub building_wizard_guild: i64,
    pub building_temple: i64,
    pub building_diamond_mine: i64,
    pub building_school: i64,
    pub building_lumberyard: i64,
    pub building_forest_haven: i64,
    pub building_factory: i64,
    pub building_guard_tower: i64,
    pub building_shrine: i64,
    pub building_barracks: i64,
    pub building_dock: i64,

    // --- improvements (invested points) ---
    pub improvement_science: i64,
    pub improvement_keep: i64,
    pub improvement_spires: i64,
    pub improvement_forges: i64,
    pub improvement_walls: i64,
    pub improvement_harbor: i64,

    // --- misc / status ---
    pub morale: i64,
    pub prestige: i64,
    pub draft_rate: i64,
    pub wizard_strength: i64,
    // Fractional accumulator for racial hourly unit production (Dark Elf
    // Spellwright's Calling: wizard guilds produce 0.05 Adepts/hr). Mirrors the
    // PHP `dominions.racial_value` column — carries the sub-unit remainder across
    // ticks so building counts not divisible by 20 still produce over time.
    #[serde(default)]
    pub racial_value: f64,
    pub highest_land_achieved: i64,
    pub protection_type: String,
    pub protection_ticks: i64,
    pub protection_ticks_remaining: i64,
    pub protection_finished: bool,
    pub daily_platinum: bool,
    pub daily_land: bool,

    // --- conquered/lost land trackers (drive explored-land cost split) ---
    pub stat_total_land_conquered: i64,
    pub stat_total_land_lost: i64,

    // --- queues & active spells ---
    pub queue: Vec<QueueEntry>,
    pub spells: Vec<ActiveSpell>,
    pub techs: Vec<String>,
}

impl DominionState {
    /// Sum of all built buildings (across every type).
    pub fn total_buildings(&self) -> i64 {
        self.building_home
            + self.building_alchemy
            + self.building_farm
            + self.building_smithy
            + self.building_masonry
            + self.building_ore_mine
            + self.building_gryphon_nest
            + self.building_tower
            + self.building_wizard_guild
            + self.building_temple
            + self.building_diamond_mine
            + self.building_school
            + self.building_lumberyard
            + self.building_forest_haven
            + self.building_factory
            + self.building_guard_tower
            + self.building_shrine
            + self.building_barracks
            + self.building_dock
    }

    /// Total land across all types.
    pub fn total_land(&self) -> i64 {
        self.land_plain
            + self.land_mountain
            + self.land_swamp
            + self.land_cavern
            + self.land_forest
            + self.land_hill
            + self.land_water
    }
}
