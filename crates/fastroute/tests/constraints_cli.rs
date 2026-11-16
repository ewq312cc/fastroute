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
        ],
        "outline": [[0.0, 0.0], [60.0, 0.0], [60.0, 40.0], [0.0, 40.0]],
        "mount_holes": [{"x": 3.0, "y": 3.0, "radius_mm": 1.6}]
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

#[test]
fn report_writes_the_sections_in_the_requested_language_and_refuses_an_invalid_pack() {
    let d = tmp("constraints-report");
    let zh = d.join("report-zh.md");
    let en = d.join("report-en.md");
    assert_eq!(code(&run(&["constraints", "report", &s(&example()), "-o", &s(&zh), "--lang", "zh"])), 0);
    // without --lang the language of the pack is used (the example says `language: en`; a pack
    // without a language gets Chinese)
    assert_eq!(code(&run(&["constraints", "report", &s(&example()), "-o", &s(&en)])), 0);
    let text = std::fs::read_to_string(&zh).unwrap();
    assert!(text.contains("硬件约束知识包报告") && text.contains("## 7. 来源列表") && text.contains("C-RESERVED"), "{text}");
    assert!(std::fs::read_to_string(&en).unwrap().contains("## 1. What must not be changed (fixed)"));
    // standard output, usage, input and output problems
    let o = run(&["constraints", "report", &s(&example()), "-o", "-"]);
    assert_eq!(code(&o), 0);
    assert!(stdout(&o).starts_with("# Acme FM-100 example module (fictional)"));
    assert_eq!(code(&run(&["constraints", "report", &s(&example())])), 2);
    assert_eq!(code(&run(&["constraints", "report", &s(&example()), "-o", &s(&d.join("x.md")), "--lang", "fr"])), 2);
    assert_eq!(code(&run(&["constraints", "report", &s(&example()), "-o", &s(&zh), "--lang", "zh"])), 4);
    let mut no_lang: Value = serde_json::from_str(&serde_json::to_string(&fr_constraint::hcp::parse(&std::fs::read_to_string(example()).unwrap()).unwrap()).unwrap()).unwrap();
    no_lang["pack"].as_object_mut().unwrap().remove("language");
    let f = d.join("no-lang.json");
    std::fs::write(&f, serde_json::to_string(&no_lang).unwrap()).unwrap();
    let o = run(&["constraints", "report", &s(&f), "-o", "-"]);
    assert!(stdout(&o).contains("硬件约束知识包报告"), "a pack without a language gets a Chinese report");
    // an invalid pack has no report, and nothing is written
    let bad = d.join("bad.json");
    let mut pack: Value = serde_json::from_str(&serde_json::to_string(&fr_constraint::hcp::parse(&std::fs::read_to_string(example()).unwrap()).unwrap()).unwrap()).unwrap();
    pack["rules"][0].as_object_mut().unwrap().remove("evidence");
    std::fs::write(&bad, serde_json::to_string(&pack).unwrap()).unwrap();
    let out = d.join("bad.md");
    assert_eq!(code(&run(&["constraints", "report", &s(&bad), "-o", &s(&out)])), 3);
    assert!(!out.exists());
}

// ---- KiCad boards and the fastroute export ---------------------------------------------------

fn fm100_board() -> PathBuf {
    manifest_dir().join("../fr-constraint/tests/data/fm100-demo.kicad_pcb")
}

/// The board text with one line of the file changed (one pad / item per line in the fixture).
fn edited_board(dir: &std::path::Path, name: &str, f: impl FnOnce(String) -> String) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, f(std::fs::read_to_string(fm100_board()).unwrap())).unwrap();
    p
}

fn line_with<'a>(text: &'a str, needle: &str) -> &'a str {
    text.lines().find(|l| l.contains(needle)).unwrap_or_else(|| panic!("no line with {needle}"))
}

#[test]
fn extract_kicad_reads_a_board_without_touching_it() {
    let d = tmp("constraints-extract");
    let before = file_hash(&fm100_board());
    let out = d.join("design.json");
    let o = run(&["constraints", "extract-kicad", &s(&fm100_board()), "-o", &s(&out)]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stderr(&o).contains("8 components, 24 pads, 6 tracks, 1 vias"), "{}", stderr(&o));
    assert_eq!(file_hash(&fm100_board()), before, "the board is only read");
    // the snapshot is what the snapshot schema says it is
    let schema: Value = serde_json::from_str(&stdout(&run(&["constraints", "schema", "--snapshot"]))).unwrap();
    assert!(fr_constraint::jsonschema::validate(&schema, &json(&out)).is_empty());
    // the board is never an output, not even with --force; an existing output is refused
    assert_eq!(code(&run(&["constraints", "extract-kicad", &s(&fm100_board()), "-o", &s(&fm100_board()), "--force"])), 4);
    assert_eq!(code(&run(&["constraints", "extract-kicad", &s(&fm100_board()), "-o", &s(&out), "-q"])), 4);
    assert_eq!(code(&run(&["constraints", "extract-kicad"])), 2);
    let junk = d.join("junk.kicad_pcb");
    std::fs::write(&junk, "(kicad_sch (version 1))").unwrap();
    assert_eq!(code(&run(&["constraints", "extract-kicad", &s(&junk), "-o", &s(&d.join("j.json"))])), 3);
    assert_eq!(file_hash(&fm100_board()), before);
}

#[test]
fn a_real_board_is_validated_read_only_and_each_mistake_fails_at_its_rule() {
    let d = tmp("constraints-board");
    let before = file_hash(&fm100_board());
    // the legal board: no rule is violated; the pack has rules this version cannot check (exit 5)
    let o = run(&["constraints", "validate", "--pack", &s(&example()), "--kicad-pcb", &s(&fm100_board()), "--json", "-q"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!((v["result"].as_str(), v["summary"]["failed"].as_u64()), (Some("pass"), Some(0)));
    assert_eq!(v["design"]["name"], "fm100-demo");
    assert_eq!(v["design"]["sha256"].as_str().unwrap().len(), 64);
    // a capacitor that is too small for the supply block
    let small = edited_board(&d, "small-cap.kicad_pcb", |t| t.replace("\"100nF\"", "\"10nF\""));
    let o = run(&["constraints", "validate", "--pack", &s(&example()), "--kicad-pcb", &s(&small)]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(stderr(&o).contains("error[C-NEED-POWER]") && stderr(&o).contains("bypass"), "{}", stderr(&o));
    // a reserved pin of the module used: pad 10 put on the ground net
    let reserved = edited_board(&d, "reserved.kicad_pcb", |t| {
        let line = line_with(&t, "(pad \"10\" smd").to_string();
        t.replace(&line, &line.replacen("(layers \"F.Cu\" \"F.Paste\" \"F.Mask\") ", "(layers \"F.Cu\" \"F.Paste\" \"F.Mask\") (net 1 \"GND\") ", 1))
    });
    let o = run(&["constraints", "validate", "--pack", &s(&example()), "--kicad-pcb", &s(&reserved), "--json", "-q"]);
    assert_eq!(code(&o), 1);
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    let f = v["findings"].as_array().unwrap().iter().find(|f| f["rule_id"] == "C-RESERVED" && f["verdict"] == "fail").unwrap();
    assert_eq!(f["object"], "U1.10");
    assert_eq!(f["evidence"][0]["quote"], "RSVD1 and RSVD2 are connected to the flash inside the module; do not use.");
    // copper in the antenna keep-out
    let keepout = edited_board(&d, "keepout.kicad_pcb", |t| t.replacen("\n)", "\n  (segment (start 45 5) (end 55 5) (width 0.2) (layer \"F.Cu\") (net 5 \"LED_DRIVE\") (uuid \"00000000-0000-0000-0000-0000000000aa\"))\n)", 1).trim_end().to_string() + "\n");
    let o = run(&["constraints", "validate", "--pack", &s(&example()), "--kicad-pcb", &s(&keepout)]);
    assert_eq!(code(&o), 1, "{}", stderr(&o));
    assert!(stderr(&o).contains("error[C-ANT]") && stderr(&o).contains("track on net LED_DRIVE"), "{}", stderr(&o));
    // the module's ground pin on another net
    let wrong_net = edited_board(&d, "wrongnet.kicad_pcb", |t| {
        let line = line_with(&t, "(pad \"1\" smd rect (at -6.0").to_string();
        t.replace(&line, &line.replace("(net 1 \"GND\")", "(net 2 \"3V3\")"))
    });
    let o = run(&["constraints", "validate", "--pack", &s(&example()), "--kicad-pcb", &s(&wrong_net)]);
    assert_eq!(code(&o), 1);
    assert!(stderr(&o).contains("error[C-GND]") && stderr(&o).contains("U1.1"), "{}", stderr(&o));
    // none of this touched the original board; a snapshot and a board together are a usage error
    assert_eq!(file_hash(&fm100_board()), before);
    assert_eq!(code(&run(&["constraints", "validate", "--pack", &s(&example()), "--kicad-pcb", &s(&fm100_board()), "--design", &s(&fm100_board())])), 2);
    // the board is never the output of a validation
    assert_eq!(code(&run(&["constraints", "validate", "--pack", &s(&example()), "--kicad-pcb", &s(&fm100_board()), "-o", &s(&fm100_board()), "--force", "-q"])), 4);
}

#[test]
fn export_fastroute_writes_the_inputs_and_the_pairs_file_is_accepted_by_the_router() {
    let d = tmp("constraints-export");
    // a pack whose USB pair is the pair of the demo design
    let mut pack: Value = serde_json::from_str(&serde_json::to_string(&fr_constraint::hcp::parse(&std::fs::read_to_string(example()).unwrap()).unwrap()).unwrap()).unwrap();
    for r in pack["rules"].as_array_mut().unwrap() {
        if r["id"] == "C-USB" {
            r["params"]["nets"] = json!(["USB_DP", "USB_DN"]);
            r["params"]["gap_mm"] = json!(0.2);
        }
    }
    let pack_file = d.join("pack.json");
    std::fs::write(&pack_file, serde_json::to_string(&pack).unwrap()).unwrap();
    let out = d.join("export");
    let o = run(&["constraints", "export-fastroute", "--pack", &s(&pack_file), "-o", &s(&out), "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let pairs = std::fs::read_to_string(out.join("pairs.txt")).unwrap();
    assert!(pairs.contains("pair USB_DP USB_DN gap=0.2 skew=0.5"), "{pairs}");
    assert_eq!(std::fs::read_to_string(out.join("route-args.txt")).unwrap(), "--pairs=pairs.txt\n");
    let report = json(&out.join("export-report.json"));
    assert_eq!(report["format"], "fastroute-hcp-export");
    assert!(report["not_exported"].to_string().contains("C-ANT"));
    // the router reads what was written and routes the pair
    let routed = d.join("usb.ses");
    let dsn = manifest_dir().join("../../docs/examples/rules/usb_demo.dsn");
    let o = run(&["route", &s(&dsn), "--pairs", &s(&out.join("pairs.txt")), "-o", &s(&routed), "--allow-partial", "--max-time", "20"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(routed.is_file());
    assert!(stderr(&o).to_lowercase().contains("pair"), "the router should say it routed a pair: {}", stderr(&o));
    // nothing is overwritten unless asked, and nothing is written if one file would be refused
    assert_eq!(code(&run(&["constraints", "export-fastroute", "--pack", &s(&pack_file), "-o", &s(&out), "-q"])), 4);
    assert_eq!(code(&run(&["constraints", "export-fastroute", "--pack", &s(&pack_file), "-o", &s(&out), "-q", "--force"])), 0);
    std::fs::write(out.join("pairs.txt"), "keep").unwrap();
    std::fs::remove_file(out.join("export-report.json")).unwrap();
    assert_eq!(code(&run(&["constraints", "export-fastroute", "--pack", &s(&pack_file), "-o", &s(&out), "-q"])), 4);
    assert!(!out.join("export-report.json").exists(), "nothing was written: one file was refused");
    // usage and input
    assert_eq!(code(&run(&["constraints", "export-fastroute", "--pack", &s(&pack_file)])), 2);
    assert_eq!(code(&run(&["constraints", "export-fastroute", "-o", &s(&out)])), 2);
}

// ---- a fixed base design and the regions an AI may edit -----------------------------------------------

/// A working copy of the base design, a pack bound to exactly that file, and the design snapshot.
struct EditEnv {
    dir: PathBuf,
    base_dir: PathBuf,
    pack: PathBuf,
    design: PathBuf,
}

fn edit_env(name: &str) -> EditEnv {
    let dir = tmp(name);
    let base_dir = dir.join("base");
    std::fs::create_dir_all(&base_dir).unwrap();
    std::fs::write(base_dir.join("carrier.kicad_pcb"), "(kicad_pcb (version 20240108))\n").unwrap();
    let digest = fr_constraint::hcp::sha256_hex(&std::fs::read(base_dir.join("carrier.kicad_pcb")).unwrap());
    let mut pack = fr_constraint::hcp::parse(&std::fs::read_to_string(example()).unwrap()).unwrap();
    pack["base_design"]["binding"]["files"][0]["sha256"] = json!(digest);
    let pack_path = dir.join("pack.json");
    std::fs::write(&pack_path, serde_json::to_string_pretty(&pack).unwrap()).unwrap();
    let design = legal_design(&dir);
    EditEnv { dir, base_dir, pack: pack_path, design }
}

fn make_draft(env: &EditEnv, changes: Value) -> PathBuf {
    let req = env.dir.join("request.json");
    if !req.exists() {
        let o = run(&["constraints", "edit-request", "--pack", &s(&env.pack), "--requirement", "add a status LED", "-o", &s(&req), "-q"]);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
    }
    let r = json(&req);
    let d = json!({
        "format": "fastroute-hcp-edit-draft", "draft_version": "1.0",
        "pack": {"id": r["pack"]["id"], "version": r["pack"]["version"], "sha256": r["pack"]["sha256"]},
        "base": {"id": r["base"]["id"], "binding_sha256": r["base"]["binding_sha256"]},
        "changes": changes
    });
    let f = env.dir.join("draft.json");
    std::fs::write(&f, serde_json::to_string_pretty(&d).unwrap()).unwrap();
    f
}

fn led() -> Value {
    json!([
        {"id": "CH-1", "op": "add_component", "region": "REG-USER", "block": "BLK-LED", "role": "led", "ref": "D1", "value": "LED", "at": [10.0, 20.0]},
        {"id": "CH-2", "op": "connect", "region": "REG-USER", "ref": "U1", "pin": "5", "net": "LED_DRIVE"},
        {"id": "CH-3", "op": "connect", "region": "REG-USER", "ref": "D1", "pin": "1", "net": "LED_DRIVE"}
    ])
}

#[test]
fn edit_request_is_deterministic_and_the_schemas_are_the_published_files() {
    let env = edit_env("constraints-edit-request");
    let (a, b) = (env.dir.join("a.json"), env.dir.join("b.json"));
    for f in [&a, &b] {
        assert_eq!(code(&run(&["constraints", "edit-request", "--pack", &s(&env.pack), "--requirement", "add a status LED", "-o", &s(f), "-q"])), 0);
    }
    assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    let req = json(&a);
    assert_eq!(req["format"], "fastroute-hcp-edit-request");
    assert_eq!(req["authorization"]["regions"][0]["id"], "REG-USER");
    assert_eq!(req["authorization"]["regions"][0]["blocks"][0]["id"], "BLK-LED");
    assert_eq!(req["authorization"]["regions"][0]["pin_pool"][0]["pins"], json!(["5"]));
    assert!(fr_constraint::jsonschema::validate(&fr_constraint::edit::request_schema(), &req).is_empty());
    for (flag, file) in [("--edit-draft", "edit-draft-v1.schema.json"), ("--edit-request", "edit-request-v1.schema.json")] {
        let o = run(&["constraints", "schema", flag]);
        assert_eq!(code(&o), 0);
        assert_eq!(stdout(&o), std::fs::read_to_string(skill().join("schemas").join(file)).unwrap());
    }
    assert_eq!(code(&run(&["constraints", "schema", "--ir", "--edit-draft"])), 2);
    // a pack without a base design has nothing to hand out
    let mut pack = json(&env.pack);
    pack.as_object_mut().unwrap().remove("base_design");
    pack["regions"].as_array_mut().unwrap().retain(|g| g["id"] != "REG-USER");
    pack["peripheral_blocks"].as_array_mut().unwrap().retain(|b| b["id"] != "BLK-LED");
    let p = env.dir.join("no-base.json");
    std::fs::write(&p, serde_json::to_string(&pack).unwrap()).unwrap();
    let o = run(&["constraints", "edit-request", "--pack", &s(&p), "-o", &s(&env.dir.join("none.json"))]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(stderr(&o).contains("no base_design"), "{}", stderr(&o));
}

#[test]
fn check_edit_accepts_a_draft_inside_the_whitelist_and_never_touches_the_base() {
    let env = edit_env("constraints-edit-ok");
    let draft = make_draft(&env, led());
    let before = std::fs::read(env.base_dir.join("carrier.kicad_pcb")).unwrap();
    let (out, md) = (env.dir.join("check.json"), env.dir.join("diff.md"));
    let o = run(&["constraints", "check-edit", &s(&draft), "--pack", &s(&env.pack), "--design", &s(&env.design), "--base-dir", &s(&env.base_dir), "-o", &s(&out), "--report", &s(&md), "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = json(&out);
    assert_eq!((v["status"].as_str(), v["complete"].as_bool(), v["base"]["verified"].as_bool()), (Some("accepted"), Some(true), Some(true)));
    assert_eq!(v["diff"]["components"]["added"][0]["ref"], "D1");
    let text = std::fs::read_to_string(&md).unwrap();
    assert!(text.contains("AI 修改草案校验报告") && text.contains("D1") && text.contains("BLK-LED"), "{text}");
    // the base design is exactly what it was
    assert_eq!(std::fs::read(env.base_dir.join("carrier.kicad_pcb")).unwrap(), before);
    assert_eq!(std::fs::read_dir(&env.base_dir).unwrap().count(), 1);
    // an English report on request
    let en = env.dir.join("diff-en.md");
    assert_eq!(code(&run(&["constraints", "check-edit", &s(&draft), "--pack", &s(&env.pack), "--design", &s(&env.design), "--base-dir", &s(&env.base_dir), "--report", &s(&en), "--lang", "en", "-q"])), 0);
    assert!(std::fs::read_to_string(&en).unwrap().contains("AI edit draft check report"));
}

#[test]
fn check_edit_exits_5_when_the_base_changed_or_could_not_be_compared() {
    let env = edit_env("constraints-edit-review");
    let draft = make_draft(&env, led());
    // no comparison of the base files: accepted, but not complete
    let o = run(&["constraints", "check-edit", &s(&draft), "--pack", &s(&env.pack), "--design", &s(&env.design), "-q"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    // the open board was updated since the pack was made
    std::fs::write(env.base_dir.join("carrier.kicad_pcb"), "(kicad_pcb (version 20240108) (generator newer))\n").unwrap();
    let out = env.dir.join("review.json");
    let o = run(&["constraints", "check-edit", &s(&draft), "--pack", &s(&env.pack), "--design", &s(&env.design), "--base-dir", &s(&env.base_dir), "-o", &s(&out)]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(stderr(&o).contains("base_changed") && stderr(&o).contains("carrier.kicad_pcb"), "{}", stderr(&o));
    assert_eq!(json(&out)["status"], "needs_review");
    // a draft made for another version of the pack
    let mut d = json(&draft);
    d["pack"]["sha256"] = json!("a".repeat(64));
    let old = env.dir.join("old-draft.json");
    std::fs::write(&old, serde_json::to_string(&d).unwrap()).unwrap();
    let o = run(&["constraints", "check-edit", &s(&old), "--pack", &s(&env.pack), "--design", &s(&env.design), "-q"]);
    assert_eq!(code(&o), 5);
    // the generator stopped
    let mut d = json(&draft);
    d["changes"] = json!([]);
    d["stop"] = json!({"reason": "no block provides 5 V", "missing": ["a boost block"]});
    let stop = env.dir.join("stop.json");
    std::fs::write(&stop, serde_json::to_string(&d).unwrap()).unwrap();
    let o = run(&["constraints", "check-edit", &s(&stop), "--pack", &s(&env.pack), "--design", &s(&env.design), "-q", "--json"]);
    assert_eq!(code(&o), 5);
    assert_eq!(serde_json::from_slice::<Value>(&o.stdout).unwrap()["status"], "stopped");
}

#[test]
fn check_edit_exits_1_for_every_kind_of_overreach_and_names_the_rule() {
    let env = edit_env("constraints-edit-reject");
    let cases: Vec<(&str, Value, &str)> = vec![
        ("an unauthorised block found by a search", json!([{"id": "CH-1", "op": "add_component", "region": "REG-USER", "block": "BLK-RESET", "role": "pull_up", "ref": "R9", "at": [10.0, 20.0]}]), "unauthorized_block"),
        ("a reserved pin", json!([{"id": "CH-1", "op": "connect", "region": "REG-USER", "ref": "U1", "pin": "10", "net": "X"}]), "reserved_pin_used"),
        ("a track out of the region", json!([{"id": "CH-1", "op": "add_track", "region": "REG-USER", "net": "X", "layer": "F.Cu", "width_mm": 0.25, "points": [[10.0, 20.0], [45.0, 20.0]]}]), "crosses_region"),
        ("a fixed region", json!([{"id": "CH-1", "op": "add_component", "region": "REG-ANT", "block": "BLK-LED", "role": "led", "ref": "D1", "at": [50.0, 5.0]}]), "region_not_editable"),
        ("moving the module", json!([{"id": "CH-1", "op": "move_component", "region": "REG-USER", "ref": "U1", "to": [20.0, 20.0]}]), "op_not_allowed"),
    ];
    for (what, changes, expected) in cases {
        let draft = make_draft(&env, changes);
        let out = env.dir.join("reject.json");
        let o = run(&["constraints", "check-edit", &s(&draft), "--pack", &s(&env.pack), "--design", &s(&env.design), "--base-dir", &s(&env.base_dir), "-o", &s(&out), "--force"]);
        assert_eq!(code(&o), 1, "{what}: {}", stderr(&o));
        assert!(stderr(&o).contains(expected), "{what}: {}", stderr(&o));
        let v = json(&out);
        assert_eq!(v["status"], "rejected", "{what}");
        assert!(v["findings"].as_array().unwrap().iter().any(|f| f["code"] == expected), "{what}: {v}");
    }
}

#[test]
fn check_edit_refuses_bad_inputs_and_outputs_before_writing_anything() {
    let env = edit_env("constraints-edit-io");
    let draft = make_draft(&env, led());
    let base = |extra: &[&str]| -> Vec<String> {
        let mut a: Vec<String> = ["constraints", "check-edit"].iter().map(|x| x.to_string()).collect();
        a.push(s(&draft));
        a.extend(["--pack", &s(&env.pack), "--design", &s(&env.design), "--base-dir", &s(&env.base_dir), "-q"].iter().map(|x| x.to_string()));
        a.extend(extra.iter().map(|x| x.to_string()));
        a
    };
    let go = |extra: &[&str]| run(&base(extra).iter().map(String::as_str).collect::<Vec<_>>());
    // the base design is never an output: not a bound file, not a file in its directory
    let o = go(&["--report", &s(&env.base_dir.join("carrier.kicad_pcb")), "--force"]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    let o = go(&["-o", &s(&env.base_dir.join("diff.json"))]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(stderr(&o).contains("never written"), "{}", stderr(&o));
    assert!(!env.base_dir.join("diff.json").exists());
    // nor the draft, the design or the pack
    assert_eq!(code(&go(&["-o", &s(&draft), "--force"])), 4);
    assert_eq!(code(&go(&["--report", &s(&env.design), "--force"])), 4);
    // a draft that is not a draft: exit 3, with the schema problems on standard error
    let mut d = json(&draft);
    d["changes"][0]["op"] = json!("rewrite_board");
    let bad = env.dir.join("bad.json");
    std::fs::write(&bad, serde_json::to_string(&d).unwrap()).unwrap();
    let o = run(&["constraints", "check-edit", &s(&bad), "--pack", &s(&env.pack), "--design", &s(&env.design)]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(stderr(&o).contains("error[schema]"), "{}", stderr(&o));
    assert_eq!(code(&run(&["constraints", "check-edit", "/nonexistent/draft.json", "--pack", &s(&env.pack)])), 3);
    // usage
    assert_eq!(code(&run(&["constraints", "check-edit", "--pack", &s(&env.pack)])), 2);
    assert_eq!(code(&run(&["constraints", "check-edit", &s(&draft)])), 2);
    assert_eq!(code(&run(&["constraints", "check-edit", &s(&draft), "--pack", &s(&env.pack), "--ir", &s(&env.pack)])), 2);
    assert_eq!(code(&run(&["constraints", "check-edit", &s(&draft), "--pack", &s(&env.pack), "--design", &s(&env.design), "--kicad-pcb", &s(&env.design)])), 2);
    assert_eq!(code(&run(&["constraints", "check-edit", "--help"])), 0);
    assert_eq!(code(&run(&["help", "constraints", "edit-request"])), 0);
    // a pack that binds a file outside the working copy is refused
    let mut pack = json(&env.pack);
    pack["base_design"]["binding"]["files"][0]["path"] = json!("../outside.kicad_pcb");
    let p = env.dir.join("escape.json");
    std::fs::write(&p, serde_json::to_string(&pack).unwrap()).unwrap();
    let o = run(&["constraints", "check-edit", &s(&draft), "--pack", &s(&p), "--design", &s(&env.design), "--base-dir", &s(&env.base_dir)]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
}

#[test]
fn check_edit_reads_a_kicad_board_as_the_base_and_compares_its_digest_with_the_pack() {
    let env = edit_env("constraints-edit-kicad");
    let board = skill().join("../../crates/fr-constraint/tests/data/fm100-demo.kicad_pcb");
    let before = std::fs::read(&board).unwrap();
    let digest = fr_constraint::hcp::sha256_hex(&before);
    let bind = |sha: &str, name: &str| {
        let mut pack = json(&env.pack);
        pack["base_design"]["binding"]["files"][0] = json!({"path": "fm100-demo.kicad_pcb", "sha256": sha});
        let p = env.dir.join(name);
        std::fs::write(&p, serde_json::to_string(&pack).unwrap()).unwrap();
        p
    };
    let bound = bind(&digest, "bound.json");
    let other = bind(&"e".repeat(64), "other.json");
    let args = |pack: &std::path::Path| {
        let o = run(&["constraints", "edit-request", "--pack", &s(pack), "-o", &s(&env.dir.join(format!("{}.req.json", pack.file_stem().unwrap().to_string_lossy()))), "-q"]);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        let r = json(&env.dir.join(format!("{}.req.json", pack.file_stem().unwrap().to_string_lossy())));
        let d = json!({
            "format": "fastroute-hcp-edit-draft", "draft_version": "1.0",
            "pack": {"id": r["pack"]["id"], "version": r["pack"]["version"], "sha256": r["pack"]["sha256"]},
            "base": {"id": r["base"]["id"], "binding_sha256": r["base"]["binding_sha256"]},
            "changes": led()
        });
        let f = env.dir.join(format!("{}.draft.json", pack.file_stem().unwrap().to_string_lossy()));
        std::fs::write(&f, serde_json::to_string(&d).unwrap()).unwrap();
        f
    };
    // the board is the file the pack was made for: accepted and fully checked, the digest came from the board itself
    let d = args(&bound);
    let o = run(&["constraints", "check-edit", &s(&d), "--pack", &s(&bound), "--kicad-pcb", &s(&board), "--json", "-q"]);
    let v: Value = serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{e}: {}", stderr(&o)));
    assert_eq!((code(&o), v["status"].as_str(), v["base"]["verified"].as_bool()), (0, Some("accepted"), Some(true)), "{}", stderr(&o));
    // the board is not the one the pack binds
    let d = args(&other);
    let o = run(&["constraints", "check-edit", &s(&d), "--pack", &s(&other), "--kicad-pcb", &s(&board), "-q"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    // the board was only read
    assert_eq!(std::fs::read(&board).unwrap(), before);
}
