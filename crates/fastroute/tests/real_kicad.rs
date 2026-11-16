//! The KiCad flows against a real KiCad: `route board.kicad_pcb` (KiCad's Python, headless),
//! `ai-pack --kicad-pcb` (KiCad's own `kicad-cli pcb drc`) and the coordinates of the DRC report.
//!
//! These need an installed KiCad and one of its demo boards, so each test says why it did not run
//! and passes when there is none (CI machines usually have no KiCad). Point them at a KiCad with
//! `FASTROUTE_KICAD_PYTHON`, `FASTROUTE_KICAD_CLI` and `FASTROUTE_KICAD_DEMO` (a `.kicad_pcb`
//! that has pads and a GND zone, such as KiCad's `pic_programmer`). What was seen with real
//! KiCads is in docs/CLI.md ("Checked against a real KiCad").

mod common;
use common::*;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

struct Kicad {
    python: PathBuf,
    cli: PathBuf,
    major: u32,
    demo: PathBuf,
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(name)).find(|f| f.is_file()))
}

fn kicad() -> Option<Kicad> {
    let python = std::env::var_os("FASTROUTE_KICAD_PYTHON").map(PathBuf::from).or_else(|| on_path("python3"))?;
    let imports = Command::new(&python).args(["-I", "-c", "import pcbnew"]).output().ok()?.status.success();
    if !imports {
        eprintln!("skipped: {} cannot import pcbnew", python.display());
        return None;
    }
    let cli = std::env::var_os("FASTROUTE_KICAD_CLI").map(PathBuf::from).or_else(|| on_path("kicad-cli"))?;
    let version = Command::new(&cli).arg("version").output().ok()?;
    let major: u32 = String::from_utf8_lossy(&version.stdout).trim().split('.').next()?.parse().ok()?;
    let demo = std::env::var_os("FASTROUTE_KICAD_DEMO")
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from("/usr/share/kicad/demos/pic_programmer/pic_programmer.kicad_pcb")).filter(|p| p.is_file()))?;
    Some(Kicad { python, cli, major, demo })
}

fn skip(why: &str) {
    eprintln!("skipped (no KiCad to test against): {why}");
}

fn hidden_files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with('.')).collect()
}

#[test]
fn a_kicad_board_is_routed_headless_or_refused_up_front_with_the_reason() {
    let Some(k) = kicad() else { return skip("kicad-cli / pcbnew / a demo board") };
    let d = tmp("real-route");
    let board = d.join("board.kicad_pcb");
    std::fs::copy(&k.demo, &board).unwrap();
    let o = run(&["route", &s(&board), "--python", &s(&k.python), "--max-time", "60", "--work-dir", &s(&d.join("work")), "--json", "-q"]);
    if k.major < 8 {
        // KiCad 7: ImportSpecctraSES works on the board of the PCB editor only
        assert_eq!(code(&o), 3, "{}", stderr(&o));
        assert!(stderr(&o).contains("KiCad 8 or newer"), "{}", stderr(&o));
        assert!(!d.join("board.routed.kicad_pcb").exists(), "a refused route must write nothing");
        return;
    }
    let v: Value = serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("no summary ({e}): {} {}", stdout(&o), stderr(&o)));
    assert!(matches!(code(&o), 0 | 5), "{}", stderr(&o));
    assert!(d.join("board.routed.kicad_pcb").is_file(), "{v}");
    // KiCad writes a project and its local settings next to what it saves: none may be left behind
    assert_eq!(hidden_files(&d), Vec::<String>::new());
    // the verdict is the adapter's (KiCad's own count of open connections), never an unknown one
    assert!(matches!(v["status"].as_str(), Some("complete") | Some("partial")), "{v}");
    assert_eq!(v["status"] == "complete", code(&o) == 0, "{v}");
    assert!(v["unrouted"].is_u64() && v["violations"].is_u64(), "the router's score line was not understood: {v}");
    // the input was never touched
    assert_eq!(file_hash(&board), file_hash(&k.demo));
}

#[test]
fn the_real_drc_is_run_read_and_located_in_the_frame_of_inspect() {
    let Some(k) = kicad() else { return skip("kicad-cli / pcbnew / a demo board") };
    let d = tmp("real-drc");
    let board = d.join("board.kicad_pcb");
    std::fs::copy(&k.demo, &board).unwrap();
    let work = d.join("work");
    let routed = d.join("board.routed.kicad_pcb");
    if k.major < 8 {
        // no `pcb drc` in KiCad 7: the pack says so and never says "passed"
        let dsn = d.join("export");
        let o = Command::new(&k.python).args([&s(&manifest_dir().join("../../integrations/kicad/plugins/route_cli.py")), &s(&board), "--export-only", "--work-dir", &s(&dsn), "--fastroute", env!("CARGO_BIN_EXE_fastroute")]).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let pack = d.join("pack");
        let o = run(&["ai-pack", "-i", &s(&dsn.join("board.dsn")), "-o", &s(&pack), "--kicad-pcb", &s(&board), "--kicad-cli", &s(&k.cli)]);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        let m = json(&pack.join("manifest.json"));
        assert_eq!((m["status"].as_str(), m["drc_status"].as_str()), (Some("partial"), Some("unavailable")), "{m}");
        return;
    }
    // 1. route (the work directory keeps the DSN that KiCad itself wrote and the session)
    let o = run(&["route", &s(&board), "--python", &s(&k.python), "--work-dir", &s(&work), "--max-time", "60", "-q", "--allow-partial"]);
    assert!(matches!(code(&o), 0 | 5), "{}", stderr(&o));
    assert!(routed.is_file());
    let dsn = work.join("board.dsn");
    let ses = std::fs::read_dir(&work).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "ses")).expect("the work directory keeps the session");

    // 2. ai-pack with the real kicad-cli on the routed board: a real report, parsed
    let pack = d.join("pack");
    let o = run(&["ai-pack", "-i", &s(&dsn), "-s", &s(&ses), "-o", &s(&pack), "--kicad-pcb", &s(&routed), "--kicad-cli", &s(&k.cli)]);
    assert!(matches!(code(&o), 0 | 5), "{}", stderr(&o));
    let m = json(&pack.join("manifest.json"));
    let status = m["drc_status"].as_str().unwrap_or("");
    assert!(status == "passed" || status == "failed", "a real kicad-cli gives a real result, got {status}: {m}");
    let inspect = json(&pack.join("inspect.json"));
    assert_eq!(inspect["checks"]["drc"]["status"], status);
    assert!(inspect["checks"]["drc"]["tool_version"].as_str().is_some_and(|v| v.starts_with(&k.major.to_string())));
    assert_eq!(code(&run(&["ai-pack", "--verify", &s(&pack)])), 0);

    // 3. the report's coordinates are those of inspect (mm, y down, the same origin), also for a
    // board whose user origin is not the page origin: every pad the report names is where inspect
    // has it. (The unrouted board: its report names pads.)
    let report = d.join("orig-drc.json");
    let o = Command::new(&k.cli).args(["pcb", "drc", "--format", "json", "--units", "mm", "--severity-all", "--output", &s(&report), &s(&board)]).output().unwrap();
    assert!(report.is_file(), "{}", String::from_utf8_lossy(&o.stderr));
    let insp = d.join("orig-inspect.json");
    let o = run(&["inspect", "-i", &s(&dsn), "-o", &s(&insp), "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let (report, insp) = (json(&report), json(&insp));
    let pads: Vec<(String, String, f64, f64)> = insp["pads"].as_array().unwrap().iter().map(|p| (p["component"].as_str().unwrap().into(), p["pin"].as_str().unwrap().into(), p["x"].as_f64().unwrap(), p["y"].as_f64().unwrap())).collect();
    let (mut compared, mut worst) = (0, 0.0f64);
    for key in ["violations", "unconnected_items"] {
        for v in report[key].as_array().unwrap() {
            for it in v["items"].as_array().unwrap() {
                // "PTH pad 8 [GND] of P3" / "pad 3 of U1"
                let text = it["description"].as_str().unwrap();
                let Some(rest) = text.split("pad ").nth(1) else { continue };
                let (Some(pin), Some(comp)) = (rest.split_whitespace().next(), text.rsplit(" of ").next()) else { continue };
                let Some(p) = pads.iter().find(|p| p.0 == comp && p.1 == pin) else { continue };
                let (dx, dy) = (it["pos"]["x"].as_f64().unwrap() - p.2, it["pos"]["y"].as_f64().unwrap() - p.3);
                worst = worst.max((dx * dx + dy * dy).sqrt());
                compared += 1;
            }
        }
    }
    assert!(compared > 0, "the demo board's report names no pad: pick another FASTROUTE_KICAD_DEMO");
    assert!(worst < 0.01, "DRC items are {worst} mm off the pads of inspect.json ({compared} compared)");
}

/// Violations of `kind` (`clearance`, `track_width`) that KiCad's own DRC finds on `routed`, checked
/// against the rules in `dru`; only those that name the net `[net]` if one is given.
fn drc_violations(k: &Kicad, routed: &Path, dru: &str, kind: &str, net: Option<&str>) -> usize {
    let d = routed.parent().unwrap();
    let board = d.join("check.kicad_pcb");
    std::fs::copy(routed, &board).unwrap();
    std::fs::write(d.join("check.kicad_dru"), dru).unwrap();
    let report = d.join("check-drc.json");
    let _ = std::fs::remove_file(&report);
    let o = Command::new(&k.cli).args(["pcb", "drc", "--format", "json", "--units", "mm", "--severity-all", "--output", &s(&report), &s(&board)]).output().unwrap();
    assert!(report.is_file(), "{}", String::from_utf8_lossy(&o.stderr));
    let tag = net.map(|n| format!("[{n}]"));
    json(&report)["violations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["type"] == kind)
        .filter(|v| tag.as_ref().is_none_or(|t| v["items"].as_array().unwrap().iter().any(|i| i["description"].as_str().unwrap_or("").contains(t.as_str()))))
        .count()
}

/// Routes a copy of the demo board with `dru` next to it (no rules if empty); returns the routed board.
fn route_with(k: &Kicad, dir: &Path, dru: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let board = dir.join("b.kicad_pcb");
    std::fs::copy(&k.demo, &board).unwrap();
    if !dru.is_empty() {
        std::fs::write(dir.join("b.kicad_dru"), dru).unwrap();
    }
    let o = run(&["route", &s(&board), "--python", &s(&k.python), "--clear", "--max-time", "60", "-q", "--allow-partial", "-o", &s(&dir.join("routed.kicad_pcb"))]);
    assert!(matches!(code(&o), 0 | 5), "{}", stderr(&o));
    assert!(!stderr(&o).contains("NOT applied"), "the rule must be carried over: {}", stderr(&o));
    dir.join("routed.kicad_pcb")
}

#[test]
fn a_kicad_dru_clearance_rule_reaches_the_router_and_the_real_drc_agrees() {
    let Some(k) = kicad() else { return skip("kicad-cli / pcbnew / a demo board") };
    if k.major < 8 {
        return skip("KiCad 7 cannot route headless (see the first test)");
    }
    // a rule for every pair of net classes (what `fastroute rules apply` writes for "all nets")
    let dru = "(version 1)\n(rule \"all\" (constraint clearance (min 0.3mm)))\n";
    let d = tmp("real-clearance");
    let without = drc_violations(&k, &route_with(&k, &d.join("without"), ""), dru, "clearance", None);
    let with = drc_violations(&k, &route_with(&k, &d.join("with"), dru), dru, "clearance", None);
    assert!(without > 0, "the demo board should break a 0.3 mm rule that the router did not see ({without})");
    assert!(with < without, "the router did not follow the rule: {with} clearance violations with it, {without} without");
}

#[test]
fn a_kicad_dru_track_width_rule_for_one_net_reaches_the_router_and_the_real_drc_agrees() {
    let Some(k) = kicad() else { return skip("kicad-cli / pcbnew / a demo board") };
    if k.major < 8 {
        return skip("KiCad 7 cannot route headless (see the first test)");
    }
    let d = tmp("real-width");
    let plain = route_with(&k, &d.join("without"), "");
    // the net with the most tracks in the plain result
    let script = "import sys, pcbnew, collections\nc = collections.Counter(t.GetNetname() for t in pcbnew.LoadBoard(sys.argv[1]).GetTracks() if \"'\" not in t.GetNetname())\nprint(c.most_common(1)[0][0])\n";
    let o = Command::new(&k.python).args(["-I", "-c", script, &s(&plain)]).output().unwrap();
    let net = String::from_utf8_lossy(&o.stdout).trim().to_string();
    assert!(!net.is_empty(), "{}", String::from_utf8_lossy(&o.stderr));
    let dru = format!("(version 1)\n(rule \"one-net\" (condition \"A.NetName == '{net}'\") (constraint track_width (min 0.6mm) (opt 0.6mm)))\n");
    let without = drc_violations(&k, &plain, &dru, "track_width", Some(&net));
    let with = drc_violations(&k, &route_with(&k, &d.join("with"), &dru), &dru, "track_width", Some(&net));
    assert!(without > 0, "the net {net} should break a 0.6 mm rule that the router did not see ({without})");
    assert!(with < without, "the router did not follow the width of {net}: {with} track_width violations with the rule, {without} without");
}
