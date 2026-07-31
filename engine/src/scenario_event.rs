//! Explicit post-protection events for OVERTURE scenario planning.
//!
//! These events are intentionally separate from protection actions. They mutate
//! only a replayed [`DominionState`], are fully serializable with the plan, and
//! can therefore be added/removed without changing protection mechanics.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::calc;
use crate::combat;
use crate::state::{DominionState, QueueEntry};

pub const FIRST_EVENT_HOUR: i64 = 49;
pub const MAX_EVENT_HOUR: i64 = 528;
pub const LAND_RETURN_HOURS: i64 = 12;
pub const MIN_INVASION_MORALE: i64 = 80;

const LAND_TYPES: [&str; 7] = [
    "plain", "swamp", "hill", "mountain", "forest", "cavern", "water",
];

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ScenarioEvent {
    #[serde(rename = "invasion")]
    Invasion {
        #[serde(default)]
        id: String,
        hour: i64,
        #[serde(rename = "targetLand")]
        target_land: i64,
        #[serde(rename = "targetDp")]
        target_dp: f64,
        sent: [i64; 4],
        #[serde(rename = "landByType", default)]
        land_by_type: HashMap<String, i64>,
        #[serde(default)]
        prestige: i64,
        #[serde(rename = "casualtiesOverride", default)]
        casualties_override: Option<[i64; 4]>,
    },
    #[serde(rename = "prestige")]
    Prestige {
        #[serde(default)]
        id: String,
        hour: i64,
        amount: i64,
    },
}

impl ScenarioEvent {
    pub fn id(&self) -> &str {
        match self {
            Self::Invasion { id, .. } | Self::Prestige { id, .. } => id,
        }
    }

    pub fn hour(&self) -> i64 {
        match self {
            Self::Invasion { hour, .. } | Self::Prestige { hour, .. } => *hour,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.id().trim().is_empty() {
            return Err("scenario event id must not be empty".to_string());
        }
        let hour = self.hour();
        if !(FIRST_EVENT_HOUR..=MAX_EVENT_HOUR).contains(&hour) {
            return Err(format!(
                "scenario event hour must be between {FIRST_EVENT_HOUR} and {MAX_EVENT_HOUR}, got {hour}"
            ));
        }
        match self {
            Self::Prestige { amount, .. } => {
                if *amount == 0 {
                    return Err("prestige event amount must be non-zero".to_string());
                }
            }
            Self::Invasion {
                target_land,
                target_dp,
                sent,
                land_by_type,
                prestige,
                casualties_override,
                ..
            } => {
                if *target_land <= 0 {
                    return Err("invasion target land must be positive".to_string());
                }
                if !target_dp.is_finite() || *target_dp < 0.0 {
                    return Err(
                        "invasion target DP must be a finite non-negative number".to_string()
                    );
                }
                if sent.iter().any(|n| *n < 0) {
                    return Err(
                        "invasion must send at least one troop and no negative counts".to_string(),
                    );
                }
                let sent_total = sent
                    .iter()
                    .try_fold(0i64, |total, count| total.checked_add(*count))
                    .ok_or_else(|| "invasion sent-troop total is too large".to_string())?;
                if sent_total == 0 {
                    return Err("invasion must send at least one troop".to_string());
                }
                if *prestige < 0 {
                    return Err(
                        "invasion prestige cannot be negative; use a prestige adjustment event"
                            .to_string(),
                    );
                }
                for (land, amount) in land_by_type {
                    if !LAND_TYPES.contains(&land.as_str()) {
                        return Err(format!("unknown conquered land type: \"{land}\""));
                    }
                    if *amount < 0 {
                        return Err(format!("negative conquered {land} land: {amount}"));
                    }
                }
                land_by_type
                    .values()
                    .try_fold(0i64, |total, amount| total.checked_add(*amount))
                    .ok_or_else(|| "conquered land total is too large".to_string())?;
                if let Some(casualties) = casualties_override {
                    for slot in 0..4 {
                        if casualties[slot] < 0 || casualties[slot] > sent[slot] {
                            return Err(format!(
                                "slot {} casualty override must be between 0 and the {} troops sent",
                                slot + 1,
                                sent[slot]
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventOutcome {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub hour: i64,
    pub sent: [i64; 4],
    pub calculated_casualties: [i64; 4],
    pub casualties: [i64; 4],
    pub survivors: [i64; 4],
    pub return_hours: [i64; 4],
    pub land_by_type: BTreeMap<String, i64>,
    pub land_total: i64,
    pub land_return_hour: Option<i64>,
    pub prestige: i64,
    pub prestige_return_hour: Option<i64>,
    pub op: f64,
    pub target_dp: f64,
    pub range_pct: f64,
    pub morale_delta: i64,
    pub population_freed: i64,
    pub manual_override: bool,
    pub boats_sent: i64,
}

impl EventOutcome {
    fn prestige(id: &str, hour: i64, amount: i64) -> Self {
        Self {
            id: id.to_string(),
            event_type: "prestige".to_string(),
            hour,
            sent: [0; 4],
            calculated_casualties: [0; 4],
            casualties: [0; 4],
            survivors: [0; 4],
            return_hours: [0; 4],
            land_by_type: BTreeMap::new(),
            land_total: 0,
            land_return_hour: None,
            prestige: amount,
            prestige_return_hour: Some(hour),
            op: 0.0,
            target_dp: 0.0,
            range_pct: 0.0,
            morale_delta: 0,
            population_freed: 0,
            manual_override: false,
            boats_sent: 0,
        }
    }
}

pub fn validate_events(events: &[ScenarioEvent]) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    for event in events {
        event.validate()?;
        if !ids.insert(event.id()) {
            return Err(format!("duplicate scenario event id: \"{}\"", event.id()));
        }
    }
    Ok(())
}

pub fn apply_events_for_hour(
    state: &mut DominionState,
    events: &[ScenarioEvent],
    hour: i64,
) -> Result<Vec<EventOutcome>, String> {
    let mut outcomes = Vec::new();
    for event in events.iter().filter(|event| event.hour() == hour) {
        outcomes.push(apply_event(state, event)?);
    }
    Ok(outcomes)
}

pub fn apply_event(
    state: &mut DominionState,
    event: &ScenarioEvent,
) -> Result<EventOutcome, String> {
    event.validate()?;
    match event {
        ScenarioEvent::Prestige { id, hour, amount } => {
            let next = state
                .prestige
                .checked_add(*amount)
                .ok_or_else(|| "prestige adjustment is too large".to_string())?;
            if next < 0 {
                return Err(format!(
                    "prestige adjustment would reduce prestige below zero ({} + {amount})",
                    state.prestige
                ));
            }
            state.prestige = next;
            Ok(EventOutcome::prestige(id, *hour, *amount))
        }
        ScenarioEvent::Invasion {
            id,
            hour,
            target_land,
            target_dp,
            sent,
            land_by_type,
            prestige,
            casualties_override,
        } => apply_invasion(
            state,
            id,
            *hour,
            *target_land,
            *target_dp,
            *sent,
            land_by_type,
            *prestige,
            *casualties_override,
        ),
    }
}

/// Combat OP of exactly the sent army against a target at `target_land`.
/// A temporary state prevents troops left at home from inflating the result.
fn sent_combat_op(attacker: &DominionState, target_land: i64, sent: [i64; 4]) -> f64 {
    let mut sent_state = attacker.clone();
    sent_state.military_unit1 = sent[0];
    sent_state.military_unit2 = sent[1];
    sent_state.military_unit3 = sent[2];
    sent_state.military_unit4 = sent[3];
    let mut target = DominionState::default();
    target.land_plain = target_land;
    combat::offensive_power_combat(&sent_state, &target)
}

fn defense_for_units(s: &DominionState, units: [i64; 4], include_draftees: bool) -> f64 {
    let mut view = s.clone();
    view.military_draftees = if include_draftees {
        s.military_draftees
    } else {
        0
    };
    view.military_unit1 = units[0];
    view.military_unit2 = units[1];
    view.military_unit3 = units[2];
    view.military_unit4 = units[3];
    calc::defensive_power_raw(&view)
        * calc::defensive_power_multiplier(&view)
        * calc::morale_multiplier(&view)
}

/// PHP's 40%-at-home rule. Returning troops still count toward the force the
/// dominion owns, while only the unsent home army is available to satisfy it.
fn passes_40_rule(s: &DominionState, sent: [i64; 4], returning: [i64; 4]) -> bool {
    let sent_dp = defense_for_units(s, sent, false);
    if sent_dp <= 0.0 {
        return true;
    }
    let current_home = calc::defensive_power(s);
    let returning_dp = defense_for_units(s, returning, false);
    current_home - sent_dp >= (current_home + returning_dp) * 0.40
}

/// PHP's 5:4 send ceiling, using unclamped after-send home DP. The land-based
/// minimum-defense floor must not inflate the legal offense ceiling.
fn passes_54_rule(s: &DominionState, target_land: i64, sent: [i64; 4]) -> bool {
    let op = sent_combat_op(s, target_land, sent);
    let home = [
        s.military_unit1 - sent[0],
        s.military_unit2 - sent[1],
        s.military_unit3 - sent[2],
        s.military_unit4 - sent[3],
    ];
    let home_dp = defense_for_units(s, home, true);
    op <= (home_dp * 1.25).ceil()
}

/// All boat-needing troops must fit in the dominion's whole boats. Boat-exempt
/// flying/amphibious units do not consume seats.
fn has_boat_capacity(s: &DominionState, sent: [i64; 4]) -> bool {
    let needed = (1..=4)
        .filter(|slot| calc::unit_need_boat(s, *slot))
        .map(|slot| sent[slot - 1])
        .sum::<i64>();
    let available =
        (s.resource_boats.max(0.0).floor() as i64).saturating_mul(calc::boat_capacity(s));
    needed <= available
}

#[allow(clippy::too_many_arguments)]
fn apply_invasion(
    state: &mut DominionState,
    id: &str,
    hour: i64,
    target_land: i64,
    target_dp: f64,
    sent: [i64; 4],
    land_by_type: &HashMap<String, i64>,
    prestige: i64,
    casualties_override: Option<[i64; 4]>,
) -> Result<EventOutcome, String> {
    for slot in 1..=4 {
        let available = calc::military_slot_count(state, slot);
        if sent[slot - 1] > available {
            return Err(format!(
                "invasion sends {} troops from slot {slot}, but only {available} are home",
                sent[slot - 1]
            ));
        }
        if sent[slot - 1] > 0 && calc::unit_offense(state, slot) <= 0.0 {
            return Err(format!("unit slot {slot} has no offensive power"));
        }
    }
    if !combat::in_range(calc::total_land(state), target_land) {
        return Err(format!(
            "target at {target_land} acres is outside the legal 40%-250% invasion range"
        ));
    }
    if !has_boat_capacity(state, sent) {
        return Err("not enough boats to carry the requested invasion army".to_string());
    }
    if state.morale < MIN_INVASION_MORALE {
        return Err(format!(
            "invasion requires at least {MIN_INVASION_MORALE} morale; the dominion has {}",
            state.morale
        ));
    }

    let op = sent_combat_op(state, target_land, sent);
    if op <= target_dp {
        return Err(format!(
            "invasion would fail: {:.0} OP does not beat {:.0} target DP",
            op, target_dp
        ));
    }
    let returning = returning_invasion_units(state);
    if !passes_40_rule(state, sent, returning) {
        return Err("invasion must leave enough defensive power at home (40% rule)".to_string());
    }
    if !passes_54_rule(state, target_land, sent) {
        return Err(
            "invasion sends too much offense for the defensive power left home (5:4 rule)"
                .to_string(),
        );
    }

    let mut target_state = DominionState::default();
    target_state.land_plain = target_land;
    let calculated = combat::offensive_casualties_given(state, &target_state, sent, op, target_dp);
    let casualties = casualties_override.unwrap_or(calculated);
    for slot in 0..4 {
        if casualties[slot] < 0 || casualties[slot] > sent[slot] {
            return Err(format!(
                "slot {} casualties must be between 0 and the {} troops sent",
                slot + 1,
                sent[slot]
            ));
        }
    }
    let survivors = std::array::from_fn(|idx| sent[idx] - casualties[idx]);
    let population_freed = casualties
        .iter()
        .try_fold(0i64, |total, count| total.checked_add(*count))
        .ok_or_else(|| "casualty total is too large".to_string())?;
    let return_hours = std::array::from_fn(|idx| combat::unit_return_hours(state, idx + 1));
    let slowest_return = combat::slowest_unit_return_hours(state, sent);

    let ordered_land = LAND_TYPES
        .iter()
        .map(|land| ((*land).to_string(), *land_by_type.get(*land).unwrap_or(&0)))
        .collect::<BTreeMap<_, _>>();
    let land_total = ordered_land
        .values()
        .try_fold(0i64, |total, amount| total.checked_add(*amount))
        .ok_or_else(|| "conquered land total is too large".to_string())?;
    calc::total_land(state)
        .checked_add(land_total)
        .ok_or_else(|| "conquered land would overflow the dominion's land total".to_string())?;
    let conquered_total = state
        .stat_total_land_conquered
        .checked_add(land_total)
        .ok_or_else(|| "conquered land would overflow the dominion's statistics".to_string())?;

    let mut boat_units_by_return = BTreeMap::<i64, i64>::new();
    for slot in 1..=4 {
        if sent[slot - 1] > 0 && calc::unit_need_boat(state, slot) {
            *boat_units_by_return
                .entry(return_hours[slot - 1])
                .or_default() += sent[slot - 1];
        }
    }
    let capacity = calc::boat_capacity(state).max(1);
    let boats_by_return = boat_units_by_return
        .into_iter()
        .filter_map(|(hours, units)| {
            let boats = units / capacity;
            (boats > 0).then_some((hours, boats))
        })
        .collect::<Vec<_>>();
    let boats_sent = boats_by_return.iter().map(|(_, boats)| *boats).sum::<i64>();

    state.military_unit1 -= sent[0];
    state.military_unit2 -= sent[1];
    state.military_unit3 -= sent[2];
    state.military_unit4 -= sent[3];
    state.resource_boats = (state.resource_boats - boats_sent as f64).max(0.0);

    for slot in 1..=4 {
        if survivors[slot - 1] > 0 {
            state.queue.push(QueueEntry {
                source: "invasion".to_string(),
                resource: format!("military_unit{slot}"),
                hours: return_hours[slot - 1],
                amount: survivors[slot - 1],
            });
        }
    }
    for (hours, boats) in boats_by_return {
        state.queue.push(QueueEntry {
            source: "invasion".to_string(),
            resource: "resource_boats".to_string(),
            hours,
            amount: boats,
        });
    }

    for (land, amount) in &ordered_land {
        if *amount > 0 {
            state.queue.push(QueueEntry {
                source: "invasion".to_string(),
                resource: format!("land_{land}"),
                hours: LAND_RETURN_HOURS,
                amount: *amount,
            });
        }
    }
    if land_total > 0 {
        state.stat_total_land_conquered = conquered_total;
        let range_pct = combat::dominion_range(calc::total_land(state), target_land);
        if range_pct >= 75.0 {
            state.queue.push(QueueEntry {
                source: "invasion".to_string(),
                resource: "discounted_land".to_string(),
                hours: LAND_RETURN_HOURS,
                amount: land_total,
            });
        }
    }
    if prestige > 0 {
        state.queue.push(QueueEntry {
            source: "invasion".to_string(),
            resource: "prestige".to_string(),
            hours: slowest_return,
            amount: prestige,
        });
    }

    let range_pct = combat::dominion_range(calc::total_land(state), target_land);
    let morale_delta = -combat::morale_cost(range_pct);
    state.morale = (state.morale + morale_delta).max(0);

    Ok(EventOutcome {
        id: id.to_string(),
        event_type: "invasion".to_string(),
        hour,
        sent,
        calculated_casualties: calculated,
        casualties,
        survivors,
        return_hours,
        land_by_type: ordered_land,
        land_total,
        land_return_hour: (land_total > 0).then_some(hour + LAND_RETURN_HOURS),
        prestige,
        prestige_return_hour: (prestige > 0).then_some(hour + slowest_return),
        op,
        target_dp,
        range_pct,
        morale_delta,
        population_freed,
        manual_override: casualties_override.is_some(),
        boats_sent,
    })
}

/// Trained units currently away on an invasion, grouped by slot. These units
/// remain part of population, food consumption, military percentage, and draft
/// calculations until their queue entry returns.
pub fn returning_invasion_units(state: &DominionState) -> [i64; 4] {
    std::array::from_fn(|idx| {
        let resource = format!("military_unit{}", idx + 1);
        state
            .queue
            .iter()
            .filter(|q| q.source == "invasion" && q.resource == resource)
            .map(|q| q.amount)
            .sum()
    })
}
