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
