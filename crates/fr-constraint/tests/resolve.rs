//! Inheritance between packs (hcp_version 1.1): a module pack takes named things from a chip pack, and nothing else.
//! The packs here are fictional (zero digests, made-up pins); they test the mechanism, not a device.

use fr_constraint::diag::Severity;
use fr_constraint::hcp;
use fr_constraint::resolve::{canonical_sha256, resolve};
use fr_constraint::compile;
use serde_json::{json, Value};

fn ev(source: &str, page: u32, quote: &str) -> Value {
    json!([{"source_id": source, "locator": {"page": page}, "quote": quote, "basis": "explicit"}])
}

fn source(id: &str, doc: &str, level: &str, devices: Value) -> Value {
    json!({"source_id": id, "document_id": doc, "title": format!("{doc} (fictional)"), "version": "v1.0", "level": level,
        "url": "https://example.invalid/x", "retrieved": "2026-11-16", "sha256": "0".repeat(64), "applies_to": devices})
}

fn pins(n: usize) -> Value {
    Value::Array((1..=n).map(|i| json!({"number": i.to_string(), "name": format!("P{i}"), "class": if i == 1 { "ground" } else { "gpio" }})).collect())
}

pub fn chip() -> Value {
    json!({
        "hcp_version": "1.1",
        "pack": {"id": "acme/cx/cx1/default", "kind": "chip", "title": "CX1 chip (fictional)", "version": "0.1.0", "created": "2026-11-16", "extractor": {"kind": "human"}, "review": {"status": "draft"}},
        "sources": [source("SRC-CX1-DS", "DOC-CX1-DS", "official_manual", json!(["DEV-CX1"]))],
        "devices": [{"id": "DEV-CX1", "vendor": "Acme", "part_number": "CX1", "package": "QFN-4", "pins": pins(4)}],
        "rules": [
            {"id": "C-CX1-GND", "kind": "fixed_net", "modifiability": "fixed", "subject": {"device": "DEV-CX1"}, "params": {"pin": "1", "net_pattern": "GND*"},
             "evidence": ev("SRC-CX1-DS", 5, "Pin 1 is ground.")},
            {"id": "C-CX1-RSVD", "kind": "reserved_pin", "modifiability": "fixed", "subject": {"device": "DEV-CX1"}, "params": {"pins": ["4"]},
             "evidence": ev("SRC-CX1-DS", 5, "Pin 4 is reserved.")}
        ]
    })
}

pub fn module(chip: &Value) -> Value {
    json!({
        "hcp_version": "1.1",
        "pack": {"id": "acme/cx/cxm1/n4", "kind": "module", "title": "CXM1 module (fictional)", "version": "0.1.0", "created": "2026-11-16", "extractor": {"kind": "human"}, "review": {"status": "draft"},
            "upstream": [{"id": "acme/cx/cx1/default", "version": "0.1.0", "kind": "chip", "canonical_sha256": canonical_sha256(chip),
                "inherit": {"devices": ["DEV-CX1"], "rules": ["C-CX1-GND", "C-CX1-RSVD"]}}]},
        "sources": [source("SRC-CXM1-DS", "DOC-CXM1-DS", "official_manual", json!(["DEV-CXM1"]))],
        "devices": [{"id": "DEV-CXM1", "vendor": "Acme", "part_number": "CXM1-N4", "package": "module", "pins": pins(6)}],
        "rules": [
            {"id": "C-CXM1-RSVD", "kind": "reserved_pin", "modifiability": "fixed", "subject": {"device": "DEV-CXM1"}, "params": {"pins": ["5", "6"]},
             "evidence": ev("SRC-CXM1-DS", 9, "Pins 5 and 6 are used by the flash inside the module.")}
        ]
    })
}

fn errors(r: Result<Value, Vec<fr_constraint::diag::Diagnostic>>) -> Vec<String> {
    r.expect_err("must fail").into_iter().map(|d| d.code).collect()
}

#[test]
fn a_module_takes_what_it_lists_from_the_chip_and_nothing_else() {
    let c = chip();
    let m = module(&c);
    let flat = resolve(&m, &[c.clone()]).expect("resolves");
    assert!(hcp::check(&flat).iter().all(|d| d.severity != Severity::Error), "{:?}", hcp::check(&flat));
    let rule_ids: Vec<&str> = flat["rules"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(rule_ids, ["C-CX1-GND", "C-CX1-RSVD", "C-CXM1-RSVD"]);
    let sources: Vec<&str> = flat["sources"].as_array().unwrap().iter().map(|s| s["source_id"].as_str().unwrap()).collect();
    assert_eq!(sources, ["SRC-CXM1-DS", "SRC-CX1-DS"], "the chip's document comes along, so the inherited evidence can still be checked");
    assert!(flat["pack"].get("upstream").is_none());
    assert_eq!(flat["pack"]["x_resolved_from"][0]["canonical_sha256"], canonical_sha256(&c));
    assert_eq!(flat["pack"]["x_inherited"]["C-CX1-GND"], "acme/cx/cx1/default@0.1.0");

    // an object that is not listed is not taken
    let mut only_one = m.clone();
    only_one["pack"]["upstream"][0]["inherit"]["rules"] = json!(["C-CX1-GND"]);
    let flat = resolve(&only_one, &[c]).unwrap();
    assert!(!flat["rules"].as_array().unwrap().iter().any(|r| r["id"] == "C-CX1-RSVD"));
}

#[test]
fn the_same_inputs_give_the_same_pack_and_the_same_ir() {
    let c = chip();
    let m = module(&c);
    let a = resolve(&m, &[c.clone()]).unwrap();
    let b = resolve(&m, &[c]).unwrap();
    assert_eq!(hcp::canonical_json(&a), hcp::canonical_json(&b));
    let ir_a = serde_json::to_string(&compile::compile(&a).expect("the flat pack compiles")).unwrap();
    let ir_b = serde_json::to_string(&compile::compile(&b).unwrap()).unwrap();
    assert_eq!(ir_a, ir_b);
}

#[test]
fn a_pack_that_inherits_is_not_usable_before_it_is_resolved() {
    let c = chip();
    let m = module(&c);
    let diags = hcp::check(&m);
    assert!(diags.iter().any(|d| d.code == "upstream_unresolved" && d.severity == Severity::Error), "{diags:?}");
    assert!(compile::compile(&m).is_err(), "an unresolved pack is not compiled");
}

#[test]
fn version_1_1_features_need_version_1_1() {
    let mut c = chip();
    c["hcp_version"] = json!("1.0");
    assert!(hcp::check(&c).iter().any(|d| d.code == "needs_version_1_1"));
}

#[test]
fn a_changed_upstream_is_refused_not_adopted() {
    let c = chip();
    let m = module(&c);
    let mut changed = c.clone();
    changed["rules"][1]["params"]["pins"] = json!(["3", "4"]);
    assert_eq!(errors(resolve(&m, &[changed])), ["upstream_digest_mismatch"]);
}

#[test]
fn a_missing_or_similar_upstream_is_not_replaced_by_a_look_alike() {
    let c = chip();
    let m = module(&c);
    assert_eq!(errors(resolve(&m, &[])), ["upstream_missing"]);
    let mut other_version = c.clone();
    other_version["pack"]["version"] = json!("0.2.0");
    assert_eq!(errors(resolve(&m, &[other_version])), ["upstream_missing"], "only the exact version counts");
    let mut other_variant = c.clone();
    other_variant["pack"]["id"] = json!("acme/cx/cx1/other");
    assert_eq!(errors(resolve(&m, &[other_variant])), ["upstream_missing"]);
}

#[test]
fn kinds_cannot_be_mixed() {
    let c = chip();
    // a chip pack inherits from nothing
    let mut chip_with_upstream = module(&c);
    chip_with_upstream["pack"]["kind"] = json!("chip");
    assert!(errors(resolve(&chip_with_upstream, &[c.clone()])).contains(&"upstream_kind_not_allowed".to_string()));
    // the upstream is called a chip but is not
    let mut not_a_chip = c.clone();
    not_a_chip["pack"]["kind"] = json!("module");
    let mut m = module(&c);
    m["pack"]["upstream"][0]["canonical_sha256"] = json!(canonical_sha256(&not_a_chip));
    assert!(errors(resolve(&m, &[not_a_chip.clone()])).contains(&"upstream_kind_mismatch".to_string()));
    // a module does not inherit from a module
    let mut m2 = module(&c);
    m2["pack"]["upstream"][0]["kind"] = json!("module");
    m2["pack"]["upstream"][0]["canonical_sha256"] = json!(canonical_sha256(&not_a_chip));
    assert!(errors(resolve(&m2, &[not_a_chip])).contains(&"upstream_kind_not_allowed".to_string()));
    // an upstream that does not say what it is cannot be inherited from
    let mut unlabeled = c.clone();
    unlabeled["pack"].as_object_mut().unwrap().remove("kind");
    let mut m3 = module(&c);
    m3["pack"]["upstream"][0]["canonical_sha256"] = json!(canonical_sha256(&unlabeled));
    assert!(errors(resolve(&m3, &[unlabeled])).contains(&"upstream_kind_unknown".to_string()));
}

#[test]
fn inheriting_something_that_is_not_there_fails() {
    let c = chip();
    let mut m = module(&c);
    m["pack"]["upstream"][0]["inherit"]["rules"] = json!(["C-CX1-GND", "C-NOPE"]);
    assert_eq!(errors(resolve(&m, &[c])), ["inherit_unknown_id"]);
}

#[test]
fn an_override_needs_evidence_of_the_pack_that_overrides() {
    let c = chip();
    let mut m = module(&c);
    // on the module the reserved pins are others than the chip's: the module's own rule (on the module device, from the module's own
    // document: a document of the module is not evidence for the chip) replaces the chip's rule
    m["rules"].as_array_mut().unwrap().push(json!({
        "id": "C-CX1-RSVD", "kind": "reserved_pin", "modifiability": "fixed", "overrides": "C-CX1-RSVD",
        "subject": {"device": "DEV-CXM1"}, "params": {"pins": ["3", "4"]},
        "evidence": ev("SRC-CXM1-DS", 10, "Pins 3 and 4 are not available on this module.")
    }));
    let flat = resolve(&m, &[c.clone()]).expect("with its own evidence");
    let rsvd: Vec<&Value> = flat["rules"].as_array().unwrap().iter().filter(|r| r["id"] == "C-CX1-RSVD").collect();
    assert_eq!(rsvd.len(), 1, "the inherited rule is replaced, not duplicated");
    assert_eq!(rsvd[0]["params"]["pins"], json!(["3", "4"]));
    assert!(flat["pack"]["x_inherited"].get("C-CX1-RSVD").is_none());

    // evidence from the chip's document is what is being replaced: not enough
    m["rules"][1]["evidence"] = ev("SRC-CX1-DS", 5, "Pin 4 is reserved.");
    assert_eq!(errors(resolve(&m, &[c.clone()])), ["override_without_own_evidence"]);

    // an override of something that is not inherited
    m["rules"][1]["evidence"] = ev("SRC-CXM1-DS", 10, "x");
    m["pack"]["upstream"][0]["inherit"]["rules"] = json!(["C-CX1-GND"]);
    assert_eq!(errors(resolve(&m, &[c])), ["override_of_uninherited"]);
}

#[test]
fn a_rule_that_contradicts_an_inherited_one_must_say_so() {
    let c = chip();
    let mut m = module(&c);
    // same thing as the chip's ground rule, another value, no `overrides`
    m["rules"].as_array_mut().unwrap().push(json!({
        "id": "C-CXM1-GND", "kind": "fixed_net", "modifiability": "fixed", "subject": {"device": "DEV-CX1"}, "params": {"pin": "1", "net_pattern": "AGND*"},
        "evidence": ev("SRC-CXM1-DS", 9, "x")
    }));
    assert_eq!(errors(resolve(&m, &[c.clone()])), ["inherit_conflict"]);
    // the same id as an inherited rule
    let mut m = module(&c);
    m["rules"].as_array_mut().unwrap().push(json!({
        "id": "C-CX1-GND", "kind": "electrical", "modifiability": "unknown", "subject": {"device": "DEV-CXM1"}, "params": {}, "missing": "x"
    }));
    assert!(errors(resolve(&m, &[c])).contains(&"id_clash".to_string()));
}

#[test]
fn two_documents_with_one_id_and_different_digests_are_a_conflict() {
    let c = chip();
    let mut m = module(&c);
    m["sources"].as_array_mut().unwrap().push({
        let mut s = source("SRC-CX1-DS", "DOC-CX1-DS", "official_manual", json!(["DEV-CXM1"]));
        s["sha256"] = json!("1".repeat(64));
        s
    });
    assert!(errors(resolve(&m, &[c])).contains(&"source_conflict".to_string()));
}
