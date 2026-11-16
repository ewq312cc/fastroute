//! `fastroute constraints`: the command line around the HCP checks.

mod common;
use common::*;
use serde_json::{json, Value};
use std::path::PathBuf;

fn skill() -> PathBuf {
    manifest_dir().join("../../skills/hardware-constraint-design")
}

fn example() -> PathBuf {
    skill().join("assets/example-device.yaml")
}

#[test]
fn check_accepts_the_example_and_says_so_in_json() {
    let o = run(&["constraints", "check", &s(&example()), "--json", "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!((v["format"].as_str(), v["valid"].as_bool(), v["errors"].as_u64()), (Some("fastroute-hcp-check"), Some(true), Some(0)));
    assert_eq!(v["input"]["sha256"].as_str().unwrap().len(), 64);
}

#[test]
fn check_refuses_a_bad_pack_with_exit_3_and_names_the_rule() {
    let d = tmp("constraints-bad");
    let mut pack: Value = serde_json::from_str(&serde_json::to_string(&fr_constraint::hcp::parse(&std::fs::read_to_string(example()).unwrap()).unwrap()).unwrap()).unwrap();
    pack["rules"][0].as_object_mut().unwrap().remove("evidence");
    let f = d.join("bad.json");
    std::fs::write(&f, serde_json::to_string(&pack).unwrap()).unwrap();
    let o = run(&["constraints", "check", &s(&f), "--json"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["valid"], json!(false));
    assert_eq!(v["diagnostics"][0]["object"], "C-RESERVED");
    assert!(stderr(&o).contains("error[schema]") && stderr(&o).contains("C-RESERVED"), "{}", stderr(&o));
}

#[test]
fn check_reports_usage_input_and_output_problems_with_their_codes() {
    assert_eq!(code(&run(&["constraints", "check"])), 2);
    assert_eq!(code(&run(&["constraints", "frobnicate"])), 2);
    assert_eq!(code(&run(&["constraints", "check", "/nonexistent/pack.yaml"])), 3);
    let d = tmp("constraints-io");
    let junk = d.join("junk.yaml");
    std::fs::write(&junk, "a: [unclosed\n").unwrap();
    assert_eq!(code(&run(&["constraints", "check", &s(&junk)])), 3);
    // an output that exists is refused, and an input is never overwritten
    let out = d.join("report.json");
    std::fs::write(&out, "x").unwrap();
    assert_eq!(code(&run(&["constraints", "check", &s(&example()), "-o", &s(&out)])), 4);
    assert_eq!(code(&run(&["constraints", "check", &s(&example()), "-o", &s(&out), "--force", "-q"])), 0);
    assert_eq!(code(&run(&["constraints", "check", &s(&example()), "-o", &s(&example()), "--force"])), 4);
}

#[test]
fn schema_prints_the_published_file_and_help_works_everywhere() {
    let o = run(&["constraints", "schema"]);
    assert_eq!(code(&o), 0);
    assert_eq!(stdout(&o), std::fs::read_to_string(skill().join("schemas/hcp-v1.schema.json")).unwrap());
    for args in [vec!["constraints", "--help"], vec!["constraints", "check", "--help"], vec!["help", "constraints"], vec!["help", "constraints", "check"]] {
        let o = run(&args);
        assert_eq!(code(&o), 0, "{args:?}");
        assert!(stdout(&o).contains("constraints"), "{args:?}");
    }
}

fn legal_design(dir: &std::path::Path) -> PathBuf {
    // the legal design of the example pack, as the validator's tests build it
    let f = dir.join("design.json");
    let v = serde_json::json!({
        "snapshot_version": "1.0", "project": {"name": "fm100-demo"},
        "components": [
            {"ref": "U1", "device": "DEV-FM100", "value": "FM-100", "x": 30.0, "y": 28.0},
            {"ref": "C1", "value": "10uF", "x": 24.0, "y": 22.0}, {"ref": "C2", "value": "100nF", "x": 25.0, "y": 22.0},
            {"ref": "R1", "value": "10k", "x": 26.0, "y": 30.0}, {"ref": "C3", "value": "1nF", "x": 27.0, "y": 30.0},
            {"ref": "SW1", "value": "BOOT", "x": 22.0, "y": 34.0}
        ],
        "pins": [
            {"ref": "U1", "pin": "1", "net": "GND", "x": 28.0, "y": 22.0}, {"ref": "U1", "pin": "2", "net": "3V3", "x": 28.0, "y": 24.0},
            {"ref": "U1", "pin": "3", "net": "EN", "x": 28.0, "y": 26.0}, {"ref": "U1", "pin": "4", "net": "BOOT", "x": 28.0, "y": 28.0},
            {"ref": "C1", "pin": "1", "net": "3V3", "x": 24.0, "y": 22.0}, {"ref": "C1", "pin": "2", "net": "GND", "x": 24.0, "y": 23.0},
            {"ref": "C2", "pin": "1", "net": "3V3", "x": 25.0, "y": 22.0}, {"ref": "C2", "pin": "2", "net": "GND", "x": 25.0, "y": 23.0},
            {"ref": "R1", "pin": "1", "net": "3V3", "x": 26.0, "y": 30.0}, {"ref": "R1", "pin": "2", "net": "EN", "x": 26.0, "y": 31.0},
            {"ref": "C3", "pin": "1", "net": "EN", "x": 27.0, "y": 30.0}, {"ref": "C3", "pin": "2", "net": "GND", "x": 27.0, "y": 31.0},
            {"ref": "SW1", "pin": "1", "net": "BOOT", "x": 22.0, "y": 34.0}, {"ref": "SW1", "pin": "2", "net": "GND", "x": 22.0, "y": 35.0}
        ]
    });
    std::fs::write(&f, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    f
}

#[test]
fn compile_is_deterministic_and_validate_uses_the_compiled_ir() {
    let d = tmp("constraints-compile");
    let ir1 = d.join("ir1.json");
    let ir2 = d.join("ir2.json");
    assert_eq!(code(&run(&["constraints", "compile", &s(&example()), "-o", &s(&ir1), "-q"])), 0);
    assert_eq!(code(&run(&["constraints", "compile", &s(&example()), "-o", &s(&ir2), "-q"])), 0);
    assert_eq!(std::fs::read(&ir1).unwrap(), std::fs::read(&ir2).unwrap(), "two compilations differ");
    // the IR is its published schema
    let o = run(&["constraints", "schema", "--ir"]);
    let schema: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert!(fr_constraint::jsonschema::validate(&schema, &json(&ir1)).is_empty());
    // a legal design: no rule is violated, but the pack has rules the validator cannot check: exit 5
    let design = legal_design(&d);
    let report = d.join("v.json");
    let md = d.join("v.md");
    let o = run(&["constraints", "validate", "--ir", &s(&ir1), "--design", &s(&design), "-o", &s(&report), "--report", &s(&md), "-q"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    let v = json(&report);
    assert_eq!((v["result"].as_str(), v["complete"].as_bool()), (Some("pass"), Some(false)));
    let text = std::fs::read_to_string(&md).unwrap();
    assert!(text.contains("硬件约束校验报告") && text.contains("C-NEED-POWER"), "{text}");
    // the same with the pack on the fly, and an English report
    let md_en = d.join("v-en.md");
    let o = run(&["constraints", "validate", "--pack", &s(&example()), "--design", &s(&design), "--report", &s(&md_en), "--lang", "en", "-q"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(std::fs::read_to_string(&md_en).unwrap().contains("Hardware constraint validation report"));
}

#[test]
fn validate_exits_1_for_a_violation_and_names_the_rule_and_the_pin() {
    let d = tmp("constraints-fail");
    let design = legal_design(&d);
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&design).unwrap()).unwrap();
    v["pins"][0]["net"] = json!("VBUS"); // pin 1 of U1 must be on GND*
    std::fs::write(&design, serde_json::to_string(&v).unwrap()).unwrap();
    let o = run(&["constraints", "validate", "--pack", &s(&example()), "--design", &s(&design), "--json"]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(stderr(&o).contains("error[C-GND]") && stderr(&o).contains("U1.1"), "{}", stderr(&o));
    let j: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(j["result"], "fail");
}

#[test]
fn validate_refuses_bad_inputs_before_anything_is_written() {
    let d = tmp("constraints-validate-io");
    let design = legal_design(&d);
    // usage
    assert_eq!(code(&run(&["constraints", "validate", "--design", &s(&design)])), 2);
    assert_eq!(code(&run(&["constraints", "validate", "--pack", &s(&example()), "--ir", &s(&example()), "--design", &s(&design)])), 2);
    assert_eq!(code(&run(&["constraints", "validate", "--pack", &s(&example()), "--design", &s(&design), "--lang", "fr"])), 2);
    // input: an IR that is not an IR, a snapshot that is not a snapshot, an IR that was edited
    assert_eq!(code(&run(&["constraints", "validate", "--ir", &s(&design), "--design", &s(&design)])), 3);
    assert_eq!(code(&run(&["constraints", "validate", "--pack", &s(&example()), "--design", &s(&example())])), 3);
    let ir = d.join("ir.json");
    assert_eq!(code(&run(&["constraints", "compile", &s(&example()), "-o", &s(&ir), "-q"])), 0);
    let mut edited = json(&ir);
    edited["rules"][0]["severity"] = json!("warning");
    let edited_path = d.join("edited.json");
    std::fs::write(&edited_path, serde_json::to_string(&edited).unwrap()).unwrap();
    let o = run(&["constraints", "validate", "--ir", &s(&edited_path), "--design", &s(&design)]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(stderr(&o).contains("integrity"), "{}", stderr(&o));
    // output: an existing report is refused and left alone; the design is never overwritten
    let out = d.join("out.json");
    std::fs::write(&out, "keep").unwrap();
    assert_eq!(code(&run(&["constraints", "validate", "--ir", &s(&ir), "--design", &s(&design), "-o", &s(&out), "-q"])), 4);
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "keep");
    assert_eq!(code(&run(&["constraints", "validate", "--ir", &s(&ir), "--design", &s(&design), "-o", &s(&design), "--force", "-q"])), 4);
    // compile: a bad pack is not compiled, and nothing is written
    let bad = d.join("bad.json");
    let mut pack: Value = serde_json::from_str(&serde_json::to_string(&fr_constraint::hcp::parse(&std::fs::read_to_string(example()).unwrap()).unwrap()).unwrap()).unwrap();
    pack["rules"][0].as_object_mut().unwrap().remove("evidence");
    std::fs::write(&bad, serde_json::to_string(&pack).unwrap()).unwrap();
    let ir_bad = d.join("ir-bad.json");
    assert_eq!(code(&run(&["constraints", "compile", &s(&bad), "-o", &s(&ir_bad)])), 3);
    assert!(!ir_bad.exists());
}
