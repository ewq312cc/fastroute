//! `skills/hardware-constraint-design/scripts/hcp_cli.py`: finding the two programs, negotiating their
//! versions, the failure branches (a program missing, too old, incompatible, no inventory) and the
//! one-run pipeline. Needs `python3`; the tests say so and pass without it.
#![cfg(unix)]

mod common;
use common::*;
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The interpreter, as an absolute path (the script runs with an empty PATH).
fn python() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join("python3")).find(|p| p.is_file())
}

fn have_python() -> bool {
    python().is_some()
}

fn skill() -> PathBuf {
    manifest_dir().join("../../skills/hardware-constraint-design")
}

fn example() -> PathBuf {
    skill().join("assets/example-device.yaml")
}

fn real_fastroute() -> String {
    env!("CARGO_BIN_EXE_fastroute").to_string()
}

/// The script as it would be installed away from the repository (so that it cannot fall back on the
/// repository's `target/`), and the assets it reads.
fn installed(name: &str) -> (PathBuf, PathBuf) {
    let d = tmp(name);
    for sub in ["scripts", "assets"] {
        std::fs::create_dir_all(d.join("skill").join(sub)).unwrap();
        for e in std::fs::read_dir(skill().join(sub)).unwrap() {
            let e = e.unwrap();
            std::fs::copy(e.path(), d.join("skill").join(sub).join(e.file_name())).unwrap();
        }
    }
    (d.join("skill/scripts/hcp_cli.py"), d)
}

fn fake(dir: &Path, name: &str, body: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p.display().to_string()
}

fn hcp(script: &Path, envs: &[(&str, &str)], args: &[&str]) -> Output {
    let mut c = Command::new(python().expect("python3"));
    c.arg("-I").arg(script).args(args).env_clear().env("PATH", "/nonexistent");
    for (k, v) in envs {
        c.env(k, v);
    }
    c.output().expect("run python3")
}

fn jv(o: &Output) -> Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{e}: {}\n{}", stdout(o), stderr(o)))
}

const OK_MANAGER: &str = r#"if [ "$1" = info ]; then echo '{"format":"hcp-manager-info","interface":"1","version":"9.9.9","capabilities":{"keyword_search":true,"vector_index":false,"pdf_text":true,"network_fetch":true}}'; fi"#;
const NEW_MANAGER: &str = r#"if [ "$1" = info ]; then echo '{"format":"hcp-manager-info","interface":"2","version":"9.9.9","capabilities":{}}'; fi"#;
const OLD_FASTROUTE: &str = r#"case "$1" in constraints) echo "check compile report";; *) echo "fastroute 0.0.1";; esac"#;

#[test]
fn doctor_says_what_was_found_and_what_still_works() {
    if !have_python() {
        eprintln!("python3 not found: skipped");
        return;
    }
    let (script, d) = installed("skill-doctor");
    let fr = real_fastroute();
    // fastroute alone: the local workflow works, research does not
    let o = hcp(&script, &[("FASTROUTE_BIN", &fr)], &["doctor"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    let v = jv(&o);
    assert_eq!((v["fastroute"]["ok"].as_bool(), v["hcp_manager"]["ok"].as_bool()), (Some(true), Some(false)));
    assert_eq!((v["workflows"]["local_pack_workflow"].as_bool(), v["workflows"]["document_research"].as_bool()), (Some(true), Some(false)));
    assert!(v["notes"].to_string().contains("hcp-manager is not available"), "{v}");
    // both, with a manager that speaks interface 1 and has no vector index
    let ok = fake(&d, "hcp-manager", OK_MANAGER);
    let o = hcp(&script, &[("FASTROUTE_BIN", &fr), ("HCP_MANAGER_BIN", &ok)], &["doctor"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = jv(&o);
    assert_eq!((v["hcp_manager"]["interface"].as_str(), v["workflows"]["keyword_search"].as_bool(), v["workflows"]["vector_search"].as_bool()), (Some("1"), Some(true), Some(false)));
    assert!(v["notes"].to_string().contains("no vector index"), "{v}");
    // a manager that speaks another interface is refused, with the reason
    let new = fake(&d, "hcp-manager-new", NEW_MANAGER);
    let o = hcp(&script, &[("FASTROUTE_BIN", &fr), ("HCP_MANAGER_BIN", &new)], &["doctor"]);
    assert_eq!(code(&o), 5);
    assert!(jv(&o)["hcp_manager"]["problems"].to_string().contains("speaks interface"), "{}", stdout(&o));
}

#[test]
fn a_missing_or_old_fastroute_stops_with_exit_6_and_says_how_to_get_one() {
    if !have_python() {
        return;
    }
    let (script, d) = installed("skill-no-fastroute");
    let old_one = fake(&d, "fastroute", OLD_FASTROUTE);
    for (what, envs) in [("not installed", vec![]), ("too old", vec![("FASTROUTE_BIN", old_one.as_str())])] {
        let o = hcp(&script, &envs, &["check", "pack.yaml"]);
        assert_eq!(code(&o), 6, "{what}: {}", stderr(&o));
        let e: Value = serde_json::from_str(&stderr(&o)).unwrap_or_else(|x| panic!("{x}: {}", stderr(&o)));
        assert_eq!((e["status"].as_str(), e["tool"].as_str()), (Some("tool_missing"), Some("fastroute")), "{what}");
        assert!(e["next"].as_str().unwrap().contains("cargo build"), "{what}");
    }
    let old = fake(&d, "fastroute-old", OLD_FASTROUTE);
    let o = hcp(&script, &[("FASTROUTE_BIN", &old)], &["doctor"]);
    assert_eq!(code(&o), 6);
    assert!(jv(&o)["fastroute"]["problems"].to_string().contains("too old"), "{}", stdout(&o));
}

#[test]
fn research_without_the_application_is_exit_6_and_the_local_workflow_is_named() {
    if !have_python() {
        return;
    }
    let (script, d) = installed("skill-no-manager");
    let fr = real_fastroute();
    let o = hcp(&script, &[("FASTROUTE_BIN", &fr)], &["research", "search", "reset", "--device", "FM-100"]);
    assert_eq!(code(&o), 6);
    let e: Value = serde_json::from_str(&stderr(&o)).unwrap();
    assert_eq!(e["tool"], "hcp-manager");
    assert!(e["next"].as_str().unwrap().contains("local workflow still works"), "{e}");
    // an incompatible one is the same branch
    let new = fake(&d, "hcp-manager", NEW_MANAGER);
    let o = hcp(&script, &[("FASTROUTE_BIN", &fr), ("HCP_MANAGER_BIN", &new)], &["research", "sources", "list"]);
    assert_eq!(code(&o), 6);
    assert!(stderr(&o).contains("speaks interface"), "{}", stderr(&o));
    // a command that is not one of the manager's is a usage error
    let ok = fake(&d, "hcp-manager-ok", OK_MANAGER);
    assert_eq!(code(&hcp(&script, &[("FASTROUTE_BIN", &fr), ("HCP_MANAGER_BIN", &ok)], &["research", "download-everything"])), 2);
}

#[test]
fn research_through_the_real_manager_recovers_from_an_empty_workspace() {
    if !have_python() {
        return;
    }
    let manager = manifest_dir().join("../../target/debug/hcp-manager");
    if !manager.is_file() {
        eprintln!("hcp-manager is not built: skipped");
        return;
    }
    let (script, d) = installed("skill-manager");
    let fr = real_fastroute();
    let m = manager.display().to_string();
    let ws = d.join("ws");
    let w = ws.display().to_string();
    let envs = [("FASTROUTE_BIN", fr.as_str()), ("HCP_MANAGER_BIN", m.as_str())];
    // no inventory yet: the manager says so (exit 3) and the script says what to do next
    let o = hcp(&script, &envs, &["--workspace", &w, "research", "search", "reset", "--device", "FM-100"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(stderr(&o).contains("research init") && stderr(&o).contains("sources import"), "{}", stderr(&o));
    // init, then the same question is answered from an inventory without documents: nothing found, no crash
    assert_eq!(code(&hcp(&script, &envs, &["--workspace", &w, "research", "init"])), 0);
    let o = hcp(&script, &envs, &["--workspace", &w, "research", "packs", "list", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(jv(&o)["packs"], json!([]));
    // the new commands go through the same front door: the embedding provider (local by default) and a block of the index
    let o = hcp(&script, &envs, &["--workspace", &w, "research", "embedding", "show", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!((jv(&o)["embedder"]["provider"].as_str(), jv(&o)["text_leaves_this_machine"].as_bool()), (Some("hashing"), Some(false)));
    let o = hcp(&script, &envs, &["--workspace", &w, "research", "chunk", "show", "CHK-NOPE@00000000:p1.1"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    // the doctor says what the vector layer is and is not
    let o = hcp(&script, &envs, &["doctor"]);
    let notes = jv(&o)["notes"].to_string();
    assert!(notes.contains("vector search is available") && notes.contains("lexical") && notes.contains("--authorize-remote"), "{notes}");
}

#[test]
fn the_rule_templates_are_listed_and_printed_from_the_skill() {
    if !have_python() {
        return;
    }
    let (script, _) = installed("skill-templates");
    let o = hcp(&script, &[], &["extract", "templates"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let list = stdout(&o);
    assert!(list.lines().count() >= 12, "{list}");
    for name in ["strapping_pin", "reset_pin", "boot_mode_pin", "differential_pair", "antenna_keepout"] {
        assert!(list.lines().any(|l| l.starts_with(&format!("{name}\t"))), "{name} in {list}");
    }
    let o = hcp(&script, &[], &["extract", "templates", "strapping_pin"]);
    assert_eq!(code(&o), 0);
    assert!(stdout(&o).contains("kind: strapping") && stdout(&o).contains("basis: explicit"), "{}", stdout(&o));
    assert_eq!(code(&hcp(&script, &[], &["extract", "templates", "nope"])), 2);
}

#[test]
fn the_pipeline_runs_from_pack_to_validation_and_leaves_the_open_questions_to_a_person() {
    if !have_python() {
        return;
    }
    let (script, d) = installed("skill-pipeline");
    let fr = real_fastroute();
    let board = manifest_dir().join("../fr-constraint/tests/data/fm100-demo.kicad_pcb");
    let out = d.join("out");
    let o = hcp(&script, &[("FASTROUTE_BIN", &fr)], &["pipeline", &s(&example()), "--out", &s(&out), "--kicad-pcb", &s(&board), "--export", "--lang", "en"]);
    assert_eq!(code(&o), 5, "{}{}", stdout(&o), stderr(&o));
    let st = json(&out.join("pipeline-status.json"));
    let status = |n: &str| st["steps"].as_array().unwrap().iter().find(|s| s["step"] == n).unwrap()["status"].as_str().unwrap().to_string();
    assert_eq!((status("check").as_str(), status("compile").as_str(), status("report").as_str()), ("ok", "ok", "ok"));
    assert_eq!((status("evidence").as_str(), status("validate").as_str(), status("export").as_str()), ("skipped", "needs_human", "ok"));
    let humans = st["needs_human"].to_string();
    for want in ["C-USB-GAP", "a draft", "not compared with the documents", "not checked or are unknown"] {
        assert!(humans.contains(want), "{want} in {humans}");
    }
    for f in ["check.json", "ir.json", "pack-report.md", "validation.json", "validation.md", "fastroute/export-report.json"] {
        assert!(out.join(f).is_file(), "{f}");
    }
    assert!(std::fs::read_to_string(out.join("pack-report.md")).unwrap().contains("Hardware"), "an English report was asked for");
    // the board was only read
    assert!(board.is_file());
}

#[test]
fn the_pipeline_stops_at_an_invalid_pack_and_reports_a_violated_rule() {
    if !have_python() {
        return;
    }
    let (script, d) = installed("skill-pipeline-fail");
    let fr = real_fastroute();
    // a pack with a rule that has no evidence
    let mut pack = fr_constraint::hcp::parse(&std::fs::read_to_string(example()).unwrap()).unwrap();
    pack["rules"][0].as_object_mut().unwrap().remove("evidence");
    let bad = d.join("bad.json");
    std::fs::write(&bad, serde_json::to_string(&pack).unwrap()).unwrap();
    let out = d.join("out-bad");
    let o = hcp(&script, &[("FASTROUTE_BIN", &fr)], &["pipeline", &s(&bad), "--out", &s(&out)]);
    assert_eq!(code(&o), 3, "{}{}", stdout(&o), stderr(&o));
    let st = json(&out.join("pipeline-status.json"));
    assert_eq!((st["steps"][0]["step"].as_str(), st["steps"][0]["status"].as_str(), st["steps"].as_array().unwrap().len()), (Some("check"), Some("failed"), 1));
    assert!(!out.join("ir.json").exists(), "nothing is compiled from an invalid pack");
    // a design that uses a reserved pin
    let design = d.join("design.json");
    std::fs::write(
        &design,
        serde_json::to_string(&json!({
            "snapshot_version": "1.0", "project": {"name": "bad-design"},
            "components": [{"ref": "U1", "device": "DEV-FM100", "value": "FM-100", "x": 30.0, "y": 28.0}],
            "pins": [{"ref": "U1", "pin": "10", "net": "FLASH_CS", "x": 32.0, "y": 30.0}, {"ref": "U1", "pin": "11", "net": "FLASH_CS", "x": 32.0, "y": 28.0}]
        }))
        .unwrap(),
    )
    .unwrap();
    let out = d.join("out-fail");
    let o = hcp(&script, &[("FASTROUTE_BIN", &fr)], &["pipeline", &s(&example()), "--out", &s(&out), "--design", &s(&design)]);
    assert_eq!(code(&o), 1, "{}{}", stdout(&o), stderr(&o));
    let st = json(&out.join("pipeline-status.json"));
    assert!(st["steps"].as_array().unwrap().iter().any(|s| s["step"] == "validate" && s["status"] == "failed"), "{st}");
    assert!(std::fs::read_to_string(out.join("validation.md")).unwrap().contains("C-RESERVED"));
    // both a snapshot and a board is a usage error
    assert_eq!(code(&hcp(&script, &[("FASTROUTE_BIN", &fr)], &["pipeline", &s(&example()), "--out", &s(&d.join("o3")), "--design", &s(&design), "--kicad-pcb", &s(&design)])), 2);
}

#[test]
fn the_packaged_skill_is_the_skill_directory() {
    if !have_python() {
        return;
    }
    let package = manifest_dir().join("../../skills/hardware-constraint-design.skill");
    assert!(package.is_file(), "skills/hardware-constraint-design.skill is missing: build it with skill-creator's `python -m scripts.package_skill skills/hardware-constraint-design skills/`");
    let check = r#"
import sys, zipfile, pathlib
pkg, root = sys.argv[1], pathlib.Path(sys.argv[2])
z = zipfile.ZipFile(pkg)
names = sorted(n for n in z.namelist() if not n.endswith('/'))
files = sorted(str(p.relative_to(root.parent)) for p in root.rglob('*') if p.is_file() and '__pycache__' not in p.parts and p.suffix != '.pyc' and p.name != '.DS_Store' and p.relative_to(root).parts[0] != 'evals')
if names != files:
    print('members differ:', sorted(set(names) ^ set(files))); sys.exit(1)
bad = [n for n in names if z.read(n) != (root.parent / n).read_bytes()]
if bad:
    print('out of date:', bad); sys.exit(2)
if sum(1 for n in names if n.endswith('/SKILL.md')) != 1:
    print('need exactly one SKILL.md'); sys.exit(3)
"#;
    let o = Command::new(python().unwrap()).args(["-I", "-c", check]).arg(&package).arg(skill()).output().unwrap();
    assert_eq!(code(&o), 0, "{} (build it again with skill-creator's `python -m scripts.package_skill skills/hardware-constraint-design skills/`)", stdout(&o));
}
