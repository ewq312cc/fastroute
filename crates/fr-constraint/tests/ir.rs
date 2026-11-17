//! The compiler: deterministic, evidence kept, nothing silently dropped.

mod common;
use common::*;
use fr_constraint::compile;
use fr_constraint::hcp;
use fr_constraint::ir::{ir_schema, Ir};
use fr_constraint::jsonschema;
use serde_json::{json, Value};

#[test]
fn the_same_pack_compiles_to_the_same_bytes_and_the_same_facts_in_any_order() {
    let a = compile::compile(&example()).unwrap().to_json_text();
    let b = compile::compile(&example()).unwrap().to_json_text();
    assert_eq!(a, b, "two compilations differ");
    // the order of the rules, pins and sources in the file does not matter
    let mut p = example();
    p["rules"].as_array_mut().unwrap().reverse();
    p["sources"].as_array_mut().unwrap().reverse();
    p["devices"][0]["pins"].as_array_mut().unwrap().reverse();
    p["peripheral_blocks"].as_array_mut().unwrap().reverse();
    let c = compile::compile(&p).unwrap();
    let orig = compile::compile(&example()).unwrap();
    assert_eq!(c.rules, orig.rules);
    assert_eq!(c.devices, orig.devices);
    assert_eq!(c.sources, orig.sources);
    // ... a different fact does (the pack's hash and the IR's integrity hash change)
    let mut p = example();
    p["rules"][0]["params"]["reason"] = json!("another reason");
    let d = compile::compile(&p).unwrap();
    assert_ne!(d.pack.sha256, orig.pack.sha256);
    assert_ne!(d.integrity.sha256, orig.integrity.sha256);
    // no clock, host or path in the output
    assert!(!a.contains("generated_at") && !a.contains("/home/"));
}

#[test]
fn the_ir_matches_its_published_schema_in_both_implementations() {
    let text = ir().to_json_text();
    let v: Value = serde_json::from_str(&text).unwrap();
    let errors = jsonschema::validate(&ir_schema(), &v);
    assert!(errors.is_empty(), "{errors:#?}");
    // a copy of the IR that is wrong is refused
    let mut bad = v.clone();
    bad["rules"][0]["status"] = json!("maybe");
    assert!(!jsonschema::validate(&ir_schema(), &bad).is_empty());
    // the reference implementation agrees
    let probe = std::process::Command::new("python3").args(["-c", "import jsonschema"]).output();
    if probe.is_ok_and(|o| o.status.success()) {
        let dir = std::env::temp_dir().join(format!("fr-ir-ref-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (i, doc) in [&v, &bad].iter().enumerate() {
            let f = dir.join(format!("{i}.json"));
            std::fs::write(&f, serde_json::to_string(doc).unwrap()).unwrap();
            let script = format!(
                "import json,jsonschema\ns=json.load(open(r'{}'))\nd=json.load(open(r'{}'))\nprint(len(list(jsonschema.Draft202012Validator(s).iter_errors(d))))",
                skill().join("schemas/constraint-ir-v1.schema.json").display(),
                f.display()
            );
            let o = std::process::Command::new("python3").args(["-c", &script]).output().unwrap();
            let n: usize = String::from_utf8_lossy(&o.stdout).trim().parse().unwrap();
            assert_eq!(n == 0, i == 0, "reference validator on case {i}: {n}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn every_rule_keeps_its_evidence_with_source_page_and_quote() {
    let pack = example();
    let ir = ir();
    for r in pack["rules"].as_array().unwrap() {
        let id = r["id"].as_str().unwrap();
        let out = ir.rule(id).unwrap_or_else(|| panic!("{id} was dropped"));
        let want = r["evidence"].as_array().cloned().unwrap_or_default();
        assert_eq!(out.evidence.len(), want.len(), "{id}");
        for (e, w) in out.evidence.iter().zip(&want) {
            assert_eq!(e.source_id, w["source_id"].as_str().unwrap());
            assert_eq!(e.quote, w["quote"].as_str().unwrap());
            assert_eq!(e.locator, w["locator"]);
            assert_eq!(e.basis, w["basis"].as_str().unwrap());
        }
        // every source an IR rule cites is in the IR with its version and digest
        for e in &out.evidence {
            let s = ir.sources.iter().find(|s| s.source_id == e.source_id).unwrap();
            assert!(!s.version.is_empty() && s.sha256.len() == 64);
        }
    }
    // blocks and regions too
    assert!(ir.block("BLK-POWER").unwrap().evidence.len() == 1);
    assert!(ir.region("REG-ANT").unwrap().evidence.len() == 1);
}

#[test]
fn the_pin_table_becomes_a_rule_that_cites_nothing_it_does_not_have() {
    let ir = ir();
    let r = ir.rule("PINMUX-FM100").expect("the pin functions are a rule");
    assert_eq!((r.kind.as_str(), r.origin.as_str(), r.status.as_str()), ("pin_functions", "pin_table", "executable"));
    // the example's pins carry no evidence of their own: said, not hidden
    assert!(r.evidence.is_empty());
    assert!(ir.diagnostics.iter().any(|d| d.code == "pin_table_without_evidence"), "{:#?}", ir.diagnostics);
}

#[test]
fn what_cannot_be_executed_is_said_per_rule() {
    let ir = ir();
    let get = |id: &str| ir.rule(id).unwrap();
    // unknown and conflict never run, whatever their kind
    assert_eq!(get("C-USB-GAP").status, "unknown");
    assert_eq!(get("C-USB-GAP").execution.validator, "unsupported");
    // a rule that nobody can execute is `unsupported` with the reason, not dropped or approximated
    let mut p = example();
    p["rules"].as_array_mut().unwrap().push(json!({
        "id": "C-SUPPLY", "kind": "electrical", "modifiability": "fixed", "subject": {"device": "DEV-FM100"},
        "params": {"quantity": "supply voltage", "condition": "normal operation", "min_v": 3.0, "max_v": 3.6},
        "evidence": [{"source_id": "SRC-FM100-DS", "locator": {"page": 9}, "quote": "3.0 V to 3.6 V.", "basis": "explicit"}]
    }));
    let ir2 = compile::compile(&p).unwrap();
    let r = ir2.rule("C-SUPPLY").unwrap();
    assert_eq!((r.status.as_str(), r.execution.validator.as_str(), r.execution.fastroute.as_str()), ("unsupported", "unsupported", "not_applicable"));
    assert!(r.execution.reason.as_ref().unwrap().contains("no electrical model"));
    assert_eq!(r.params["min_v"], json!(3.0), "the values are kept");
    // high speed: what the router cannot take is listed as unmapped, never dropped
    let usb = ir2.rule("C-USB").unwrap();
    assert_eq!(usb.execution.fastroute, "supported");
    assert_eq!(usb.execution.unmapped_params, vec!["impedance_ohm".to_string()]);
    // a keep-out cannot be handed to the router
    assert_eq!(get("C-ANT").execution.fastroute, "unsupported");
    assert!(get("C-ANT").execution.reason.as_ref().unwrap().contains("keepouts come from the design"));
}

#[test]
fn a_pack_with_errors_is_not_compiled_and_says_why() {
    let mut p = example();
    p["rules"][0].as_object_mut().unwrap().remove("evidence");
    let err = compile::compile(&p).unwrap_err();
    assert!(err.iter().any(|d| d.code == "schema" && d.object.as_deref() == Some("C-RESERVED")), "{err:#?}");
}

#[test]
fn an_edited_ir_is_recognised_and_refused_by_the_validator() {
    let good = ir();
    assert!(good.verify_integrity());
    let mut edited = good.clone();
    edited.rules[0].severity = "warning".into(); // somebody softened a rule by hand
    assert!(!edited.verify_integrity());
    let snap = legal();
    let e = fr_constraint::validate::validate(&edited, &snap, "x", None).unwrap_err();
    assert!(e.contains("integrity"), "{e}");
    // the IR survives a round trip through its text
    let again: Ir = serde_json::from_str(&good.to_json_text()).unwrap();
    assert!(again.verify_integrity());
    assert_eq!(again, good);
}

#[test]
fn compiling_does_not_read_anything_but_the_pack() {
    // a pack that carries retrieval data is refused before anything is compiled
    let mut p = example();
    p["rules"][0]["x_similarity"] = json!(0.99);
    assert!(compile::compile(&p).is_err());
    // the hash is of the canonical content: a pure reformatting does not change it
    let a = compile::compile(&example()).unwrap();
    let text = serde_json::to_string_pretty(&example()).unwrap();
    let b = compile::compile(&hcp::parse(&text).unwrap()).unwrap();
    assert_eq!(a.pack.sha256, b.pack.sha256);
    assert_eq!(a.to_json_text(), b.to_json_text());
}
