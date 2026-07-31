//! Data-driven game content loaded from the pinned game source. Ruleset
//! provenance and the live roster travel with every compiled engine.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use include_dir::{include_dir, Dir};
use serde::Deserialize;
use serde_json::Value;

/// Stable identity for the game-data package compiled into this binary.
pub const ENGINE_RULESET_ID: &str = "round51";
pub const ENGINE_RULESET_ROUND: i64 = 51;
pub const ENGINE_SOURCE_TAG: &str = "1.51.0";
pub const ENGINE_SOURCE_COMMIT: &str = "35b977df6b47fd24636f920657b5c4edb46bbff7";

#[derive(Deserialize, Default, Clone)]
pub struct Tech {
    #[serde(default)]
    pub name: String,
    /// Grid position in the current tech-tree screen (mirrors the PHP `techs.x/y`).
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
    /// Unit perks (e.g. "defense_from_land": "forest,20,4.5",
    /// "ore_production": 0.5, "not_trainable": 1). Values are scalars or
    /// comma-separated strings, parsed per-perk by `calc`.
    #[serde(default)]
    pub perks: HashMap<String, Value>,
    /// Whether this unit needs a boat to be sent on invasion. Absent in the data ⇒
    /// true by default; flying/amphibious units set `need_boat: false`.
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
    /// Live in the pinned source data? Mirrors PHP `Race.playable`
    /// (`Race::where('playable', true)`). Source-playable can still be overridden
    /// by the ruleset's live administrative exclusions. Default true.
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
    /// Effect perks (e.g. "ore_production": 20). Values are numeric.
    #[serde(default)]
    pub perks: HashMap<String, f64>,
    /// Races allowed to cast it; `None` = common (any race).
    #[serde(default)]
    pub races: Option<Vec<String>>,
    /// Live in the current round? Disabled entries use `active:false`.
    #[serde(default = "default_true")]
    pub active: bool,
}

fn default_true() -> bool {
    true
}

/// Provenance and any live administrative roster override not represented by
/// the source `playable` flags.
#[derive(serde::Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RulesetConfig {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub round: i64,
    #[serde(default)]
    pub source_tag: String,
    #[serde(default)]
    pub source_commit: String,
    #[serde(default)]
    pub disabled_despite_playable: Vec<String>,
    #[serde(default)]
    pub production_overrides: HashMap<String, String>,
}

pub struct GameData {
    pub techs: HashMap<String, Tech>,
    pub races: HashMap<String, Race>,
    pub spells: HashMap<String, Spell>,
    pub ruleset: RulesetConfig,
    /// Source-playable races disabled by live administration.
    pub live_disabled: HashSet<String>,
}

static DATA: OnceLock<GameData> = OnceLock::new();

pub fn get() -> &'static GameData {
    DATA.get_or_init(load)
}

/// The pinned game data, EMBEDDED into the binary at compile time. A shipped app therefore needs
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
    for file in DATA_DIR
        .get_dir("races")
        .expect("embedded data/races")
        .files()
    {
        if file.path().extension().and_then(|e| e.to_str()) == Some("json") {
            let r: Race = serde_json::from_str(file.contents_utf8().expect("utf8 race json"))
                .unwrap_or_else(|e| panic!("parse {:?}: {e}", file.path()));
            races.insert(r.key.clone(), r);
        }
    }
    let spells: HashMap<String, Spell> =
        serde_json::from_str(embedded("spells.json")).expect("parse spells.json");
    let ruleset: RulesetConfig =
        serde_json::from_str(embedded("ruleset.json")).expect("parse ruleset.json");
    let live_disabled: HashSet<String> =
        ruleset.disabled_despite_playable.iter().cloned().collect();
    GameData {
        techs,
        races,
        spells,
        ruleset,
        live_disabled,
    }
}

/// Is `race_key` offered by the current live ruleset?
pub fn is_live_race(race_key: &str) -> bool {
    let d = get();
    d.races.get(race_key).map(|r| r.playable).unwrap_or(false)
        && !d.live_disabled.contains(race_key)
}

/// All race keys offered by the current live ruleset, unsorted.
pub fn live_race_keys() -> Vec<String> {
    let d = get();
    d.races
        .values()
        .filter(|r| r.playable && !d.live_disabled.contains(&r.key))
        .map(|r| r.key.clone())
        .collect()
}

/// Resolve either an engine spell key or the game's display name to the
/// canonical data key.
pub fn spell_key_for_name(name: &str) -> Option<&'static str> {
    let needle = name.trim();
    get()
        .spells
        .iter()
        .find(|(key, spell)| {
            key.eq_ignore_ascii_case(needle) || spell.name.eq_ignore_ascii_case(needle)
        })
        .map(|(key, _)| key.as_str())
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
/// protection (`!protection_finished`) — not flagged `invalid_protection`. Mirrors
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
    const SEND_OFFENSE_PERKS: &[&str] = &[
        "offense",
        "offense_from_barren_land",
        "offense_from_spell",
        "offense_unit1",
    ];
    let mut out: Vec<&'static str> = get()
        .spells
        .iter()
        .filter(|(key, sp)| {
            sp.races.is_some() // racial only (skips universal/optional self-spells)
                && SEND_OFFENSE_PERKS
                    .iter()
                    .any(|perk| sp.perks.get(*perk).copied().unwrap_or(0.0) > 0.0)
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
/// The live game treats a tech's `requires` list as adjacent unlock routes: no
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
    fn current_ruleset_and_live_roster_are_source_faithful() {
        assert_eq!(get().ruleset.id, ENGINE_RULESET_ID);
        assert_eq!(get().ruleset.round, ENGINE_RULESET_ROUND);
        assert_eq!(get().ruleset.source_tag, ENGINE_SOURCE_TAG);
        assert_eq!(get().ruleset.source_commit, ENGINE_SOURCE_COMMIT);
        let live: HashSet<String> = live_race_keys().into_iter().collect();
        assert_eq!(live.len(), 21, "round 51 offers 21 races");

        // Reworked races: the LIVE variant is the `*-rework` key, never the classic.
        for (classic, rework) in [
            ("undead", "undead-rework"),
            ("kobold", "kobold-rework"),
            ("dark-elf", "dark-elf-rework"),
            ("nomad", "nomad-rework"),
            ("spirit", "spirit-rework"),
            ("wood-elf", "wood-elf-rework"),
        ] {
            assert!(is_live_race(rework), "{rework} should be live");
            assert!(live.contains(rework));
            assert!(
                !is_live_race(classic),
                "{classic} (classic) is not in the live ruleset"
            );
            assert!(!live.contains(classic));
        }

        // Planewalker is disabled directly in the tagged source.
        assert!(!get()
            .races
            .get("planewalker")
            .map(|r| r.playable)
            .unwrap_or(false));
        assert!(!is_live_race("planewalker"));
        assert!(!live.contains("planewalker"));

        // Legacy / nox variants are excluded too.
        for dead in ["undead-legacy", "spirit-legacy", "nox"] {
            assert!(!is_live_race(dead) && !live.contains(dead));
        }
    }
}
