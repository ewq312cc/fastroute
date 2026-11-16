//! The validator: a legal design passes, each kind of mistake fails at the right rule and object,
//! the answer is the same every time, and what it cannot check is said.

mod common;
use common::*;
use fr_constraint::validate::{validate, Validation, Verdict};
use serde_json::{json, Value};

fn run(snap: &fr_constraint::snapshot::Snapshot) -> Validation {
    validate(&ir(), snap, "demo", None).unwrap()
}

fn failed(v: &Validation, rule: &str) -> Vec<fr_constraint::validate::Finding> {
    v.findings.iter().filter(|f| f.rule_id == rule && f.verdict == Verdict::Fail).cloned().collect()
}

fn verdict_of(v: &Validation, rule: &str) -> Verdict {
    v.findings.iter().filter(|f| f.rule_id == rule).map(|f| f.verdict).min().unwrap()
}

#[test]
fn a_legal_design_passes_and_the_gaps_are_listed_not_hidden() {
    let v = run(&legal());
    assert_eq!(v.result, "pass", "{:#?}", v.findings.iter().filter(|f| f.verdict == Verdict::Fail).collect::<Vec<_>>());
    assert_eq!(v.summary.failed, 0);
    for id in ["C-RESERVED", "C-GND", "C-NEED-POWER", "C-NEED-RESET", "C-NEED-BOOT", "C-ANT", "C-USB-EXCL", "PINMUX-FM100"] {
        assert_eq!(verdict_of(&v, id), Verdict::Pass, "{id}: {:#?}", v.findings.iter().filter(|f| f.rule_id == id).collect::<Vec<_>>());
    }
    // what this version cannot do, or what the documents do not say, is not a pass
    assert_eq!(verdict_of(&v, "C-USB-GAP"), Verdict::Unknown);
    assert_eq!(verdict_of(&v, "C-STRAP-IO0"), Verdict::Pass);
    assert_eq!(verdict_of(&v, "C-USB"), Verdict::Unchecked, "the impedance is not checked (it needs the stackup)");
    assert!(!v.complete);
    assert_eq!(v.summary.rules, ir().rules.len());
}

#[test]
fn the_answer_is_the_same_every_time_and_in_any_order() {
    let a = serde_json::to_string(&run(&legal())).unwrap();
    let b = serde_json::to_string(&run(&legal())).unwrap();
    assert_eq!(a, b);
    let shuffled = changed(|v| {
        for k in ["components", "pins", "tracks", "assignments"] {
            v[k].as_array_mut().unwrap().reverse();
        }
    });
    let c = serde_json::to_string(&run(&shuffled)).unwrap();
    // (the design digest of a re-ordered snapshot differs; the findings do not)
    let (fa, fc): (Value, Value) = (serde_json::from_str::<Value>(&a).unwrap()["findings"].clone(), serde_json::from_str::<Value>(&c).unwrap()["findings"].clone());
    assert_eq!(fa, fc);
}

#[test]
fn removing_a_required_peripheral_fails_the_block_rule() {
    // no 100 nF capacitor on the supply: the block needs a bypass capacitor of at least 100 nF
    let s = changed(|v| {
        v["components"].as_array_mut().unwrap().retain(|c| c["ref"] != "C2");
        v["pins"].as_array_mut().unwrap().retain(|p| p["ref"] != "C2");
    });
    let v = run(&s);
    assert_eq!(v.result, "fail");
    let f = failed(&v, "C-NEED-POWER");
    assert_eq!(f.len(), 1, "{:#?}", v.findings);
    assert_eq!(f[0].object.as_deref(), Some("block BLK-POWER"));
    assert!(f[0].message.contains("bypass") && f[0].message.contains("found 0"), "{}", f[0].message);
    assert_eq!(f[0].required.as_ref().unwrap()["min_count"], json!(1));
    assert!(!f[0].evidence.is_empty(), "a failure points at the manual");
    // a capacitor that is too small does not count either
    let small = changed(|v| {
        for c in v["components"].as_array_mut().unwrap() {
            if c["ref"] == "C2" {
                c["value"] = json!("10nF");
            }
        }
    });
    assert_eq!(failed(&run(&small), "C-NEED-POWER").len(), 1);
    // the reset circuit: remove the pull-up resistor
    let s = changed(|v| {
        v["components"].as_array_mut().unwrap().retain(|c| c["ref"] != "R1");
        v["pins"].as_array_mut().unwrap().retain(|p| p["ref"] != "R1");
    });
    let f = failed(&run(&s), "C-NEED-RESET");
    assert_eq!(f.len(), 1);
    assert!(f[0].message.contains("resistor"), "{}", f[0].message);
    // a pull-up of the wrong value is not the pull-up the block asks for
    let wrong = changed(|v| {
        for c in v["components"].as_array_mut().unwrap() {
            if c["ref"] == "R1" {
                c["value"] = json!("100k");
            }
        }
    });
    assert_eq!(failed(&run(&wrong), "C-NEED-RESET").len(), 1);
}

#[test]
fn a_required_block_whose_pin_is_on_the_wrong_net_fails_at_that_pin() {
    let s = changed(|v| {
        for p in v["pins"].as_array_mut().unwrap() {
            if p["ref"] == "U1" && p["pin"] == "3" {
                p["net"] = json!("RESET_OTHER");
            }
        }
    });
    let f = failed(&run(&s), "C-NEED-RESET");
    assert!(f.iter().any(|f| f.object.as_deref() == Some("U1.3") && f.message.contains("EN*")), "{f:#?}");
}

#[test]
fn using_a_reserved_pin_fails_at_the_pin() {
    let s = changed(|v| {
        for p in v["pins"].as_array_mut().unwrap() {
            if p["ref"] == "U1" && p["pin"] == "10" {
                p["net"] = json!("SPI_CLK");
            }
        }
        v["pins"].as_array_mut().unwrap().push(json!({"ref": "J1", "pin": "3", "net": "SPI_CLK", "x": 50.0, "y": 38.0}));
    });
    let f = failed(&run(&s), "C-RESERVED");
    assert_eq!(f.len(), 1, "{f:#?}");
    assert_eq!(f[0].object.as_deref(), Some("U1.10"));
    assert_eq!(f[0].actual.as_ref().unwrap()["net"], json!("SPI_CLK"));
    assert_eq!(f[0].severity, "error");
    assert_eq!(f[0].evidence[0].source_id, "SRC-FM100-DS");
    // a pin with a net that nothing else uses (a stub) is not "used"
    let stub = changed(|v| {
        for p in v["pins"].as_array_mut().unwrap() {
            if p["ref"] == "U1" && p["pin"] == "11" {
                p["net"] = json!("unconnected-(U1-RSVD2)");
            }
        }
    });
    assert!(failed(&run(&stub), "C-RESERVED").is_empty());
    // an assignment alone (the schematic wants the pin) counts too
    let intent = changed(|v| v["assignments"].as_array_mut().unwrap().push(json!({"ref": "U1", "pin": "11", "function": "GPIO9"})));
    assert!(failed(&run(&intent), "C-RESERVED").iter().any(|f| f.object.as_deref() == Some("U1.11")));
}

#[test]
fn a_fixed_net_mapping_that_is_wrong_or_missing_fails() {
    let wrong = changed(|v| {
        for p in v["pins"].as_array_mut().unwrap() {
            if p["ref"] == "U1" && p["pin"] == "1" {
                p["net"] = json!("VBUS");
            }
        }
    });
    let f = failed(&run(&wrong), "C-GND");
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].object.as_deref(), Some("U1.1"));
    assert_eq!(f[0].required.as_ref().unwrap()["net_pattern"], json!("GND*"));
    // a sheet-qualified net name still matches
    let sheet = changed(|v| {
        for p in v["pins"].as_array_mut().unwrap() {
            if p["ref"] == "U1" && p["pin"] == "1" {
                p["net"] = json!("/power/GND");
            }
        }
    });
    assert!(failed(&run(&sheet), "C-GND").is_empty());
    let none = changed(|v| {
        for p in v["pins"].as_array_mut().unwrap() {
            if p["ref"] == "U1" && p["pin"] == "1" {
                p.as_object_mut().unwrap().remove("net");
            }
        }
    });
    assert!(failed(&run(&none), "C-GND")[0].message.contains("no net"));
}

#[test]
fn pin_multiplexing_conflicts_fail() {
    // a function the pin does not have
    let s = changed(|v| v["assignments"].as_array_mut().unwrap().push(json!({"ref": "U1", "pin": "6", "function": "SPI_MOSI"})));
    let f = failed(&run(&s), "PINMUX-FM100");
    assert!(f.iter().any(|f| f.object.as_deref() == Some("U1.6") && f.message.contains("cannot be SPI_MOSI")), "{f:#?}");
    assert!(f.iter().any(|f| f.message.contains("2 functions at once")), "{f:#?}");
    // two exclusive functions in use together
    let s = changed(|v| v["assignments"].as_array_mut().unwrap().push(json!({"ref": "U1", "pin": "8", "function": "GPIO4"})));
    let v = run(&s);
    assert_eq!(failed(&v, "C-USB-EXCL").len(), 1, "{:#?}", v.findings);
    assert!(failed(&v, "C-USB-EXCL")[0].message.contains("USB_DP and GPIO4") || failed(&v, "C-USB-EXCL")[0].message.contains("GPIO4 and USB_DP"));
    // no assignments at all: not checked, not passed
    let none = changed(|v| v["assignments"] = json!([]));
    let v = run(&none);
    assert_eq!(verdict_of(&v, "C-USB-EXCL"), Verdict::Unchecked);
    assert_eq!(verdict_of(&v, "PINMUX-FM100"), Verdict::Unchecked);
    assert_eq!(v.result, "pass");
    assert!(!v.complete);
}

#[test]
fn copper_in_a_keepout_fails_and_names_what_and_where() {
    // the antenna keep-out is x 40..60, y 0..10
    let track = changed(|v| v["tracks"].as_array_mut().unwrap().push(json!({"net": "LED_DRIVE", "layer": "B.Cu", "width_mm": 0.2, "x1": 30.0, "y1": 5.0, "x2": 45.0, "y2": 5.0})));
    let f = failed(&run(&track), "C-ANT");
    assert_eq!(f.len(), 1, "{f:#?}");
    assert_eq!(f[0].object.as_deref(), Some("region REG-ANT"));
    assert!(f[0].message.contains("track on net LED_DRIVE"));
    assert_eq!(f[0].actual.as_ref().unwrap()["count"], json!(1));
    // a track that only passes close but outside is fine; one whose width reaches in is not
    let near = changed(|v| v["tracks"].as_array_mut().unwrap().push(json!({"net": "X", "layer": "F.Cu", "width_mm": 0.2, "x1": 30.0, "y1": 10.2, "x2": 45.0, "y2": 10.2})));
    assert!(failed(&run(&near), "C-ANT").is_empty());
    let wide = changed(|v| v["tracks"].as_array_mut().unwrap().push(json!({"net": "X", "layer": "F.Cu", "width_mm": 0.6, "x1": 30.0, "y1": 10.2, "x2": 45.0, "y2": 10.2})));
    assert_eq!(failed(&run(&wide), "C-ANT").len(), 1);
    // a via, a component and a copper zone
    let via = changed(|v| v["vias"] = json!([{"net": "GND", "x": 50.0, "y": 5.0, "diameter_mm": 0.6}]));
    assert!(failed(&run(&via), "C-ANT")[0].message.contains("via on net GND"));
    let comp = changed(|v| v["components"].as_array_mut().unwrap().push(json!({"ref": "C9", "value": "1u", "x": 55.0, "y": 5.0})));
    assert!(failed(&run(&comp), "C-ANT")[0].message.contains("component C9"));
    let zone = changed(|v| v["zones"] = json!([{"net": "GND", "layer": "F.Cu", "polygon": [[35.0, 5.0], [45.0, 5.0], [45.0, 20.0], [35.0, 20.0]]}]));
    assert!(failed(&run(&zone), "C-ANT")[0].message.contains("zone on net GND"));
    // a design's own keep-out (a rule area) is not copper
    let keep = changed(|v| v["zones"] = json!([{"layer": "F.Cu", "polygon": [[35.0, 5.0], [45.0, 5.0], [45.0, 20.0], [35.0, 20.0]], "keepout": true}]));
    assert!(failed(&run(&keep), "C-ANT").is_empty());
}

#[test]
fn a_findings_always_says_rule_object_values_severity_and_evidence() {
    let s = changed(|v| {
        for p in v["pins"].as_array_mut().unwrap() {
            if p["ref"] == "U1" && p["pin"] == "1" {
                p["net"] = json!("VBUS");
            }
        }
    });
    let v = run(&s);
    let j = serde_json::to_value(&v).unwrap();
    let f = j["findings"].as_array().unwrap().iter().find(|f| f["verdict"] == "fail").unwrap();
    for key in ["rule_id", "kind", "object", "message", "actual", "required", "severity", "evidence"] {
        assert!(f.get(key).is_some(), "{key} is missing from {f}");
    }
    assert_eq!(f["evidence"][0]["quote"], json!("GND: ground."));
    assert_eq!(j["format"], json!("fastroute-hcp-validation"));
    assert!(j["ir"]["pack_sha256"].as_str().unwrap().len() == 64);
}

#[test]
fn a_device_that_is_not_in_the_design_is_not_applicable_not_a_pass() {
    let s = changed(|v| {
        v["components"].as_array_mut().unwrap().retain(|c| c["ref"] != "U1");
    });
    let v = run(&s);
    assert_eq!(verdict_of(&v, "C-RESERVED"), Verdict::NotApplicable);
    assert!(v.findings.iter().any(|f| f.verdict == Verdict::NotApplicable && f.message.contains("no component for DEV-FM100")));
    // the part number as the value is enough to find the device
    let by_value = changed(|v| {
        for c in v["components"].as_array_mut().unwrap() {
            if c["ref"] == "U1" {
                c.as_object_mut().unwrap().remove("device");
            }
        }
    });
    assert_eq!(verdict_of(&run(&by_value), "C-RESERVED"), Verdict::Pass);
}

#[test]
fn a_warning_rule_does_not_fail_the_result_but_is_reported() {
    let mut p = example();
    p["rules"].as_array_mut().unwrap().iter_mut().find(|r| r["id"] == "C-GND").unwrap()["severity"] = json!("warning");
    let ir = fr_constraint::compile::compile(&p).unwrap();
    let s = changed(|v| {
        for p in v["pins"].as_array_mut().unwrap() {
            if p["ref"] == "U1" && p["pin"] == "1" {
                p["net"] = json!("VBUS");
            }
        }
    });
    let v = validate(&ir, &s, "demo", None).unwrap();
    assert_eq!(v.result, "pass");
    assert_eq!(v.summary.warnings, 1);
    assert_eq!(v.findings.iter().find(|f| f.verdict == Verdict::Fail).unwrap().severity, "warning");
}

#[test]
fn trace_width_rules_are_checked_per_net() {
    let mut p = example();
    p["rules"].as_array_mut().unwrap().push(json!({
        "id": "C-W3V3", "kind": "trace_width", "modifiability": "configurable",
        "params": {"width_mm": 0.5, "nets": ["3V3"]},
        "evidence": [{"source_id": "SRC-FM100-HDG", "locator": {"section": "4.3 Power"}, "quote": "Use wide traces for the supply.", "basis": "explicit"}]
    }));
    let ir = fr_constraint::compile::compile(&p).unwrap();
    let v = validate(&ir, &legal(), "demo", None).unwrap();
    let f: Vec<_> = v.findings.iter().filter(|f| f.rule_id == "C-W3V3" && f.verdict == Verdict::Fail).collect();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].object.as_deref(), Some("net 3V3"));
    assert_eq!(f[0].actual.as_ref().unwrap()["min_width_mm"], json!(0.4));
    // other nets are not looked at
    let ok = changed(|v| v["tracks"].as_array_mut().unwrap()[2]["width_mm"] = json!(0.5));
    let v = validate(&ir, &ok, "demo", None).unwrap();
    assert!(v.findings.iter().all(|f| !(f.rule_id == "C-W3V3" && f.verdict == Verdict::Fail)));
}

#[test]
fn the_validator_reads_nothing_but_the_ir_and_the_design() {
    // retrieval data is not in the IR, so there is nothing for it to read: the same IR, the same
    // design, the same findings, with or without whatever index exists next to the pack
    let ir = ir();
    let text = ir.to_json_text();
    assert!(!text.contains("embedding") && !text.contains("similarity") && !text.contains("rank"));
    let a = serde_json::to_string(&validate(&ir, &legal(), "demo", None).unwrap()).unwrap();
    let again: fr_constraint::ir::Ir = serde_json::from_str(&text).unwrap();
    let b = serde_json::to_string(&validate(&again, &legal(), "demo", None).unwrap()).unwrap();
    assert_eq!(a, b);
}
