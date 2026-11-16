//! HCP -> Constraint IR. Deterministic (the same pack and compiler give the same bytes), keeps the
//! evidence of every rule, and says for each rule what the validator and the fastroute adapter can
//! and cannot do with it.

use crate::diag::{Diagnostic, Severity};
use crate::hcp::{self, canonical_json, sha256_hex};
use crate::ir::*;
use serde_json::{json, Value};

fn s(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

/// Natural order of pin numbers: 2 before 10, numbers before names ("A1").
pub fn pin_order(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        (Ok(_), Err(_)) => std::cmp::Ordering::Less,
        (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
        _ => a.cmp(b),
    }
}

fn evidence(v: Option<&Value>) -> Vec<IrEvidence> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|e| IrEvidence {
                    source_id: s(e, "source_id").unwrap_or_default(),
                    chunk_id: s(e, "chunk_id"),
                    locator: e.get("locator").cloned().unwrap_or(Value::Null),
                    quote: s(e, "quote").unwrap_or_default(),
                    basis: s(e, "basis").unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn has_param(params: &Value, key: &str) -> bool {
    params.get(key).is_some_and(|v| !v.is_null())
}

/// What the validator and the fastroute adapter do with a rule of this kind.
pub fn execution(kind: &str, params: &Value) -> Execution {
    let mk = |validator: &str, fastroute: &str, reason: Option<&str>| Execution {
        validator: validator.into(),
        fastroute: fastroute.into(),
        reason: reason.map(str::to_string),
        unchecked_params: vec![],
        unmapped_params: vec![],
    };
    match kind {
        "reserved_pin" | "nc_pin" | "fixed_net" | "exclusive_functions" | "required_block" | "pin_functions" => mk("supported", "not_applicable", None),
        "keepout" => mk(
            "supported",
            "unsupported",
            Some("fastroute: keepouts come from the design (the DSN); a rule file cannot add one - a new core capability would be needed"),
        ),
        "trace_width" => mk("supported", "supported", None),
        "clearance" => mk("unsupported", "supported", Some("validator: clearance is the job of KiCad's DRC; this validator has no geometry check for it")),
        "electrical" => mk("unsupported", "not_applicable", Some("validator: there is no electrical model; a limit of the device is reported, not checked")),
        "high_speed" => {
            let mut e = mk("unsupported", "supported", Some("validator: high-speed checks are not implemented in this version"));
            let mappable = ["gap_mm", "skew_mm", "max_length_mm", "length_match_tolerance_mm"].iter().any(|k| has_param(params, k));
            if !mappable {
                e.fastroute = "unsupported".into();
                e.reason = Some("fastroute: no spacing, skew or length parameter to hand to the router".into());
            }
            e.unmapped_params = ["impedance_ohm", "layers", "min_width_mm"].iter().filter(|k| has_param(params, k)).map(|k| k.to_string()).collect();
            e
        }
        "strapping" | "reset_pin" | "boot_pin" | "mechanical_fixed" => mk("unsupported", "not_applicable", Some("validator: not implemented in this version")),
        _ => mk("unsupported", "unsupported", Some("unknown kind of rule")),
    }
}

fn ir_rule(r: &Value) -> IrRule {
    let kind = s(r, "kind").unwrap_or_default();
    let modifiability = s(r, "modifiability").unwrap_or_default();
    let params = r.get("params").cloned().unwrap_or_else(|| json!({}));
    let exec = execution(&kind, &params);
    let status = match modifiability.as_str() {
        "unknown" => "unknown",
        "conflict" => "conflict",
        _ if exec.validator == "supported" || exec.fastroute == "supported" => "executable",
        _ => "unsupported",
    };
    IrRule {
        rule_id: s(r, "id").unwrap_or_default(),
        kind,
        modifiability,
        severity: s(r, "severity").unwrap_or_else(|| "error".into()),
        status: status.into(),
        title: s(r, "title"),
        subject: r.get("subject").cloned().unwrap_or_else(|| json!({})),
        params,
        applies_when: list(r, "applies_when").iter().filter_map(Value::as_str).map(str::to_string).collect(),
        evidence: evidence(r.get("evidence")),
        derived_from: {
            let mut d: Vec<String> = list(r, "derived_from").iter().filter_map(Value::as_str).map(str::to_string).collect();
            d.sort();
            d
        },
        execution: exec,
        missing: s(r, "missing"),
        conflict: r.get("conflict").cloned(),
        origin: "hcp".into(),
    }
}

fn polygon(v: &Value) -> Option<Vec<[f64; 2]>> {
    let pts: Vec<[f64; 2]> = v.as_array()?.iter().filter_map(|p| Some([p.get(0)?.as_f64()?, p.get(1)?.as_f64()?])).collect();
    Some(pts)
}

/// Compiles a pack. A pack with errors is not compiled: the diagnostics are returned.
pub fn compile(pack: &Value) -> Result<Ir, Vec<Diagnostic>> {
    let found = hcp::check(pack);
    if found.iter().any(|d| d.severity == Severity::Error) {
        return Err(found);
    }
    let mut diagnostics: Vec<Diagnostic> = found;

    let pack_info = &pack["pack"];
    let mut sources: Vec<IrSource> = list(pack, "sources")
        .iter()
        .map(|src| IrSource {
            source_id: s(src, "source_id").unwrap_or_default(),
            document_id: s(src, "document_id").unwrap_or_default(),
            title: s(src, "title").unwrap_or_default(),
            version: s(src, "version").unwrap_or_default(),
            level: s(src, "level").unwrap_or_default(),
            sha256: s(src, "sha256").unwrap_or_default(),
            applies_to: {
                let mut a: Vec<String> = list(src, "applies_to").iter().filter_map(Value::as_str).map(str::to_string).collect();
                a.sort();
                a
            },
            url: s(src, "url"),
        })
        .collect();
    sources.sort_by(|a, b| a.source_id.cmp(&b.source_id));

    let mut devices: Vec<IrDevice> = list(pack, "devices")
        .iter()
        .map(|d| {
            let mut pins: Vec<IrPin> = list(d, "pins")
                .iter()
                .map(|p| IrPin {
                    number: s(p, "number").unwrap_or_default(),
                    name: s(p, "name").unwrap_or_default(),
                    class: s(p, "class").unwrap_or_default(),
                    functions: list(p, "functions").iter().filter_map(Value::as_str).map(str::to_string).collect(),
                })
                .collect();
            pins.sort_by(|a, b| pin_order(&a.number, &b.number));
            IrDevice { id: s(d, "id").unwrap_or_default(), vendor: s(d, "vendor").unwrap_or_default(), part_number: s(d, "part_number").unwrap_or_default(), revision: s(d, "revision"), package: s(d, "package").unwrap_or_default(), pins }
        })
        .collect();
    devices.sort_by(|a, b| a.id.cmp(&b.id));

    let mut blocks: Vec<IrBlock> = list(pack, "peripheral_blocks")
        .iter()
        .map(|b| IrBlock {
            id: s(b, "id").unwrap_or_default(),
            title: s(b, "title").unwrap_or_default(),
            kind: s(b, "kind").unwrap_or_default(),
            modifiability: s(b, "modifiability").unwrap_or_default(),
            device: s(b, "device"),
            components: list(b, "components").to_vec(),
            connects: list(b, "connects").to_vec(),
            evidence: evidence(b.get("evidence")),
            missing: s(b, "missing"),
        })
        .collect();
    blocks.sort_by(|a, b| a.id.cmp(&b.id));

    let mut regions: Vec<IrRegion> = list(pack, "regions")
        .iter()
        .map(|g| IrRegion {
            id: s(g, "id").unwrap_or_default(),
            kind: s(g, "kind").unwrap_or_default(),
            frame: s(g, "frame").unwrap_or_else(|| "board".into()),
            device: s(g, "device"),
            polygon: g.get("polygon").and_then(polygon),
            circle: g.get("circle").cloned(),
            layers: {
                let l: Vec<String> = list(g, "layers").iter().filter_map(Value::as_str).map(str::to_string).collect();
                if l.is_empty() { vec!["all".into()] } else { l }
            },
            modifiability: s(g, "modifiability").unwrap_or_default(),
            evidence: evidence(g.get("evidence")),
        })
        .collect();
    regions.sort_by(|a, b| a.id.cmp(&b.id));

    let mut rules: Vec<IrRule> = list(pack, "rules").iter().map(ir_rule).collect();

    // the pin table of a device is a rule too: a function that a pin does not have cannot be used,
    // and one pin cannot carry two functions. It cites the evidence of the pins.
    for d in list(pack, "devices") {
        let any_functions = list(d, "pins").iter().any(|p| !list(p, "functions").is_empty());
        if !any_functions {
            continue;
        }
        let id = s(d, "id").unwrap_or_default();
        let mut ev: Vec<IrEvidence> = Vec::new();
        for p in list(d, "pins") {
            for e in evidence(p.get("evidence")) {
                if !ev.contains(&e) {
                    ev.push(e);
                }
            }
        }
        if ev.is_empty() {
            diagnostics.push(Diagnostic::warning(
                "pin_table_without_evidence",
                "$.devices",
                Some(id.clone()),
                format!("the pins of {id} list functions but none of them cites evidence: pin-multiplexing findings cannot point to the manual"),
            ));
        }
        rules.push(IrRule {
            rule_id: format!("PINMUX-{}", id.trim_start_matches("DEV-")),
            kind: "pin_functions".into(),
            modifiability: "fixed".into(),
            severity: "error".into(),
            status: "executable".into(),
            title: Some(format!("Pin functions of {id}")),
            subject: json!({"device": id}),
            params: json!({}),
            applies_when: vec![],
            evidence: ev,
            derived_from: vec![],
            execution: execution("pin_functions", &json!({})),
            missing: None,
            conflict: None,
            origin: "pin_table".into(),
        });
    }
    rules.sort_by(|a, b| a.rule_id.cmp(&b.rule_id));

    diagnostics.retain(|d| d.severity == Severity::Warning);
    let mut ir = Ir {
        ir_version: IR_VERSION.into(),
        compiler: Compiler { name: "fr-constraint".into(), version: env!("CARGO_PKG_VERSION").into() },
        pack: PackRef {
            id: s(pack_info, "id").unwrap_or_default(),
            version: s(pack_info, "version").unwrap_or_default(),
            sha256: sha256_hex(canonical_json(pack).as_bytes()),
            review: pack_info.get("review").and_then(|r| s(r, "status")).unwrap_or_else(|| "draft".into()),
        },
        sources,
        devices,
        blocks,
        regions,
        base_design: pack.get("base_design").cloned(),
        rules,
        diagnostics,
        integrity: Integrity { algorithm: "sha256-canonical-json".into(), sha256: String::new() },
    };
    ir.integrity.sha256 = ir.compute_integrity();
    Ok(ir)
}
