//! BOOT/RST, high-speed and mechanical rules: the checks of the validator and their negatives.

mod common;
use common::*;
use fr_constraint::compile;
use fr_constraint::snapshot::Snapshot;
use fr_constraint::validate::{validate, Finding, Validation, Verdict};
use serde_json::{json, Value};

fn run_with(pack: &Value, snap: &Snapshot) -> Validation {
    validate(&compile::compile(pack).expect("the pack compiles"), snap, "demo", None).unwrap()
}

fn run(snap: &Snapshot) -> Validation {
    run_with(&example(), snap)
}

fn failed(v: &Validation, rule: &str) -> Vec<Finding> {
    v.findings.iter().filter(|f| f.rule_id == rule && f.verdict == Verdict::Fail).cloned().collect()
}

fn verdict_of(v: &Validation, rule: &str) -> Verdict {
    v.findings.iter().filter(|f| f.rule_id == rule).map(|f| f.verdict).min().unwrap()
}

fn set_net(v: &mut Value, reference: &str, pin: &str, net: Option<&str>) {
    for p in v["pins"].as_array_mut().unwrap() {
        if p["ref"] == reference && p["pin"] == pin {
            match net {
                Some(n) => p["net"] = json!(n),
                None => {
                    p.as_object_mut().unwrap().remove("net");
                }
            }
        }
    }
}

fn with_rule(rule: Value) -> Value {
    let mut p = example();
    p["rules"].as_array_mut().unwrap().push(rule);
    p
}

fn ev() -> Value {
    json!([{"source_id": "SRC-FM100-DS", "locator": {"page": 8}, "quote": "x", "basis": "explicit"}])
}

// ---- strapping ------------------------------------------------------------------------------------

#[test]
fn the_legal_design_passes_the_strapping_reset_and_mechanical_rules() {
    let v = run(&legal());
    for id in ["C-STRAP-IO0", "C-RESET", "C-MOUNT", "C-USB"] {
        let want = if id == "C-USB" { Verdict::Unchecked } else { Verdict::Pass };
        assert_eq!(verdict_of(&v, id), want, "{id}: {:#?}", v.findings.iter().filter(|f| f.rule_id == id).collect::<Vec<_>>());
    }
    assert_eq!(v.result, "pass");
}

#[test]
fn a_boot_pin_pulled_hard_by_something_outside_fails_the_strapping_rule() {
    // tied straight to ground
    let s = changed(|v| set_net(v, "U1", "4", Some("GND")));
    let f = failed(&run(&s), "C-STRAP-IO0");
    assert_eq!(f.len(), 1, "{f:#?}");
    assert_eq!(f[0].object.as_deref(), Some("U1.4"));
    assert!(f[0].message.contains("tied directly to the ground net GND"), "{}", f[0].message);
    assert_eq!(f[0].required.as_ref().unwrap()["external_drive_allowed"], json!(false));
    // tied straight to the supply
    assert_eq!(failed(&run(&changed(|v| set_net(v, "U1", "4", Some("3V3")))), "C-STRAP-IO0").len(), 1);
    // a sheet-qualified rail name is still a rail
    assert_eq!(failed(&run(&changed(|v| set_net(v, "U1", "4", Some("/power/GND")))), "C-STRAP-IO0").len(), 1);
    // driven by a chip or a connector: an external driver decides the level
    let driven = changed(|v| {
        v["components"].as_array_mut().unwrap().push(json!({"ref": "U2", "value": "74LVC1G", "x": 40.0, "y": 30.0}));
        v["pins"].as_array_mut().unwrap().push(json!({"ref": "U2", "pin": "4", "net": "BOOT", "x": 40.0, "y": 30.0}));
    });
    let f = failed(&run(&driven), "C-STRAP-IO0");
    assert!(f.len() == 1 && f[0].message.contains("U2 can drive"), "{f:#?}");
    // a button, a resistor or a capacitor on the net is the intended circuit
    assert!(failed(&run(&legal()), "C-STRAP-IO0").is_empty());
    // if the pack says an external drive is allowed (a programmer's auto-boot circuit), it is
    let mut p = example();
    for r in p["rules"].as_array_mut().unwrap() {
        if r["id"] == "C-STRAP-IO0" {
            r["params"]["external_drive_allowed"] = json!(true);
        }
    }
    assert!(failed(&run_with(&p, &changed(|v| set_net(v, "U1", "4", Some("GND")))), "C-STRAP-IO0").is_empty());
}

#[test]
fn a_strapping_pin_is_not_a_free_gpio() {
    let s = changed(|v| v["assignments"].as_array_mut().unwrap().push(json!({"ref": "U1", "pin": "4", "function": "GPIO0"})));
    let f = failed(&run(&s), "C-STRAP-IO0");
    assert_eq!(f.len(), 1, "{f:#?}");
    assert!(f[0].message.contains("not a free GPIO") && f[0].message.contains("GPIO0"), "{}", f[0].message);
    assert_eq!(f[0].actual.as_ref().unwrap()["assigned_function"], json!("GPIO0"));
    // the pin's own boot function is not a misuse
    let ok = changed(|v| v["assignments"].as_array_mut().unwrap().push(json!({"ref": "U1", "pin": "4", "function": "BOOT_SEL"})));
    assert!(failed(&run(&ok), "C-STRAP-IO0").is_empty());
}

// ---- reset ----------------------------------------------------------------------------------------

#[test]
fn a_reset_pin_that_is_held_or_floating_fails() {
    // active low: tied to ground, the device is held in reset
    let held = changed(|v| set_net(v, "U1", "3", Some("GND")));
    let f = failed(&run(&held), "C-RESET");
    assert_eq!(f.len(), 1, "{f:#?}");
    assert!(f[0].message.contains("held in reset"), "{}", f[0].message);
    assert_eq!(f[0].object.as_deref(), Some("U1.3"));
    // tied to the supply is not a reset conflict (the reset circuit rule looks at that block)
    assert!(failed(&run(&changed(|v| set_net(v, "U1", "3", Some("3V3")))), "C-RESET").is_empty());
    // unconnected: a reset input must not float
    let floating = changed(|v| set_net(v, "U1", "3", None));
    assert!(failed(&run(&floating), "C-RESET")[0].message.contains("must not float"));
    let stub = changed(|v| set_net(v, "U1", "3", Some("unconnected-(U1-EN)")));
    assert!(failed(&run(&stub), "C-RESET")[0].message.contains("unconnected"));
    // active high: the other way round
    let mut p = example();
    for r in p["rules"].as_array_mut().unwrap() {
        if r["id"] == "C-RESET" {
            r["params"]["active_level"] = json!("high");
        }
    }
    assert_eq!(failed(&run_with(&p, &changed(|v| set_net(v, "U1", "3", Some("3V3")))), "C-RESET").len(), 1);
    assert!(failed(&run_with(&p, &changed(|v| set_net(v, "U1", "3", Some("GND")))), "C-RESET").is_empty());
}

#[test]
fn a_missing_reset_or_boot_peripheral_is_found() {
    // no boot button: the block is missing, and the boot pin has no way to be set
    let no_button = changed(|v| {
        v["components"].as_array_mut().unwrap().retain(|c| c["ref"] != "SW1");
        v["pins"].as_array_mut().unwrap().retain(|p| p["ref"] != "SW1");
    });
    let v = run(&no_button);
    assert_eq!(failed(&v, "C-NEED-BOOT").len(), 1, "{:#?}", v.findings);
    let p = with_rule(json!({
        "id": "C-BOOTPIN", "kind": "boot_pin", "modifiability": "fixed", "subject": {"device": "DEV-FM100", "pin": "4"},
        "params": {"boot_level": "low", "mode": "serial download"}, "evidence": ev()
    }));
    let f = failed(&run_with(&p, &no_button), "C-BOOTPIN");
    assert_eq!(f.len(), 1);
    assert!(f[0].message.contains("no button, jumper or header"), "{}", f[0].message);
    // with the button it passes; a header is as good as a button
    assert!(failed(&run_with(&p, &legal()), "C-BOOTPIN").is_empty());
    let header = changed(|v| {
        v["components"].as_array_mut().unwrap().retain(|c| c["ref"] != "SW1");
        v["components"].as_array_mut().unwrap().push(json!({"ref": "J5", "value": "BOOT", "x": 22.0, "y": 34.0}));
        for q in v["pins"].as_array_mut().unwrap() {
            if q["ref"] == "SW1" {
                q["ref"] = json!("J5");
            }
        }
    });
    assert!(failed(&run_with(&p, &header), "C-BOOTPIN").is_empty());
    // tied to a rail: the boot mode cannot be selected
    let tied = changed(|v| set_net(v, "U1", "4", Some("3V3")));
    assert!(failed(&run_with(&p, &tied), "C-BOOTPIN")[0].message.contains("cannot be selected"));
    let nonet = changed(|v| set_net(v, "U1", "4", None));
    assert!(failed(&run_with(&p, &nonet), "C-BOOTPIN")[0].message.contains("no way to select"));
}

// ---- high speed -----------------------------------------------------------------------------------

#[test]
fn a_pair_with_too_much_skew_fails_and_an_unrouted_pair_is_not_a_pass() {
    // the legal design's pair is equally long; stretch one track
    let skewed = changed(|v| v["tracks"].as_array_mut().unwrap()[1]["x2"] = json!(52.0));
    let f = failed(&run(&skewed), "C-USB");
    assert_eq!(f.len(), 1, "{f:#?}");
    assert_eq!(f[0].object.as_deref(), Some("nets USB_DM / USB_DP"));
    assert!(f[0].message.contains("at most 0.5 mm"), "{}", f[0].message);
    assert!(f[0].actual.as_ref().unwrap()["skew_mm"].as_f64().unwrap() > 0.5);
    // nothing routed: not a pass
    let unrouted = changed(|v| v["tracks"].as_array_mut().unwrap().retain(|t| t["net"] != "USB_DP" && t["net"] != "USB_DM"));
    let v = run(&unrouted);
    assert!(v.findings.iter().any(|f| f.rule_id == "C-USB" && f.verdict == Verdict::Unchecked && f.message.contains("not routed yet")), "{:#?}", v.findings);
    // the interface is not in the design: not applicable
    let none = changed(|v| {
        v["pins"].as_array_mut().unwrap().retain(|p| p["net"] != "USB_DP" && p["net"] != "USB_DM");
        v["tracks"].as_array_mut().unwrap().retain(|t| t["net"] != "USB_DP" && t["net"] != "USB_DM");
    });
    assert_eq!(verdict_of(&run(&none), "C-USB"), Verdict::NotApplicable);
}

#[test]
fn a_rule_with_no_parameter_to_check_is_unchecked_never_passed() {
    // a differential rule whose parameters are all missing: the pack keeps it unknown ...
    let p = with_rule(json!({
        "id": "C-DIFF", "kind": "high_speed", "modifiability": "unknown", "params": {"nets": ["USB_DP", "USB_DM"], "differential": true, "gap_mm": null, "skew_mm": null},
        "missing": "the guide gives no pair parameters"
    }));
    assert_eq!(verdict_of(&run_with(&p, &legal()), "C-DIFF"), Verdict::Unknown);
    // ... and a configurable one that names only the nets has nothing the validator looks at
    let bare = with_rule(json!({
        "id": "C-BARE", "kind": "high_speed", "modifiability": "configurable", "params": {"nets": ["USB_DP", "USB_DM"], "differential": true}, "evidence": ev()
    }));
    let v = run_with(&bare, &legal());
    assert_eq!(verdict_of(&v, "C-BARE"), Verdict::Unchecked);
    assert!(v.findings.iter().any(|f| f.rule_id == "C-BARE" && f.message.contains("no parameter that this validator checks")));
    // the parameters that cannot be checked are named, rule by rule
    let usb = v.findings.iter().filter(|f| f.rule_id == "C-USB" && f.verdict == Verdict::Unchecked).map(|f| f.message.clone()).collect::<Vec<_>>();
    assert!(usb.iter().any(|m| m.contains("impedance_ohm")), "{usb:?}");
    assert!(!v.complete);
}

#[test]
fn length_layers_and_width_limits_of_an_interface_are_checked() {
    let p = with_rule(json!({
        "id": "C-HS", "kind": "high_speed", "modifiability": "configurable",
        "params": {"nets": ["TXD", "RXD"], "differential": false, "max_length_mm": 10, "length_match_tolerance_mm": 1, "min_width_mm": 0.15, "layers": ["F.Cu"]},
        "evidence": ev()
    }));
    let design = changed(|v| {
        let t = v["tracks"].as_array_mut().unwrap();
        t.push(json!({"net": "TXD", "layer": "F.Cu", "width_mm": 0.2, "x1": 0.0, "y1": 0.0, "x2": 8.0, "y2": 0.0}));
        t.push(json!({"net": "RXD", "layer": "B.Cu", "width_mm": 0.1, "x1": 0.0, "y1": 1.0, "x2": 14.0, "y2": 1.0}));
    });
    let v = run_with(&p, &design);
    let msgs: Vec<String> = failed(&v, "C-HS").iter().map(|f| f.message.clone()).collect();
    assert!(msgs.iter().any(|m| m.contains("net RXD is 14.000 mm long, at most 10 mm")), "{msgs:?}");
    assert!(msgs.iter().any(|m| m.contains("differ by 6.000 mm, at most 1 mm")), "{msgs:?}");
    assert!(msgs.iter().any(|m| m.contains("narrower than 0.15 mm")), "{msgs:?}");
    assert!(msgs.iter().any(|m| m.contains("tracks on B.Cu, the rule allows F.Cu")), "{msgs:?}");
    // the same limits met: pass
    let fine = changed(|v| {
        let t = v["tracks"].as_array_mut().unwrap();
        t.push(json!({"net": "TXD", "layer": "F.Cu", "width_mm": 0.2, "x1": 0.0, "y1": 0.0, "x2": 8.0, "y2": 0.0}));
        t.push(json!({"net": "RXD", "layer": "F.Cu", "width_mm": 0.2, "x1": 0.0, "y1": 1.0, "x2": 8.5, "y2": 1.0}));
    });
    assert_eq!(verdict_of(&run_with(&p, &fine), "C-HS"), Verdict::Pass);
}

// ---- mechanical -----------------------------------------------------------------------------------

#[test]
fn a_moved_mounting_hole_a_changed_outline_and_a_missing_connector_fail() {
    let moved = changed(|v| v["mount_holes"] = json!([{"x": 3.0, "y": 5.0, "radius_mm": 1.6}]));
    let f = failed(&run(&moved), "C-MOUNT");
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].object.as_deref(), Some("region REG-MOUNT1"));
    assert!(f[0].message.contains("no mounting hole at (3, 3)"), "{}", f[0].message);
    // a hole of another size is not the fixed one either
    assert_eq!(failed(&run(&changed(|v| v["mount_holes"] = json!([{"x": 3.0, "y": 3.0, "radius_mm": 2.0}]))), "C-MOUNT").len(), 1);
    // the outline and a connector, as rules of the pack
    let mut p = example();
    p["regions"].as_array_mut().unwrap().push(json!({
        "id": "REG-USBJ", "kind": "connector", "unit": "mm", "modifiability": "fixed", "polygon": [[48.0, 33.0], [56.0, 33.0], [56.0, 39.0], [48.0, 39.0]],
        "evidence": [{"source_id": "SRC-OPENBOARD", "locator": {"section": "Mechanical drawing"}, "quote": "USB connector at the edge", "basis": "observed"}]
    }));
    p["rules"].as_array_mut().unwrap().extend([
        json!({"id": "C-OUT", "kind": "mechanical_fixed", "modifiability": "fixed", "subject": {"region": "REG-OUTLINE"},
               "evidence": [{"source_id": "SRC-OPENBOARD", "locator": {"section": "Mechanical drawing"}, "quote": "Board 60 x 40 mm.", "basis": "observed"}]}),
        json!({"id": "C-USBJ", "kind": "mechanical_fixed", "modifiability": "fixed", "subject": {"region": "REG-USBJ"},
               "evidence": [{"source_id": "SRC-OPENBOARD", "locator": {"section": "Mechanical drawing"}, "quote": "USB connector at the edge", "basis": "observed"}]}),
    ]);
    let ok = run_with(&p, &legal());
    assert_eq!(verdict_of(&ok, "C-OUT"), Verdict::Pass, "{:#?}", ok.findings.iter().filter(|f| f.rule_id == "C-OUT").collect::<Vec<_>>());
    assert_eq!(verdict_of(&ok, "C-USBJ"), Verdict::Pass);
    let smaller = changed(|v| v["outline"] = json!([[0.0, 0.0], [58.0, 0.0], [58.0, 40.0], [0.0, 40.0]]));
    let f = failed(&run_with(&p, &smaller), "C-OUT");
    assert_eq!(f.len(), 1);
    assert!(f[0].message.contains("not the fixed one"));
    let no_connector = changed(|v| v["components"].as_array_mut().unwrap().iter_mut().find(|c| c["ref"] == "J1").unwrap()["x"] = json!(10.0));
    assert_eq!(failed(&run_with(&p, &no_connector), "C-USBJ").len(), 1);
}

#[test]
fn copper_in_the_antenna_keepout_is_refused_whatever_enters_it() {
    // (the keep-out checks exist since the first validator; here with the module's own rotation and layers)
    let via = changed(|v| v["vias"] = json!([{"net": "GND", "x": 59.9, "y": 0.1, "diameter_mm": 0.3}]));
    assert_eq!(failed(&run(&via), "C-ANT").len(), 1);
    // the keep-out applies to all layers: a bottom-layer track too
    let bottom = changed(|v| v["tracks"].as_array_mut().unwrap().push(json!({"net": "X", "layer": "B.Cu", "width_mm": 0.2, "x1": 41.0, "y1": 9.0, "x2": 42.0, "y2": 9.0})));
    assert_eq!(failed(&run(&bottom), "C-ANT").len(), 1);
}

#[test]
fn a_module_body_outside_the_board_is_found() {
    let mut p = example();
    p["regions"].as_array_mut().unwrap().push(json!({
        "id": "REG-BODY", "kind": "module_body", "unit": "mm", "frame": "device", "device": "DEV-FM100", "modifiability": "fixed",
        "polygon": [[-9.0, -9.0], [9.0, -9.0], [9.0, 9.0], [-9.0, 9.0]],
        "evidence": [{"source_id": "SRC-FM100-DS", "locator": {"page": 4}, "quote": "Module size 18 x 18 mm.", "basis": "explicit"}]
    }));
    p["rules"].as_array_mut().unwrap().push(json!({
        "id": "C-BODY", "kind": "mechanical_fixed", "modifiability": "fixed", "subject": {"region": "REG-BODY"},
        "evidence": [{"source_id": "SRC-FM100-DS", "locator": {"page": 4}, "quote": "Module size 18 x 18 mm.", "basis": "explicit"}]
    }));
    // the module sits at (30, 28): its 18 mm body is inside the 60 x 40 board
    assert_eq!(verdict_of(&run_with(&p, &legal()), "C-BODY"), Verdict::Pass);
    let off = changed(|v| v["components"].as_array_mut().unwrap().iter_mut().find(|c| c["ref"] == "U1").unwrap()["x"] = json!(55.0));
    let f = failed(&run_with(&p, &off), "C-BODY");
    assert!(!f.is_empty() && f[0].message.contains("outside the board outline"), "{f:#?}");
}

// ---- the templates ---------------------------------------------------------------------------------

#[test]
fn every_rule_template_instantiates_to_a_valid_rule() {
    let text = std::fs::read_to_string(skill().join("assets/rule-templates.yaml")).unwrap();
    let doc = fr_constraint::hcp::parse(&text).unwrap();
    assert_eq!(doc["template_version"], "1.0");
    let templates = doc["templates"].as_array().unwrap();
    assert!(templates.len() >= 12, "{}", templates.len());
    let mut seen = std::collections::BTreeSet::new();
    for t in templates {
        let name = t["name"].as_str().unwrap();
        assert!(seen.insert(name.to_string()), "duplicate template {name}");
        assert!(t["use_when"].as_str().is_some_and(|s| !s.is_empty()) && t["evidence_needed"].as_str().is_some_and(|s| !s.is_empty()), "{name}");
        // fill the placeholders with things the example pack has
        let filled = serde_json::to_string(&t["rule"]).unwrap().replace("DEV-EXAMPLE", "DEV-FM100").replace("SRC-EXAMPLE", "SRC-FM100-DS").replace("BLK-EXAMPLE", "BLK-POWER").replace("REG-EXAMPLE-KEEPOUT", "REG-ANT").replace("REG-EXAMPLE-HOLE", "REG-MOUNT1");
        let mut rule: Value = serde_json::from_str(&filled).unwrap();
        rule["id"] = json!(format!("C-T-{}", name.to_uppercase().replace('_', "-")));
        let mut p = example();
        p["rules"].as_array_mut().unwrap().push(rule);
        let d = fr_constraint::hcp::check(&p);
        assert!(d.iter().all(|x| x.severity == fr_constraint::diag::Severity::Warning), "template {name}: {d:#?}");
        // and it compiles
        compile::compile(&p).unwrap_or_else(|e| panic!("template {name}: {e:#?}"));
    }
}
