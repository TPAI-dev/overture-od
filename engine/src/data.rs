//! Data-driven game content loaded from `data/` (exported from the round-50 game
//! tables). Loaded once via OnceLock. Keeps techs/races/units
//! out of hard-coded constants and enables all-race support.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use include_dir::{include_dir, Dir};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, Default, Clone)]
pub struct Tech {
    #[serde(default)]
    pub name: String,
    /// Grid position in the round-50 tech-tree screen (mirrors the PHP `techs.x/y`).
    /// Display-only — the engine math is position-agnostic — but surfaced so the app can
    /// render the spatial graph instead of a flat list. Was previously dropped at load.
    #[serde(default)]
    pub x: i64,
    #[serde(default)]
    pub y: i64,
    #[serde(default)]
    pub perks: HashMap<String, f64>,
    #[serde(default)]
    pub requires: Vec<String>,
}

#[derive(Deserialize, Default, Clone)]
pub struct UnitData {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub cost: HashMap<String, i64>,
    #[serde(default)]
    pub power: HashMap<String, f64>,
    /// Round-50 unit perks (e.g. "defense_from_land": "forest,20,4.5",
    /// "ore_production": 0.5, "not_trainable": 1). Values are scalars or
    /// comma-separated strings, parsed per-perk by `calc`.
    #[serde(default)]
    pub perks: HashMap<String, Value>,
    /// Whether this unit needs a boat to be sent on invasion. Absent in the data ⇒
    /// true (the round-50 default); flying/amphibious units set `need_boat: false`.
    #[serde(default = "default_true")]
    pub need_boat: bool,
}

#[derive(Deserialize, Default, Clone)]
pub struct Race {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub home_land_type: String,
    #[serde(default)]
    pub perks: HashMap<String, f64>,
    #[serde(default)]
    pub units: Vec<UnitData>,
    /// Live in the round-50 *source data*? Mirrors PHP `Race.playable`
    /// (`Race::where('playable', true)`); round-50 marks classic/legacy variants
    /// `playable:false`. NB: source-playable is NOT the same as enabled in the
    /// *live* round — see `GameData::round50_disabled` / `is_round50_live` for the
    /// admin override (Planewalker is source-playable but disabled live). Default true.
    #[serde(default = "default_true")]
    pub playable: bool,
}

#[derive(Deserialize, Default, Clone)]
pub struct Spell {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub category: String,
    /// Mana cost coefficient: mana = round(cost_mana * total_land).
    #[serde(default)]
    pub cost_mana: f64,
    /// Active duration in ticks once cast.
    #[serde(default)]
    pub duration: i64,
    /// Cooldown in hours after casting. The live PHP checks action history; the sim
    /// tracks this explicitly when it needs cooldown-aware casting.
    #[serde(default)]
    pub cooldown: i64,
    /// Effect perks (e.g. "ore_production": 20). All round-50 values are numeric.
    #[serde(default)]
    pub perks: HashMap<String, f64>,
    /// Races allowed to cast it; `None` = common (any race).
    #[serde(default)]
    pub races: Option<Vec<String>>,
    /// Live in the current round? (round-50 disables some via active:false.)
    #[serde(default = "default_true")]
    pub active: bool,
}

fn default_true() -> bool {
    true
}

/// `data/round50.json`: the project-level "what is actually enabled in the LIVE
/// round 50" override. The source data marks 22 races `playable:true`, but the
/// live round runs 21 — Planewalker is playable in the source yet was disabled by
/// the admins. We keep each race's `playable` flag bit-exact with PHP (golden
/// vectors depend on it) and record the live-round delta HERE. See the note inside
/// round50.json before editing.
#[derive(Deserialize, Default)]
struct Round50Config {
    #[serde(default)]
    disabled_despite_playable: Vec<String>,
}

pub struct GameData {
    pub techs: HashMap<String, Tech>,
    pub races: HashMap<String, Race>,
    pub spells: HashMap<String, Spell>,
    /// Race keys that are `playable` in the source but DISABLED in the live
    /// round 50 (loaded from `data/round50.json`). Used by `is_round50_live`.
    pub round50_disabled: HashSet<String>,
}

static DATA: OnceLock<GameData> = OnceLock::new();

pub fn get() -> &'static GameData {
    DATA.get_or_init(load)
}

/// The round-50 game data, EMBEDDED into the binary at compile time. A shipped app therefore needs
/// no external files (it does not read the dev tree's `data/`, which only exists on the dev machine)
/// and the user cannot accidentally alter the engine's inputs. Updating the game data for a new round
/// = re-embedding it in a fresh release build. (Previously `load()` read `CARGO_MANIFEST_DIR/../data`
/// at runtime, so any shipped binary panicked on the first `data::get()`.)
static DATA_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../data");

fn embedded(name: &str) -> &'static str {
    DATA_DIR
        .get_file(name)
        .and_then(|f| f.contents_utf8())
        .unwrap_or_else(|| panic!("embedded data missing or non-utf8: {name}"))
}

fn load() -> GameData {
    let techs: HashMap<String, Tech> =
        serde_json::from_str(embedded("techs.json")).expect("parse techs.json");
    let mut races = HashMap::new();
    for file in DATA_DIR.get_dir("races").expect("embedded data/races").files() {
        if file.path().extension().and_then(|e| e.to_str()) == Some("json") {
            let r: Race = serde_json::from_str(file.contents_utf8().expect("utf8 race json"))
                .unwrap_or_else(|e| panic!("parse {:?}: {e}", file.path()));
            races.insert(r.key.clone(), r);
        }
    }
    let spells: HashMap<String, Spell> =
        serde_json::from_str(embedded("spells.json")).expect("parse spells.json");
    let round50_disabled: HashSet<String> =
        serde_json::from_str::<Round50Config>(embedded("round50.json"))
            .expect("parse round50.json")
            .disabled_despite_playable
            .into_iter()
            .collect();
    GameData {
        techs,
        races,
        spells,
        round50_disabled,
    }
}

/// Is `race_key` enabled in the LIVE round 50 (the 21-race roster)?
///
/// Two gates: (1) the race is `playable` in the round-50 source data (mirrors PHP
/// `Race::where('playable', true)` — classic/legacy variants are `false`), AND
/// (2) it is not in the round-50 admin-disabled override (`data/round50.json`,
/// currently just Planewalker). The engine can still *simulate* a disabled race
/// when handed its key directly (needed for fidelity / golden vectors); this only
/// governs what selection layers OFFER.
pub fn is_round50_live(race_key: &str) -> bool {
    let d = get();
    d.races.get(race_key).map(|r| r.playable).unwrap_or(false)
        && !d.round50_disabled.contains(race_key)
}

/// All race keys enabled in the live round 50 (the 21-race roster), unsorted.
/// Selection layers (OVERTURE picker, `python list_races`, full-round break-even
/// table) use this so only current/active races are offered.
pub fn round50_live_keys() -> Vec<String> {
    let d = get();
    d.races
        .values()
        .filter(|r| r.playable && !d.round50_disabled.contains(&r.key))
        .map(|r| r.key.clone())
        .collect()
}

/// Self-spell perk value (e.g. "ore_production" for an active spell), or 0.
pub fn spell_perk(spell: &str, perk: &str) -> f64 {
    get()
        .spells
        .get(spell)
        .and_then(|sp| sp.perks.get(perk))
        .copied()
        .unwrap_or(0.0)
}

fn nonstacking_spell_perk_value(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let max_value = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if max_value < 0.0 {
        values.iter().copied().fold(f64::INFINITY, f64::min)
    } else {
        max_value
    }
}

/// Resolve an active-spell perk using the PHP rule:
/// - same-key spell perks in the same category do not stack;
/// - positive/zero values use the maximum value;
/// - all-negative values use the minimum value;
/// - resolved category values are summed.
///
/// This mirrors `Dominion::getSpellPerkValue` plus `SpellCalculator::resolveSpellPerk`
/// for the categories represented in live spell effects.
pub fn resolved_spell_perk<'a, I>(spells: I, perk: &str) -> f64
where
    I: IntoIterator<Item = &'a str>,
{
    let d = get();
    let mut self_values = Vec::new();
    let mut hostile_values = Vec::new();
    let mut war_values = Vec::new();
    let mut friendly_values = Vec::new();
    let mut effect_values = Vec::new();

    for key in spells {
        let Some(spell) = d.spells.get(key) else {
            continue;
        };
        let Some(value) = spell.perks.get(perk).copied() else {
            continue;
        };
        match spell.category.as_str() {
            "self" => self_values.push(value),
            "hostile" => hostile_values.push(value),
            "war" => war_values.push(value),
            "friendly" => friendly_values.push(value),
            "effect" => effect_values.push(value),
            _ => {}
        }
    }

    nonstacking_spell_perk_value(&self_values)
        + nonstacking_spell_perk_value(&hostile_values)
        + nonstacking_spell_perk_value(&war_values)
        + nonstacking_spell_perk_value(&friendly_values)
        + nonstacking_spell_perk_value(&effect_values)
}

/// Mana-cost coefficient of a spell (cost_mana), or 0 if unknown.
pub fn spell_cost_mana(spell: &str) -> f64 {
    get()
        .spells
        .get(spell)
        .map(|sp| sp.cost_mana)
        .unwrap_or(0.0)
}

/// Active duration (ticks) of a spell once cast; defaults to 12.
pub fn spell_duration(spell: &str) -> i64 {
    get()
        .spells
        .get(spell)
        .map(|sp| sp.duration)
        .filter(|d| *d > 0)
        .unwrap_or(12)
}

/// Cooldown in hours after casting a spell, or 0 if the spell has no cooldown.
pub fn spell_cooldown(spell: &str) -> i64 {
    get().spells.get(spell).map(|sp| sp.cooldown).unwrap_or(0)
}

/// Can `race` cast `spell` given whether protection has finished? Live (active) self-spell,
/// either common (no race restriction) or listed for this race, and — when STILL under
/// protection (`!protection_finished`) — not flagged `invalid_protection`. Mirrors round-50
/// `SpellActionService::castSpell`: `if invalid_protection && !protection_finished → refuse`.
/// So an `invalid_protection` racial spell (e.g. Undead-rework's Death and Decay, Dark-Elf's
/// Spellwright's Calling) is refused during protection but becomes castable once out (post-OOP).
pub fn spell_castable_in_context(spell: &str, race: &str, protection_finished: bool) -> bool {
    let Some(sp) = get().spells.get(spell) else {
        return false;
    };
    if !sp.active || sp.category != "self" {
        return false;
    }
    if !protection_finished && sp.perks.get("invalid_protection").copied().unwrap_or(0.0) != 0.0 {
        return false;
    }
    match &sp.races {
        None => true,
        Some(rs) => rs.iter().any(|r| r == race),
    }
}

/// Can `race` cast `spell` during PROTECTION? Convenience for the protection-only callers
/// (the protection sim): `spell_castable_in_context(.., false)`. Refuses
/// `invalid_protection` spells. Behavior-identical to the original — every existing caller
/// keeps the protection-only semantics.
pub fn spell_castable(spell: &str, race: &str) -> bool {
    spell_castable_in_context(spell, race, false)
}

/// The RACE-SPECIFIC self-cast spells that buff `offense` and are live this round — the
/// always-on attacker war-cries (Howling, Crusade, Killing Rage, Bloodrage, Nightfall, …).
/// A live attacker is assumed to keep its racial OP buff up, so an intel estimate must fold
/// it in even when no Revelation op scouted it. COMMON (race-less) self-spells are excluded:
/// they're an optional per-player choice, not a standing racial assumption.
pub fn racial_offense_self_spells(race: &str) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = get()
        .spells
        .iter()
        .filter(|(key, sp)| {
            sp.races.is_some() // racial only (skips universal/optional self-spells)
                && sp.perks.get("offense").copied().unwrap_or(0.0) > 0.0
                && spell_castable_in_context(key, race, true) // self + active + allowed for this race
        })
        .map(|(k, _)| k.as_str())
        .collect();
    out.sort_unstable(); // deterministic order (HashMap iteration is not)
    out
}

/// Sum of a perk across a set of researched tech keys.
pub fn tech_perk(researched: &[String], perk: &str) -> f64 {
    let d = get();
    researched
        .iter()
        .filter_map(|k| d.techs.get(k))
        .filter_map(|t| t.perks.get(perk))
        .sum()
}

/// Is `key` unlockable from the researched set?
///
/// Round-50 treats a tech's `requires` list as adjacent unlock routes: no
/// prereq means available, otherwise any one listed prereq is enough.
pub fn tech_prereqs_met(key: &str, researched: &[String]) -> bool {
    let d = get();
    match d.techs.get(key) {
        None => false,
        Some(t) => {
            t.requires.is_empty() || t.requires.iter().any(|r| researched.iter().any(|x| x == r))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tech_requires_any_adjacent_prerequisite_not_all() {
        assert!(tech_prereqs_met("tech_1_1", &[]));
        assert!(tech_prereqs_met("tech_3_1", &["tech_1_1".to_string()]));
        assert!(!tech_prereqs_met("tech_3_1", &[]));
    }

    #[test]
    fn racial_offense_self_spells_are_the_always_on_war_cries() {
        // Kobold's Howling is its racial OP self-buff → always assumed up for an attacker estimate.
        assert_eq!(racial_offense_self_spells("kobold-rework"), vec!["howling"]);
        // Human/Nomad get Crusade; Goblin Killing Rage; Orc Bloodrage.
        assert_eq!(racial_offense_self_spells("human"), vec!["crusade"]);
        assert_eq!(racial_offense_self_spells("goblin"), vec!["killing_rage"]);
        assert_eq!(racial_offense_self_spells("orc"), vec!["bloodrage"]);
        // A race with no racial OP self-spell gets nothing auto-assumed.
        assert!(racial_offense_self_spells("lizardfolk").is_empty());
        // COMMON (race-less) self-spells like ares_call are NOT auto-assumed (optional choice).
        assert!(!racial_offense_self_spells("kobold-rework").contains(&"ares_call"));
    }

    #[test]
    fn round50_live_roster_is_21_reworks_only() {
        let live: HashSet<String> = round50_live_keys().into_iter().collect();
        assert_eq!(live.len(), 21, "the live round 50 runs 21 races");

        // Reworked races: the LIVE variant is the `*-rework` key, never the classic.
        for (classic, rework) in [
            ("undead", "undead-rework"),
            ("kobold", "kobold-rework"),
            ("dark-elf", "dark-elf-rework"),
            ("nomad", "nomad-rework"),
            ("spirit", "spirit-rework"),
            ("wood-elf", "wood-elf-rework"),
        ] {
            assert!(is_round50_live(rework), "{rework} should be live");
            assert!(live.contains(rework));
            assert!(
                !is_round50_live(classic),
                "{classic} (classic) is not in round 50"
            );
            assert!(!live.contains(classic));
        }

        // Planewalker is `playable` in the source but admin-disabled in round 50.
        assert!(get()
            .races
            .get("planewalker")
            .map(|r| r.playable)
            .unwrap_or(false));
        assert!(
            !is_round50_live("planewalker"),
            "planewalker is disabled in round 50"
        );
        assert!(!live.contains("planewalker"));

        // Legacy / nox variants are excluded too.
        for dead in ["undead-legacy", "spirit-legacy", "nox"] {
            assert!(!is_round50_live(dead) && !live.contains(dead));
        }
    }
}
