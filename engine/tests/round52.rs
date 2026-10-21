use engine::{calc, config, data, plan};
use serde_json::json;

#[test]
fn round52_elite_orders_pay_updated_prices() {
    for (race, slot, price, ore) in [("icekin", 3, 995, 100), ("lycanthrope", 4, 1150, 110)] {
        let mut state = config::start_state("advanced", 0, race);
        state.resource_ore = 1000;
        let platinum = state.resource_platinum;
        plan::apply_action(
            &mut state,
            &json!({"type": "train", "data": {format!("military_unit{slot}"): 2}}),
        );
        assert_eq!(state.resource_platinum, platinum - 2 * price);
        assert_eq!(state.resource_ore, 1000 - 2 * ore);
        assert_eq!(state.military_draftees, 298);
        assert_eq!(state.queue[0].hours, 12);
    }
}

#[test]
fn official_round52_data_keeps_previous_live_corrections() {
    assert_eq!(data::get().races["sylvan"].units[3].cost["platinum"], 950);
    assert_eq!(
        data::get().spells["arcane_infusion"].perks["train_archmages_from_draftees"],
        1400.0
    );
    assert_eq!(data::get().races["demon"].units[1].power["offense"], 0.0);
    assert_eq!(data::get().races["demon"].units[2].power["offense"], 1.0);
    assert!(data::get().ruleset.production_overrides.is_empty());
}

#[test]
fn round_perks_respect_start_alignment_and_inclusive_days() {
    let mut good = config::start_state("advanced", 0, "human");
    assert_eq!(calc::investment_multiplier(&good, "gems", "keep"), 1.0);
    good.protection_finished = true;
    for (day, expected) in [(1, 1.10), (18, 1.10), (19, 1.05), (33, 1.05), (34, 1.0)] {
        good.round_day = day;
        assert_eq!(calc::investment_multiplier(&good, "gems", "keep"), expected);
    }
    let mut evil = config::start_state("advanced", 1, "lycanthrope");
    evil.building_school = 35;
    assert_eq!(calc::investment_multiplier(&evil, "gems", "keep"), 1.0);
    assert_eq!(calc::tech_production(&evil), 36);
    evil.round_day = 34;
    assert_eq!(calc::tech_production(&evil), 31);
}

#[test]
fn empty_hours_advance_round_perk_calendar_and_override_context_survives_import() {
    let imported = plan::scenario_value_from_overture_plan(&json!({
        "race": "human", "daysLate": 17, "opening": {"home": 150, "farm": 120, "alchemy": 80},
        "hours": vec![json!([]); 25]
    }));
    let scenario: plan::Scenario = serde_json::from_value(imported).unwrap();
    let steps = plan::run(&scenario);
    assert_eq!(steps[26].round_day, 18);
    assert_eq!(steps[27].round_day, 19);
    assert_eq!(
        calc::investment_multiplier(&steps[26], "gems", "keep"),
        1.10
    );
    assert_eq!(
        calc::investment_multiplier(&steps[27], "gems", "keep"),
        1.05
    );

    let imported = plan::scenario_value_from_overture_plan(&json!({
        "race": "human", "daysLate": 1, "roundStart": "-1 days", "roundPerks": []
    }));
    let scenario: plan::Scenario = serde_json::from_value(imported).unwrap();
    let state = plan::start_state(&scenario);
    assert_eq!(state.round_perks, Some(vec![]));
    assert_eq!(calc::investment_multiplier(&state, "gems", "keep"), 1.0);
}

#[test]
fn direct_actions_preserve_expired_round_perk_context() {
    let mut state = config::start_state("advanced", 33, "human");
    state.resource_gems = 100;
    plan::apply_action(
        &mut state,
        &json!({"type": "improve", "resource": "gems", "data": {"keep": 100}}),
    );
    assert_eq!(state.round_day, 34);
    assert_eq!(state.improvement_keep, 1200);
}

#[test]
fn empty_oop_and_post_oop_plans_share_the_same_completed_protection_state() {
    let mut scenario: plan::Scenario = serde_json::from_value(json!({
        "race": "lycanthrope", "protectionType": "advanced",
        "openingBuild": {"home": 150, "farm": 120, "alchemy": 45, "school": 35},
        "ticks": vec![json!([]); 48]
    }))
    .unwrap();
    let protection = plan::run(&scenario);
    let oop = protection.last().unwrap();
    assert!(oop.protection_finished);
    assert!(!protection[protection.len() - 2].protection_finished);
    // Before the final tick, schools yield 31; OOP enables the +15% perk (36).
    assert_eq!(oop.resource_tech, 48 * 31);
    assert_eq!(calc::tech_production(oop), 36);
    scenario.post_oop_ticks = vec![vec![]];
    let continuation = plan::run(&scenario);
    assert_eq!(oop, &continuation[protection.len() - 1]);
    assert_eq!(continuation.last().unwrap().resource_tech, 48 * 31 + 36);
}
