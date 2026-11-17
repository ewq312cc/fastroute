//! The HCP schema and the checks of a pack: the example is valid, and every kind of bad pack is
//! refused with a code that says what is wrong and where.

use fr_constraint::diag::Diagnostic;
use fr_constraint::hcp;
use serde_json::{json, Value};
use std::path::PathBuf;

fn skill() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills/hardware-constraint-design")
}

fn example() -> Value {
    hcp::parse(&std::fs::read_to_string(skill().join("assets/example-device.yaml")).unwrap()).unwrap()
}

fn codes(d: &[Diagnostic]) -> Vec<String> {
    d.iter().map(|d| d.code.clone()).collect()
}

fn find<'a>(d: &'a [Diagnostic], code: &str) -> &'a Diagnostic {
    d.iter().find(|d| d.code == code).unwrap_or_else(|| panic!("no {code} in {:#?}", d))
}

fn rule_mut<'a>(pack: &'a mut Value, id: &str) -> &'a mut Value {
    pack["rules"].as_array_mut().unwrap().iter_mut().find(|r| r["id"] == id).unwrap()
}

#[test]
fn the_example_pack_is_valid() {
    let d = hcp::check(&example());
    assert!(d.is_empty(), "{d:#?}");
}

#[test]
fn yaml_and_json_are_the_same_pack() {
    let yaml_pack = example();
    let as_json = serde_json::to_string_pretty(&yaml_pack).unwrap();
    let json_pack = hcp::parse(&as_json).unwrap();
    assert_eq!(hcp::canonical_json(&yaml_pack), hcp::canonical_json(&json_pack));
    assert!(hcp::check(&json_pack).is_empty());
    // the canonical form does not depend on the order of keys
    assert_eq!(hcp::canonical_json(&json!({"b": 1, "a": [2, {"d": 1, "c": 2}]})), hcp::canonical_json(&json!({"a": [2, {"c": 2, "d": 1}], "b": 1})));
}

#[test]
fn a_rule_without_evidence_is_refused_where_it_is() {
    let mut p = example();
    rule_mut(&mut p, "C-RESERVED").as_object_mut().unwrap().remove("evidence");
    let d = hcp::check(&p);
    let e = find(&d, "schema");
    assert_eq!(e.object.as_deref(), Some("C-RESERVED"));
    assert!(e.message.contains("missing required property 'evidence'"), "{e:?}");
    assert!(e.path.starts_with("$.rules["), "{e:?}");
}

#[test]
fn an_illegal_modifiability_is_refused() {
    let mut p = example();
    rule_mut(&mut p, "C-GND")["modifiability"] = json!("probably_fixed");
    let d = hcp::check(&p);
    let e = find(&d, "schema");
    assert!(e.message.contains("probably_fixed") && e.message.contains("fixed"), "{e:?}");
}

#[test]
fn a_number_without_a_unit_is_refused() {
    let mut p = example();
    rule_mut(&mut p, "C-USB")["params"]["gap"] = json!(0.2);
    rule_mut(&mut p, "C-USB")["params"]["width_inch"] = json!(0.01);
    let d = hcp::check(&p);
    let units: Vec<&Diagnostic> = d.iter().filter(|d| d.code == "unknown_unit").collect();
    assert_eq!(units.len(), 2, "{d:#?}");
    assert!(units.iter().any(|u| u.path.ends_with("params.gap") && u.object.as_deref() == Some("C-USB")), "{units:?}");
    assert!(units.iter().any(|u| u.path.ends_with("params.width_inch")), "{units:?}");
}

#[test]
fn a_duplicate_pin_is_refused() {
    let mut p = example();
    let pins = p["devices"][0]["pins"].as_array_mut().unwrap();
    let dup = pins[4].clone();
    pins.push(dup); // number "5" twice, name IO1 twice
    let d = hcp::check(&p);
    let dups: Vec<&Diagnostic> = d.iter().filter(|d| d.code == "duplicate_pin").collect();
    assert_eq!(dups.len(), 2, "{d:#?}");
    assert!(dups.iter().any(|x| x.message.contains("pin number '5'")));
    assert!(dups.iter().any(|x| x.message.contains("pin name 'IO1'")));
    // two ground pins may share a name
    let mut p = example();
    p["devices"][0]["pins"][11]["name"] = json!("GND");
    assert!(hcp::check(&p).is_empty());
}

#[test]
fn a_conflict_needs_two_different_values_with_evidence() {
    let mut p = example();
    let ev = rule_mut(&mut p, "C-RESERVED")["evidence"].clone();
    let r = rule_mut(&mut p, "C-USB-GAP");
    r["modifiability"] = json!("conflict");
    r.as_object_mut().unwrap().remove("missing");
    r["params"] = json!({"nets": ["USB_DP"], "differential": true, "gap_mm": null});
    r["conflict"] = json!({"candidates": [{"value": 0.2, "evidence": ev.clone()}, {"value": 0.2, "evidence": ev.clone()}]});
    let d = hcp::check(&p);
    assert!(codes(&d).contains(&"conflict_state".to_string()), "{d:#?}");
    // one candidate is not a conflict either (the schema says so)
    rule_mut(&mut p, "C-USB-GAP")["conflict"] = json!({"candidates": [{"value": 0.2, "evidence": ev.clone()}]});
    assert!(hcp::check(&p).iter().any(|d| d.code == "schema" && d.message.contains("at least 2")));
    // two different values: a valid conflict, and no number may be chosen in params
    rule_mut(&mut p, "C-USB-GAP")["conflict"] = json!({"candidates": [{"value": 0.2, "evidence": ev.clone()}, {"value": 0.3, "evidence": ev}]});
    assert!(hcp::check(&p).is_empty(), "{:#?}", hcp::check(&p));
    rule_mut(&mut p, "C-USB-GAP")["params"]["gap_mm"] = json!(0.25);
    assert!(codes(&hcp::check(&p)).contains(&"unknown_value".to_string()));
}

#[test]
fn an_unknown_rule_never_holds_a_guessed_number() {
    let mut p = example();
    rule_mut(&mut p, "C-USB-GAP")["params"]["gap_mm"] = json!(0.15);
    let d = hcp::check(&p);
    let e = find(&d, "unknown_value");
    assert_eq!(e.object.as_deref(), Some("C-USB-GAP"));
    // ... and it says what is missing
    let mut p = example();
    rule_mut(&mut p, "C-USB-GAP").as_object_mut().unwrap().remove("missing");
    assert!(hcp::check(&p).iter().any(|d| d.code == "schema" && d.message.contains("'missing'")));
}

#[test]
fn vector_and_retrieval_data_is_refused_anywhere() {
    let mut p = example();
    rule_mut(&mut p, "C-GND")["embedding"] = json!([0.1, 0.2]);
    p["sources"][0]["x_similarity"] = json!(0.93);
    p["devices"][0]["pins"][0]["rank"] = json!(1);
    let d = hcp::check(&p);
    let v: Vec<&Diagnostic> = d.iter().filter(|d| d.code == "vector_field").collect();
    assert_eq!(v.len(), 3, "{d:#?}");
    assert!(v.iter().all(|d| d.message.contains("hcp-manager")));
    // extension keys are allowed for anything else
    let mut p = example();
    p["x_note"] = json!("free text");
    rule_mut(&mut p, "C-GND")["x_review_comment"] = json!("ok");
    assert!(hcp::check(&p).is_empty());
}

#[test]
fn evidence_from_a_document_of_another_model_is_refused() {
    let mut p = example();
    p["devices"].as_array_mut().unwrap().push(json!({
        "id": "DEV-OTHER", "vendor": "Acme", "part_number": "FM-100U", "package": "module",
        "pins": [{"number": "1", "name": "GND", "class": "ground"}]
    }));
    // the datasheet of the FM-100 used for a rule about the FM-100U
    p["rules"].as_array_mut().unwrap().push(json!({
        "id": "C-OTHER", "kind": "reserved_pin", "modifiability": "fixed", "subject": {"device": "DEV-OTHER"},
        "params": {"pins": ["1"]},
        "evidence": [{"source_id": "SRC-FM100-DS", "locator": {"page": 5}, "quote": "x", "basis": "explicit"}]
    }));
    let d = hcp::check(&p);
    let e = find(&d, "source_not_applicable");
    assert_eq!(e.object.as_deref(), Some("C-OTHER"));
    assert!(e.message.contains("another model or version"), "{e:?}");
}

#[test]
fn a_reference_board_does_not_make_the_chip_require_something() {
    let mut p = example();
    // the same strapping rule, fixed, but backed only by a quotation from an open board
    let r = rule_mut(&mut p, "C-STRAP-IO0");
    r["evidence"] = json!([{"source_id": "SRC-OPENBOARD", "locator": {"section": "Boot"}, "quote": "IO0 has a pull-up", "basis": "observed"}]);
    let d = hcp::check(&p);
    let e = find(&d, "weak_evidence");
    assert_eq!(e.object.as_deref(), Some("C-STRAP-IO0"));
    assert!(e.message.contains("official"), "{e:?}");
    // configurable is fine: it states what a board does, not what the chip demands
    rule_mut(&mut p, "C-STRAP-IO0")["modifiability"] = json!("configurable");
    assert!(hcp::check(&p).is_empty(), "{:#?}", hcp::check(&p));
    // an official source with basis derived is not a quotation either
    let r = rule_mut(&mut p, "C-STRAP-IO0");
    r["modifiability"] = json!("fixed");
    r["evidence"] = json!([{"source_id": "SRC-FM100-DS", "locator": {"page": 8}, "quote": "x", "basis": "derived"}]);
    assert!(codes(&hcp::check(&p)).contains(&"weak_evidence".to_string()));
}

#[test]
fn unknown_references_are_named() {
    let mut p = example();
    rule_mut(&mut p, "C-NEED-POWER")["subject"]["block"] = json!("BLK-NOPE");
    rule_mut(&mut p, "C-RESERVED")["params"]["pins"] = json!(["10", "99"]);
    rule_mut(&mut p, "C-GND")["evidence"][0]["source_id"] = json!("SRC-NOPE");
    let d = hcp::check(&p);
    assert!(d.iter().any(|d| d.code == "unknown_reference" && d.message.contains("BLK-NOPE")), "{d:#?}");
    assert!(d.iter().any(|d| d.code == "unknown_pin" && d.message.contains("'99'")), "{d:#?}");
    assert!(d.iter().any(|d| d.code == "unknown_reference" && d.message.contains("SRC-NOPE")), "{d:#?}");
}

#[test]
fn duplicate_ids_are_refused() {
    let mut p = example();
    let dup = p["rules"][0].clone();
    p["rules"].as_array_mut().unwrap().push(dup);
    assert!(codes(&hcp::check(&p)).contains(&"duplicate_id".to_string()));
}

#[test]
fn a_derived_rule_names_what_it_comes_from() {
    let mut p = example();
    let r = rule_mut(&mut p, "C-GND");
    r["modifiability"] = json!("derived");
    r["derived_from"] = json!(["C-NOPE"]);
    let d = hcp::check(&p);
    assert!(d.iter().any(|d| d.code == "derived_basis"), "{d:#?}");
    assert!(d.iter().any(|d| d.code == "unknown_reference" && d.message.contains("C-NOPE")), "{d:#?}");
    let r = rule_mut(&mut p, "C-GND");
    r.as_object_mut().unwrap().remove("derived_from");
    assert!(hcp::check(&p).iter().any(|d| d.code == "schema" && d.message.contains("derived_from")));
}

#[test]
fn only_what_is_whitelisted_can_be_ai_editable() {
    let mut p = example();
    // a region marked ai_editable that the base design does not list
    p["regions"].as_array_mut().unwrap().push(json!({
        "id": "REG-SNEAKY", "kind": "editable", "unit": "mm", "modifiability": "ai_editable",
        "polygon": [[0, 0], [1, 0], [1, 1]],
        "evidence": [{"source_id": "SRC-OPENBOARD", "locator": {"section": "x"}, "quote": "x", "basis": "observed"}]
    }));
    let d = hcp::check(&p);
    assert_eq!(find(&d, "editable_not_whitelisted").object.as_deref(), Some("REG-SNEAKY"));
    // without a base design nothing can be ai_editable
    let mut p = example();
    p.as_object_mut().unwrap().remove("base_design");
    let d = hcp::check(&p);
    assert!(d.iter().filter(|d| d.code == "editable_not_whitelisted").count() >= 2, "{d:#?}");
    // an immutable region cannot also be editable
    let mut p = example();
    p["base_design"]["immutable"]["regions"] = json!(["REG-USER"]);
    assert!(codes(&hcp::check(&p)).contains(&"editable_not_whitelisted".to_string()));
}

#[test]
fn regions_have_a_shape() {
    let mut p = example();
    p["regions"][2]["polygon"] = json!([[0, 0], [1, 1], [2, 2]]);
    assert!(codes(&hcp::check(&p)).contains(&"degenerate_region".to_string()));
    let mut p = example();
    p["rules"].as_array_mut().unwrap().iter_mut().find(|r| r["id"] == "C-ANT").unwrap()["subject"]["region"] = json!("REG-USER");
    assert!(codes(&hcp::check(&p)).contains(&"wrong_region_kind".to_string()));
}

#[test]
fn newer_versions_say_so() {
    let mut p = example();
    p["hcp_version"] = json!("1.3");
    let d = hcp::check(&p);
    assert_eq!(d.iter().filter(|d| d.code == "unsupported_version").count(), 1, "{d:#?}");
    assert!(!d.iter().any(|d| d.path == "$.hcp_version" && d.code == "schema"));
    p["hcp_version"] = json!("2.0");
    assert!(hcp::check(&p).iter().any(|d| d.code == "unsupported_version"));
}

#[test]
fn a_pack_that_is_reviewed_says_by_whom() {
    let mut p = example();
    p["pack"]["review"] = json!({"status": "approved"});
    assert!(codes(&hcp::check(&p)).contains(&"review_incomplete".to_string()));
}

#[test]
fn diagnostics_are_sorted_and_stable() {
    let mut p = example();
    rule_mut(&mut p, "C-USB")["params"]["gap"] = json!(1);
    rule_mut(&mut p, "C-GND")["modifiability"] = json!("x");
    let a = serde_json::to_string(&hcp::check(&p)).unwrap();
    let b = serde_json::to_string(&hcp::check(&p)).unwrap();
    assert_eq!(a, b);
}

/// The reference implementation (Python's jsonschema) agrees with this program's interpreter on
/// the example and on structural mistakes, so the schema file means what it says to other tools too.
#[test]
fn the_reference_validator_agrees_when_it_is_available() {
    let probe = std::process::Command::new("python3").args(["-c", "import jsonschema, yaml"]).output();
    if !probe.is_ok_and(|o| o.status.success()) {
        eprintln!("skipped: python3 with jsonschema and yaml is not available");
        return;
    }
    let dir = std::env::temp_dir().join(format!("fr-constraint-ref-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut cases: Vec<(String, Value)> = vec![("example".into(), example())];
    let mut p = example();
    rule_mut(&mut p, "C-RESERVED").as_object_mut().unwrap().remove("evidence");
    cases.push(("no evidence".into(), p));
    let mut p = example();
    rule_mut(&mut p, "C-GND")["modifiability"] = json!("maybe");
    cases.push(("bad modifiability".into(), p));
    let mut p = example();
    rule_mut(&mut p, "C-GND")["embedding"] = json!([1]);
    cases.push(("vector".into(), p));
    let mut p = example();
    rule_mut(&mut p, "C-STRAP-IO0")["params"].as_object_mut().unwrap().remove("sampled_at");
    cases.push(("strapping without sampled_at".into(), p));
    let mut p = example();
    p["devices"][0]["pins"][0]["class"] = json!("magic");
    cases.push(("bad pin class".into(), p));
    let mut p = example();
    p["regions"][0]["polygon"] = json!([[0, 0], [1, 1]]);
    cases.push(("two-point polygon".into(), p));
    for (i, (name, pack)) in cases.iter().enumerate() {
        let file = dir.join(format!("case{i}.json"));
        std::fs::write(&file, serde_json::to_string(pack).unwrap()).unwrap();
        let script = format!(
            "import json,sys,jsonschema\ns=json.load(open(r'{}'))\nd=json.load(open(r'{}'))\nprint(len(list(jsonschema.Draft202012Validator(s).iter_errors(d))))",
            skill().join("schemas/hcp-v1.schema.json").display(),
            file.display()
        );
        let o = std::process::Command::new("python3").args(["-c", &script]).output().unwrap();
        let reference: usize = String::from_utf8_lossy(&o.stdout).trim().parse().unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&o.stderr)));
        let mine = fr_constraint::jsonschema::validate(&hcp::schema(), pack).len();
        assert_eq!(reference == 0, mine == 0, "{name}: reference found {reference} problem(s), this program {mine}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
