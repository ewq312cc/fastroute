//! `fastroute render`: whole board and regions, end to end through the binary.

mod common;
use common::*;

fn render_json(args: &[&str]) -> (std::process::Output, serde_json::Value) {
    let d = tmp("render-json");
    let out = d.join("o.png");
    let mut a: Vec<String> = vec!["render".into(), "-i".into(), s(&demo_dsn()), "-o".into(), s(&out), "--manifest".into(), "-".into(), "--force".into()];
    a.extend(args.iter().map(|x| x.to_string()));
    let o = run(&a);
    let v = serde_json::from_slice(&o.stdout).unwrap_or(serde_json::Value::Null);
    (o, v)
}

#[test]
fn whole_board_png_without_a_gui() {
    let d = tmp("whole");
    let out = d.join("board.png");
    let before = file_hash(&demo_dsn());
    let o = run(&["render", "--input", &s(&demo_dsn()), "--output", &s(&out), "--size", "1200"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let (w, h) = png_size(&out);
    assert_eq!(w.max(h), 1200);
    assert!(stdout(&o).contains("wrote") && stdout(&o).contains("1200"), "{}", stdout(&o));
    assert_eq!(file_hash(&demo_dsn()), before, "the input changed");
    assert!(!d.join("board.png.tmp").exists());
}

#[test]
fn board_bounds_units_and_content_are_right() {
    // the demo board is 40 x 30 mm in a um design; the region is the board plus a 2 % margin
    let (o, m) = render_json(&["--size", "2000"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let r = &m["images"][0]["region_mm"];
    let (x, y, w, h) = (r["x"].as_f64().unwrap(), r["y"].as_f64().unwrap(), r["width"].as_f64().unwrap(), r["height"].as_f64().unwrap());
    assert!((x + 0.8).abs() < 0.01 && (y + 0.8).abs() < 0.01 && (w - 41.6).abs() < 0.02 && (h - 31.6).abs() < 0.02, "{r}");
    let c = &m["images"][0]["contains"];
    assert_eq!(c["components"], serde_json::json!(["J1", "R1", "U1"]));
    assert_eq!(c["nets"], serde_json::json!(["GND", "NC", "SIG", "VCC"]));
    // issues: one clearance violation and the unrouted GND/GND-plane/VCC connections
    let issues: Vec<&str> = c["issues"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert!(issues.contains(&"CLR1") && issues.iter().filter(|i| i.starts_with("OPEN")).count() >= 2, "{issues:?}");
}

#[test]
fn millimetre_designs_get_the_same_millimetres() {
    // synth_planes.dsn is (resolution mm 1000), boundary 50 x 40; Specctra's y points up so the
    // KiCad-style y of the board is -40..0
    let d = tmp("mm");
    let o = run(&["render", "-i", &s(&mm_dsn()), "-o", &s(&d.join("p.png")), "--manifest", "-"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let m: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let r = &m["images"][0]["region_mm"];
    assert!((r["width"].as_f64().unwrap() - 52.0).abs() < 0.01, "{r}");
    assert!((r["y"].as_f64().unwrap() + 41.0).abs() < 0.05 && (r["height"].as_f64().unwrap() - 42.0).abs() < 0.05, "{r}");
}

#[test]
fn layers_select_single_and_stacked_images() {
    let (_, both) = render_json(&[]);
    let (o, top) = render_json(&["--layers", "F.Cu"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let (_, bottom) = render_json(&["--layers", "b.cu"]);
    assert_eq!(both["images"][0]["layers"], serde_json::json!(["F.Cu", "B.Cu"]));
    assert_eq!(top["images"][0]["layers"], serde_json::json!(["F.Cu"]));
    assert_eq!(bottom["images"][0]["layers"], serde_json::json!(["B.Cu"]));
    // the SMD parts are on F.Cu only; the through-hole header J1 on both
    assert_eq!(top["images"][0]["contains"]["components"], serde_json::json!(["J1", "R1", "U1"]));
    assert_eq!(bottom["images"][0]["contains"]["components"], serde_json::json!(["J1"]));
}

#[test]
fn around_a_component_covers_the_whole_package_plus_padding() {
    let (o, m) = render_json(&["--around", "U1", "--padding-mm", "2", "--px-per-mm", "60", "--layers", "F.Cu"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let img = &m["images"][0];
    assert_eq!(img["selection"], serde_json::json!({"type": "component", "targets": ["U1"], "padding_mm": 2.0}));
    // U1 at (10, 10): pads at x 8.73..11.27 (+-0.5), y 9.0..11.0 (+-0.3), outline 8..12 x 8.5..11.5
    let r = &img["region_mm"];
    let (x0, x1) = (r["x"].as_f64().unwrap(), r["x"].as_f64().unwrap() + r["width"].as_f64().unwrap());
    let (y0, y1) = (r["y"].as_f64().unwrap(), r["y"].as_f64().unwrap() + r["height"].as_f64().unwrap());
    assert!(x0 <= 8.0 - 2.0 + 1e-6 && x1 >= 12.0 + 2.0 - 1e-6, "x {x0}..{x1}");
    assert!(y0 <= 8.5 - 2.0 + 1e-6 && y1 >= 11.5 + 2.0 - 1e-6, "y {y0}..{y1}");
    assert!(x1 - x0 < 4.0 + 4.0 + 1.0, "padding is not larger than asked: {x0}..{x1}");
    assert_eq!(img["contains"]["components"], serde_json::json!(["U1"]));
    // the pixel size follows from region and scale
    assert!((img["px_per_mm"].as_f64().unwrap() - 60.0).abs() < 1e-9);
    assert_eq!(img["width_px"].as_u64().unwrap(), ((x1 - x0) * 60.0).round() as u64);
}

#[test]
fn nets_region_and_differential_pair_style_repeat() {
    let (o, m) = render_json(&["--net", "SIG", "--padding-mm", "1", "--layers", "all"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let img = &m["images"][0];
    assert_eq!(img["selection"]["type"], "net");
    let r = &img["region_mm"];
    // SIG runs from U1-2 (8.73, 9) over the vias at (8.73, 6) / (19.2, 6) to R1-1 (19.2, 10)
    assert!(r["x"].as_f64().unwrap() <= 8.73 - 0.5 - 1.0 + 0.01);
    assert!(r["x"].as_f64().unwrap() + r["width"].as_f64().unwrap() >= 19.2 + 0.4 + 1.0 - 0.01);
    assert!(r["y"].as_f64().unwrap() <= 6.0 - 0.3 - 1.0 + 0.01);
    // two nets: the region is the union
    let (o, m2) = render_json(&["--net", "SIG", "--net", "GND", "--padding-mm", "1"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(m2["images"][0]["selection"]["targets"], serde_json::json!(["SIG", "GND"]));
    assert!(m2["images"][0]["region_mm"]["height"].as_f64().unwrap() > r["height"].as_f64().unwrap());
}

#[test]
fn rectangle_region_and_scales_keep_the_world_position() {
    let (_, a) = render_json(&["--region-mm", "5,4,20,10", "--px-per-mm", "20"]);
    let (_, b) = render_json(&["--region-mm", "5,4,20,10", "--px-per-mm", "50"]);
    let (ia, ib) = (&a["images"][0], &b["images"][0]);
    assert_eq!((ia["width_px"].as_u64(), ia["height_px"].as_u64()), (Some(400), Some(200)));
    assert_eq!((ib["width_px"].as_u64(), ib["height_px"].as_u64()), (Some(1000), Some(500)));
    // the world point (12.5, 9) maps to proportional pixels through each manifest transform
    let to_px = |i: &serde_json::Value, x: f64, y: f64| {
        let t = &i["transform"];
        let f = |k: &str, v: f64| t[k]["scale"].as_f64().unwrap() * v + t[k]["offset"].as_f64().unwrap();
        (f("world_to_pixel_u", x), f("world_to_pixel_v", y))
    };
    let (ua, va) = to_px(ia, 12.5, 9.0);
    let (ub, vb) = to_px(ib, 12.5, 9.0);
    assert!((ua - 150.0).abs() < 1e-6 && (va - 100.0).abs() < 1e-6, "{ua},{va}");
    assert!((ub - 375.0).abs() < 1e-6 && (vb - 250.0).abs() < 1e-6, "{ub},{vb}");
    // and back
    let t = &ib["transform"];
    let back = t["pixel_to_world_x"]["scale"].as_f64().unwrap() * ub + t["pixel_to_world_x"]["offset"].as_f64().unwrap();
    assert!((back - 12.5).abs() < 1e-9);
}

#[test]
fn identical_runs_give_identical_files() {
    let d = tmp("det");
    let args = |o: &str| vec!["render".to_string(), "-i".into(), s(&demo_dsn()), "-o".into(), s(&d.join(o)), "--size".into(), "1000".into(), "--annotate".into(), "all".into()];
    assert_eq!(code(&run(&args("a.png"))), 0);
    assert_eq!(code(&run(&args("b.png"))), 0);
    assert_eq!(file_hash(&d.join("a.png")), file_hash(&d.join("b.png")));
}

#[test]
fn existing_output_is_protected_and_the_input_is_never_overwritten() {
    let d = tmp("protect");
    let out = d.join("o.png");
    let args = vec!["render".to_string(), "-i".into(), s(&demo_dsn()), "-o".into(), s(&out), "--size".into(), "300".into()];
    assert_eq!(code(&run(&args)), 0);
    let first = file_hash(&out);
    // again: refused, file untouched
    let o = run(&args);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(stderr(&o).contains("already exists") && stderr(&o).contains("--force"), "{}", stderr(&o));
    assert_eq!(file_hash(&out), first);
    // with --force: replaced
    let mut forced = args.clone();
    forced.extend(["--force".into(), "--size".into(), "310".into()]);
    assert_eq!(code(&run(&forced)), 0);
    assert_ne!(file_hash(&out), first);
    // the output may not be the input, not even with --force and not under another spelling
    let copy = d.join("copy.dsn");
    std::fs::copy(demo_dsn(), &copy).unwrap();
    std::fs::create_dir_all(d.join("sub")).unwrap();
    let o = run(&["render", "-i", &s(&copy), "-o", &s(&d.join("sub/../copy.dsn")), "--force"]);
    assert_eq!(code(&o), 4);
    assert!(stderr(&o).contains("never overwrites its input"), "{}", stderr(&o));
    assert_eq!(file_hash(&copy), file_hash(&demo_dsn()));
    // nor the session
    let o = run(&["render", "-i", &s(&demo_dsn()), "-s", &s(&copy), "-o", &s(&copy), "--force"]);
    assert_eq!(code(&o), 4);
}

#[test]
fn errors_have_messages_and_distinct_exit_codes() {
    let d = tmp("errors");
    let out = s(&d.join("o.png"));
    let dsn = s(&demo_dsn());
    // (args, exit code, text that must be in stderr)
    let cases: Vec<(Vec<&str>, i32, &str)> = vec![
        (vec!["render", "-i", "/no/such/board.dsn", "-o", &out], 3, "/no/such/board.dsn"),
        (vec!["render", "-o", &out], 2, "no input design"),
        (vec!["render", "-i", &dsn], 2, "no output file"),
        (vec!["render", "-i", &dsn, "-o", &out, "--bogus"], 2, "unknown option '--bogus'"),
        (vec!["render", "-i", &dsn, "-o", &out, "--layers", "In9.Cu"], 2, "unknown layer 'In9.Cu'"),
        (vec!["render", "-i", &dsn, "-o", &out, "--around", "X99"], 2, "no component 'X99'"),
        (vec!["render", "-i", &dsn, "-o", &out, "--net", "NOPE"], 2, "no net 'NOPE'"),
        (vec!["render", "-i", &dsn, "-o", &out, "--region-mm", "1,2,3"], 2, "four numbers"),
        (vec!["render", "-i", &dsn, "-o", &out, "--region-mm", "1,2,0,3"], 2, "positive"),
        (vec!["render", "-i", &dsn, "-o", &out, "--size", "huge"], 2, "neither N nor WxH"),
        (vec!["render", "-i", &dsn, "-o", &out, "--size", "100000"], 2, "limit"),
        (vec!["render", "-i", &dsn, "-o", &out, "--size", "100", "--px-per-mm", "5"], 2, "cannot be combined"),
        (vec!["render", "-i", &dsn, "-o", &out, "--around", "U1", "--net", "GND"], 2, "choose one way"),
        (vec!["render", "-i", &dsn, "-o", &out, "--padding-mm", "2"], 2, "only applies"),
        (vec!["render", "-i", &dsn, "-o", &out, "--annotate", "glitter"], 2, "unknown annotation"),
        (vec!["render", "-i", &dsn, "-o", &out, "--annotate", "grid=0.01", "--size", "300"], 2, "grid step"),
        (vec!["render", "-i", &dsn, "-o", &out, "--select", "OPEN99"], 2, "no issue 'OPEN99'"),
    ];
    for (args, want, text) in cases {
        let o = run(&args);
        assert_eq!(code(&o), want, "{args:?}\n{}", stderr(&o));
        assert!(stderr(&o).contains(text), "{args:?}: expected '{text}' in\n{}", stderr(&o));
        assert!(!d.join("o.png").exists(), "{args:?} left an output behind");
    }
}

#[test]
fn unreadable_designs_are_input_errors() {
    let d = tmp("badinput");
    let out = s(&d.join("o.png"));
    for (name, content, needle) in [
        ("empty.dsn", "".to_string(), "empty"),
        ("text.dsn", "hello".to_string(), "not a Specctra design"),
        ("board.kicad_pcb", "(kicad_pcb (version 20240108))".to_string(), "Specctra DSN"),
        ("noout.dsn", "(pcb x (structure (layer A (type signal))))".to_string(), "outline"),
        ("broken.dsn", "(pcb x (structure (layer A (type signal)".to_string(), ""),
    ] {
        let p = d.join(name);
        std::fs::write(&p, content).unwrap();
        let o = run(&["render", "-i", &s(&p), "-o", &out]);
        assert_eq!(code(&o), 3, "{name}: {}", stderr(&o));
        assert!(stderr(&o).contains(needle) && stderr(&o).starts_with("fastroute render:"), "{name}: {}", stderr(&o));
    }
    assert!(!d.join("o.png").exists());
    // a bad session file is named as such
    let ses = d.join("x.ses");
    std::fs::write(&ses, "not a session").unwrap();
    let o = run(&["render", "-i", &s(&demo_dsn()), "-s", &s(&ses), "-o", &out]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(stderr(&o).contains("x.ses"));
}

#[test]
fn report_locations_select_regions_and_draw_markers() {
    let d = tmp("report");
    // a KiCad DRC report with two problems
    let drc = d.join("drc.json");
    std::fs::write(
        &drc,
        r#"{"$schema":"https://schemas.kicad.org/drc.v1.json","coordinate_units":"mm",
            "violations":[{"type":"clearance","severity":"error","description":"Clearance violation",
              "items":[{"description":"Track","pos":{"x":10.0,"y":6.0}},{"description":"Track","pos":{"x":11.0,"y":6.3}}]}],
            "unconnected_items":[{"type":"unconnected_items","items":[{"pos":{"x":30.0,"y":21.0}},{"pos":{"x":30.0,"y":18.0}}]}],
            "schematic_parity":[]}"#,
    )
    .unwrap();
    let (o, m) = render_json(&["--from-report", &s(&drc), "--select", "DRC2", "--padding-mm", "2", "--px-per-mm", "40"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let img = &m["images"][0];
    assert_eq!(img["selection"], serde_json::json!({"type": "issues", "targets": ["DRC2"], "padding_mm": 2.0}));
    let r = &img["region_mm"];
    // around both ends of the unconnected pair (x 30, y 18..21) plus 2 mm
    assert!(r["y"].as_f64().unwrap() <= 16.0 + 1e-6 && r["y"].as_f64().unwrap() + r["height"].as_f64().unwrap() >= 23.0 - 1e-6, "{r}");
    assert!(img["contains"]["issues"].as_array().unwrap().iter().any(|i| i == "DRC2"));
    assert_eq!(img["contains"]["components"], serde_json::json!(["J1"]));
    // without --select: all problems of the report
    let (o, m) = render_json(&["--from-report", &s(&drc)]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(m["images"][0]["selection"]["targets"], serde_json::json!(["DRC1", "DRC2"]));
    // a report that is not one
    let bad = d.join("bad.json");
    std::fs::write(&bad, "{}").unwrap();
    let (o, _) = render_json(&["--from-report", &s(&bad)]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(stderr(&o).contains("unrecognised report"));
    // the board's own problems can be selected without a file
    let (o, m) = render_json(&["--select", "violations", "--padding-mm", "1"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(m["images"][0]["selection"]["targets"], serde_json::json!(["CLR1"]));
}

#[test]
fn annotations_are_reported_in_the_manifest() {
    let (_, m) = render_json(&["--annotate", "grid,nets,pads,refs,scale,layer,markers", "--size", "1600"]);
    let img = &m["images"][0];
    assert_eq!(img["annotations"], serde_json::json!(["grid", "scale", "layer", "nets", "pads", "refs", "markers"]));
    assert!(img["labels"]["drawn"].as_u64().unwrap() > 10, "{}", img["labels"]);
    let (_, m) = render_json(&["--annotate", "none"]);
    assert_eq!(m["images"][0]["annotations"], serde_json::json!([]));
    assert_eq!(m["images"][0]["labels"]["drawn"], 0);
}

#[test]
fn help_is_available_and_consistent() {
    let o = run(&["render", "--help"]);
    assert_eq!(code(&o), 0);
    for needle in ["usage: fastroute render", "--input", "--output", "--size", "--layers", "--region-mm", "--around", "--net", "--from-report", "--force", "exit codes"] {
        assert!(stdout(&o).contains(needle), "{needle}\n{}", stdout(&o));
    }
    // `help render` is the same text
    assert_eq!(stdout(&run(&["help", "render"])), stdout(&o));
}

#[test]
fn quiet_prints_nothing_on_success() {
    let d = tmp("quiet");
    let o = run(&["render", "-i", &s(&demo_dsn()), "-o", &s(&d.join("o.png")), "-q", "--size", "200"]);
    assert_eq!(code(&o), 0);
    assert!(o.stdout.is_empty() && o.stderr.is_empty(), "{} / {}", stdout(&o), stderr(&o));
}

#[test]
fn manifest_file_is_next_to_the_image_and_refuses_overwrite() {
    let d = tmp("manifest");
    let args = vec!["render".to_string(), "-i".into(), s(&demo_dsn()), "-o".into(), s(&d.join("o.png")), "--manifest".into(), s(&d.join("manifest.json")), "--size".into(), "200".into()];
    assert_eq!(code(&run(&args)), 0);
    let m = json(&d.join("manifest.json"));
    assert_eq!(m["images"][0]["file"], "o.png");
    assert_eq!(m["format"], "fastroute-manifest");
    assert_eq!(m["images"][0]["bytes"].as_u64().unwrap(), std::fs::metadata(d.join("o.png")).unwrap().len());
    // the second run would replace both files: refused before anything is written
    let o = run(&args);
    assert_eq!(code(&o), 4);
}

#[test]
fn a_session_replaces_the_wiring_of_the_design_instead_of_doubling_it() {
    // The demo design has pre-routed traces and vias. Routing writes them back into the session
    // (as protected wiring) next to what the router added, so drawing design + session must not
    // load them twice: that would show up as vias on top of each other (clearance violations at
    // distance 0).
    let ses = routed_demo_ses();
    let (o, m) = render_json(&["--session", &s(&ses)]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(m["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("session replaces the 7 traces and vias")), "{}", m["warnings"]);
    assert_eq!(m["session"]["path"], s(&ses));
    let issues: Vec<&str> = m["images"][0]["contains"]["issues"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    // the one violation is the planted one between the VCC and SIG traces
    assert_eq!(issues.iter().filter(|i| i.starts_with("CLR")).count(), 1, "{issues:?}");
    // the router's own result: the SIG net is done, the GND plane / J1 and VCC connections remain
    assert!(issues.iter().filter(|i| i.starts_with("OPEN")).count() <= 3, "{issues:?}");
}

#[test]
fn a_session_that_is_not_a_session_is_refused() {
    let d = tmp("badses");
    let out = s(&d.join("o.png"));
    for (name, text) in [("a.ses", "(pcb x)"), ("b.ses", ""), ("c.ses", "garbage")] {
        std::fs::write(d.join(name), text).unwrap();
        let o = run(&["render", "-i", &s(&demo_dsn()), "-s", &s(&d.join(name)), "-o", &out]);
        assert_eq!(code(&o), 3, "{name}: {}", stderr(&o));
        assert!(stderr(&o).contains("not a Specctra session"), "{name}: {}", stderr(&o));
    }
    assert!(!d.join("o.png").exists());
}

#[test]
fn the_manifest_matches_the_published_schema() {
    let (o, m) = render_json(&["--annotate", "all", "--size", "800"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_valid("manifest-1.0.schema.json", &m);
    // with a session, a region and a selection too
    let (_, m) = render_json(&["--session", &s(&routed_demo_ses()), "--around", "R1", "--layers", "F.Cu"]);
    assert_valid("manifest-1.0.schema.json", &m);
    assert_eq!(m["status"], "ok");
    assert_eq!(m["format_version"], "1.0");
    // and the checker does reject a wrong document (so the test is not vacuous)
    let mut bad = m.clone();
    bad["images"][0]["width_px"] = serde_json::json!(0);
    bad["coordinate_system"]["y_axis"] = serde_json::json!("up");
    bad["images"][0].as_object_mut().unwrap().remove("transform");
    let schema: serde_json::Value = serde_json::from_slice(&std::fs::read(manifest_dir().join("../../docs/schema/manifest-1.0.schema.json")).unwrap()).unwrap();
    let mut errors = Vec::new();
    validate(&schema, &bad, "$", &mut errors);
    assert!(errors.len() >= 3, "{errors:?}");
}
