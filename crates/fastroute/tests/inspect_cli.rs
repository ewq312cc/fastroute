//! `fastroute inspect`: structure, numbers, statuses, errors and format compatibility.

mod common;
use common::*;
use serde_json::{json, Value};

fn inspect(args: &[&str]) -> (std::process::Output, Value) {
    let d = tmp("inspect");
    let out = d.join("inspect.json");
    let mut a: Vec<String> = vec!["inspect".into(), "-i".into(), s(&demo_dsn()), "-o".into(), s(&out)];
    a.extend(args.iter().map(|x| x.to_string()));
    let o = run(&a);
    let v = std::fs::read(&out).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
    (o, v)
}

fn find<'a>(v: &'a Value, key: &str, field: &str, name: &str) -> &'a Value {
    v[key].as_array().unwrap().iter().find(|x| x[field] == name).unwrap_or_else(|| panic!("no {key} with {field}={name}"))
}

#[test]
fn describes_the_board_and_matches_the_schema() {
    let (o, v) = inspect(&[]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_valid("inspect-1.0.schema.json", &v);
    assert_eq!((v["format"].as_str(), v["format_version"].as_str(), v["status"].as_str()), (Some("fastroute-inspect"), Some("1.0"), Some("ok")));
    assert_eq!(v["fastroute_version"], env!("CARGO_PKG_VERSION"));
    // input digest
    let data = std::fs::read(demo_dsn()).unwrap();
    assert_eq!(v["input"]["bytes"].as_u64(), Some(data.len() as u64));
    assert_eq!(v["input"]["sha256"].as_str().unwrap().len(), 64);
    // the 40 x 30 mm board, y down
    let b = &v["board"];
    assert_eq!(b["outline_bbox_mm"], json!({"x": 0.0, "y": 0.0, "width": 40.0, "height": 30.0}));
    assert_eq!(b["layers"], json!([{"index": 0, "name": "F.Cu", "type": "signal"}, {"index": 1, "name": "B.Cu", "type": "signal"}]));
    assert_eq!((b["component_count"].as_u64(), b["pad_count"].as_u64(), b["net_count"].as_u64()), (Some(3), Some(8), Some(4)));
    // components sorted, with positions in mm
    let refs: Vec<&str> = v["components"].as_array().unwrap().iter().map(|c| c["ref"].as_str().unwrap()).collect();
    assert_eq!(refs, ["J1", "R1", "U1"]);
    let u1 = find(&v, "components", "ref", "U1");
    assert_eq!((u1["x"].as_f64(), u1["y"].as_f64(), u1["side"].as_str(), u1["pad_count"].as_u64()), (Some(10.0), Some(10.0), Some("front"), Some(4)));
    // pads: pin 1 of U1 is at (8.73, 11.0) on F.Cu, GND; the through-hole pins of J1 are on both layers
    let pad = |c: &str, p: &str| v["pads"].as_array().unwrap().iter().find(|x| x["component"] == c && x["pin"] == p).unwrap().clone();
    let p = pad("U1", "1");
    assert_eq!((p["x"].as_f64(), p["y"].as_f64(), p["net"].as_str(), p["layers"].clone()), (Some(8.73), Some(11.0), Some("GND"), json!(["F.Cu"])));
    assert_eq!(pad("J1", "2")["layers"], json!(["F.Cu", "B.Cu"]));
    assert_eq!(pad("U1", "4")["net"], "NC");
    // pads are in natural order
    let order: Vec<String> = v["pads"].as_array().unwrap().iter().map(|x| format!("{}-{}", x["component"].as_str().unwrap(), x["pin"].as_str().unwrap())).collect();
    assert_eq!(order, ["J1-1", "J1-2", "R1-1", "R1-2", "U1-1", "U1-2", "U1-3", "U1-4"]);
    // rules
    let class = &v["rules"]["net_classes"][0];
    assert_eq!(class["trace_width_mm"][0], json!({"layer": "F.Cu", "value_mm": 0.25}));
    assert_eq!(class["clearance_mm"][0], json!({"layer": "F.Cu", "value_mm": 0.2}));
    assert_eq!(v["rules"]["source"], "dsn");
    assert_eq!(class["nets"], json!(["GND", "SIG", "VCC", "NC"]));
}

#[test]
fn routing_numbers_are_consistent_and_have_coordinates() {
    let (o, v) = inspect(&[]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let r = &v["routing"];
    assert_eq!(r["status"], "incomplete");
    assert_eq!(r["unrouted_connections"].as_u64().unwrap() as usize, v["unrouted"].as_array().unwrap().len());
    assert_eq!(r["clearance_violations"].as_u64().unwrap() as usize, v["violations"].as_array().unwrap().len());
    assert_eq!(v["checks"]["connectivity"]["count"], r["unrouted_connections"]);
    assert_eq!(v["checks"]["fastroute_clearance"]["count"], r["clearance_violations"]);
    // the design has 5 pre-routed traces and 2 vias; the SIG net is routed, GND and VCC are not
    assert_eq!((r["trace_count"].as_u64(), r["via_count"].as_u64()), (Some(5), Some(2)));
    assert_eq!(find(&v, "nets", "name", "SIG")["status"], "complete");
    assert_eq!(find(&v, "nets", "name", "VCC")["status"], "incomplete");
    assert_eq!(find(&v, "nets", "name", "NC")["status"], "not_applicable");
    // ids run OPEN1.. / CLR1.. and cannot be taken for components
    for (i, u) in v["unrouted"].as_array().unwrap().iter().enumerate() {
        assert_eq!(u["id"], format!("OPEN{}", i + 1));
    }
    assert_eq!(v["violations"][0]["id"], "CLR1");
    // the planted violation: two B.Cu traces 0.1 mm apart where 0.2 is required, at (11.0, 6.175)
    let c = &v["violations"][0];
    assert_eq!(c["layer"], "B.Cu");
    assert!((c["x"].as_f64().unwrap() - 11.0).abs() < 0.01 && (c["y"].as_f64().unwrap() - 6.175).abs() < 0.01, "{c}");
    assert!((c["required_mm"].as_f64().unwrap() - 0.2).abs() < 1e-6 && (c["actual_mm"].as_f64().unwrap() - 0.1).abs() < 0.001, "{c}");
    // both traces are wiring that the design protects, so the router cannot move either
    assert_eq!((c["unfixable"].as_bool(), v["routing"]["clearance_violations_unfixable"].as_u64()), (Some(true), Some(1)));
    // every unrouted connection between pins ends at the centre of the pads that it names
    for u in v["unrouted"].as_array().unwrap() {
        for end in ["from", "to"] {
            let e = &u[end];
            if e["item"]["kind"] == "pin" {
                let pad = v["pads"].as_array().unwrap().iter().find(|p| p["component"] == e["item"]["component"] && p["pin"] == e["item"]["pin"]).unwrap();
                assert!((pad["x"].as_f64().unwrap() - e["x"].as_f64().unwrap()).abs() < 1e-3 && (pad["y"].as_f64().unwrap() - e["y"].as_f64().unwrap()).abs() < 1e-3, "{u}");
            }
        }
    }
}

#[test]
fn with_a_session_the_numbers_are_those_of_the_router() {
    let (ses, report) = routed_demo();
    let (o, v) = inspect(&["--session", &s(&ses)]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_valid("inspect-1.0.schema.json", &v);
    assert_eq!(v["session"]["path"], s(&ses));
    assert!(v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("session replaces")), "{}", v["warnings"]);
    // `fastroute --report` of the routing run itself
    let rep = json(&report);
    let st = &rep["stats"];
    assert_eq!(v["routing"]["unrouted_connections"], st["unrouted"], "unrouted");
    assert_eq!(v["routing"]["clearance_violations"], st["violations"], "violations");
    assert_eq!(v["routing"]["via_count"], st["vias"], "vias");
    // (not compared: the router still had the design's wiring protected; wiring that comes
    // from a session counts as movable, as in `route --initial-session`, see the format docs)
    assert_eq!(v["routing"]["clearance_violations_unfixable"], 0);
    // and the same locations, in the same order
    for (i, u) in rep["unrouted"].as_array().unwrap().iter().enumerate() {
        let mine = &v["unrouted"][i];
        assert_eq!(mine["net"], u["net"]);
        assert!((mine["from"]["x"].as_f64().unwrap() - u["from_xy"][0].as_f64().unwrap()).abs() < 1e-3);
        assert!((mine["from"]["y"].as_f64().unwrap() - u["from_xy"][1].as_f64().unwrap()).abs() < 1e-3);
    }
    // trace length of the report is in mm too (mine leaves out the bridge stubs, so allow a little)
    let (a, b) = (v["routing"]["trace_length_mm"].as_f64().unwrap(), st["trace_length_mm"].as_f64().unwrap());
    assert!((a - b).abs() < 0.5, "{a} vs {b}");
}

#[test]
fn drc_is_never_passed_unless_a_real_report_says_so() {
    // nothing given: not_run
    let (o, v) = inspect(&[]);
    assert_eq!(code(&o), 0);
    let drc = &v["checks"]["drc"];
    assert_eq!(drc["status"], "not_run");
    assert!(drc["tool"].is_null() && drc["violations"].is_null() && drc["issues"] == json!([]));
    assert!(drc["detail"].as_str().unwrap().contains("does not run"), "{drc}");
    let d = tmp("drc");
    // a clean report
    let clean = d.join("clean.json");
    std::fs::write(&clean, r#"{"kicad_version":"9.0.1","coordinate_units":"mm","violations":[],"unconnected_items":[],"schematic_parity":[]}"#).unwrap();
    let (o, v) = inspect(&["--drc-report", &s(&clean)]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_valid("inspect-1.0.schema.json", &v);
    assert_eq!(v["checks"]["drc"]["status"], "passed");
    assert_eq!(v["checks"]["drc"]["tool_version"], "9.0.1");
    assert_eq!(v["checks"]["drc"]["report_file"], s(&clean));
    // a report with problems
    let bad = d.join("bad.json");
    std::fs::write(
        &bad,
        r#"{"kicad_version":"9.0.1","coordinate_units":"mm",
            "violations":[{"type":"clearance","severity":"error","description":"Clearance violation","items":[{"description":"Track","pos":{"x":10,"y":6}},{"description":"Track","pos":{"x":12,"y":6}}]}],
            "unconnected_items":[{"type":"unconnected_items","severity":"error","description":"Missing connection","items":[{"description":"Pad","pos":{"x":30,"y":18}}]}]}"#,
    )
    .unwrap();
    let (o, v) = inspect(&["--drc-report", &s(&bad)]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_valid("inspect-1.0.schema.json", &v);
    let drc = &v["checks"]["drc"];
    assert_eq!((drc["status"].as_str(), drc["violations"].as_u64(), drc["unconnected"].as_u64(), drc["errors"].as_u64()), (Some("failed"), Some(1), Some(1), Some(2)));
    assert_eq!(drc["issues"][0]["id"], "DRC1");
    assert_eq!((drc["issues"][0]["x"].as_f64(), drc["issues"][0]["y"].as_f64()), (Some(11.0), Some(6.0)));
    // fastroute's own clearance check is a different thing and stays named so
    assert!(v["checks"]["fastroute_clearance"]["detail"].as_str().unwrap().contains("not KiCad's DRC"));
    // reports that cannot be read are errors, not "not_run" and not "passed"
    for (name, text, needle) in [("empty.json", "", "not valid JSON"), ("obj.json", "{}", "not a KiCad DRC report"), ("inch.json", r#"{"coordinate_units":"inch","violations":[]}"#, "coordinate unit")] {
        let p = d.join(name);
        std::fs::write(&p, text).unwrap();
        let (o, v) = inspect(&["--drc-report", &s(&p)]);
        assert_eq!(code(&o), 3, "{name}: {}", stderr(&o));
        assert!(stderr(&o).contains(needle), "{name}: {}", stderr(&o));
        assert_eq!(v["status"], "error", "{name}");
    }
}

#[test]
fn failures_are_documented_in_the_json_and_have_exit_codes() {
    let d = tmp("failures");
    for (name, content, needle, code_name) in [
        ("empty.dsn", "".to_string(), "empty", "input_invalid"),
        ("text.dsn", "hello".to_string(), "not a Specctra design", "input_invalid"),
        ("board.kicad_pcb", "(kicad_pcb (version 20240108))".to_string(), "Specctra DSN", "input_invalid"),
        ("noout.dsn", "(pcb x (structure (layer A (type signal))))".to_string(), "outline", "input_invalid"),
    ] {
        let input = d.join(name);
        std::fs::write(&input, content).unwrap();
        let out = d.join(format!("{name}.json"));
        let o = run(&["inspect", "-i", &s(&input), "-o", &s(&out)]);
        assert_eq!(code(&o), 3, "{name}: {}", stderr(&o));
        assert!(stderr(&o).contains(needle), "{name}: {}", stderr(&o));
        // the same message is in the file
        let v = json(&out);
        assert_valid("inspect-1.0.schema.json", &v);
        assert_eq!(v["status"], "error", "{name}");
        assert_eq!(v["errors"][0]["code"], code_name);
        assert!(v["errors"][0]["message"].as_str().unwrap().contains(needle), "{name}: {v}");
        assert!(v.get("board").is_none() && v.get("routing").is_none(), "{name}: an error document must not carry made-up results");
    }
    // a missing input: nothing to hash, but still a document
    let out = d.join("missing.json");
    let o = run(&["inspect", "-i", "/no/such.dsn", "-o", &s(&out)]);
    assert_eq!(code(&o), 3);
    let v = json(&out);
    assert!(v["input"].is_null() && v["status"] == "error");
    // to standard output
    let o = run(&["inspect", "-i", "/no/such.dsn", "-o", "-"]);
    assert_eq!(code(&o), 3);
    assert_eq!(serde_json::from_slice::<Value>(&o.stdout).unwrap()["status"], "error");
}

#[test]
fn usage_errors_and_overwrite_protection() {
    let d = tmp("usage");
    let dsn = s(&demo_dsn());
    let out = s(&d.join("i.json"));
    for (args, want, text) in [
        (vec!["inspect", "-o", &out], 2, "no input design"),
        (vec!["inspect", "-i", &dsn], 2, "no output file"),
        (vec!["inspect", "-i", &dsn, "-o", &out, "--layers", "F.Cu"], 2, "unknown option '--layers'"),
        (vec!["inspect", "-i", &dsn, "-o", &s(&demo_dsn())], 4, "never overwrites its input"),
    ] {
        let o = run(&args);
        assert_eq!(code(&o), want, "{args:?}: {}", stderr(&o));
        assert!(stderr(&o).contains(text), "{args:?}: {}", stderr(&o));
    }
    assert!(!d.join("i.json").exists());
    assert_eq!(code(&run(&["inspect", "-i", &dsn, "-o", &out])), 0);
    let first = file_hash(&d.join("i.json"));
    let o = run(&["inspect", "-i", &dsn, "-o", &out]);
    assert_eq!(code(&o), 4);
    assert!(stderr(&o).contains("--force"));
    assert_eq!(file_hash(&d.join("i.json")), first);
    assert_eq!(code(&run(&["inspect", "-i", &dsn, "-o", &out, "--force", "-q"])), 0);
    assert_eq!(file_hash(&d.join("i.json")), first, "same input, same bytes");
}

#[test]
fn millimetre_designs_and_empty_boards() {
    let d = tmp("mm");
    let out = d.join("i.json");
    let o = run(&["inspect", "-i", &s(&mm_dsn()), "-o", &s(&out), "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = json(&out);
    assert_valid("inspect-1.0.schema.json", &v);
    // 50 x 40 mm, y up in the file so -40..0 here
    assert_eq!(v["board"]["outline_bbox_mm"], json!({"x": 0.0, "y": -40.0, "width": 50.0, "height": 40.0}));
    assert_eq!(v["board"]["layers"].as_array().unwrap().len(), 4);
    // a board without components or nets is described as such, not refused
    let empty = d.join("empty-board.dsn");
    std::fs::write(&empty, "(pcb e (resolution um 10) (unit um) (structure (layer F.Cu (type signal)) (layer B.Cu (type signal)) (boundary (path pcb 0 0 0 10000 0 10000 -10000 0 -10000 0 0))))").unwrap();
    let out = d.join("e.json");
    let o = run(&["inspect", "-i", &s(&empty), "-o", &s(&out), "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = json(&out);
    assert_valid("inspect-1.0.schema.json", &v);
    // empty lists are present and empty (not missing: absent would not mean "none")
    for k in ["components", "pads", "nets", "unrouted", "violations"] {
        assert_eq!(v[k], json!([]), "{k}");
    }
    assert_eq!(v["routing"]["status"], "complete");
    assert_eq!(v["board"]["pad_count"], 0);
}

#[test]
fn format_versions_are_forward_compatible_within_a_major_version() {
    let (_, v) = inspect(&[]);
    let d = tmp("compat");
    // a 1.9 document with properties that 1.0 does not know: still readable
    let mut future = v.clone();
    future["format_version"] = json!("1.9");
    future["new_top_level_thing"] = json!({"anything": [1, 2, 3]});
    future["unrouted"][0]["new_field"] = json!("x");
    future["checks"]["drc"]["new_field"] = json!(7);
    assert_valid("inspect-1.0.schema.json", &future);
    let path = d.join("future.json");
    std::fs::write(&path, serde_json::to_vec(&future).unwrap()).unwrap();
    let o = run(&["render", "-i", &s(&demo_dsn()), "--from-report", &s(&path), "--select", "OPEN1", "-o", &s(&d.join("o.png")), "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    // a new major version is refused with a message, not misread
    let mut next = v.clone();
    next["format_version"] = json!("2.0");
    std::fs::write(&path, serde_json::to_vec(&next).unwrap()).unwrap();
    let o = run(&["render", "-i", &s(&demo_dsn()), "--from-report", &s(&path), "-o", &s(&d.join("o2.png"))]);
    assert_eq!(code(&o), 3);
    assert!(stderr(&o).contains("not supported") && stderr(&o).contains("1.x"), "{}", stderr(&o));
    // the published example documents stay valid
    for (example, schema) in [("inspect-1.0.example.json", "inspect-1.0.schema.json"), ("manifest-1.0.example.json", "manifest-1.0.schema.json")] {
        let p = manifest_dir().join("../../docs/examples").join(example);
        assert_valid(schema, &json(&p));
    }
    let example = json(&manifest_dir().join("../../docs/examples/inspect-1.0.example.json"));
    assert_eq!(example["format_version"], "1.0");
}

#[test]
fn millimetres_convert_to_pixels_and_back_through_the_manifest() {
    // draw the board and read the pixel at the centre of a pad, found through inspect.json and
    // converted with nothing but the numbers of the manifest
    let d = tmp("roundtrip");
    let (png, manifest) = (d.join("b.png"), d.join("manifest.json"));
    let o = run(&["render", "-i", &s(&demo_dsn()), "-o", &s(&png), "--manifest", &s(&manifest), "--px-per-mm", "40", "--annotate", "none", "--no-airlines", "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let (_, v) = inspect(&[]);
    let m = json(&manifest);
    let t = &m["images"][0]["transform"];
    let apply = |k: &str, x: f64| t[k]["scale"].as_f64().unwrap() * x + t[k]["offset"].as_f64().unwrap();
    let (w, h, px) = png_pixels(&png);
    assert_eq!((w as u64, h as u64), (m["images"][0]["width_px"].as_u64().unwrap(), m["images"][0]["height_px"].as_u64().unwrap()));
    let at = |u: f64, v: f64| {
        let i = ((v.floor() as u32 * w + u.floor() as u32) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    };
    // pad U1-3 (a 1.0 x 0.6 mm SMD pad on F.Cu, drawn light red)
    let pad = v["pads"].as_array().unwrap().iter().find(|p| p["component"] == "U1" && p["pin"] == "3").unwrap();
    let (x, y) = (pad["x"].as_f64().unwrap(), pad["y"].as_f64().unwrap());
    let (u, vv) = (apply("world_to_pixel_u", x), apply("world_to_pixel_v", y));
    assert_eq!(at(u, vv), [255, 135, 135], "pad centre at ({x}, {y}) mm -> pixel ({u}, {vv})");
    // 0.4 mm right of the centre is still the pad (half width 0.5), 0.6 mm is the background
    assert_eq!(at(apply("world_to_pixel_u", x + 0.4), vv), [255, 135, 135]);
    assert_eq!(at(apply("world_to_pixel_u", x + 0.6), vv), [14, 17, 22]);
    // and the other way: the pixel of the pad centre maps back to its millimetres
    let back_x = t["pixel_to_world_x"]["scale"].as_f64().unwrap() * u + t["pixel_to_world_x"]["offset"].as_f64().unwrap();
    let back_y = t["pixel_to_world_y"]["scale"].as_f64().unwrap() * vv + t["pixel_to_world_y"]["offset"].as_f64().unwrap();
    assert!((back_x - x).abs() < 1e-9 && (back_y - y).abs() < 1e-9);
    // the via of SIG at (8.73, 6.0) is drawn as the light grey ring
    let via = apply("world_to_pixel_u", 8.73 + 0.25);
    assert_eq!(at(via, apply("world_to_pixel_v", 6.0)), [215, 215, 215]);
}

#[test]
fn the_published_example_is_what_the_tool_writes() {
    // docs/examples is self-contained: the design, the session of a route run, and what inspect
    // wrote for them. If the format or the numbers change, this fails until the example (and the
    // format description) are updated.
    let dir = manifest_dir().join("../../docs/examples");
    let d = tmp("example");
    let out = d.join("inspect.json");
    let o = run_in(&dir, &["inspect", "-i", "render_demo.dsn", "-s", "render_demo.ses", "-o", &s(&out), "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let (mut now, mut committed) = (json(&out), json(&dir.join("inspect-1.0.example.json")));
    for v in [&mut now, &mut committed] {
        v["fastroute_version"] = json!("x");
    }
    assert_eq!(now, committed, "docs/examples/inspect-1.0.example.json is out of date");
    // the image files that the example manifest names exist and have the stated size
    let manifest = json(&dir.join("manifest-1.0.example.json"));
    for img in manifest["images"].as_array().unwrap() {
        let (w, h) = png_size(&dir.join(img["file"].as_str().unwrap()));
        assert_eq!((w as u64, h as u64), (img["width_px"].as_u64().unwrap(), img["height_px"].as_u64().unwrap()), "{}", img["file"]);
        assert_eq!(std::fs::metadata(dir.join(img["file"].as_str().unwrap())).unwrap().len(), img["bytes"].as_u64().unwrap());
    }
}
