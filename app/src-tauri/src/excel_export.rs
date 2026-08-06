//! Lossless OVERTURE -> community Excel-sim handoff.
//!
//! The Round 51 workbook remains an opaque template: formulas, tables, VBA,
//! drawings, controls, and styles are copied unchanged. We patch the
//! workbook's established orange input cells, discard the template's stale
//! formula caches/calculation chain, and force a full recalculation when the
//! generated `.xlsm` is opened.

use engine::state::DominionState;
use engine::{calc, data, plan, race_resources};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read, Write};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

const TEMPLATE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../OpenDominion OOP Sim -- Round 51 --.xlsm"
));
const REVIEWED_TEMPLATE_SHA256: &str =
    "a82e6ff9f03169d496b3bd87198d3a62f00f11d2dfe154da54cd0838fa49840f";

const PROTECTION_HOURS: usize = 48;
const OOP_HOUR: usize = PROTECTION_HOURS + 1;
const FIRST_ACTION_ROW: usize = 4;
const LAST_ACTION_ROW: usize = 136;
const MAX_EXCEL_HOUR: usize = LAST_ACTION_ROW - FIRST_ACTION_ROW + 1;
const MAX_SOURCE_HOURS: usize = PROTECTION_HOURS + 480;
const MAX_PLAN_BYTES: usize = 2 * 1024 * 1024;
const MAX_ACTIONS_PER_HOUR: usize = 256;
const MAX_TOTAL_ACTIONS: usize = 8_192;
const MAX_EVENTS: usize = 256;
const MAX_ACTION_AMOUNT: i64 = 1_000_000_000;

const WORKBOOK: &str = "xl/workbook.xml";
const WORKBOOK_RELS: &str = "xl/_rels/workbook.xml.rels";
const CONTENT_TYPES: &str = "[Content_Types].xml";
const CALC_CHAIN: &str = "xl/calcChain.xml";
const OVERVIEW: &str = "xl/worksheets/sheet1.xml";
const PRODUCTION: &str = "xl/worksheets/sheet3.xml";
const CONSTRUCTION: &str = "xl/worksheets/sheet4.xml";
const EXPLORE: &str = "xl/worksheets/sheet5.xml";
const REZONE: &str = "xl/worksheets/sheet6.xml";
const MILITARY: &str = "xl/worksheets/sheet7.xml";
const MAGIC: &str = "xl/worksheets/sheet8.xml";
const TECHS: &str = "xl/worksheets/sheet9.xml";
const IMPS: &str = "xl/worksheets/sheet10.xml";

const WORKBOOK_RACES: &[&str] = &[
    "Human",
    "Nomad",
    "Dwarf",
    "Wood Elf",
    "Halfling",
    "Gnome",
    "Merfolk",
    "Sylvan",
    "Goblin",
    "Troll",
    "Dark Elf",
    "Undead",
    "Spirit",
    "Lycanthrope",
    "Kobold",
    "Lizardfolk",
    "Icekin",
    "Firewalker",
    "Orc",
    "Vampire",
    "Demon",
];

const RESOURCE_COLUMNS: &[(&str, &str)] = &[
    ("platinum", "BC"),
    ("lumber", "BD"),
    ("ore", "BE"),
    ("gems", "BF"),
];

#[derive(Clone, Debug)]
enum CellValue {
    Blank,
    Number(String),
    Text(String),
}

#[derive(Default)]
struct WorkbookEdits {
    sheets: HashMap<&'static str, BTreeMap<String, CellValue>>,
    numbers: HashMap<(&'static str, String), i64>,
}

impl WorkbookEdits {
    fn set(&mut self, sheet: &'static str, cell: impl Into<String>, value: CellValue) {
        self.sheets
            .entry(sheet)
            .or_default()
            .insert(cell.into(), value);
    }

    fn blank(&mut self, sheet: &'static str, cell: impl Into<String>) {
        self.set(sheet, cell, CellValue::Blank);
    }

    fn set_i64(&mut self, sheet: &'static str, cell: impl Into<String>, value: i64) {
        self.set(sheet, cell, CellValue::Number(value.to_string()));
    }

    fn set_f64(&mut self, sheet: &'static str, cell: impl Into<String>, value: f64) {
        self.set(sheet, cell, CellValue::Number(format!("{value:.12}")));
    }

    fn text(&mut self, sheet: &'static str, cell: impl Into<String>, value: impl Into<String>) {
        self.set(sheet, cell, CellValue::Text(value.into()));
    }

    fn add_i64(
        &mut self,
        sheet: &'static str,
        cell: impl Into<String>,
        amount: i64,
    ) -> Result<(), String> {
        let cell = cell.into();
        let key = (sheet, cell.clone());
        let value = self
            .numbers
            .get(&key)
            .copied()
            .unwrap_or_default()
            .checked_add(amount)
            .ok_or_else(|| format!("Excel input total overflowed at {cell}"))?;
        self.numbers.insert(key, value);
        self.set_i64(sheet, cell, value);
        Ok(())
    }
}

#[derive(Default)]
struct ResolvedHour {
    bank_deltas: BTreeMap<String, i64>,
    invest_all_amounts: Vec<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ImprovementAllocation {
    resource: String,
    improvement: String,
    amount: i64,
}

pub struct RenderedWorkbook {
    pub bytes: Vec<u8>,
    pub warnings: Vec<String>,
    pub race_name: String,
}

pub fn render_overture_plan(plan_in: &Value) -> Result<RenderedWorkbook, String> {
    validate_export_plan(plan_in)?;

    let race_key = plan_in
        .get("race")
        .and_then(Value::as_str)
        .unwrap_or("human");
    let race = data::get()
        .races
        .get(race_key)
        .ok_or_else(|| format!("unknown OVERTURE race {race_key:?}"))?;
    let race_name = race.name.clone();
    if !WORKBOOK_RACES.contains(&race_name.as_str()) {
        return Err(format!(
            "the Round 51 Excel workbook does not support {race_name}; keep this build in OVERTURE"
        ));
    }

    let source_hour_count = plan_in
        .get("hours")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    let has_events = plan_in
        .get("events")
        .and_then(Value::as_array)
        .is_some_and(|events| !events.is_empty());

    // OVERTURE always has 48 protection hours. Pad hand-edited/imported plans
    // before replay so the OOP boundary cannot move early merely because empty
    // trailing hours were omitted from JSON. The workbook has only 133 action
    // rows, so cap the replay at that same boundary. Events have no Excel input
    // surface; omitting them from this replay prevents their effects from
    // contaminating resolved bank/invest-all values in a workbook that cannot
    // reproduce those events.
    let mut replay_plan = plan_in.clone();
    let obj = replay_plan
        .as_object_mut()
        .ok_or_else(|| "OVERTURE plan must be an object".to_string())?;
    let hours = obj
        .entry("hours")
        .or_insert_with(|| Value::Array(Vec::new()));
    let hours = hours
        .as_array_mut()
        .ok_or_else(|| "OVERTURE plan hours must be an array".to_string())?;
    while hours.len() < PROTECTION_HOURS {
        hours.push(Value::Array(Vec::new()));
    }
    hours.truncate(MAX_EXCEL_HOUR);
    obj.insert("events".to_string(), Value::Array(Vec::new()));

    if let Some(error) = plan::overture_plan_error(&replay_plan) {
        return Err(error);
    }

    let scenario_value = plan::scenario_value_from_overture_plan(&replay_plan);
    let scenario: plan::Scenario = serde_json::from_value(scenario_value)
        .map_err(|error| format!("could not build Excel replay scenario: {error}"))?;
    let start_land = calc::total_land(&plan::start_state(&scenario));
    if let Some(error) = plan::opening_build_error(&scenario.opening_build, start_land) {
        return Err(error);
    }
    let resolved = resolve_hour_actions(&replay_plan, &scenario)?;
    let mut edits = WorkbookEdits::default();
    reset_inputs(&mut edits);
    write_overview(&mut edits, &replay_plan, &race_name, start_land)?;

    let max_planned_hour = source_hour_count.max(PROTECTION_HOURS).max(
        replay_plan
            .get("oopActions")
            .or_else(|| replay_plan.get("oop_actions"))
            .and_then(Value::as_array)
            .filter(|actions| !actions.is_empty())
            .map(|_| OOP_HOUR)
            .unwrap_or(0),
    );
    let export_hours = max_planned_hour.min(MAX_EXCEL_HOUR);
    let mut warnings = Vec::new();
    if max_planned_hour > MAX_EXCEL_HOUR {
        warnings.push(format!(
            "Excel ends at hour {MAX_EXCEL_HOUR}; later OVERTURE actions were not exported"
        ));
    }
    if has_events {
        warnings.push(
            "scenario invasions/prestige events have no Excel input cells and were not exported"
                .to_string(),
        );
    }

    let mut order_risk_hours = Vec::new();
    for hour in 1..=export_hours {
        let actions = app_actions_for_hour(&replay_plan, hour);
        if has_excel_order_risk(&actions) {
            order_risk_hours.push(hour);
        }
        write_hour(&mut edits, hour, &actions, resolved.get(&hour))?;
    }
    if !order_risk_hours.is_empty() {
        let sample = order_risk_hours
            .iter()
            .take(6)
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let suffix = if order_risk_hours.len() > 6 {
            ", …"
        } else {
            ""
        };
        warnings.push(format!(
            "Excel has a fixed within-hour order; compare OVERTURE after edits around hour(s) {sample}{suffix}"
        ));
    }
    warnings.push(
        "the workbook recalculates with its own formulas; OVERTURE remains the bit-exact source of truth"
            .to_string(),
    );

    let bytes = patch_template(&edits.sheets)?;
    Ok(RenderedWorkbook {
        bytes,
        warnings,
        race_name,
    })
}

fn validate_export_plan(plan_in: &Value) -> Result<(), String> {
    let obj = plan_in
        .as_object()
        .ok_or_else(|| "OVERTURE plan must be an object".to_string())?;
    let encoded = serde_json::to_vec(plan_in)
        .map_err(|error| format!("could not validate OVERTURE plan size: {error}"))?;
    if encoded.len() > MAX_PLAN_BYTES {
        return Err(format!(
            "OVERTURE plan is too large for Excel export ({} bytes; limit is {MAX_PLAN_BYTES})",
            encoded.len()
        ));
    }

    if obj.contains_key("oopActions") && obj.contains_key("oop_actions") {
        return Err("OVERTURE plan must use only one OOP-action field".to_string());
    }
    if let Some(race) = obj.get("race") {
        let race = race
            .as_str()
            .ok_or_else(|| "OVERTURE plan race must be text".to_string())?;
        if race.len() > 64 {
            return Err("OVERTURE plan race is too long".to_string());
        }
    }
    if let Some(days_late) = obj.get("daysLate") {
        let days_late = days_late
            .as_i64()
            .ok_or_else(|| "OVERTURE daysLate must be an integer".to_string())?;
        if days_late != 0 {
            return Err(
                "the Round 51 Excel workbook cannot represent a late-start plan".to_string(),
            );
        }
    }
    if let Some(opening) = obj.get("opening") {
        let opening = opening
            .as_object()
            .ok_or_else(|| "OVERTURE plan opening must be an object".to_string())?;
        if opening.len() > opening_columns().len() {
            return Err("OVERTURE opening contains too many building entries".to_string());
        }
    }

    let mut total_actions = 0usize;
    if let Some(hours) = obj.get("hours") {
        let hours = hours
            .as_array()
            .ok_or_else(|| "OVERTURE plan hours must be an array".to_string())?;
        if hours.len() > MAX_SOURCE_HOURS {
            return Err(format!(
                "OVERTURE plan has {} hours; Excel export accepts at most {MAX_SOURCE_HOURS}",
                hours.len()
            ));
        }
        for (index, hour) in hours.iter().enumerate() {
            let actions = hour
                .as_array()
                .ok_or_else(|| format!("OVERTURE hour {} must be an action array", index + 1))?;
            validate_action_list(actions, &format!("hour {}", index + 1), &mut total_actions)?;
        }
    }

    if let Some(actions) = obj.get("oopActions").or_else(|| obj.get("oop_actions")) {
        let actions = actions
            .as_array()
            .ok_or_else(|| "OVERTURE OOP actions must be an array".to_string())?;
        validate_action_list(actions, "OOP", &mut total_actions)?;
    }

    if let Some(events) = obj.get("events") {
        let events = events
            .as_array()
            .ok_or_else(|| "OVERTURE events must be an array".to_string())?;
        if events.len() > MAX_EVENTS {
            return Err(format!(
                "OVERTURE plan has {} events; Excel export accepts at most {MAX_EVENTS}",
                events.len()
            ));
        }
        // Events are intentionally omitted from the workbook, but still reject
        // malformed imported data instead of silently accepting it at the IPC boundary.
        plan::scenario_events_from_overture_plan(plan_in)?;
    }
    Ok(())
}

fn validate_action_list(
    actions: &[Value],
    where_: &str,
    total_actions: &mut usize,
) -> Result<(), String> {
    if actions.len() > MAX_ACTIONS_PER_HOUR {
        return Err(format!(
            "{where_} has {} actions; Excel export accepts at most {MAX_ACTIONS_PER_HOUR}",
            actions.len()
        ));
    }
    *total_actions = total_actions
        .checked_add(actions.len())
        .ok_or_else(|| "OVERTURE action count overflowed".to_string())?;
    if *total_actions > MAX_TOTAL_ACTIONS {
        return Err(format!(
            "OVERTURE plan has too many actions for Excel export (limit is {MAX_TOTAL_ACTIONS})"
        ));
    }
    for action in actions {
        let action = action
            .as_object()
            .ok_or_else(|| format!("{where_} contains a non-object action"))?;
        for key in ["n", "amount", "rate"] {
            if let Some(value) = action.get(key) {
                let amount = value
                    .as_i64()
                    .ok_or_else(|| format!("{where_} action {key} must be an integer"))?;
                if !(-MAX_ACTION_AMOUNT..=MAX_ACTION_AMOUNT).contains(&amount) {
                    return Err(format!(
                        "{where_} action {key} exceeds the Excel export safety limit"
                    ));
                }
            }
        }
        if let Some(data) = action.get("data").and_then(Value::as_object) {
            for (key, value) in data {
                if let Some(amount) = value.as_i64() {
                    if !(-MAX_ACTION_AMOUNT..=MAX_ACTION_AMOUNT).contains(&amount) {
                        return Err(format!(
                            "{where_} allocation {key:?} exceeds the Excel export safety limit"
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

fn reset_inputs(edits: &mut WorkbookEdits) {
    for row in 16..=34 {
        edits.set_i64(OVERVIEW, format!("L{row}"), 0);
    }
    edits.set_i64(OVERVIEW, "B18", 0);

    let numeric_ranges: &[(&str, &[&str])] = &[
        (PRODUCTION, &["C", "BD", "BE", "BF"]),
        (
            CONSTRUCTION,
            &[
                "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z", "AA", "AB", "AC", "AD",
                "AE", "AF", "AG", "BW", "BX", "BY", "BZ", "CA", "CB", "CC", "CD", "CE", "CF", "CG",
                "CH", "CI", "CJ", "CK", "CL", "CM", "CN", "CO",
            ],
        ),
        (EXPLORE, &["S", "T", "U", "V", "W", "X", "Y", "Z"]),
        (REZONE, &["L", "M", "N", "O", "P", "Q", "R"]),
        (
            MILITARY,
            &[
                "Y", "AG", "AH", "AI", "AJ", "AK", "AL", "AM", "AN", "AX", "AY", "AZ", "BA", "BB",
                "BC", "BD", "BE", "BF",
            ],
        ),
        (
            MAGIC,
            &[
                "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W",
            ],
        ),
        // The template pre-populates resource/improvement labels in all three
        // allocation slots. Clear those labels as well as the amounts so an
        // exported row contains only the allocations OVERTURE actually made.
        (IMPS, &["O", "P", "Q", "R", "S", "T", "U", "V", "W"]),
    ];
    for row in FIRST_ACTION_ROW..=LAST_ACTION_ROW {
        for (sheet, columns) in numeric_ranges {
            for column in *columns {
                edits.blank(sheet, format!("{column}{row}"));
            }
        }
    }
    for group in 0usize..=10 {
        let length = 11 - group;
        let base = 4 + 12 * group - (group * group.saturating_sub(1)) / 2;
        for offset in 0..length {
            edits.blank(TECHS, format!("F{}", base + offset));
        }
    }
}

fn write_overview(
    edits: &mut WorkbookEdits,
    plan_in: &Value,
    race_name: &str,
    start_land: i64,
) -> Result<(), String> {
    edits.text(OVERVIEW, "B14", race_name);
    let opening = plan_in
        .get("opening")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let columns = opening_columns();
    let mut values: HashMap<String, i64> = HashMap::new();
    let mut specified = 0i64;
    for (key, value) in opening {
        let amount = value
            .as_i64()
            .ok_or_else(|| format!("opening building {key:?} must be an integer"))?;
        if amount < 0 {
            return Err(format!("opening building {key:?} cannot be negative"));
        }
        if !columns.contains_key(key.as_str()) {
            return Err(format!(
                "the Round 51 Excel workbook has no opening cell for building {key:?}"
            ));
        }
        values.insert(key, amount);
        specified = specified
            .checked_add(amount)
            .ok_or_else(|| "opening building total overflowed".to_string())?;
    }
    if specified > start_land {
        return Err(format!(
            "opening build places {specified} buildings on only {start_land} starting acres"
        ));
    }
    let homes = values.get("home").copied().unwrap_or(0) + start_land - specified;
    values.insert("home".to_string(), homes);
    for (building, cell) in columns {
        edits.set_i64(OVERVIEW, cell, values.get(building).copied().unwrap_or(0));
    }
    Ok(())
}

fn write_hour(
    edits: &mut WorkbookEdits,
    hour: usize,
    actions: &[Value],
    resolved: Option<&ResolvedHour>,
) -> Result<(), String> {
    let row = FIRST_ACTION_ROW + hour - 1;
    let mut improvements = Vec::new();
    let mut invest_all_index = 0usize;

    for action in actions {
        let kind = action.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "construct" | "destroy" => {
                let building = action
                    .get("building")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("hour {hour} {kind} action has no building"))?;
                let amount = action_amount(action, "n", hour, kind)?;
                let column = if kind == "construct" {
                    construct_columns().get(building).copied()
                } else {
                    destroy_columns().get(building).copied()
                }
                .ok_or_else(|| {
                    format!("the Round 51 Excel workbook cannot {kind} building {building:?}")
                })?;
                edits.add_i64(CONSTRUCTION, format!("{column}{row}"), amount)?;
            }
            "explore" => {
                let land = action
                    .get("land")
                    .and_then(Value::as_str)
                    .unwrap_or("plain");
                let amount = action_amount(action, "n", hour, kind)?;
                let column = land_columns().get(land).copied().ok_or_else(|| {
                    format!("the Round 51 Excel workbook cannot explore land {land:?}")
                })?;
                edits.add_i64(EXPLORE, format!("{column}{row}"), amount)?;
            }
            "rezone" => {
                let from = action
                    .get("from")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("hour {hour} rezone action has no source land"))?;
                let to = action
                    .get("to")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("hour {hour} rezone action has no target land"))?;
                let amount = action_amount(action, "n", hour, kind)?;
                let columns = rezone_columns();
                let from_column = columns.get(from).copied().ok_or_else(|| {
                    format!("the Round 51 Excel workbook cannot rezone land {from:?}")
                })?;
                let to_column = columns.get(to).copied().ok_or_else(|| {
                    format!("the Round 51 Excel workbook cannot rezone land {to:?}")
                })?;
                let removed = amount
                    .checked_neg()
                    .ok_or_else(|| format!("hour {hour} rezone amount is too large"))?;
                edits.add_i64(REZONE, format!("{from_column}{row}"), removed)?;
                edits.add_i64(REZONE, format!("{to_column}{row}"), amount)?;
            }
            "train" => {
                let slot = action_slot(action)?;
                let amount = action_amount(action, "n", hour, kind)?;
                let column = train_columns().get(slot.as_str()).copied().ok_or_else(|| {
                    format!("the Round 51 Excel workbook cannot train slot {slot:?}")
                })?;
                edits.add_i64(MILITARY, format!("{column}{row}"), amount)?;
            }
            "release" => {
                let slot = release_slot(action)?;
                let amount = action_amount(action, "n", hour, kind)?;
                let column = release_columns().get(slot.as_str()).copied().ok_or_else(|| {
                    format!("the Round 51 Excel workbook cannot release slot {slot:?}")
                })?;
                edits.add_i64(MILITARY, format!("{column}{row}"), amount)?;
            }
            "spell" => {
                let spell = action
                    .get("spell")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("hour {hour} spell action has no spell"))?;
                let column = spell_columns().get(spell).copied().ok_or_else(|| {
                    format!(
                        "the Round 51 Excel workbook has no input column for spell {spell:?} (hour {hour})"
                    )
                })?;
                edits.set_i64(MAGIC, format!("{column}{row}"), 1);
            }
            "bank" => {}
            "claim_platinum" => edits.set_i64(PRODUCTION, format!("C{row}"), 1),
            "claim_land" => edits.set_i64(EXPLORE, format!("S{row}"), 1),
            "draft_rate" => {
                let rate = action
                    .get("rate")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| format!("hour {hour} draft-rate action has no integer rate"))?;
                edits.set_f64(MILITARY, format!("Y{row}"), rate as f64 / 100.0);
            }
            "research" => {
                let tech = action
                    .get("tech")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("hour {hour} research action has no tech"))?;
                let tech_row = excel_tech_row(tech).ok_or_else(|| {
                    format!("the Round 51 Excel workbook has no row for tech {tech:?}")
                })?;
                edits.set_i64(TECHS, format!("F{tech_row}"), hour as i64);
            }
            "improve" => {
                let all_amount = if action.get("all").and_then(Value::as_bool) == Some(true) {
                    let amount = resolved
                        .and_then(|hour| hour.invest_all_amounts.get(invest_all_index))
                        .copied()
                        .ok_or_else(|| {
                            format!("could not resolve hour {hour} invest-all action")
                        })?;
                    invest_all_index += 1;
                    Some(amount)
                } else {
                    None
                };
                append_improvements(hour, action, all_amount, &mut improvements)?;
            }
            "" => return Err(format!("hour {hour} action has no type")),
            other => {
                return Err(format!(
                    "the Round 51 Excel workbook cannot represent OVERTURE action {other:?} at hour {hour}"
                ))
            }
        }
    }

    if let Some(resolved) = resolved {
        if !resolved.bank_deltas.is_empty() {
            for (resource, column) in RESOURCE_COLUMNS {
                edits.set_i64(
                    PRODUCTION,
                    format!("{column}{row}"),
                    resolved.bank_deltas.get(*resource).copied().unwrap_or(0),
                );
            }
        }
    }
    write_improvements(edits, hour, row, improvements)?;
    Ok(())
}

fn append_improvements(
    hour: usize,
    action: &Value,
    all_amount: Option<i64>,
    output: &mut Vec<ImprovementAllocation>,
) -> Result<(), String> {
    let resource = action
        .get("resource")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("hour {hour} improvement action has no resource"))?
        .trim_start_matches("resource_")
        .to_string();
    if !RESOURCE_COLUMNS.iter().any(|(name, _)| *name == resource) {
        return Err(format!(
            "the Round 51 Excel workbook cannot invest resource {resource:?} at hour {hour}"
        ));
    }
    let data = action
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("hour {hour} improvement action has no allocation data"))?;
    let nonzero = data
        .iter()
        .filter_map(|(improvement, value)| {
            let amount = value.as_i64()?;
            (amount > 0).then_some((improvement.clone(), amount))
        })
        .collect::<Vec<_>>();
    if let Some(amount) = all_amount {
        let target = data
            .keys()
            .next()
            .ok_or_else(|| format!("hour {hour} invest-all action has no target"))?;
        output.push(ImprovementAllocation {
            resource,
            improvement: target.clone(),
            amount,
        });
    } else {
        for (improvement, amount) in nonzero {
            output.push(ImprovementAllocation {
                resource: resource.clone(),
                improvement,
                amount,
            });
        }
    }
    Ok(())
}

fn write_improvements(
    edits: &mut WorkbookEdits,
    hour: usize,
    row: usize,
    allocations: Vec<ImprovementAllocation>,
) -> Result<(), String> {
    let mut merged: Vec<ImprovementAllocation> = Vec::new();
    for allocation in allocations
        .into_iter()
        .filter(|allocation| allocation.amount > 0)
    {
        if !improvement_names().contains_key(allocation.improvement.as_str()) {
            return Err(format!(
                "the Round 51 Excel workbook has no improvement named {:?} (hour {hour})",
                allocation.improvement
            ));
        }
        if let Some(existing) = merged.iter_mut().find(|existing| {
            existing.resource == allocation.resource
                && existing.improvement == allocation.improvement
        }) {
            existing.amount = existing
                .amount
                .checked_add(allocation.amount)
                .ok_or_else(|| format!("hour {hour} improvement allocation overflowed"))?;
        } else {
            merged.push(allocation);
        }
    }
    if merged.len() > 3 {
        return Err(format!(
            "hour {hour} has {} distinct improvement allocations, but Excel exposes only 3 slots",
            merged.len()
        ));
    }
    for (index, allocation) in merged.iter().enumerate() {
        let (resource_column, amount_column, improvement_column) =
            [("O", "P", "Q"), ("R", "S", "T"), ("U", "V", "W")][index];
        edits.text(
            IMPS,
            format!("{resource_column}{row}"),
            title_case(&allocation.resource),
        );
        edits.set_i64(IMPS, format!("{amount_column}{row}"), allocation.amount);
        edits.text(
            IMPS,
            format!("{improvement_column}{row}"),
            improvement_names()[allocation.improvement.as_str()],
        );
    }
    Ok(())
}

fn resolve_hour_actions(
    replay_plan: &Value,
    scenario: &plan::Scenario,
) -> Result<HashMap<usize, ResolvedHour>, String> {
    let events = plan::scenario_events_from_overture_plan(replay_plan)?;
    let states = plan::run_with_events(scenario, &events)?;
    const BASE: usize = 2;
    let protection = scenario.ticks.len();
    if protection != PROTECTION_HOURS {
        return Err(format!(
            "Excel export expected {PROTECTION_HOURS} protection hours, got {protection}"
        ));
    }
    let has_oop = !scenario.oop_actions.is_empty()
        || !scenario.post_oop_ticks.is_empty()
        || !events.is_empty();
    let state_index = |hour: usize| -> usize {
        if has_oop && hour > protection {
            BASE + hour
        } else {
            BASE + hour.saturating_sub(1)
        }
    };
    let mut output: HashMap<usize, ResolvedHour> = HashMap::new();

    for hour in 1..=PROTECTION_HOURS {
        let mut state = states
            .get(state_index(hour))
            .ok_or_else(|| format!("missing entering state for hour {hour}"))?
            .clone();
        resolve_actions_into(
            hour,
            &mut state,
            scenario
                .ticks
                .get(hour - 1)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            output.entry(hour).or_default(),
        )?;
    }

    if !scenario.oop_actions.is_empty() {
        let mut state = states
            .get(BASE + PROTECTION_HOURS)
            .ok_or_else(|| "missing pre-OOP state".to_string())?
            .clone();
        resolve_actions_into(
            OOP_HOUR,
            &mut state,
            &scenario.oop_actions,
            output.entry(OOP_HOUR).or_default(),
        )?;
    }

    for (index, actions) in scenario.post_oop_ticks.iter().enumerate() {
        let hour = OOP_HOUR + index;
        if hour > MAX_EXCEL_HOUR {
            break;
        }
        let mut state = states
            .get(state_index(hour))
            .ok_or_else(|| format!("missing entering state for post-OOP hour {hour}"))?
            .clone();
        resolve_actions_into(hour, &mut state, actions, output.entry(hour).or_default())?;
    }
    Ok(output)
}

fn resolve_actions_into(
    hour: usize,
    state: &mut DominionState,
    actions: &[Value],
    output: &mut ResolvedHour,
) -> Result<(), String> {
    for action in actions {
        let action_type = action.get("type").and_then(Value::as_str).unwrap_or("");
        if action_type == "improve" && action.get("all").and_then(Value::as_bool) == Some(true) {
            let resource = action.get("resource").and_then(Value::as_str).unwrap_or("");
            output
                .invest_all_amounts
                .push(race_resources::resource_get(state, resource).max(0));
        }
        if action_type == "bank" {
            let source = action
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim_start_matches("resource_");
            let target = action
                .get("target")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim_start_matches("resource_");
            if !RESOURCE_COLUMNS
                .iter()
                .any(|(resource, _)| *resource == source)
                || !RESOURCE_COLUMNS
                    .iter()
                    .any(|(resource, _)| *resource == target)
            {
                return Err(format!(
                    "the Round 51 Excel workbook cannot represent bank exchange {source}->{target} at hour {hour}"
                ));
            }
            let before = workbook_resource_snapshot(state);
            plan::apply_action(state, action);
            let after = workbook_resource_snapshot(state);
            for (resource, _) in RESOURCE_COLUMNS {
                let delta = after[resource] - before[resource];
                if delta != 0 {
                    *output
                        .bank_deltas
                        .entry((*resource).to_string())
                        .or_default() += delta;
                }
            }
        } else {
            plan::apply_action(state, action);
        }
    }
    Ok(())
}

fn workbook_resource_snapshot(state: &DominionState) -> BTreeMap<&'static str, i64> {
    RESOURCE_COLUMNS
        .iter()
        .map(|(resource, _)| (*resource, race_resources::resource_get(state, resource)))
        .collect()
}

fn app_actions_for_hour(plan_in: &Value, hour: usize) -> Vec<Value> {
    let mut output = Vec::new();
    if hour == OOP_HOUR {
        if let Some(actions) = plan_in
            .get("oopActions")
            .or_else(|| plan_in.get("oop_actions"))
            .and_then(Value::as_array)
        {
            output.extend(actions.iter().cloned());
        }
    }
    if let Some(actions) = plan_in
        .get("hours")
        .and_then(Value::as_array)
        .and_then(|hours| hours.get(hour - 1))
        .and_then(Value::as_array)
    {
        output.extend(actions.iter().cloned());
    }
    output
}

fn has_excel_order_risk(actions: &[Value]) -> bool {
    let claim_position = actions
        .iter()
        .position(|action| action.get("type").and_then(Value::as_str) == Some("claim_land"));
    let scaled_position = actions.iter().position(|action| {
        matches!(
            action.get("type").and_then(Value::as_str),
            Some("construct" | "explore" | "rezone")
        )
    });
    claim_position
        .zip(scaled_position)
        .is_some_and(|(claim, scaled)| claim < scaled)
        || actions
            .iter()
            .any(|action| action.get("type").and_then(Value::as_str) == Some("destroy"))
            && actions
                .iter()
                .any(|action| action.get("type").and_then(Value::as_str) == Some("construct"))
}

fn patch_template(
    sheet_edits: &HashMap<&'static str, BTreeMap<String, CellValue>>,
) -> Result<Vec<u8>, String> {
    let template_digest = format!("{:x}", Sha256::digest(TEMPLATE));
    if template_digest != REVIEWED_TEMPLATE_SHA256 {
        return Err(
            "embedded Excel template failed its reviewed SHA-256 integrity check".to_string(),
        );
    }
    let mut source = ZipArchive::new(Cursor::new(TEMPLATE))
        .map_err(|error| format!("could not read embedded Excel template: {error}"))?;
    let mut output = ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..source.len() {
        let mut entry = source
            .by_index(index)
            .map_err(|error| format!("could not read Excel entry {index}: {error}"))?;
        let name = entry.name().to_string();
        // The chain belongs to the template's old inputs. Keeping it can make
        // Excel-compatible readers trust a partial dependency graph and show
        // the template's cached 425-acre result instead of the exported plan.
        if name == CALC_CHAIN {
            continue;
        }
        let options = SimpleFileOptions::default().compression_method(entry.compression());
        if entry.is_dir() {
            output
                .add_directory(&name, options)
                .map_err(|error| format!("could not copy Excel directory {name}: {error}"))?;
            continue;
        }
        output
            .start_file(&name, options)
            .map_err(|error| format!("could not copy Excel entry {name}: {error}"))?;
        if name.starts_with("xl/worksheets/") && name.ends_with(".xml") {
            let mut xml = String::new();
            entry
                .read_to_string(&mut xml)
                .map_err(|error| format!("could not read Excel sheet {name}: {error}"))?;
            if let Some(edits) = sheet_edits.get(name.as_str()) {
                for (cell, value) in edits {
                    patch_cell(&mut xml, cell, value)
                        .map_err(|error| format!("{name}: {error}"))?;
                }
            }
            remove_formula_cached_values(&mut xml).map_err(|error| format!("{name}: {error}"))?;
            output
                .write_all(xml.as_bytes())
                .map_err(|error| format!("could not write Excel sheet {name}: {error}"))?;
        } else if name == WORKBOOK {
            let mut xml = String::new();
            entry
                .read_to_string(&mut xml)
                .map_err(|error| format!("could not read Excel workbook metadata: {error}"))?;
            remove_absolute_path_metadata(&mut xml)?;
            mark_full_recalculation(&mut xml)?;
            output
                .write_all(xml.as_bytes())
                .map_err(|error| format!("could not write Excel workbook metadata: {error}"))?;
        } else if name == WORKBOOK_RELS {
            let mut xml = String::new();
            entry
                .read_to_string(&mut xml)
                .map_err(|error| format!("could not read Excel workbook relationships: {error}"))?;
            remove_calc_chain_relationship(&mut xml)?;
            output.write_all(xml.as_bytes()).map_err(|error| {
                format!("could not write Excel workbook relationships: {error}")
            })?;
        } else if name == CONTENT_TYPES {
            let mut xml = String::new();
            entry
                .read_to_string(&mut xml)
                .map_err(|error| format!("could not read Excel content types: {error}"))?;
            remove_calc_chain_content_type(&mut xml)?;
            output
                .write_all(xml.as_bytes())
                .map_err(|error| format!("could not write Excel content types: {error}"))?;
        } else {
            std::io::copy(&mut entry, &mut output)
                .map_err(|error| format!("could not copy Excel entry {name}: {error}"))?;
        }
    }
    output
        .finish()
        .map(|cursor| cursor.into_inner())
        .map_err(|error| format!("could not finish Excel workbook: {error}"))
}

/// Excel stores the author's last local workbook directory in an optional
/// `x15ac:absPath` compatibility block. It is unnecessary for calculation and
/// should not be propagated into files users distribute.
fn remove_absolute_path_metadata(xml: &mut String) -> Result<(), String> {
    let Some(path_at) = xml.find("<x15ac:absPath") else {
        return Ok(());
    };
    let start = xml[..path_at]
        .rfind("<mc:AlternateContent")
        .ok_or_else(|| "Excel template has malformed absolute-path metadata".to_string())?;
    let close = "</mc:AlternateContent>";
    let end = xml[path_at..]
        .find(close)
        .map(|offset| path_at + offset + close.len())
        .ok_or_else(|| "Excel template has unterminated absolute-path metadata".to_string())?;
    xml.replace_range(start..end, "");
    Ok(())
}

/// Cached formula results belong to the blank template. Ask Excel to rebuild
/// them once when the populated copy opens, then resume its ordinary automatic
/// dependency-based calculation behavior.
fn mark_full_recalculation(xml: &mut String) -> Result<(), String> {
    let start = xml
        .find("<calcPr")
        .ok_or_else(|| "Excel template has no workbook calculation properties".to_string())?;
    let tag_end = xml[start..]
        .find('>')
        .map(|offset| start + offset)
        .ok_or_else(|| "Excel template has malformed calculation properties".to_string())?;
    let tag = &xml[start..=tag_end];
    let mut attributes = tag
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim_end_matches('/')
        .split_whitespace()
        .skip(1)
        .filter(|attribute| {
            !attribute.starts_with("calcId=")
                && !attribute.starts_with("calcMode=")
                && !attribute.starts_with("fullCalcOnLoad=")
                && !attribute.starts_with("forceFullCalc=")
                && !attribute.starts_with("calcOnSave=")
        })
        .collect::<Vec<_>>();
    attributes.push("calcId=\"0\"");
    attributes.push("calcMode=\"auto\"");
    attributes.push("fullCalcOnLoad=\"1\"");
    attributes.push("forceFullCalc=\"1\"");
    attributes.push("calcOnSave=\"1\"");
    let replacement = format!("<calcPr {}/>", attributes.join(" "));
    xml.replace_range(start..=tag_end, &replacement);
    Ok(())
}

/// Formula `<v>` nodes are cached display results, not formulas. They still
/// contain the template author's prior 50-building/425-acre simulation. Direct
/// OOXML input edits do not invalidate those values, and some readers display
/// them instead of honoring `fullCalcOnLoad`. Remove only caches from cells
/// that contain a formula; numeric/text inputs remain untouched.
fn remove_formula_cached_values(xml: &mut String) -> Result<usize, String> {
    let mut search_from = 0usize;
    let mut ranges = Vec::new();
    while let Some(relative_start) = xml[search_from..].find("<c ") {
        let cell_start = search_from + relative_start;
        let tag_end = xml[cell_start..]
            .find('>')
            .map(|offset| cell_start + offset)
            .ok_or_else(|| "formula-cache scan found a malformed cell tag".to_string())?;
        if xml[cell_start..=tag_end].ends_with("/>") {
            search_from = tag_end + 1;
            continue;
        }
        let cell_close = xml[tag_end + 1..]
            .find("</c>")
            .map(|offset| tag_end + 1 + offset)
            .ok_or_else(|| "formula-cache scan found an unterminated cell".to_string())?;
        let body_start = tag_end + 1;
        let body = &xml[body_start..cell_close];
        if body.contains("<f") {
            if let Some(value_offset) = body.find("<v") {
                let value_start = body_start + value_offset;
                let value_tag_end = xml[value_start..cell_close]
                    .find('>')
                    .map(|offset| value_start + offset)
                    .ok_or_else(|| {
                        "formula-cache scan found a malformed cached value".to_string()
                    })?;
                let value_end = if xml[value_start..=value_tag_end].ends_with("/>") {
                    value_tag_end + 1
                } else {
                    xml[value_tag_end + 1..cell_close]
                        .find("</v>")
                        .map(|offset| value_tag_end + 1 + offset + 4)
                        .ok_or_else(|| {
                            "formula-cache scan found an unterminated cached value".to_string()
                        })?
                };
                ranges.push((value_start, value_end));
            }
        }
        search_from = cell_close + 4;
    }
    for (start, end) in ranges.iter().rev().copied() {
        xml.replace_range(start..end, "");
    }
    Ok(ranges.len())
}

fn remove_calc_chain_relationship(xml: &mut String) -> Result<(), String> {
    remove_self_closing_element(
        xml,
        "Relationship",
        "Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain\"",
        "calculation-chain relationship",
    )
}

fn remove_calc_chain_content_type(xml: &mut String) -> Result<(), String> {
    remove_self_closing_element(
        xml,
        "Override",
        "PartName=\"/xl/calcChain.xml\"",
        "calculation-chain content type",
    )
}

fn remove_self_closing_element(
    xml: &mut String,
    element: &str,
    marker: &str,
    description: &str,
) -> Result<(), String> {
    let marker_at = xml
        .find(marker)
        .ok_or_else(|| format!("Excel template has no {description}"))?;
    let opening = format!("<{element} ");
    let start = xml[..marker_at]
        .rfind(&opening)
        .ok_or_else(|| format!("Excel template has a malformed {description}"))?;
    let end = xml[marker_at..]
        .find("/>")
        .map(|offset| marker_at + offset + 2)
        .ok_or_else(|| format!("Excel template has an unterminated {description}"))?;
    xml.replace_range(start..end, "");
    Ok(())
}

fn patch_cell(xml: &mut String, coordinate: &str, value: &CellValue) -> Result<(), String> {
    let marker = format!("r=\"{coordinate}\"");
    let marker_at = xml
        .find(&marker)
        .ok_or_else(|| format!("template is missing expected input cell {coordinate}"))?;
    let start = xml[..marker_at]
        .rfind("<c ")
        .ok_or_else(|| format!("could not find opening tag for cell {coordinate}"))?;
    let tag_end_rel = xml[start..]
        .find('>')
        .ok_or_else(|| format!("could not find end of opening tag for cell {coordinate}"))?;
    let tag_end = start + tag_end_rel;
    let tag = &xml[start..=tag_end];
    let self_closing = tag.ends_with("/>");
    let end = if self_closing {
        tag_end + 1
    } else {
        let close_rel = xml[tag_end + 1..]
            .find("</c>")
            .ok_or_else(|| format!("could not find closing tag for cell {coordinate}"))?;
        tag_end + 1 + close_rel + 4
    };
    let base_tag = normalize_cell_tag(tag);
    let replacement = match value {
        CellValue::Blank => format!("{base_tag}/>"),
        CellValue::Number(number) => format!("{base_tag}><v>{number}</v></c>"),
        CellValue::Text(text) => format!(
            "{base_tag} t=\"inlineStr\"><is><t>{}</t></is></c>",
            xml_escape(text)
        ),
    };
    xml.replace_range(start..end, &replacement);
    Ok(())
}

fn normalize_cell_tag(tag: &str) -> String {
    let inner = tag
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim_end_matches('/')
        .trim();
    let mut parts = inner.split_whitespace();
    let name = parts.next().unwrap_or("c");
    let attrs = parts
        .filter(|part| !part.starts_with("t="))
        .collect::<Vec<_>>();
    if attrs.is_empty() {
        format!("<{name}")
    } else {
        format!("<{name} {}", attrs.join(" "))
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn action_amount(action: &Value, key: &str, hour: usize, kind: &str) -> Result<i64, String> {
    action
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("hour {hour} {kind} action has no integer {key}"))
}

fn action_slot(action: &Value) -> Result<String, String> {
    if let Some(slot) = action.get("slot").and_then(Value::as_i64) {
        return Ok(slot.to_string());
    }
    action
        .get("slot")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| "training action has no slot".to_string())
}

fn release_slot(action: &Value) -> Result<String, String> {
    if action.get("unit").and_then(Value::as_str) == Some("draftees") {
        return Ok("draftees".to_string());
    }
    if let Some(unit) = action.get("unit").and_then(Value::as_str) {
        return Ok(unit.to_string());
    }
    action_slot(action)
}

fn excel_tech_row(key: &str) -> Option<usize> {
    let rest = key.strip_prefix("tech_")?;
    let (x, y) = rest.split_once('_')?;
    let x = x.parse::<usize>().ok()?;
    let y = y.parse::<usize>().ok()?;
    if y == 0 || y > 21 || y % 2 == 0 {
        return None;
    }
    let group = (y - 1) / 2;
    let start_x = group + 1;
    if x < start_x || (x - start_x) % 2 != 0 {
        return None;
    }
    let offset = (x - start_x) / 2;
    let length = 11usize.checked_sub(group)?;
    if offset >= length {
        return None;
    }
    let base = 4 + 12 * group - (group * group.saturating_sub(1)) / 2;
    Some(base + offset)
}

fn title_case(value: &str) -> String {
    value
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn opening_columns() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("home", "L16"),
        ("alchemy", "L17"),
        ("farm", "L18"),
        ("smithy", "L19"),
        ("masonry", "L20"),
        ("lumberyard", "L21"),
        ("forest_haven", "L22"),
        ("ore_mine", "L23"),
        ("gryphon_nest", "L24"),
        ("barracks", "L25"),
        ("factory", "L26"),
        ("guard_tower", "L27"),
        ("shrine", "L28"),
        ("tower", "L29"),
        ("temple", "L30"),
        ("wizard_guild", "L31"),
        ("diamond_mine", "L32"),
        ("school", "L33"),
        ("dock", "L34"),
    ])
}

fn construct_columns() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("home", "O"),
        ("alchemy", "P"),
        ("farm", "Q"),
        ("smithy", "R"),
        ("masonry", "S"),
        ("lumberyard", "T"),
        ("forest_haven", "U"),
        ("ore_mine", "V"),
        ("gryphon_nest", "W"),
        ("factory", "X"),
        ("guard_tower", "Y"),
        ("barracks", "Z"),
        ("shrine", "AA"),
        ("tower", "AB"),
        ("temple", "AC"),
        ("wizard_guild", "AD"),
        ("diamond_mine", "AE"),
        ("school", "AF"),
        ("dock", "AG"),
    ])
}

fn destroy_columns() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("home", "BW"),
        ("alchemy", "BX"),
        ("farm", "BY"),
        ("smithy", "BZ"),
        ("masonry", "CA"),
        ("lumberyard", "CB"),
        ("forest_haven", "CC"),
        ("ore_mine", "CD"),
        ("gryphon_nest", "CE"),
        ("factory", "CF"),
        ("guard_tower", "CG"),
        ("barracks", "CH"),
        ("shrine", "CI"),
        ("tower", "CJ"),
        ("temple", "CK"),
        ("wizard_guild", "CL"),
        ("diamond_mine", "CM"),
        ("school", "CN"),
        ("dock", "CO"),
    ])
}

fn land_columns() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("plain", "T"),
        ("forest", "U"),
        ("mountain", "V"),
        ("hill", "W"),
        ("swamp", "X"),
        ("cavern", "Y"),
        ("water", "Z"),
    ])
}

fn rezone_columns() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("plain", "L"),
        ("forest", "M"),
        ("mountain", "N"),
        ("hill", "O"),
        ("swamp", "P"),
        ("cavern", "Q"),
        ("water", "R"),
    ])
}

fn train_columns() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("1", "AG"),
        ("2", "AH"),
        ("3", "AI"),
        ("4", "AJ"),
        ("spies", "AK"),
        ("assassins", "AL"),
        ("wizards", "AM"),
        ("archmages", "AN"),
    ])
}

fn release_columns() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("draftees", "AX"),
        ("1", "AY"),
        ("2", "AZ"),
        ("3", "BA"),
        ("4", "BB"),
        ("spies", "BC"),
        ("assassins", "BD"),
        ("wizards", "BE"),
        ("archmages", "BF"),
    ])
}

fn spell_columns() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("gaias_watch", "G"),
        ("mining_strength", "H"),
        ("ares_call", "I"),
        ("midas_touch", "J"),
        ("harmony", "K"),
        ("miners_sight", "L"),
        ("mechanical_genius", "M"),
        ("death_and_decay", "N"),
        ("killing_rage", "O"),
        ("bloodrage", "O"),
        ("crusade", "O"),
        ("warsong", "O"),
        ("gaias_blessing", "P"),
        ("howling", "Q"),
        ("arcane_infusion", "R"),
        ("alchemist_flame", "S"),
        ("nightfall", "T"),
        ("favorable_terrain", "U"),
        ("infernal_command", "V"),
    ])
}

fn improvement_names() -> HashMap<&'static str, &'static str> {
    HashMap::from([
        ("science", "Science"),
        ("keep", "Keep"),
        ("spires", "Spires"),
        ("forges", "Forges"),
        ("walls", "Walls"),
        ("harbor", "Harbor"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn unzip_entry(bytes: &[u8], name: &str) -> Vec<u8> {
        let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("valid zip");
        let mut entry = archive.by_name(name).expect("entry exists");
        let mut output = Vec::new();
        entry.read_to_end(&mut output).expect("read entry");
        output
    }

    fn zip_entry_exists(bytes: &[u8], name: &str) -> bool {
        let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("valid zip");
        let exists = archive.by_name(name).is_ok();
        exists
    }

    fn cell_fragment(bytes: &[u8], sheet: &str, cell: &str) -> String {
        let xml = String::from_utf8(unzip_entry(bytes, sheet)).expect("utf8 sheet");
        let marker = format!("r=\"{cell}\"");
        let marker_at = xml.find(&marker).expect("cell marker");
        let start = xml[..marker_at].rfind("<c ").expect("cell start");
        let tag_end = xml[start..]
            .find('>')
            .map(|offset| start + offset)
            .expect("opening tag end");
        let end = if xml[start..=tag_end].ends_with("/>") {
            tag_end + 1
        } else {
            xml[tag_end + 1..]
                .find("</c>")
                .map(|offset| tag_end + 1 + offset + 4)
                .expect("cell end")
        };
        xml[start..end].to_string()
    }

    #[test]
    fn exports_inputs_and_preserves_vba_verbatim() {
        let plan = json!({
            "race": "human",
            "opening": {"alchemy": 100, "farm": 25},
            "hours": [
                [
                    {"type":"claim_platinum"},
                    {"type":"claim_land"},
                    {"type":"explore","land":"plain","n":2},
                    {"type":"rezone","from":"plain","to":"hill","n":3},
                    {"type":"construct","building":"guard_tower","n":3},
                    {"type":"spell","spell":"midas_touch"},
                    {"type":"draft_rate","rate":75},
                    {"type":"research","tech":"tech_1_1"}
                ],
                [
                    {"type":"bank","source":"resource_platinum","target":"resource_ore","amount":1000},
                    {"type":"train","slot":2,"n":4},
                    {"type":"improve","resource":"gems","data":{"keep":12}}
                ]
            ]
        });
        let rendered = render_overture_plan(&plan).expect("export succeeds");
        assert_eq!(
            unzip_entry(&rendered.bytes, "xl/vbaProject.bin"),
            unzip_entry(TEMPLATE, "xl/vbaProject.bin"),
            "the VBA project must be copied byte-for-byte"
        );
        let workbook = String::from_utf8(unzip_entry(&rendered.bytes, WORKBOOK))
            .expect("workbook metadata is XML");
        let workbook_rels = String::from_utf8(unzip_entry(&rendered.bytes, WORKBOOK_RELS))
            .expect("workbook relationships are XML");
        let content_types = String::from_utf8(unzip_entry(&rendered.bytes, CONTENT_TYPES))
            .expect("content types are XML");
        assert!(workbook.contains("calcId=\"0\""));
        assert!(workbook.contains("calcMode=\"auto\""));
        assert!(workbook.contains("fullCalcOnLoad=\"1\""));
        assert!(workbook.contains("forceFullCalc=\"1\""));
        assert!(workbook.contains("calcOnSave=\"1\""));
        assert!(!zip_entry_exists(&rendered.bytes, CALC_CHAIN));
        assert!(!workbook_rels.contains("calcChain"));
        assert!(!content_types.contains("calcChain"));
        assert!(!workbook.contains("x15ac:absPath"));
        assert!(!workbook.contains("D:\\Desktop Files"));
        let opening_total = cell_fragment(&rendered.bytes, OVERVIEW, "L35");
        assert!(opening_total.contains("<f>SUM(Table20[Amount])</f>"));
        assert!(
            !opening_total.contains("<v>"),
            "stale template result must not survive export"
        );
        assert!(cell_fragment(&rendered.bytes, OVERVIEW, "B14").contains("Human"));
        assert!(cell_fragment(&rendered.bytes, OVERVIEW, "L16").contains("<v>225</v>"));
        assert!(cell_fragment(&rendered.bytes, OVERVIEW, "L17").contains("<v>100</v>"));
        assert!(cell_fragment(&rendered.bytes, EXPLORE, "S4").contains("<v>1</v>"));
        assert!(cell_fragment(&rendered.bytes, EXPLORE, "T4").contains("<v>2</v>"));
        assert!(cell_fragment(&rendered.bytes, REZONE, "L4").contains("<v>-3</v>"));
        assert!(cell_fragment(&rendered.bytes, REZONE, "O4").contains("<v>3</v>"));
        assert!(cell_fragment(&rendered.bytes, CONSTRUCTION, "Y4").contains("<v>3</v>"));
        assert!(cell_fragment(&rendered.bytes, MAGIC, "J4").contains("<v>1</v>"));
        assert!(cell_fragment(&rendered.bytes, MILITARY, "Y4").contains("<v>0.750000000000</v>"));
        assert!(cell_fragment(&rendered.bytes, TECHS, "F4").contains("<v>1</v>"));
        assert!(cell_fragment(&rendered.bytes, MILITARY, "AH5").contains("<v>4</v>"));
        assert!(cell_fragment(&rendered.bytes, IMPS, "P5").contains("<v>12</v>"));
        assert!(cell_fragment(&rendered.bytes, IMPS, "Q5").contains("Keep"));
        assert!(!cell_fragment(&rendered.bytes, IMPS, "R5").contains("<v>"));
        assert!(!cell_fragment(&rendered.bytes, IMPS, "T5").contains("<v>"));
        assert!(cell_fragment(&rendered.bytes, PRODUCTION, "BC5").contains("<v>-1000</v>"));
        assert!(cell_fragment(&rendered.bytes, PRODUCTION, "BE5").contains("<v>500</v>"));
    }

    #[test]
    fn removes_formula_caches_without_touching_inputs() {
        let mut xml = concat!(
            "<worksheet><sheetData><row>",
            "<c r=\"A1\"><f>1+1</f><v>2</v></c>",
            "<c r=\"A2\"><f t=\"shared\" si=\"0\"/><v>3</v></c>",
            "<c r=\"A3\"><v>7</v></c>",
            "</row></sheetData></worksheet>"
        )
        .to_string();
        assert_eq!(remove_formula_cached_values(&mut xml).unwrap(), 2);
        assert!(xml.contains("<c r=\"A1\"><f>1+1</f></c>"));
        assert!(xml.contains("<c r=\"A2\"><f t=\"shared\" si=\"0\"/></c>"));
        assert!(xml.contains("<c r=\"A3\"><v>7</v></c>"));
    }

    #[test]
    fn rejects_more_improvement_allocations_than_excel_exposes() {
        let plan = json!({
            "race": "human",
            "opening": {},
            "hours": [[
                {"type":"improve","resource":"gems","data":{
                    "science":1,"keep":1,"spires":1,"walls":1
                }}
            ]]
        });
        let error = render_overture_plan(&plan).err().expect("export must fail");
        assert!(error.contains("only 3 slots"), "unexpected error: {error}");
    }

    #[test]
    fn tech_coordinates_match_round_51_excel_rows() {
        assert_eq!(excel_tech_row("tech_1_1"), Some(4));
        assert_eq!(excel_tech_row("tech_21_1"), Some(14));
        assert_eq!(excel_tech_row("tech_2_3"), Some(16));
        assert_eq!(excel_tech_row("tech_11_21"), Some(79));
    }

    #[test]
    fn rejects_action_floods_before_replay() {
        let actions = (0..=MAX_ACTIONS_PER_HOUR)
            .map(|_| json!({"type":"claim_land"}))
            .collect::<Vec<_>>();
        let plan = json!({
            "race": "human",
            "opening": {},
            "hours": [actions]
        });
        let error = render_overture_plan(&plan).err().expect("export must fail");
        assert!(error.contains("at most 256"), "unexpected error: {error}");
    }

    #[test]
    fn rejects_action_values_that_exceed_the_export_safety_limit() {
        let plan = json!({
            "race": "human",
            "opening": {},
            "hours": [[{
                "type":"construct",
                "building":"home",
                "n": MAX_ACTION_AMOUNT + 1
            }]]
        });
        let error = render_overture_plan(&plan).err().expect("export must fail");
        assert!(error.contains("safety limit"), "unexpected error: {error}");
    }

    #[test]
    fn rejects_oversized_plan_payloads() {
        let plan = json!({
            "race": "human",
            "opening": {},
            "hours": [],
            "padding": "x".repeat(MAX_PLAN_BYTES)
        });
        let error = render_overture_plan(&plan).err().expect("export must fail");
        assert!(error.contains("too large"), "unexpected error: {error}");
    }

    #[test]
    fn safely_truncates_hours_beyond_the_excel_action_grid() {
        let plan = json!({
            "race": "human",
            "opening": {},
            "hours": vec![json!([]); MAX_EXCEL_HOUR + 1]
        });
        let rendered = render_overture_plan(&plan).expect("bounded export succeeds");
        assert!(rendered
            .warnings
            .iter()
            .any(|warning| warning.contains("later OVERTURE actions were not exported")));
    }
}
