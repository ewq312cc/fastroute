//! `rules apply`: turns a confirmed draft into inputs for fastroute and KiCad - and nothing else.
//!
//! It runs nothing, routes nothing and trusts nothing it was not told by a person: the
//! confirmation names the draft by its SHA-256, lists every rule that is executed, every
//! loosening that is accepted and every rule that is **not** executed (acknowledged one by one).
//! Only valid, supported, accepted `hard` / `soft` rules produce output. Values that go into files
//! come from checked numbers; net names go into files only if they are safe there.
//!
//! Acceptance of the result is not decided here: it is the real DRC of the routed board
//! (`fastroute ai-pack --kicad-pcb ...`), see `docs/AI_RULES.md`.

use std::collections::HashSet;

use serde::Serialize;
use serde_json::Value;

use super::check::CheckReport;

pub const CONFIRMATION_FORMAT: &str = "fastroute-rule-confirmation";

#[derive(Serialize)]
pub struct Executed {
    pub id: String,
    pub kind: String,
    /// The file that holds it, or `report` for a manual step.
    pub artifact: String,
    pub detail: String,
}

#[derive(Serialize)]
pub struct NotExecuted {
    pub id: String,
    pub kind: String,
    pub strength: String,
    pub reason: String,
}

pub struct Artifacts {
    pub pairs: Option<String>,
    pub tune: Option<String>,
    pub dru: Option<String>,
    pub route_args: Option<String>,
    pub executed: Vec<Executed>,
    pub not_executed: Vec<NotExecuted>,
    pub manual_steps: Vec<String>,
}

/// A net name that can be written into a pairs / tune file and a `.kicad_dru` condition as is.
pub fn safe_name(n: &str) -> bool {
    !n.is_empty() && n.chars().all(|c| !c.is_whitespace() && !c.is_control() && !matches!(c, '\'' | '"' | '\\' | '(' | ')' | '#' | '*' | '?'))
}

fn mm(v: f64) -> String {
    let s = format!("{:.6}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn condition(applies: &Value) -> Result<Option<String>, String> {
    if let Some(nets) = applies["nets"].as_array() {
        let names: Vec<&str> = nets.iter().filter_map(Value::as_str).collect();
        if let Some(bad) = names.iter().find(|n| !safe_name(n)) {
            return Err(format!("the net name '{bad}' cannot be written into a rule file safely"));
        }
        return Ok(Some(names.iter().map(|n| format!("A.NetName == '{n}'")).collect::<Vec<_>>().join(" || ")));
    }
    if let Some(c) = applies["net_class"].as_str() {
        if !safe_name(c) {
            return Err(format!("the net class name '{c}' cannot be written into a rule file safely"));
        }
        return Ok(Some(format!("A.NetClass == '{c}'")));
    }
    Ok(None)
}

/// The artifacts for the accepted rules of an already checked draft.
pub fn build(draft: &Value, report: &CheckReport, accepted: &HashSet<String>, draft_sha256: &str) -> Result<Artifacts, String> {
    let mut pairs = String::new();
    let mut tune = String::new();
    let mut dru = String::new();
    let mut a = Artifacts { pairs: None, tune: None, dru: None, route_args: None, executed: Vec::new(), not_executed: Vec::new(), manual_steps: Vec::new() };

    for rule in draft["rules"].as_array().into_iter().flatten() {
        let id = rule["id"].as_str().unwrap_or("").to_string();
        let kind = rule["kind"].as_str().unwrap_or("").to_string();
        let verdict = report.rules.iter().find(|v| v.id == id).ok_or_else(|| format!("rule {id} has no verdict"))?;
        if !accepted.contains(&id) {
            a.not_executed.push(NotExecuted {
                id,
                kind,
                strength: verdict.strength.clone(),
                reason: if verdict.executable { "not accepted in the confirmation".into() } else { verdict.reason.clone() },
            });
            continue;
        }
        let p = &rule["params"];
        let num = |k: &str| p[k].as_f64();
        let (artifact, detail) = match kind.as_str() {
            "diff_pair" => {
                let nets: Vec<&str> = rule["applies_to"]["nets"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                if nets.len() != 2 || nets.iter().any(|n| !safe_name(n)) {
                    return Err(format!("rule {id}: the pair nets cannot be written into a pairs file safely"));
                }
                let mut line = format!("pair {} {}", nets[0], nets[1]);
                if let Some(g) = num("gap_mm") {
                    line.push_str(&format!(" gap={}", mm(g)));
                }
                if let Some(k) = num("skew_mm") {
                    line.push_str(&format!(" skew={}", mm(k)));
                }
                pairs.push_str(&format!("# {id}\n{line}\n"));
                ("pairs.txt", line)
            }
            "length_match" => {
                let nets: Vec<&str> = rule["applies_to"]["nets"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                if nets.iter().any(|n| !safe_name(n)) {
                    return Err(format!("rule {id}: a net name cannot be written into a tune file safely"));
                }
                let mut line = format!("group {id} tolerance={}", mm(num("tolerance_mm").unwrap_or(0.5)));
                if let Some(t) = num("target_mm") {
                    line.push_str(&format!(" target={}", mm(t)));
                }
                tune.push_str(&format!("{line}\n  {}\n", nets.join("\n  ")));
                ("tune.txt", line)
            }
            "trace_width" | "clearance" | "length_max" => {
                let (constraint, value) = match kind.as_str() {
                    "trace_width" => (format!("track_width (min {w}mm) (opt {w}mm)", w = mm(num("width_mm").unwrap_or(0.0))), mm(num("width_mm").unwrap_or(0.0))),
                    "clearance" => (format!("clearance (min {}mm)", mm(num("clearance_mm").unwrap_or(0.0))), mm(num("clearance_mm").unwrap_or(0.0))),
                    _ => (format!("length (max {}mm)", mm(num("max_mm").unwrap_or(0.0))), mm(num("max_mm").unwrap_or(0.0))),
                };
                let cond = condition(&rule["applies_to"]).map_err(|e| format!("rule {id}: {e}"))?;
                let layer = p["layer"].as_str().filter(|l| safe_name(l.replace(' ', "_").as_str()));
                dru.push_str(&format!("(rule \"{id}\"\n"));
                if let Some(l) = layer {
                    dru.push_str(&format!("  (layer \"{l}\")\n"));
                }
                if let Some(c) = &cond {
                    dru.push_str(&format!("  (condition \"{c}\")\n"));
                }
                dru.push_str(&format!("  (constraint {constraint}))\n"));
                ("rules.kicad_dru", format!("{kind} {value} mm"))
            }
            "impedance" => {
                let ohm = num("target_ohm").unwrap_or(0.0);
                let diff = if p["differential"].as_bool() == Some(true) { "d" } else { "" };
                a.manual_steps.push(format!(
                    "{id}: put the nets {} into a KiCad net class, then run `python3 integrations/kicad/plugins/impedance_cli.py board.kicad_pcb --class <CLASS>={}{diff} --write` with KiCad's Python; it needs the board's stackup",
                    rule["applies_to"]["nets"].as_array().map(|n| n.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_else(|| "of the rule".into()),
                    mm(ohm)
                ));
                ("report", format!("impedance {} ohm: a manual step (see manual_steps)", mm(ohm)))
            }
            other => return Err(format!("rule {id}: a {other} rule has no way to be applied (the checker should have refused it)")),
        };
        a.executed.push(Executed { id, kind, artifact: artifact.to_string(), detail });
    }

    let header = |what: &str| format!("# {what}\n# generated by `fastroute rules apply` from the draft with SHA-256 {draft_sha256}\n# only rules that a person accepted; see apply-report.json\n\n");
    let mut args = Vec::new();
    if !pairs.is_empty() {
        a.pairs = Some(format!("{}{pairs}", header("differential pairs: fastroute --pairs=pairs.txt")));
        args.push("--pairs=pairs.txt");
    }
    if !tune.is_empty() {
        a.tune = Some(format!("{}{tune}", header("length matching: fastroute --tune=tune.txt")));
        args.push("--tune=tune.txt");
    }
    if !dru.is_empty() {
        // (a .kicad_dru starts with its version; `#` lines are comments)
        a.dru = Some(format!("(version 1)\n{}{dru}", header("rules for KiCad: put next to the board as <board>.kicad_dru. KiCad's DRC enforces all of them; the plugin hands the router clearance rules, diff_pair_gap and per-class track_width and prints NOT applied for the rest")));
    }
    if !args.is_empty() {
        a.route_args = Some(args.join("\n") + "\n");
    }
    Ok(a)
}
