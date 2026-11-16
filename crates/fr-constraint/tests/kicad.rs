//! The read-only KiCad reader and the fastroute export.

mod common;
use common::*;
use fr_constraint::export::export_fastroute;
use fr_constraint::kicad::{parse_sexpr, read_pcb, Sx};
use fr_constraint::snapshot::Snapshot;
use serde_json::{json, Value};
use std::path::PathBuf;

fn board_text() -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/fm100-demo.kicad_pcb")).unwrap()
}

fn board() -> Snapshot {
    read_pcb(&board_text(), "fm100-demo").unwrap()
}

#[test]
fn the_sexpression_parser_handles_strings_with_parentheses_and_escapes() {
    let t = parse_sexpr(r#"(a "b (c) \"d\"" (e 1.5 -2) f)"#).unwrap();
    let Sx::List(items) = &t else { panic!() };
    assert_eq!(items.len(), 4);
    assert_eq!(items[1], Sx::Str("b (c) \"d\"".into()));
    assert!(parse_sexpr("(a (b)").is_err());
    assert!(parse_sexpr("(a \"b)").is_err());
    assert!(parse_sexpr("").is_err());
}

#[test]
fn a_file_that_is_not_a_board_is_refused() {
    assert!(read_pcb("(kicad_sch (version 1))", "x").unwrap_err().contains("not a KiCad board"));
    assert!(read_pcb("hello", "x").is_err());
}

#[test]
fn the_fixture_is_read_with_absolute_pad_positions_nets_and_kinds() {
    let s = board();
    assert_eq!(s.project.source_kind.as_deref(), Some("kicad_pcb"));
    assert_eq!(s.project.source_sha256.as_ref().unwrap().len(), 64);
    let refs: Vec<&str> = s.components.iter().map(|c| c.reference.as_str()).collect();
    assert_eq!(refs, vec!["U1", "C1", "C2", "R1", "C3", "SW1", "J1", "H1"], "the order of the file");
    let u1 = s.component("U1").unwrap();
    assert_eq!((u1.device.as_deref(), u1.rotation_deg, u1.kind.as_deref()), (Some("DEV-FM100"), 90.0, Some("ic")));
    // pad 1 is at (-6,-5) in the module's frame, which is turned 90 degrees counter-clockwise on the
    // screen (y down) and sits at (30,28): the same as pcbnew reports
    let p1 = s.pin("U1", "1").unwrap();
    assert!((p1.x - 25.0).abs() < 1e-9 && (p1.y - 34.0).abs() < 1e-9, "{p1:?}");
    assert_eq!(p1.net.as_deref(), Some("GND"));
    assert_eq!(s.pin("U1", "10").unwrap().net, None, "a pad without a net");
    assert_eq!(s.component("C1").map(Snapshot::kind_of).as_deref(), Some("capacitor"));
    assert_eq!(s.mount_holes.len(), 1);
    assert_eq!((s.mount_holes[0].x, s.mount_holes[0].y, s.mount_holes[0].radius_mm), (3.0, 3.0, 1.6));
    assert!(s.pins.iter().all(|p| p.reference != "H1"), "a mounting hole is not a pin");
    assert_eq!((s.tracks.len(), s.vias.len(), s.zones.len()), (6, 1, 0));
    assert_eq!(s.tracks[0].net.as_deref(), Some("USB_DP"));
    assert_eq!(s.outline.len(), 4);
}

#[test]
fn the_reader_is_deterministic() {
    assert_eq!(serde_json::to_string(&board()).unwrap(), serde_json::to_string(&board()).unwrap());
}

#[test]
fn the_net_syntax_of_every_kicad_version_is_understood() {
    // KiCad 6-9: (net 3 "GND") with a table; KiCad 10: (net "GND") without numbers
    let old = r#"(kicad_pcb (version 20240108) (net 0 "") (net 3 "GND")
      (footprint "X:Y" (layer "F.Cu") (at 1 2) (property "Reference" "U1") (property "Value" "V")
        (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net 3 "GND")) (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu")))
      (segment (start 0 0) (end 1 1) (width 0.2) (layer "F.Cu") (net 3)))"#;
    let new = r#"(kicad_pcb (version 20260101)
      (footprint "X:Y" (layer "F.Cu") (at 1 2) (property "Reference" "U1") (property "Value" "V")
        (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "GND")) (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu")))
      (segment (start 0 0) (end 1 1) (width 0.2) (layer "F.Cu") (net "GND")))"#;
    for (name, text) in [("old", old), ("new", new)] {
        let s = read_pcb(text, name).unwrap();
        assert_eq!(s.pin("U1", "1").unwrap().net.as_deref(), Some("GND"), "{name}");
        assert_eq!(s.pin("U1", "2").unwrap().net, None, "{name}");
        assert_eq!(s.tracks[0].net.as_deref(), Some("GND"), "{name}");
    }
}

#[test]
fn the_older_module_syntax_and_text_items_are_read() {
    let text = r#"(kicad_pcb (version 20171130) (net 0 "") (net 1 "A")
      (module "Lib:Part" (layer F.Cu) (tedit 5) (tstamp 5A) (at 10 10 180)
        (fp_text reference "R7" (at 0 0) (layer F.SilkS)) (fp_text value "10k" (at 0 1) (layer F.Fab))
        (pad 1 smd rect (at -1 0) (size 1 1) (layers F.Cu) (net 1 "A"))))"#;
    let s = read_pcb(text, "old").unwrap();
    let c = s.component("R7").unwrap();
    assert_eq!((c.value.as_deref(), c.kind.as_deref(), c.rotation_deg), (Some("10k"), Some("resistor"), 180.0));
    let p = s.pin("R7", "1").unwrap();
    // (-1,0) turned by 180 degrees is (1,0)
    assert!((p.x - 11.0).abs() < 1e-9 && (p.y - 10.0).abs() < 1e-9, "{p:?}");
}

#[test]
fn arcs_zones_keepouts_and_the_outline_are_read_and_the_approximations_are_the_documented_ones() {
    let text = r#"(kicad_pcb (version 20240108) (net 0 "") (net 1 "GND")
      (gr_rect (start 0 0) (end 10 5) (stroke (width 0.1) (type default)) (fill none) (layer "Edge.Cuts"))
      (arc (start 0 0) (mid 1 1) (end 2 0) (width 0.3) (layer "B.Cu") (net 1))
      (zone (net 1) (net_name "GND") (layers "F&B.Cu") (polygon (pts (xy 0 0) (xy 4 0) (xy 4 4) (xy 0 4))))
      (zone (net 0) (net_name "") (layers "*.Cu") (keepout (tracks not_allowed) (vias not_allowed)) (polygon (pts (xy 6 0) (xy 8 0) (xy 8 2))))
    )"#;
    let s = read_pcb(text, "x").unwrap();
    assert_eq!(s.outline.len(), 4);
    assert_eq!(s.tracks.len(), 2, "an arc is two chords: start-mid and mid-end");
    assert_eq!((s.tracks[0].x2, s.tracks[0].y2), (1.0, 1.0));
    assert_eq!(s.zones.iter().filter(|z| !z.keepout).map(|z| z.layer.as_str()).collect::<Vec<_>>(), vec!["F.Cu", "B.Cu"]);
    let k = s.zones.iter().find(|z| z.keepout).unwrap();
    assert_eq!((k.layer.as_str(), k.net.clone()), ("all", None));
    // an outline that does not close falls back to the bounding box of the Edge.Cuts items
    let open = r#"(kicad_pcb (version 20240108)
      (gr_line (start 0 0) (end 10 0) (layer "Edge.Cuts")) (gr_line (start 10 0) (end 10 5) (layer "Edge.Cuts")))"#;
    let s = read_pcb(open, "x").unwrap();
    assert_eq!(s.outline, vec![[0.0, 0.0], [10.0, 0.0], [10.0, 5.0], [0.0, 5.0]]);
}

#[test]
fn the_legal_board_is_valid_against_the_example_pack() {
    let v = fr_constraint::validate::validate(&ir(), &board(), "fm100-demo", None).unwrap();
    let failed: Vec<_> = v.findings.iter().filter(|f| f.verdict == fr_constraint::validate::Verdict::Fail).collect();
    assert_eq!(v.result, "pass", "{failed:#?}");
    // no assignments in a board file: the pin-function rules are not checked, and say so
    assert!(v.findings.iter().any(|f| f.rule_id == "PINMUX-FM100" && f.verdict == fr_constraint::validate::Verdict::Unchecked));
    assert!(v.findings.iter().any(|f| f.rule_id == "C-NEED-POWER" && f.verdict == fr_constraint::validate::Verdict::Pass));
    assert!(v.findings.iter().any(|f| f.rule_id == "C-ANT" && f.verdict == fr_constraint::validate::Verdict::Pass));
}

// ---- the export -----------------------------------------------------------------------------

#[test]
fn the_export_hands_over_what_fastroute_can_execute_and_lists_the_rest() {
    let e = export_fastroute(&ir(), Some(&board())).unwrap();
    // the USB pair: skew from the rule, no gap (that rule is unknown and never exported)
    let pairs = e.pairs.expect("a pairs file");
    assert!(pairs.contains("pair USB_DP USB_DM skew=0.5") && !pairs.contains("gap="), "{pairs}");
    assert_eq!(e.route_args.as_deref(), Some("--pairs=pairs.txt\n"));
    assert!(e.tune.is_none() && e.dru.is_none());
    let r = &e.report;
    let not: Vec<(&str, &str)> = r["not_exported"].as_array().unwrap().iter().map(|x| (x["rule_id"].as_str().unwrap(), x["status"].as_str().unwrap())).collect();
    assert!(not.contains(&("C-USB-GAP", "unknown")), "{not:?}");
    let ant = r["not_exported"].as_array().unwrap().iter().find(|x| x["rule_id"] == "C-ANT").unwrap();
    assert!(ant["reason"].as_str().unwrap().contains("keepouts come from the design"), "{ant}");
    let unmapped: Vec<(&str, &str)> = r["unmapped_params"].as_array().unwrap().iter().map(|x| (x["rule_id"].as_str().unwrap(), x["param"].as_str().unwrap())).collect();
    assert!(unmapped.contains(&("C-USB", "impedance_ohm")), "{unmapped:?}");
    assert_eq!(r["conditions_not_evaluated"][0]["rule_id"], "C-USB");
    let design_only: Vec<&str> = r["checked_by_the_validator_not_the_router"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(design_only.contains(&"C-RESERVED") && design_only.contains(&"C-NEED-POWER"), "{design_only:?}");
    // an unknown or conflicting rule is never handed to the router, whatever it says
    assert!(!pairs.contains("C-USB-GAP"));
}

#[test]
fn clearance_width_and_length_matching_rules_become_inputs_and_unsupported_kinds_are_reported() {
    let mut p = example();
    p["rules"].as_array_mut().unwrap().extend([
        json!({"id": "C-CLR", "kind": "clearance", "modifiability": "configurable", "params": {"clearance_mm": 0.15},
               "evidence": [{"source_id": "SRC-FM100-HDG", "locator": {"section": "5"}, "quote": "0.15 mm", "basis": "explicit"}]}),
        json!({"id": "C-W", "kind": "trace_width", "modifiability": "configurable", "params": {"width_mm": 0.4, "nets": ["3V3", "GND*"]},
               "evidence": [{"source_id": "SRC-FM100-HDG", "locator": {"section": "5"}, "quote": "wide supply", "basis": "explicit"}]}),
        json!({"id": "C-LM", "kind": "high_speed", "modifiability": "configurable", "params": {"nets": ["TXD", "RXD"], "differential": false, "length_match_tolerance_mm": 1.5, "max_length_mm": 80},
               "evidence": [{"source_id": "SRC-FM100-HDG", "locator": {"section": "5"}, "quote": "match TX and RX", "basis": "explicit"}]}),
        json!({"id": "C-SUP", "kind": "electrical", "modifiability": "fixed", "subject": {"device": "DEV-FM100"}, "params": {"quantity": "supply", "condition": "normal", "min_v": 3.0},
               "evidence": [{"source_id": "SRC-FM100-DS", "locator": {"page": 9}, "quote": "3.0 V", "basis": "explicit"}]}),
    ]);
    let ir = fr_constraint::compile::compile(&p).unwrap();
    // without the design a pattern cannot be resolved: said, and the exact name is still exported
    let e = export_fastroute(&ir, None).unwrap();
    let dru = e.dru.clone().unwrap();
    assert!(dru.starts_with("(version 1)\n"));
    assert!(dru.contains("(rule \"C-CLR\"\n  (constraint clearance (min 0.15mm)))"), "{dru}");
    assert!(dru.contains("(rule \"C-W\"\n  (condition \"A.NetName == '3V3'\")\n  (constraint track_width (min 0.4mm) (opt 0.4mm)))"), "{dru}");
    assert!(e.report["unmapped_params"].to_string().contains("GND*") && e.report["unmapped_params"].to_string().contains("--design"));
    // with the design the pattern is resolved to the nets that exist
    let e2 = export_fastroute(&ir, Some(&board())).unwrap();
    assert!(e2.dru.unwrap().contains("A.NetName == '3V3' || A.NetName == 'GND'"));
    let tune = e2.tune.unwrap();
    assert!(tune.contains("group C_LM tolerance=1.5\n  TXD\n  RXD"), "{tune}");
    // a maximum length is the router's blind spot: said
    assert!(e2.report["unmapped_params"].to_string().contains("max_length_mm"));
    // an electrical limit has no router form: listed with its reason
    assert!(e2.report["checked_by_the_validator_not_the_router"].to_string().contains("C-SUP") || e2.report["not_exported"].to_string().contains("C-SUP"));
    assert_eq!(e2.route_args.as_deref(), Some("--pairs=pairs.txt\n--tune=tune.txt\n"));
}

#[test]
fn the_export_refuses_an_ir_that_was_edited_and_names_that_are_not_safe() {
    let mut edited = ir();
    edited.rules[0].severity = "warning".into();
    assert!(export_fastroute(&edited, None).unwrap_err().contains("integrity"));
    let mut p = example();
    p["rules"].as_array_mut().unwrap().push(json!({
        "id": "C-BAD", "kind": "trace_width", "modifiability": "configurable", "params": {"width_mm": 0.4, "nets": ["A'B"]},
        "evidence": [{"source_id": "SRC-FM100-HDG", "locator": {"section": "5"}, "quote": "x", "basis": "explicit"}]
    }));
    let e = export_fastroute(&fr_constraint::compile::compile(&p).unwrap(), None).unwrap();
    assert!(e.dru.is_none(), "an unsafe net name is never written into a rule file");
    assert!(e.report["not_exported"].to_string().contains("cannot be written into a fastroute input safely"));
}
