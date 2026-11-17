//! The fastroute adapter: the rules of a Constraint IR that fastroute can execute, written as the
//! inputs fastroute reads (`--pairs`, `--tune`, a `.kicad_dru`), and a report of everything that was
//! not handed over and why. fastroute reads only these files: it never sees the pack, the IR, an
//! index or a vector score, and a rule that cannot be executed is listed, never dropped quietly.

use crate::ir::{Ir, IrRule};
use crate::snapshot::Snapshot;
use crate::validate::net_matches;
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub struct Export {
    pub pairs: Option<String>,
    pub tune: Option<String>,
    pub dru: Option<String>,
    pub route_args: Option<String>,
    pub report: Value,
}

/// A name that is safe to write into a fastroute input file or a rule file.
pub fn safe_name(n: &str) -> bool {
    !n.is_empty() && n.chars().all(|c| !c.is_whitespace() && !c.is_control() && !"\"'\\()#".contains(c))
}

fn mm(v: f64) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn nets_of(rule: &IrRule) -> Vec<String> {
    rule.params.get("nets").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// The nets a list of names or patterns stands for: with the design, every net that matches; without
/// it, only exact names (a pattern cannot be resolved and is reported).
fn resolve(patterns: &[String], design: Option<&Snapshot>) -> (Vec<String>, Vec<String>) {
    let mut found: Vec<String> = Vec::new();
    let mut unresolved = Vec::new();
    let known: BTreeSet<String> = design
        .map(|d| d.pins.iter().filter_map(|p| p.net.clone()).chain(d.tracks.iter().filter_map(|t| t.net.clone())).chain(d.vias.iter().filter_map(|v| v.net.clone())).collect())
        .unwrap_or_default();
    for p in patterns {
        let wild = p.contains(['*', '?']);
        match design {
            Some(_) => {
                let m: Vec<&String> = known.iter().filter(|n| net_matches(p, n)).collect();
                if m.is_empty() {
                    unresolved.push(p.clone());
                }
                for n in m {
                    if !found.contains(n) {
                        found.push(n.clone());
                    }
                }
            }
            None if wild => unresolved.push(p.clone()),
            None => found.push(p.clone()),
        }
    }
    (found, unresolved)
}

fn condition(nets: &[String]) -> Option<String> {
    if nets.is_empty() {
        None
    } else {
        Some(nets.iter().map(|n| format!("A.NetName == '{n}'")).collect::<Vec<_>>().join(" || "))
    }
}

/// Builds the fastroute inputs from the IR. `design` (optional) resolves net patterns and says which
/// nets exist.
pub fn export_fastroute(ir: &Ir, design: Option<&Snapshot>) -> Result<Export, String> {
    if !ir.verify_integrity() {
        return Err("the Constraint IR does not match its integrity hash: it was edited after compilation; compile the pack again".into());
    }
    let mut pairs = String::new();
    let mut tune = String::new();
    let mut dru = String::new();
    let mut exported: Vec<Value> = Vec::new();
    let mut not_exported: Vec<Value> = Vec::new();
    let mut unmapped: Vec<Value> = Vec::new();
    let mut conditions: Vec<Value> = Vec::new();
    let mut design_rules: Vec<String> = Vec::new();

    for r in &ir.rules {
        let skip = |reason: String, out: &mut Vec<Value>| out.push(json!({"rule_id": r.rule_id, "kind": r.kind, "status": r.status, "reason": reason}));
        match r.status.as_str() {
            "unknown" => {
                skip(format!("the documents do not say: {}", r.missing.clone().unwrap_or_default()), &mut not_exported);
                continue;
            }
            "conflict" => {
                skip("the documents disagree; a person has to decide before anything is handed to the router".into(), &mut not_exported);
                continue;
            }
            _ => {}
        }
        match r.execution.fastroute.as_str() {
            "not_applicable" => {
                design_rules.push(r.rule_id.clone());
                continue;
            }
            "unsupported" => {
                skip(r.execution.reason.clone().unwrap_or_else(|| "fastroute cannot execute this".into()), &mut not_exported);
                continue;
            }
            _ => {}
        }
        for p in &r.execution.unmapped_params {
            unmapped.push(json!({"rule_id": r.rule_id, "param": p, "reason": match p.as_str() {
                "impedance_ohm" => "the router has no impedance model (the plugin's impedance_cli.py derives widths from the stackup)",
                "layers" => "the router cannot restrict a net to layers by a rule file (a new core capability would be needed)",
                _ => "not handed over",
            }}));
        }
        if !r.applies_when.is_empty() {
            conditions.push(json!({"rule_id": r.rule_id, "applies_when": r.applies_when, "note": "conditions are words, not something this adapter can evaluate: the rule is exported for the nets it names"}));
        }
        let (nets, unresolved) = resolve(&nets_of(r), design);
        if !unresolved.is_empty() {
            unmapped.push(json!({"rule_id": r.rule_id, "param": "nets", "reason": format!("{} not resolved{}", unresolved.join(", "), if design.is_none() { " (a pattern needs the design: give --design)" } else { " (no such net in the design)" })}));
        }
        for n in &nets {
            if !safe_name(n) {
                skip(format!("the net name '{n}' cannot be written into a fastroute input safely"), &mut not_exported);
            }
        }
        if nets.iter().any(|n| !safe_name(n)) {
            continue;
        }
        match r.kind.as_str() {
            "high_speed" => {
                let diff = r.params.get("differential").and_then(Value::as_bool) == Some(true);
                let gap = r.params.get("gap_mm").and_then(Value::as_f64);
                let skew = r.params.get("skew_mm").and_then(Value::as_f64);
                let tol = r.params.get("length_match_tolerance_mm").and_then(Value::as_f64);
                if r.params.get("max_length_mm").is_some_and(|v| !v.is_null()) {
                    unmapped.push(json!({"rule_id": r.rule_id, "param": "max_length_mm", "reason": "the router does not enforce a maximum length (KiCad's DRC checks it)"}));
                }
                if diff {
                    if nets.len() != 2 {
                        skip(format!("a differential rule names exactly two nets, this resolves to {}", nets.len()), &mut not_exported);
                        continue;
                    }
                    let mut line = format!("pair {} {}", nets[0], nets[1]);
                    if let Some(g) = gap {
                        line.push_str(&format!(" gap={}", mm(g)));
                    }
                    if let Some(k) = skew {
                        line.push_str(&format!(" skew={}", mm(k)));
                    }
                    pairs.push_str(&format!("# {}\n{line}\n", r.rule_id));
                    exported.push(json!({"rule_id": r.rule_id, "kind": r.kind, "artifact": "pairs.txt", "detail": line}));
                } else if let Some(t) = tol {
                    if nets.len() < 2 {
                        skip("length matching needs at least two nets".into(), &mut not_exported);
                        continue;
                    }
                    let line = format!("group {} tolerance={}", r.rule_id.replace('-', "_"), mm(t));
                    tune.push_str(&format!("{line}\n  {}\n", nets.join("\n  ")));
                    exported.push(json!({"rule_id": r.rule_id, "kind": r.kind, "artifact": "tune.txt", "detail": line}));
                } else if gap.is_none() && skew.is_none() {
                    skip("nothing to hand to the router (no spacing, skew or length-matching parameter that is not unknown)".into(), &mut not_exported);
                } else {
                    skip("spacing and skew apply to a differential pair; this rule is single-ended".into(), &mut not_exported);
                }
            }
            "clearance" | "trace_width" => {
                let (constraint, detail) = if r.kind == "clearance" {
                    let v = r.params["clearance_mm"].as_f64().unwrap_or(0.0);
                    (format!("clearance (min {}mm)", mm(v)), format!("clearance {} mm", mm(v)))
                } else {
                    let v = r.params["width_mm"].as_f64().unwrap_or(0.0);
                    (format!("track_width (min {w}mm) (opt {w}mm)", w = mm(v)), format!("track width {} mm", mm(v)))
                };
                dru.push_str(&format!("(rule \"{}\"\n", r.rule_id));
                if let Some(c) = condition(&nets) {
                    dru.push_str(&format!("  (condition \"{c}\")\n"));
                }
                dru.push_str(&format!("  (constraint {constraint}))\n"));
                exported.push(json!({"rule_id": r.rule_id, "kind": r.kind, "artifact": "rules.kicad_dru", "detail": detail}));
            }
            other => skip(format!("no mapping for the kind '{other}'"), &mut not_exported),
        }
    }

    let header = |what: &str| format!("# {what}\n# generated by `fastroute constraints export-fastroute` from the Constraint IR of {} {} (IR sha256 {})\n\n", ir.pack.id, ir.pack.version, ir.integrity.sha256);
    let mut args = Vec::new();
    let pairs_out = (!pairs.is_empty()).then(|| {
        args.push("--pairs=pairs.txt");
        format!("{}{pairs}", header("differential pairs: fastroute --pairs=pairs.txt"))
    });
    let tune_out = (!tune.is_empty()).then(|| {
        args.push("--tune=tune.txt");
        format!("{}{tune}", header("length matching: fastroute --tune=tune.txt"))
    });
    let dru_out = (!dru.is_empty()).then(|| format!("(version 1)\n{}{dru}", header("rules for KiCad and the plugin: put next to the board as <board>.kicad_dru")));
    let route_args = (!args.is_empty()).then(|| args.join("\n") + "\n");
    let mut next = Vec::new();
    if route_args.is_some() {
        next.push("route with the written options: `fastroute route board.dsn -o routed.ses --pairs pairs.txt --tune tune.txt` (use the files that exist; route-args.txt lists them)".to_string());
    }
    if dru_out.is_some() {
        next.push("put rules.kicad_dru next to the board as <board>.kicad_dru: KiCad's DRC checks it and the KiCad plugin hands clearance and track-width rules to the router".to_string());
    }
    next.push("check the routed board with KiCad's own DRC (`fastroute ai-pack ... --kicad-pcb routed.kicad_pcb --require-drc`) and with `fastroute constraints validate`: they are separate results".to_string());
    let report = json!({
        "format": "fastroute-hcp-export",
        "format_version": "1.0",
        "fastroute_version": env!("CARGO_PKG_VERSION"),
        "ir": {"pack_id": ir.pack.id, "pack_version": ir.pack.version, "pack_sha256": ir.pack.sha256, "ir_sha256": ir.integrity.sha256},
        "exported": exported,
        "not_exported": not_exported,
        "unmapped_params": unmapped,
        "conditions_not_evaluated": conditions,
        "checked_by_the_validator_not_the_router": design_rules,
        "next_steps": next,
    });
    Ok(Export { pairs: pairs_out, tune: tune_out, dru: dru_out, route_args, report })
}
