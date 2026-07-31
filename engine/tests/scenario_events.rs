use std::collections::HashMap;

use engine::state::{ActiveSpell, DominionState};
use engine::{calc, combat, scenario_event, tick};

fn invasion_state() -> DominionState {
    let mut state = DominionState::default();
    state.race = "human".to_string();
    state.land_plain = 1_000;
    state.peasants = 1_000;
    state.military_draftees = 3_000;
    state.military_unit1 = 300;
    state.military_unit4 = 300;
    state.resource_boats = 20.0;
    state.resource_food = 1_000_000;
    state.morale = 100;
    state.prestige = 250;
    state
}

#[test]
fn invasion_queues_typed_land_and_slot_specific_returns() {
    let mut state = invasion_state();
    let before_population = calc::population(&state);
    let event: scenario_event::ScenarioEvent = serde_json::from_value(serde_json::json!({
        "type": "invasion",
        "id": "hit-1",
        "hour": 49,
        "targetLand": 800,
        "targetDp": 1_000,
        "sent": [300, 0, 0, 300],
        "landByType": {
            "plain": 10, "swamp": 10, "hill": 10, "mountain": 10,
            "forest": 10, "cavern": 10, "water": 10
        },
        "prestige": 40,
        "casualtiesOverride": [10, 0, 0, 20]
    }))
    .unwrap();

    let outcome = scenario_event::apply_event(&mut state, &event).unwrap();
    assert_eq!(outcome.return_hours, [12, 12, 12, 9]);
    assert_eq!(outcome.casualties, [10, 0, 0, 20]);
    assert_eq!(outcome.survivors, [290, 0, 0, 280]);
    assert_eq!(outcome.land_return_hour, Some(61));
    assert_eq!(outcome.prestige_return_hour, Some(61));
    assert_eq!(outcome.population_freed, 30);
    assert_eq!(calc::population(&state), before_population - 30);
    assert_eq!(
        scenario_event::returning_invasion_units(&state),
        [290, 0, 0, 280]
    );
    assert_eq!(state.military_unit1, 0);
    assert_eq!(state.military_unit4, 0);
    assert_eq!(state.stat_total_land_conquered, 70);
    assert_eq!(
        state.total_land(),
        1_000,
        "conquered land is incoming for 12h"
    );
    assert_eq!(
        state.prestige, 250,
        "positive invasion prestige returns with the army"
    );

    for _ in 0..9 {
        state = tick::tick(&state);
    }
    assert_eq!(state.military_unit4, 280, "9h Cavalry survivors returned");
    assert_eq!(
        state.military_unit1, 0,
        "12h Spearman survivors are still away"
    );
    assert_eq!(state.total_land(), 1_000);

    for _ in 0..3 {
        state = tick::tick(&state);
    }
    assert_eq!(state.military_unit1, 290);
    assert_eq!(state.total_land(), 1_070);
    assert_eq!(state.land_plain, 1_010);
    assert_eq!(state.land_water, 10);
    assert_eq!(state.prestige, 290);
    assert_eq!(state.discounted_land, 70);
    assert_eq!(state.resource_boats, 20.0);
}

#[test]
fn casualty_calculation_uses_active_spells_and_researched_techs() {
    let mut attacker = DominionState::default();
    attacker.race = "human".to_string();
    attacker.land_plain = 1_000;
    attacker.military_unit1 = 10_000;
    let mut target = DominionState::default();
    target.land_plain = 800;
    let sent = [10_000, 0, 0, 0];
    let op = 30_000.0;
    let dp = 29_000.0;

    let base = combat::offensive_casualties_given(&attacker, &target, sent, op, dp)[0];
    assert_eq!(base, 822);

    attacker.spells.push(ActiveSpell {
        key: "regeneration".to_string(),
        duration: 12,
    });
    attacker.techs.push("tech_11_13".to_string()); // Field Surgery: -7.5%
    let reduced = combat::offensive_casualties_given(&attacker, &target, sent, op, dp)[0];
    assert_eq!(
        reduced, 514,
        "-30% spell and -7.5% tech combine to a 0.625 multiplier"
    );

    attacker.spells = vec![ActiveSpell {
        key: "bloodrage".to_string(),
        duration: 12,
    }];
    let penalized = combat::offensive_casualties_given(&attacker, &target, sent, op, dp)[0];
    assert_eq!(penalized, 843, "+10% Bloodrage and -7.5% tech net to 1.025");
}

#[test]
fn invalid_manual_casualties_cannot_mint_or_overkill_troops() {
    let mut state = invasion_state();
    let event = scenario_event::ScenarioEvent::Invasion {
        id: "bad".to_string(),
        hour: 49,
        target_land: 800,
        target_dp: 1_000.0,
        sent: [20, 0, 0, 0],
        land_by_type: HashMap::new(),
        prestige: 0,
        casualties_override: Some([21, 0, 0, 0]),
    };
    let error = scenario_event::apply_event(&mut state, &event).unwrap_err();
    assert!(error.contains("casualty override"));
    assert_eq!(
        state.military_unit1, 300,
        "rejected event leaves state untouched"
    );
}

#[test]
fn oversized_event_values_are_rejected_without_mutating_state() {
    let mut state = invasion_state();
    let prestige = scenario_event::ScenarioEvent::Prestige {
        id: "too-much-prestige".to_string(),
        hour: 49,
        amount: i64::MAX,
    };
    let before = state.prestige;
    assert!(scenario_event::apply_event(&mut state, &prestige)
        .unwrap_err()
        .contains("too large"));
    assert_eq!(state.prestige, before);

    let event: scenario_event::ScenarioEvent = serde_json::from_value(serde_json::json!({
        "type": "invasion",
        "id": "too-much-land",
        "hour": 49,
        "targetLand": 800,
        "targetDp": 1,
        "sent": [1, 0, 0, 0],
        "landByType": { "plain": i64::MAX, "swamp": 1 },
        "prestige": 0
    }))
    .unwrap();
    assert!(event
        .validate()
        .unwrap_err()
        .contains("land total is too large"));
    assert_eq!(state.total_land(), 1_000);
}

#[test]
fn prestige_adjustments_apply_immediately_and_cannot_go_negative() {
    let mut state = invasion_state();
    let add = scenario_event::ScenarioEvent::Prestige {
        id: "prestige-add".to_string(),
        hour: 49,
        amount: 125,
    };
    let outcome = scenario_event::apply_event(&mut state, &add).unwrap();
    assert_eq!(state.prestige, 375);
    assert_eq!(outcome.prestige, 125);
    assert_eq!(outcome.prestige_return_hour, Some(49));

    let remove_too_much = scenario_event::ScenarioEvent::Prestige {
        id: "prestige-remove".to_string(),
        hour: 50,
        amount: -376,
    };
    let before = state.clone();
    assert!(scenario_event::apply_event(&mut state, &remove_too_much)
        .unwrap_err()
        .contains("below zero"));
    assert_eq!(state, before);
}

#[test]
fn invasion_enforces_live_morale_and_home_force_rules_without_mutating_state() {
    let mut low_morale = invasion_state();
    low_morale.morale = 79;
    let event = scenario_event::ScenarioEvent::Invasion {
        id: "illegal-hit".to_string(),
        hour: 49,
        target_land: 800,
        target_dp: 100.0,
        sent: [0, 0, 0, 100],
        land_by_type: HashMap::new(),
        prestige: 0,
        casualties_override: None,
    };
    let before = low_morale.clone();
    assert!(scenario_event::apply_event(&mut low_morale, &event)
        .unwrap_err()
        .contains("80 morale"));
    assert_eq!(low_morale, before);

    let mut exposed_home = invasion_state();
    exposed_home.military_draftees = 0;
    exposed_home.military_unit4 = 3_000;
    exposed_home.resource_boats = 100.0;
    let exposed_event = scenario_event::ScenarioEvent::Invasion {
        id: "exposed-home".to_string(),
        hour: 49,
        target_land: 800,
        target_dp: 100.0,
        sent: [0, 0, 0, 3_000],
        land_by_type: HashMap::new(),
        prestige: 0,
        casualties_override: None,
    };
    let before = exposed_home.clone();
    assert!(
        scenario_event::apply_event(&mut exposed_home, &exposed_event)
            .unwrap_err()
            .contains("40% rule")
    );
    assert_eq!(exposed_home, before);

    let mut excess_offense = invasion_state();
    excess_offense.military_draftees = 0;
    excess_offense.military_unit1 = 1_000;
    excess_offense.resource_boats = 100.0;
    let ceiling_event = scenario_event::ScenarioEvent::Invasion {
        id: "excess-offense".to_string(),
        hour: 49,
        target_land: 800,
        target_dp: 100.0,
        sent: [1_000, 0, 0, 0],
        land_by_type: HashMap::new(),
        prestige: 0,
        casualties_override: None,
    };
    let before = excess_offense.clone();
    assert!(
        scenario_event::apply_event(&mut excess_offense, &ceiling_event)
            .unwrap_err()
            .contains("5:4 rule")
    );
    assert_eq!(excess_offense, before);
}

#[test]
fn overture_adapter_extends_only_for_the_event_return_window() {
    let base = serde_json::json!({
        "race": "human",
        "opening": {},
        "hours": (0..48).map(|_| serde_json::json!([])).collect::<Vec<_>>()
    });
    let mut prestige = base.clone();
    prestige["events"] = serde_json::json!([{
        "type": "prestige", "id": "p", "hour": 49, "amount": 10
    }]);
    let prestige_scenario = engine::plan::scenario_value_from_overture_plan(&prestige);
    assert_eq!(
        prestige_scenario["postOopTicks"].as_array().unwrap().len(),
        1,
        "an immediate prestige event needs its own hour but no 12h return tail"
    );

    let mut invasion = base;
    invasion["events"] = serde_json::json!([{
        "type": "invasion", "id": "i", "hour": 49,
        "targetLand": 300, "targetDp": 1, "sent": [1, 0, 0, 0],
        "landByType": { "plain": 1 }, "prestige": 0
    }]);
    let invasion_scenario = engine::plan::scenario_value_from_overture_plan(&invasion);
    assert_eq!(
        invasion_scenario["postOopTicks"].as_array().unwrap().len(),
        12,
        "an invasion replay includes the +12 land-arrival row"
    );
}
