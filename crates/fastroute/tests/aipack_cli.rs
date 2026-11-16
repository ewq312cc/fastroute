//! `fastroute ai-pack`: the pack, the DRC states (with a scripted stand-in for kicad-cli),
//! overwrite protection and verification.

mod common;
use common::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn pack_args(dir: &Path, extra: &[&str]) -> Vec<String> {
    let mut a: Vec<String> = vec!["ai-pack".into(), "-i".into(), s(&demo_dsn()), "-s".into(), s(&routed_demo_ses()), "-o".into(), s(dir)];
    a.extend(extra.iter().map(|x| x.to_string()));
    a
}

/// The design alone: only the wiring that it has itself (no router result on top).
fn pack_args_design_only(dir: &Path, extra: &[&str]) -> Vec<String> {
    let mut a: Vec<String> = vec!["ai-pack".into(), "-i".into(), s(&demo_dsn()), "-o".into(), s(dir)];
    a.extend(extra.iter().map(|x| x.to_string()));
    a
}

fn files(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

#[cfg(unix)]
mod fake {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in for kicad-cli: answers --version, checks its arguments, records them in
    /// `<script>.args`, then does `body` (a shell snippet; `$out` is the --output path).
    pub fn kicad_cli(dir: &Path, body: &str) -> PathBuf {
        let script = dir.join("kicad-cli");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nPATH=/usr/bin:/bin\nif [ \"$1\" = \"--version\" ]; then echo '9.0.0-fake'; exit 0; fi\n\
                 echo \"$@\" > \"$0.args\"\n\
                 if [ \"$1\" != pcb ] || [ \"$2\" != drc ]; then echo \"unexpected arguments: $*\" >&2; exit 9; fi\n\
                 out=\"\"\nwhile [ $# -gt 0 ]; do if [ \"$1\" = --output ]; then out=\"$2\"; shift; fi; shift; done\n{body}\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    pub const VIOLATIONS: &str = r#"cat > "$out" <<'JSON'
{"$schema":"https://schemas.kicad.org/drc.v1.json","kicad_version":"9.0.0","coordinate_units":"mm",
 "violations":[{"type":"clearance","severity":"error","description":"Clearance violation (netclass 'Default' clearance 0.2 mm; actual 0.1 mm)",
   "items":[{"description":"Track [VCC] on B.Cu","pos":{"x":10.0,"y":6.35}},{"description":"Track [SIG] on B.Cu","pos":{"x":12.0,"y":6.0}}]}],
 "unconnected_items":[{"type":"unconnected_items","severity":"error","description":"Missing connection between items",
   "items":[{"description":"Pad 1 [GND] of J1","pos":{"x":30.0,"y":21.27}},{"description":"Pad 2 [GND] of R1","pos":{"x":20.8,"y":10.0}}]}],
 "schematic_parity":[]}
JSON"#;

    pub const CLEAN: &str = r#"cat > "$out" <<'JSON'
{"kicad_version":"9.0.0","coordinate_units":"mm","violations":[],"unconnected_items":[],"schematic_parity":[]}
JSON"#;
}

fn pcb(dir: &Path) -> PathBuf {
    let p = dir.join("routed.kicad_pcb");
    std::fs::write(&p, "(kicad_pcb (version 20240108))\n").unwrap();
    p
}

fn manifest(dir: &Path) -> Value {
    json(&dir.join("manifest.json"))
}

#[test]
fn one_command_writes_the_whole_pack_without_a_gui_and_without_a_drc() {
    let d = tmp("pack").join("pack");
    let o = run(&pack_args(&d, &[]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(files(&d), ["inspect.json", "manifest.json", "overview.png", "region-B.Cu.png", "region-F.Cu.png", "region-combined.png", "route-report.json", "rules.json"]);
    let m = manifest(&d);
    assert_valid("manifest-1.0.schema.json", &m);
    // no DRC was possible: said so, warned, carried on - and it is not a pass
    assert_eq!((m["status"].as_str(), m["drc_status"].as_str()), (Some("ok"), Some("not_run")));
    assert_eq!(m["reports"]["drc_report"], json!({"file": null, "status": "not_run", "detail": "no KiCad board (--kicad-pcb) and no finished report (--drc-report) was given"}));
    assert!(!d.join("drc-report.json").exists());
    assert!(stderr(&o).contains("not a pass") && stderr(&o).contains("\"not_run\""), "{}", stderr(&o));
    assert_eq!(json(&d.join("inspect.json"))["checks"]["drc"]["status"], "not_run");
    // the pictures: overview of the whole board, then the region around the problems
    let imgs = m["images"].as_array().unwrap();
    assert_eq!(imgs.iter().map(|i| i["file"].as_str().unwrap()).collect::<Vec<_>>(), ["overview.png", "region-combined.png", "region-F.Cu.png", "region-B.Cu.png"]);
    assert_eq!(imgs.iter().map(|i| i["purpose"].as_str().unwrap()).collect::<Vec<_>>(), ["overview", "region-combined", "region-layer", "region-layer"]);
    assert_eq!(imgs[0]["selection"]["type"], "board");
    assert_eq!(imgs[1]["selection"]["type"], "issues");
    assert_eq!(imgs[2]["layers"], json!(["F.Cu"]));
    // all pictures of the region show the same rectangle
    for i in &imgs[2..] {
        assert_eq!(i["region_mm"], imgs[1]["region_mm"]);
    }
    // every file named exists with its hash and size; the manifest links pictures, nets and issues to the data
    assert_eq!(code(&run(&["ai-pack", "--verify", &s(&d)])), 0);
    let inspect = json(&d.join("inspect.json"));
    assert_valid("inspect-1.0.schema.json", &inspect);
    let ids: Vec<&str> = inspect["unrouted"].as_array().unwrap().iter().chain(inspect["violations"].as_array().unwrap()).map(|x| x["id"].as_str().unwrap()).collect();
    for id in imgs[1]["contains"]["issues"].as_array().unwrap() {
        assert!(ids.contains(&id.as_str().unwrap()), "{id}");
    }
    assert_valid("rules-1.0.schema.json", &json(&d.join("rules.json")));
    let report = json(&d.join("route-report.json"));
    assert_eq!(report["stats"]["unrouted"], inspect["routing"]["unrouted_connections"]);
    assert_eq!(m["session"]["path"], s(&routed_demo_ses()));
}

#[test]
fn region_options_choose_the_pictures_and_only_layers_with_copper_get_one() {
    let d = tmp("around").join("pack");
    let o = run(&pack_args_design_only(&d, &["--around", "U1", "--padding-mm", "1.5", "--px-per-mm", "50"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let m = manifest(&d);
    let names: Vec<&str> = m["images"].as_array().unwrap().iter().map(|i| i["file"].as_str().unwrap()).collect();
    // around U1 (package 8..12 x 8.5..11.5 mm, its outline 0.06 wider) the design has only
    // top-layer copper: no picture of an empty B.Cu. The router's result, on the other hand, runs
    // a GND wire on B.Cu through this region.
    assert_eq!(names, ["overview.png", "region-combined.png", "region-F.Cu.png"]);
    assert_eq!(m["images"][1]["selection"], json!({"type": "component", "targets": ["U1"], "padding_mm": 1.5}));
    assert_eq!(m["images"][1]["px_per_mm"], 50.0);
    assert_eq!(m["images"][1]["contains"]["components"], json!(["U1"]));
    let with_session = tmp("around-session").join("pack");
    assert_eq!(code(&run(&pack_args(&with_session, &["--around", "U1", "--padding-mm", "1.5", "-q"]))), 0);
    assert!(manifest(&with_session)["images"].as_array().unwrap().iter().any(|i| i["file"] == "region-B.Cu.png"));
    // layers option
    let d2 = tmp("layers").join("pack");
    assert_eq!(code(&run(&pack_args(&d2, &["--layers", "B.Cu"]))), 0);
    let names: Vec<String> = manifest(&d2)["images"].as_array().unwrap().iter().map(|i| i["file"].as_str().unwrap().to_string()).collect();
    assert_eq!(names, ["overview.png", "region-combined.png", "region-B.Cu.png"]);
    assert_eq!(manifest(&d2)["images"][1]["layers"], json!(["B.Cu"]));
    // nets and rectangles
    let d3 = tmp("net").join("pack");
    assert_eq!(code(&run(&pack_args(&d3, &["--net", "SIG", "--net", "GND"]))), 0);
    assert_eq!(manifest(&d3)["images"][1]["selection"]["targets"], json!(["SIG", "GND"]));
    let d4 = tmp("rect").join("pack");
    assert_eq!(code(&run(&pack_args(&d4, &["--region-mm", "5,4,20,10", "--size", "800"]))), 0);
    let m4 = manifest(&d4);
    assert_eq!((m4["images"][1]["width_px"].as_u64(), m4["images"][1]["height_px"].as_u64()), (Some(800), Some(400)));
    // a tiny region is not blown up beyond 150 px/mm by default
    let d5 = tmp("tiny").join("pack");
    assert_eq!(code(&run(&pack_args(&d5, &["--region-mm", "8,9,3,2"]))), 0);
    assert_eq!(manifest(&d5)["images"][1]["px_per_mm"], 150.0);
    // errors in the region options are reported, and nothing is left behind
    let d6 = tmp("badregion").join("pack");
    let o = run(&pack_args(&d6, &["--around", "X99"]));
    assert_eq!(code(&o), 2, "{}", stderr(&o));
    assert!(stderr(&o).contains("no component 'X99'"));
}

#[cfg(unix)]
#[test]
fn kicad_cli_runs_and_its_findings_end_up_in_the_pack() {
    let t = tmp("drc-fail");
    let tool = fake::kicad_cli(&t, fake::VIOLATIONS);
    let board = pcb(&t);
    let d = t.join("pack");
    let o = run(&pack_args(&d, &["--kicad-pcb", &s(&board), "--kicad-cli", &s(&tool)]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    // the tool got the arguments of a real kicad-cli call
    let args = std::fs::read_to_string(format!("{}.args", tool.display())).unwrap();
    assert!(args.starts_with("pcb drc --format json --units mm --severity-all --output "), "{args}");
    assert!(args.trim_end().ends_with(&s(&board)), "{args}");
    let m = manifest(&d);
    assert_valid("manifest-1.0.schema.json", &m);
    assert_eq!((m["status"].as_str(), m["drc_status"].as_str()), (Some("ok"), Some("failed")));
    assert_eq!(m["reports"]["drc_report"]["status"], "present");
    assert!(m["reports"]["drc_report"]["detail"].as_str().unwrap().contains("9.0.0-fake"));
    // drc-report.json is kicad-cli's own output, unchanged
    let raw = std::fs::read_to_string(d.join("drc-report.json")).unwrap();
    assert!(raw.contains("schemas.kicad.org/drc.v1.json") && raw.contains("Clearance violation (netclass"));
    let inspect = json(&d.join("inspect.json"));
    assert_valid("inspect-1.0.schema.json", &inspect);
    let drc = &inspect["checks"]["drc"];
    assert_eq!((drc["status"].as_str(), drc["violations"].as_u64(), drc["unconnected"].as_u64()), (Some("failed"), Some(1), Some(1)));
    assert_eq!(drc["tool_version"], "9.0.0-fake");
    assert_eq!(drc["report_file"], "drc-report.json");
    // the findings are markers in the pictures and ids in the manifest, tied to inspect.json
    let combined = &m["images"][1];
    let in_picture: Vec<&str> = combined["contains"]["issues"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert!(in_picture.contains(&"DRC1") && in_picture.contains(&"DRC2"), "{in_picture:?}");
    assert_eq!(combined["selection"]["type"], "issues");
    assert!(combined["selection"]["targets"].as_array().unwrap().iter().any(|t| t == "DRC2"));
    assert_eq!(code(&run(&["ai-pack", "--verify", &s(&d)])), 0);
    // --require-drc is satisfied by a report with findings (failed is still a real result)
    let d2 = t.join("pack2");
    let o = run(&pack_args(&d2, &["--kicad-pcb", &s(&board), "--kicad-cli", &s(&tool), "--require-drc"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
}

#[cfg(unix)]
#[test]
fn a_clean_report_is_the_only_way_to_passed() {
    let t = tmp("drc-pass");
    let tool = fake::kicad_cli(&t, fake::CLEAN);
    let d = t.join("pack");
    let o = run(&pack_args(&d, &["--kicad-pcb", &s(&pcb(&t)), "--kicad-cli", &s(&tool), "-q"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(manifest(&d)["drc_status"], "passed");
    assert_eq!(json(&d.join("inspect.json"))["checks"]["drc"]["status"], "passed");
    assert_eq!(code(&run(&["ai-pack", "--verify", &s(&d)])), 0);
    // the same through a report that was made elsewhere
    let rep = t.join("elsewhere.json");
    std::fs::write(&rep, r#"{"kicad_version":"9.0.0","coordinate_units":"mm","violations":[],"unconnected_items":[]}"#).unwrap();
    let d2 = t.join("pack2");
    assert_eq!(code(&run(&pack_args(&d2, &["--drc-report", &s(&rep), "-q"]))), 0);
    assert_eq!(manifest(&d2)["drc_status"], "passed");
    assert_eq!(std::fs::read(d2.join("drc-report.json")).unwrap(), std::fs::read(&rep).unwrap());
}

#[test]
fn a_missing_kicad_cli_warns_and_carries_on_as_unavailable() {
    let t = tmp("drc-missing");
    let board = pcb(&t);
    let d = t.join("pack");
    // explicit path that does not exist
    let o = run(&pack_args(&d, &["--kicad-pcb", &s(&board), "--kicad-cli", "/no/such/kicad-cli"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let m = manifest(&d);
    assert_eq!((m["status"].as_str(), m["drc_status"].as_str()), (Some("partial"), Some("unavailable")));
    assert_eq!(m["reports"]["drc_report"]["status"], "unavailable");
    assert_eq!(m["reports"]["drc_report"]["file"], Value::Null);
    assert!(m["reports"]["drc_report"]["detail"].as_str().unwrap().contains("/no/such/kicad-cli"));
    assert!(!d.join("drc-report.json").exists());
    assert!(m["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("\"unavailable\", which is not a pass")), "{}", m["warnings"]);
    assert!(stderr(&o).contains("warning: DRC:"), "{}", stderr(&o));
    assert_eq!(json(&d.join("inspect.json"))["checks"]["drc"]["status"], "unavailable");
    assert_eq!(code(&run(&["ai-pack", "--verify", &s(&d)])), 0);
    // not found anywhere (empty PATH, nothing configured)
    let d2 = t.join("pack2");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_fastroute"))
        .args(pack_args(&d2, &["--kicad-pcb", &s(&board)]))
        .env("PATH", "")
        .env("FASTROUTE_KICAD_CLI", "/no/such/either")
        .output()
        .unwrap();
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(manifest(&d2)["drc_status"], "unavailable");
    // --require-drc: the pack is written, but the exit code says there is no DRC result
    let d3 = t.join("pack3");
    let o = run(&pack_args(&d3, &["--kicad-pcb", &s(&board), "--kicad-cli", "/no/such/kicad-cli", "--require-drc"]));
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert!(d3.join("manifest.json").exists() && stderr(&o).contains("no DRC result"));
    // asked for no DRC at all
    let d4 = t.join("pack4");
    assert_eq!(code(&run(&pack_args(&d4, &["--no-drc", "-q"]))), 0);
    assert_eq!(manifest(&d4)["drc_status"], "not_run");
}

#[cfg(unix)]
#[test]
fn timeouts_crashes_and_garbage_are_errors_never_passes() {
    let t = tmp("drc-bad");
    let board = pcb(&t);
    // (script body, status, text expected in the detail)
    let cases: Vec<(&str, &str, &str)> = vec![
        ("sleep 30", "timeout", "did not finish within 1 s"),
        ("echo 'Failed to load board' >&2; exit 3", "error", "exited with code 3: Failed to load board"),
        ("echo 'this is not json' > \"$out\"", "error", "cannot be read"),
        ("echo '{}' > \"$out\"", "error", "not a KiCad DRC report"),
        ("exit 0", "error", "wrote no readable report"),
        ("kill -9 $$", "error", "a signal"),
    ];
    for (i, (body, status, needle)) in cases.into_iter().enumerate() {
        let tool = fake::kicad_cli(&t.join(""), body);
        let d = t.join(format!("pack{i}"));
        let started = std::time::Instant::now();
        let o = run(&pack_args(&d, &["--kicad-pcb", &s(&board), "--kicad-cli", &s(&tool), "--drc-timeout", "1"]));
        assert_eq!(code(&o), 0, "{body}: {}", stderr(&o));
        assert!(started.elapsed().as_secs() < 20, "{body}: took {:?}", started.elapsed());
        let m = manifest(&d);
        assert_eq!(m["drc_status"], status, "{body}");
        assert_eq!(m["status"], "partial", "{body}");
        assert_ne!(m["drc_status"], "passed");
        let detail = m["reports"]["drc_report"]["detail"].as_str().unwrap();
        assert!(detail.contains(needle), "{body}: {detail}");
        assert!(!d.join("drc-report.json").exists(), "{body}: a failed run must not leave a report behind");
        assert_eq!(json(&d.join("inspect.json"))["checks"]["drc"]["status"], status);
        assert_eq!(code(&run(&["ai-pack", "--verify", &s(&d)])), 0, "{body}");
    }
}

#[test]
fn a_bad_report_or_board_is_an_input_error() {
    let t = tmp("bad-drc-input");
    let d = t.join("pack");
    for (name, text, needle) in [("a.json", "", "not valid JSON"), ("b.json", "{}", "not a KiCad DRC report")] {
        let f = t.join(name);
        std::fs::write(&f, text).unwrap();
        let o = run(&pack_args(&d, &["--drc-report", &s(&f)]));
        assert_eq!(code(&o), 3, "{name}: {}", stderr(&o));
        assert!(stderr(&o).contains(needle), "{name}: {}", stderr(&o));
        assert!(!d.exists(), "nothing is written for a report that cannot be read");
    }
    let o = run(&pack_args(&d, &["--kicad-pcb", "/no/such.kicad_pcb"]));
    assert_eq!(code(&o), 3);
    assert!(stderr(&o).contains("/no/such.kicad_pcb"));
    // contradictions
    for (args, text) in [
        (vec!["--no-drc", "--kicad-pcb", "x"], "contradicts"),
        (vec!["--kicad-pcb", "x", "--drc-report", "y"], "either --kicad-pcb"),
        (vec!["--drc-timeout", "0"], "at least 1 second"),
        (vec!["--overview-size", "big"], "not a number of pixels"),
        (vec!["--annotate", "glitter"], "unknown annotation"),
    ] {
        let o = run(&pack_args(&d, &args));
        assert_eq!(code(&o), 2, "{args:?}: {}", stderr(&o));
        assert!(stderr(&o).contains(text), "{args:?}: {}", stderr(&o));
    }
}

#[test]
fn a_pack_is_never_written_over_another_one_unless_forced_and_then_only_its_own_files() {
    let t = tmp("overwrite");
    let d = t.join("pack");
    assert_eq!(code(&run(&pack_args(&d, &["-q"]))), 0);
    let before: Vec<Vec<u8>> = files(&d).iter().map(|f| std::fs::read(d.join(f)).unwrap()).collect();
    // again: refused, nothing changed
    let o = run(&pack_args(&d, &["-q"]));
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(stderr(&o).contains("never written over another one") && stderr(&o).contains("--force"), "{}", stderr(&o));
    let after: Vec<Vec<u8>> = files(&d).iter().map(|f| std::fs::read(d.join(f)).unwrap()).collect();
    assert_eq!(before, after);
    // a stale picture of an older pack and an unrelated file
    std::fs::write(d.join("region-OLD.Cu.png"), b"stale").unwrap();
    std::fs::write(d.join("notes.txt"), b"mine").unwrap();
    let o = run(&pack_args(&d, &["-q", "--force", "--layers", "F.Cu"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(!d.join("region-OLD.Cu.png").exists() && !d.join("region-B.Cu.png").exists(), "stale pack pictures are gone");
    assert_eq!(std::fs::read(d.join("notes.txt")).unwrap(), b"mine", "files that are not the pack's stay");
    assert_eq!(code(&run(&["ai-pack", "--verify", &s(&d)])), 0);
    // a non-empty directory of something else is refused without --force too
    let other = t.join("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("keep.txt"), b"x").unwrap();
    assert_eq!(code(&run(&pack_args(&other, &["-q"]))), 4);
    assert_eq!(files(&other), ["keep.txt"]);
    // an empty existing directory is fine; a file is not a directory
    let empty = t.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert_eq!(code(&run(&pack_args(&empty, &["-q"]))), 0);
    let file = t.join("file");
    std::fs::write(&file, b"x").unwrap();
    let o = run(&pack_args(&file, &["-q", "--force"]));
    assert_eq!(code(&o), 4);
    assert!(stderr(&o).contains("not a directory"));
    // the inputs are never overwritten: not by the pack files of an in-place run
    let report = d.join("route-report.json");
    let o = run(&pack_args(&d, &["--force", "--report", &s(&report)]));
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(stderr(&o).contains("an input file would be overwritten"), "{}", stderr(&o));
    assert!(report.exists());
    // the original design and session are byte-identical after all of this
    let ses = routed_demo_ses();
    assert_eq!(file_hash(&demo_dsn()), std::fs::read(manifest_dir().join("testdata/render_demo.dsn")).unwrap());
    assert!(ses.exists());
}

#[test]
fn own_rules_and_route_report_are_included_as_given() {
    let t = tmp("given");
    let (_, report) = routed_demo();
    let rules = t.join("my-rules.json");
    std::fs::write(&rules, r#"{"my":"rules","usb":{"diff_pair_gap_mm":0.15}}"#).unwrap();
    let d = t.join("pack");
    let o = run(&pack_args(&d, &["--report", &s(&report), "--rules", &s(&rules), "-q"]));
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(std::fs::read(d.join("rules.json")).unwrap(), std::fs::read(&rules).unwrap());
    assert_eq!(std::fs::read(d.join("route-report.json")).unwrap(), std::fs::read(&report).unwrap());
    let m = manifest(&d);
    assert!(m["reports"]["rules"]["detail"].as_str().unwrap().starts_with("copied from"));
    assert_eq!(code(&run(&["ai-pack", "--verify", &s(&d)])), 0);
    // files that are not JSON are refused
    let bad = t.join("bad.json");
    std::fs::write(&bad, "nope").unwrap();
    let o = run(&pack_args(&t.join("pack2"), &["--rules", &s(&bad)]));
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(stderr(&o).contains("not valid JSON"));
}

#[test]
fn an_unreadable_input_is_recorded_in_the_pack_directory_too() {
    let t = tmp("badinput");
    let bad = t.join("board.kicad_pcb");
    std::fs::write(&bad, "(kicad_pcb (version 20240108))").unwrap();
    let d = t.join("pack");
    let o = run(&["ai-pack", "-i", &s(&bad), "-o", &s(&d)]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(stderr(&o).contains("Specctra DSN"));
    let m = manifest(&d);
    assert_valid("manifest-1.0.schema.json", &m);
    assert_eq!(m["status"], "error");
    assert_eq!(m["errors"][0]["code"], "input_invalid");
    assert!(m["images"].as_array().unwrap().is_empty());
    assert_eq!(files(&d), ["manifest.json"]);
    // verify refuses it, and says why
    let o = run(&["ai-pack", "--verify", &s(&d)]);
    assert_eq!(code(&o), 3);
    assert!(stderr(&o).contains("records a failed run"), "{}", stderr(&o));
}

#[test]
fn verify_finds_what_is_wrong_with_a_pack() {
    let t = tmp("verify");
    let d = t.join("pack");
    assert_eq!(code(&run(&pack_args(&d, &["-q"]))), 0);
    let verify = |d: &Path| run(&["ai-pack", "--verify", &s(d)]);
    assert_eq!(code(&verify(&d)), 0);
    let copy = |name: &str| {
        let c = t.join(name);
        std::fs::create_dir_all(&c).unwrap();
        for f in files(&d) {
            std::fs::copy(d.join(&f), c.join(&f)).unwrap();
        }
        c
    };
    // a changed picture
    let c = copy("changed");
    let mut png = std::fs::read(c.join("overview.png")).unwrap();
    let n = png.len() / 2;
    png[n] ^= 0xff;
    std::fs::write(c.join("overview.png"), png).unwrap();
    let o = verify(&c);
    assert_eq!(code(&o), 3);
    assert!(stderr(&o).contains("overview.png") && stderr(&o).contains("sha256"), "{}", stderr(&o));
    // a missing picture and a missing report
    let c = copy("missing");
    std::fs::remove_file(c.join("region-F.Cu.png")).unwrap();
    std::fs::remove_file(c.join("inspect.json")).unwrap();
    let o = verify(&c);
    assert_eq!(code(&o), 3);
    assert!(stderr(&o).contains("region-F.Cu.png") && stderr(&o).contains("inspect.json"), "{}", stderr(&o));
    // a DRC status that does not match the data
    let c = copy("status");
    let mut m = manifest(&c);
    m["drc_status"] = json!("passed");
    std::fs::write(c.join("manifest.json"), serde_json::to_vec_pretty(&m).unwrap()).unwrap();
    let o = verify(&c);
    assert_eq!(code(&o), 3);
    assert!(stderr(&o).contains("drc_status"), "{}", stderr(&o));
    // an issue in a picture that the data does not have
    let c = copy("issue");
    let mut m = manifest(&c);
    m["images"][1]["contains"]["issues"].as_array_mut().unwrap().push(json!("CLR99"));
    std::fs::write(c.join("manifest.json"), serde_json::to_vec_pretty(&m).unwrap()).unwrap();
    let o = verify(&c);
    assert_eq!(code(&o), 3);
    assert!(stderr(&o).contains("CLR99"), "{}", stderr(&o));
    // a stray drc-report.json that the manifest does not know
    let c = copy("stray");
    std::fs::write(c.join("drc-report.json"), "{}").unwrap();
    let o = verify(&c);
    assert_eq!(code(&o), 3);
    assert!(stderr(&o).contains("drc-report.json"), "{}", stderr(&o));
    // not a pack at all
    let o = verify(&t.join("nothing"));
    assert_eq!(code(&o), 3);
    assert!(stderr(&o).contains("manifest.json"));
    assert_eq!(code(&run(&["ai-pack", "--verify", &s(&d), "-i", "x"])), 2);
}

#[test]
fn millimetre_designs_work_and_pictures_are_reproducible() {
    let t = tmp("mmpack");
    let args = |d: &Path| vec!["ai-pack".to_string(), "-i".into(), s(&mm_dsn()), "-o".into(), s(d), "-q".into()];
    let (a, b) = (t.join("a"), t.join("b"));
    assert_eq!(code(&run(&args(&a))), 0);
    assert_eq!(code(&run(&args(&b))), 0);
    let m = manifest(&a);
    assert_eq!(m["images"][0]["region_mm"]["width"].as_f64().map(|w| w.round()), Some(52.0));
    assert_eq!(code(&run(&["ai-pack", "--verify", &s(&a)])), 0);
    // the same input gives the same pictures
    for f in ["overview.png", "region-combined.png"] {
        assert_eq!(std::fs::read(a.join(f)).unwrap(), std::fs::read(b.join(f)).unwrap(), "{f}");
    }
}

#[test]
fn help_and_usage() {
    let o = run(&["ai-pack", "--help"]);
    assert_eq!(code(&o), 0);
    for needle in ["usage: fastroute ai-pack", "--output DIR", "--kicad-pcb", "--kicad-cli", "--drc-timeout", "--require-drc", "--verify", "--force", "never \"passed\"", "Nothing is uploaded"] {
        assert!(stdout(&o).contains(needle), "{needle}\n{}", stdout(&o));
    }
    assert_eq!(stdout(&run(&["help", "ai-pack"])), stdout(&o));
    for (args, want, text) in [
        (vec!["ai-pack", "-o", "x"], 2, "no input design"),
        (vec!["ai-pack", "-i", "x.dsn"], 2, "no output directory"),
        (vec!["ai-pack", "-i", "/no/such.dsn", "-o", &s(&tmp("help").join("pack"))], 3, "/no/such.dsn"),
        (vec!["ai-pack", "-i", "x", "-o", "y", "--wat"], 2, "unknown option '--wat'"),
    ] {
        let o = run(&args);
        assert_eq!(code(&o), want, "{args:?}: {}", stderr(&o));
        assert!(stderr(&o).contains(text), "{args:?}: {}", stderr(&o));
    }
}
