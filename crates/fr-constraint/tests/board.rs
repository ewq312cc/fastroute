//! A development board embedded in a carrier as a fixed module: the board pack (a fictional "Acme DevBoard" with two headers, an
//! antenna that overhangs, a reset and a boot button, one free sensor block), the request, the carrier pack that comes out, and the edit
//! boundary of #17 applied to it. Positive and negative cases; nothing here uses a network, a model or an index.

use fr_constraint::board::{self, report};
use fr_constraint::compile;
use fr_constraint::diag::Severity;
use fr_constraint::edit::{self, BaseFile};
use fr_constraint::hcp;
use fr_constraint::resolve::{canonical_sha256, resolve};
use fr_constraint::snapshot::Snapshot;
use serde_json::{json, Value};

const DEV: &str = "DEV-DEVBOARD";
const ID: &str = "acme/devs/devboard/rev-b";

fn ev(page: u32, quote: &str) -> Value {
    json!([{"source_id": "SRC-DB-UG", "locator": {"page": page}, "quote": quote, "basis": "explicit"}])
}

fn pin(h: &str, n: u32, name: &str, kind: &str, use_: &str, reason: Option<&str>, x: f64, y: f64) -> Value {
    let mut p = json!({"number": n.to_string(), "name": name, "position": [x, y], "kind": kind, "usability": use_});
    match reason {
        Some(r) => p["reason"] = json!(r),
        None => p["evidence"] = ev(3, &format!("{h} pin {n} is {name}")),
    }
    p
}

fn dev_pin(h: &str, n: u32, name: &str, class: &str) -> Value {
    json!({"number": format!("{h}.{n}"), "name": name, "class": class})
}

/// The board pack, flat, in the board's own frame: 20 mm x 60 mm, the antenna keep-out sticks out of the top end.
fn board() -> Value {
    json!({
        "hcp_version": "1.1",
        "pack": {"id": ID, "kind": "board", "title": "Acme DevBoard rev B (fictional)", "version": "0.1.0", "created": "2026-11-16", "language": "en", "extractor": {"kind": "human"}, "review": {"status": "draft"}},
        "sources": [{"source_id": "SRC-DB-UG", "document_id": "DOC-DB-UG", "title": "DevBoard user guide (fictional)", "version": "v1.1", "level": "official_devkit", "url": "https://example.invalid/ug",
                     "retrieved": "2026-11-16", "sha256": "0".repeat(64), "applies_to": [DEV]}],
        "devices": [{"id": DEV, "vendor": "Acme", "part_number": "DEVBOARD-B", "revision": "B", "package": "development board", "pins": [
            dev_pin("J1", 1, "3V3", "power"), dev_pin("J1", 2, "GND", "ground"), dev_pin("J1", 3, "RST", "reset"), dev_pin("J1", 4, "GPIO4", "gpio"), dev_pin("J1", 5, "GPIO5", "gpio"), dev_pin("J1", 6, "GPIO6", "reserved"),
            dev_pin("J3", 1, "GND", "ground"), dev_pin("J3", 2, "TX", "uart"), dev_pin("J3", 3, "RX", "uart"), dev_pin("J3", 4, "GPIO0", "boot"), dev_pin("J3", 5, "GPIO7", "gpio")
        ]}],
        "peripheral_blocks": [
            {"id": "BLK-ONBOARD-RESET", "title": "Reset button and RC on the board", "kind": "reset_circuit", "modifiability": "fixed", "device": DEV, "evidence": ev(4, "The Reset button restarts the system.")},
            {"id": "BLK-ONBOARD-BOOT", "title": "Boot button on the board", "kind": "boot_button", "modifiability": "fixed", "device": DEV, "evidence": ev(4, "Holding down Boot and pressing Reset starts download mode.")},
            {"id": "BLK-SENSOR", "title": "A sensor on the carrier", "kind": "generic", "modifiability": "ai_editable", "components": [{"role": "sensor", "kind": "ic", "min_count": 1}],
             "evidence": [{"source_id": "SRC-DB-UG", "locator": {"page": 5}, "quote": "Any peripheral may be connected to the free GPIO pins.", "basis": "observed"}]}
        ],
        "rules": [
            {"id": "C-DB-RSVD", "kind": "reserved_pin", "modifiability": "fixed", "subject": {"device": DEV}, "params": {"pins": ["J1.6"]}, "evidence": ev(3, "GPIO6 is used by the on-board flash.")}
        ],
        "regions": [
            {"id": "REG-DB-OUTLINE", "kind": "board_outline", "unit": "mm", "frame": "device", "device": DEV, "polygon": [[0, 0], [20, 0], [20, 60], [0, 60]], "modifiability": "fixed", "evidence": ev(2, "Board size 20 mm x 60 mm.")},
            {"id": "REG-DB-ANT", "kind": "antenna_keepout", "unit": "mm", "frame": "device", "device": DEV, "polygon": [[2, 60], [18, 60], [18, 64], [2, 64]], "modifiability": "fixed", "evidence": ev(2, "The antenna end must stay clear.")}
        ],
        "board_integration": {
            "hardware_revision": "B", "board_device": DEV,
            "frame": {"unit": "mm", "origin": "lower-left corner of the board outline, top view", "orientation": "x to the right, y up; the antenna end is at +y"},
            "exposed_headers": [
                {"id": "J1", "pitch_mm": 2.54, "pins": [
                    pin("J1", 1, "3V3", "power", "reserved", Some("power rail of the board"), 1.27, 58.0), pin("J1", 2, "GND", "ground", "reserved", Some("ground"), 1.27, 55.46),
                    pin("J1", 3, "RST", "reset", "onboard", Some("the reset button and its RC are on the board"), 1.27, 52.92), pin("J1", 4, "GPIO4", "gpio", "usable", None, 1.27, 50.38),
                    pin("J1", 5, "GPIO5", "gpio", "usable", None, 1.27, 47.84), pin("J1", 6, "GPIO6", "gpio", "reserved", Some("used by the on-board flash"), 1.27, 45.3)]},
                {"id": "J3", "pitch_mm": 2.54, "pins": [
                    pin("J3", 1, "GND", "ground", "reserved", Some("ground"), 18.73, 58.0), pin("J3", 2, "TX", "uart", "onboard", Some("connected to the USB-UART bridge"), 18.73, 55.46),
                    pin("J3", 3, "RX", "uart", "onboard", Some("connected to the USB-UART bridge"), 18.73, 52.92), pin("J3", 4, "GPIO0", "boot", "onboard", Some("the boot button is on the board"), 18.73, 50.38),
                    pin("J3", 5, "GPIO7", "gpio", "unknown", Some("the documents do not say whether it is free"), 18.73, 47.84)]}
            ],
            "onboard_blocks": ["BLK-ONBOARD-RESET", "BLK-ONBOARD-BOOT"],
            "carrier": {"allowed_blocks": ["BLK-SENSOR"], "allowed_ops": ["add_component", "connect", "add_track"], "margin_mm": 0}
        }
    })
}

fn request(b: &Value, rot: i64, at: [f64; 2]) -> Value {
    json!({
        "request_version": "1.0",
        "board": {"id": ID, "version": "0.1.0", "hardware_revision": "B", "canonical_sha256": canonical_sha256(b)},
        "carrier": {"name": "myboard", "outline": {"x_mm": 0, "y_mm": 0, "width_mm": 100, "height_mm": 100}},
        "placement": {"ref": "U1", "origin_mm": at, "rotation_deg": rot},
        "targets": [{"name": "temperature sensor", "block": "BLK-SENSOR", "pins": ["J1.4", "J1.5"]}]
    })
}

fn embed(rot: i64) -> board::Embedding {
    let b = board();
    board::embed(&b, &canonical_sha256(&b), &request(&b, rot, [20.0, 10.0])).unwrap_or_else(|e| panic!("{e:#?}"))
}

fn codes(r: Result<board::Embedding, Vec<fr_constraint::diag::Diagnostic>>) -> Vec<String> {
    r.err().expect("must fail").into_iter().map(|d| d.code).collect()
}

fn region<'a>(e: &'a board::Embedding, id: &str) -> &'a Value {
    e.carrier_pack["regions"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap_or_else(|| panic!("no region {id}"))
}

fn area(poly: &Value) -> f64 {
    let p: Vec<(f64, f64)> = poly.as_array().unwrap().iter().map(|v| (v[0].as_f64().unwrap(), v[1].as_f64().unwrap())).collect();
    (0..p.len()).map(|i| p[i].0 * p[(i + 1) % p.len()].1 - p[(i + 1) % p.len()].0 * p[i].1).sum::<f64>().abs() / 2.0
}

// ---- the board pack ----------------------------------------------------------------------------------------------------------

#[test]
fn a_board_pack_is_valid_and_its_checks_find_what_is_wrong() {
    let b = board();
    assert!(hcp::check(&b).iter().all(|d| d.severity != Severity::Error), "{:#?}", hcp::check(&b));
    let bad = |f: &dyn Fn(&mut Value)| {
        let mut p = board();
        f(&mut p);
        hcp::check(&p).into_iter().filter(|d| d.severity == Severity::Error).map(|d| d.code).collect::<Vec<_>>()
    };
    // a pin of the chip that the board does not bring out is not a pin of the board
    assert!(bad(&|p| p["devices"][0]["pins"].as_array_mut().unwrap().push(dev_pin("J9", 1, "GPIO37", "gpio"))).contains(&"device_pin_not_exposed".to_string()));
    // a header pin that is not a pin of the board device
    assert!(bad(&|p| p["devices"][0]["pins"].as_array_mut().unwrap().retain(|x| x["number"] != "J1.4")).contains(&"header_pin_not_in_device".to_string()));
    // a pin that may be used needs evidence; one that may not needs a reason
    assert!(bad(&|p| { p["board_integration"]["exposed_headers"][0]["pins"][3].as_object_mut().unwrap().remove("evidence"); }).contains(&"schema".to_string()));
    assert!(bad(&|p| { p["board_integration"]["exposed_headers"][0]["pins"][0].as_object_mut().unwrap().remove("reason"); }).contains(&"schema".to_string()));
    // the board needs one outline in its own frame
    assert!(bad(&|p| p["regions"].as_array_mut().unwrap().retain(|r| r["kind"] != "board_outline")).contains(&"board_outline_missing".to_string()));
    // what is on the board is not added again on the carrier
    assert!(bad(&|p| p["board_integration"]["carrier"]["allowed_blocks"] = json!(["BLK-SENSOR", "BLK-ONBOARD-BOOT"])).iter().any(|c| c == "onboard_block_allowed_on_carrier" || c == "carrier_block_not_editable"));
    // only a pack of kind board
    assert!(bad(&|p| p["pack"]["kind"] = json!("module")).contains(&"board_integration_needs_board_kind".to_string()));
    // version 1.1
    assert!(bad(&|p| p["hcp_version"] = json!("1.0")).contains(&"needs_version_1_1".to_string()));
}

#[test]
fn a_pin_that_the_board_says_is_usable_cannot_be_one_the_module_protects() {
    // the module pack protects its pin 7; the board wires header pin J1.5 to it and says it may be used: the packs disagree
    let module = json!({
        "hcp_version": "1.1",
        "pack": {"id": "acme/devs/mod/a", "kind": "module", "title": "module", "version": "0.1.0", "created": "2026-11-16", "extractor": {"kind": "human"}},
        "sources": [{"source_id": "SRC-MOD-DS", "document_id": "DOC-MOD-DS", "title": "module ds", "version": "v1", "level": "official_manual", "url": "https://example.invalid/m", "retrieved": "2026-11-16", "sha256": "0".repeat(64), "applies_to": ["DEV-MOD"]}],
        "devices": [{"id": "DEV-MOD", "vendor": "Acme", "part_number": "MOD", "package": "module", "pins": [{"number": "7", "name": "IO7", "class": "gpio"}]}],
        "rules": [{"id": "C-MOD-RSVD", "kind": "reserved_pin", "modifiability": "fixed", "subject": {"device": "DEV-MOD"}, "params": {"pins": ["7"]},
                   "evidence": [{"source_id": "SRC-MOD-DS", "locator": {"page": 9}, "quote": "IO7 is used by the PSRAM.", "basis": "explicit"}]}]
    });
    let mut b = board();
    b["pack"]["upstream"] = json!([{"id": "acme/devs/mod/a", "version": "0.1.0", "kind": "module", "canonical_sha256": canonical_sha256(&module), "inherit": {"devices": ["DEV-MOD"], "rules": ["C-MOD-RSVD"]}}]);
    b["board_integration"]["upstream_device"] = json!("DEV-MOD");
    b["board_integration"]["exposed_headers"][0]["pins"][4]["module_pin"] = json!("7");
    let e = resolve(&b, std::slice::from_ref(&module)).expect_err("the two packs disagree");
    assert!(e.iter().any(|d| d.code == "board_pin_conflicts_with_upstream" && d.object.as_deref() == Some("C-MOD-RSVD")), "{e:#?}");
    // marked reserved with the module's evidence behind it, it resolves, and the pin is not in the pool
    b["board_integration"]["exposed_headers"][0]["pins"][4] = pin("J1", 5, "GPIO5", "gpio", "reserved", Some("wired to IO7 of the module, which the module datasheet reserves"), 1.27, 47.84);
    b["board_integration"]["exposed_headers"][0]["pins"][4]["module_pin"] = json!("7");
    let flat = resolve(&b, &[module]).expect("consistent");
    // the request names the board pack as it is stored (its own document, with `upstream`), the embedding works on the merged one
    let e = board::embed(&flat, &canonical_sha256(&b), &request(&b, 0, [20.0, 10.0])).expect("embeds");
    assert_eq!(e.result["pin_pool"], json!(["J1.4"]), "the pin that is wired to a protected pin of the module is not in the pool");
    assert_eq!(codes(board::embed(&flat, &canonical_sha256(&flat), &request(&b, 0, [20.0, 10.0]))), ["board_pack_changed"], "the merged document is not the pack the request names");
}

// ---- the embedding -----------------------------------------------------------------------------------------------------------

#[test]
fn the_carrier_pack_is_valid_compiles_and_carries_the_whitelist() {
    let e = embed(0);
    let d = hcp::check(&e.carrier_pack);
    assert!(d.iter().all(|x| x.severity != Severity::Error), "{d:#?}");
    let ir = compile::compile(&e.carrier_pack).expect("the carrier pack compiles");
    let a = edit::authorization(&ir).expect("it has a base design");
    // only the header pins that are usable are in the pool: nothing of the chip that is not brought out, nothing that is reserved or on the board
    assert!(a.regions.len() >= 3 && a.regions.iter().all(|r| r.id.starts_with("REG-CARRIER-FREE-")));
    for r in &a.regions {
        assert_eq!(r.pin_pool[DEV], vec!["J1.4".to_string(), "J1.5".to_string()], "{r:?}");
        assert_eq!(r.allowed_blocks, vec!["BLK-SENSOR".to_string()]);
        assert_eq!(r.allowed_ops, vec!["add_component", "connect", "add_track"]);
    }
    assert_eq!(e.result["pin_pool"], json!(["J1.4", "J1.5"]));
    let reasons: std::collections::BTreeMap<&str, &str> = e.result["excluded_pins"].as_array().unwrap().iter().map(|p| (p["pin"].as_str().unwrap(), p["usability"].as_str().unwrap())).collect();
    assert_eq!((reasons["J1.1"], reasons["J1.3"], reasons["J1.6"], reasons["J3.2"], reasons["J3.4"], reasons["J3.5"]), ("reserved", "onboard", "reserved", "onboard", "onboard", "unknown"));
    assert!(a.immutable_components.contains("U1"));
    assert!(a.fixed_regions.iter().any(|g| g.id == "REG-BOARD-BODY") && a.fixed_regions.iter().any(|g| g.id == "REG-DB-ANT"), "the body and the antenna keep-out are fixed");
    // the on-board blocks are in the carrier pack as fixed, the carrier block as ai_editable
    let mods: Vec<(&str, &str)> = e.carrier_pack["peripheral_blocks"].as_array().unwrap().iter().map(|b| (b["id"].as_str().unwrap(), b["modifiability"].as_str().unwrap())).collect();
    assert_eq!(mods, [("BLK-ONBOARD-RESET", "fixed"), ("BLK-ONBOARD-BOOT", "fixed"), ("BLK-SENSOR", "ai_editable")]);
    // nothing about pins that the board does not bring out
    let dev_pins: Vec<&str> = e.carrier_pack["devices"][0]["pins"].as_array().unwrap().iter().map(|p| p["number"].as_str().unwrap()).collect();
    assert_eq!(dev_pins.len(), 11);
    assert!(dev_pins.iter().all(|p| p.starts_with("J1.") || p.starts_with("J3.")));
    // the request an AI gets is computed from it, and the result validates against the schema of an edit request
    let q = edit::edit_request(&ir, "add a temperature sensor").unwrap();
    assert!(fr_constraint::jsonschema::validate(&edit::request_schema(), &q).is_empty());
}

#[test]
fn the_free_area_is_exactly_the_carrier_without_the_body_and_the_keepouts() {
    for (rot, at) in [(0, [20.0, 10.0]), (90, [80.0, 15.0]), (180, [70.0, 85.0]), (270, [15.0, 85.0])] {
        let b = board();
        let e = board::embed(&b, &canonical_sha256(&b), &request(&b, rot, at)).unwrap_or_else(|x| panic!("rot {rot}: {x:#?}"));
        let free: Vec<&Value> = e.carrier_pack["regions"].as_array().unwrap().iter().filter(|r| r["kind"] == "editable").collect();
        let total: f64 = free.iter().map(|r| area(&r["polygon"])).sum();
        let (body, ant) = (area(&region(&e, "REG-BOARD-BODY")["polygon"]), area(&region(&e, "REG-DB-ANT")["polygon"]));
        let carrier = 100.0 * 100.0;
        // (the keep-out may stick out of the carrier: only its part inside takes area away; here it is inside for every placement above)
        assert!((total - (carrier - body - ant)).abs() < 1e-6, "rot {rot}: free {total}, expected {}", carrier - body - ant);
        // they do not overlap each other, and none touches a fixed region
        for (i, a) in free.iter().enumerate() {
            for c in free.iter().skip(i + 1) {
                assert!(!overlap(&a["polygon"], &c["polygon"]), "rot {rot}: {} and {} overlap", a["id"], c["id"]);
            }
            for fixed in ["REG-BOARD-BODY", "REG-DB-ANT"] {
                assert!(!overlap(&a["polygon"], &region(&e, fixed)["polygon"]), "rot {rot}: {} meets {fixed}", a["id"]);
            }
        }
        // the body has the size of the board, turned
        let (w, h) = if rot % 180 == 0 { (20.0, 60.0) } else { (60.0, 20.0) };
        assert!((body - w * h).abs() < 1e-6);
        // header pins moved with the board: J1.1 was at (1.27, 58)
        let p = &e.result["board_in_carrier"]["headers"][0]["pins"][0]["position_mm"];
        let want = match rot {
            0 => [at[0] + 1.27, at[1] + 58.0],
            90 => [at[0] - 58.0, at[1] + 1.27],
            180 => [at[0] - 1.27, at[1] - 58.0],
            _ => [at[0] + 58.0, at[1] - 1.27],
        };
        assert!((p[0].as_f64().unwrap() - want[0]).abs() < 1e-6 && (p[1].as_f64().unwrap() - want[1]).abs() < 1e-6, "rot {rot}: {p} vs {want:?}");
        // the same pool whatever the placement
        assert_eq!(e.result["pin_pool"], json!(["J1.4", "J1.5"]));
    }
}

fn overlap(a: &Value, b: &Value) -> bool {
    let r = |p: &Value| {
        let pts: Vec<(f64, f64)> = p.as_array().unwrap().iter().map(|v| (v[0].as_f64().unwrap(), v[1].as_f64().unwrap())).collect();
        (pts.iter().map(|p| p.0).fold(f64::MAX, f64::min), pts.iter().map(|p| p.1).fold(f64::MAX, f64::min), pts.iter().map(|p| p.0).fold(f64::MIN, f64::max), pts.iter().map(|p| p.1).fold(f64::MIN, f64::max))
    };
    let (x, y) = (r(a), r(b));
    x.0 < y.2 - 1e-9 && y.0 < x.2 - 1e-9 && x.1 < y.3 - 1e-9 && y.1 < x.3 - 1e-9
}

#[test]
fn the_same_request_gives_the_same_bytes() {
    let (a, b) = (embed(0), embed(0));
    assert_eq!(hcp::canonical_json(&a.carrier_pack), hcp::canonical_json(&b.carrier_pack));
    assert_eq!(hcp::canonical_json(&a.result), hcp::canonical_json(&b.result));
    let (x, y) = (compile::compile(&a.carrier_pack).unwrap(), compile::compile(&b.carrier_pack).unwrap());
    assert_eq!(serde_json::to_string(&x).unwrap(), serde_json::to_string(&y).unwrap());
    assert_eq!(a.result["carrier_pack_canonical_sha256"], json!(canonical_sha256(&a.carrier_pack)));
}

#[test]
fn targets_are_checked_and_grant_nothing() {
    let b = board();
    let mut r = request(&b, 0, [20.0, 10.0]);
    r["targets"] = json!([
        {"name": "ok", "block": "BLK-SENSOR", "pins": ["J1.4"]},
        {"name": "a reserved pin", "pins": ["J1.6"]},
        {"name": "an on-board pin", "pins": ["J3.4", "J1.3"]},
        {"name": "unknown pin", "pins": ["J3.5"]},
        {"name": "a chip pin that is not brought out", "pins": ["J9.1"]},
        {"name": "the boot button again", "block": "BLK-ONBOARD-BOOT"},
        {"name": "a block the pack does not know", "block": "BLK-RADAR"}
    ]);
    let e = board::embed(&b, &canonical_sha256(&b), &r).unwrap();
    let t = |i: usize| (e.result["targets"][i]["status"].as_str().unwrap(), e.result["targets"][i]["reasons"].to_string());
    assert_eq!(t(0).0, "feasible");
    assert!(t(1).0 == "blocked" && t(1).1.contains("used by the on-board flash"), "{:?}", t(1));
    assert!(t(2).0 == "blocked" && t(2).1.contains("boot button") && t(2).1.contains("reset button"), "{:?}", t(2));
    assert!(t(3).0 == "blocked" && t(3).1.contains("do not say"), "{:?}", t(3));
    assert!(t(4).0 == "blocked" && t(4).1.contains("not a pin that the board brings out"), "{:?}", t(4));
    assert!(t(5).0 == "blocked" && t(5).1.contains("implemented by the board itself"), "{:?}", t(5));
    assert!(t(6).0 == "blocked" && t(6).1.contains("not a block that the pack allows"), "{:?}", t(6));
    assert_eq!(e.result["status"], "needs_attention");
    // asking for it does not put anything in the pool
    assert_eq!(e.result["pin_pool"], json!(["J1.4", "J1.5"]));
    assert!(e.result["unknown"].as_array().unwrap().iter().any(|u| u.as_str().unwrap().contains("J3.5")));
}

#[test]
fn a_request_for_the_wrong_board_revision_version_or_content_is_refused() {
    let b = board();
    let dig = canonical_sha256(&b);
    let with = |f: &dyn Fn(&mut Value)| {
        let mut r = request(&b, 0, [20.0, 10.0]);
        f(&mut r);
        codes(board::embed(&b, &dig, &r))
    };
    assert_eq!(with(&|r| r["board"]["hardware_revision"] = json!("A")), ["board_revision_mismatch"]);
    assert_eq!(with(&|r| r["board"]["version"] = json!("0.2.0")), ["board_mismatch"]);
    assert_eq!(with(&|r| r["board"]["id"] = json!("acme/devs/devboard/rev-a")), ["board_mismatch"]);
    assert_eq!(with(&|r| r["board"]["canonical_sha256"] = json!("1".repeat(64))), ["board_pack_changed"]);
    // the pack itself changed after the request was made
    let mut changed = board();
    changed["board_integration"]["exposed_headers"][0]["pins"][3]["usability"] = json!("reserved");
    changed["board_integration"]["exposed_headers"][0]["pins"][3]["reason"] = json!("now it is not");
    assert_eq!(codes(board::embed(&changed, &canonical_sha256(&changed), &request(&b, 0, [20.0, 10.0]))), ["board_pack_changed"]);
    // geometry and shape of the request
    assert_eq!(with(&|r| r["placement"]["rotation_deg"] = json!(45)), ["request_schema"]);
    assert_eq!(with(&|r| r["carrier"]["outline"]["width_mm"] = json!(-5)), ["request_schema"]);
    let off = with(&|r| r["placement"]["origin_mm"] = json!([95.0, 10.0]));
    assert!(!off.is_empty() && off.iter().all(|c| c == "header_outside_carrier"), "the second header lands outside the carrier: {off:?}");
    // not a board pack
    let mut no = board();
    no.as_object_mut().unwrap().remove("board_integration");
    assert_eq!(codes(board::embed(&no, &canonical_sha256(&no), &request(&b, 0, [20.0, 10.0]))), ["no_board_integration"]);
}

// ---- the edit boundary of the carrier ----------------------------------------------------------------------------------------

fn carrier_snapshot(e: &board::Embedding, at: [f64; 2]) -> Snapshot {
    let pins: Vec<Value> = e.result["board_in_carrier"]["headers"].as_array().unwrap().iter().flat_map(|h| h["pins"].as_array().unwrap().iter()).map(|p| json!({"ref": "U1", "pin": p["id"], "x": p["position_mm"][0], "y": p["position_mm"][1]})).collect();
    serde_json::from_value(json!({
        "snapshot_version": "1.0", "project": {"name": "myboard"},
        "components": [{"ref": "U1", "device": DEV, "value": "DevBoard", "x": at[0], "y": at[1], "rotation_deg": 0.0}],
        "pins": pins, "tracks": [], "outline": [[0.0, 0.0], [60.0, 0.0], [60.0, 100.0], [0.0, 100.0]], "mount_holes": []
    }))
    .unwrap()
}

struct Fx {
    e: board::Embedding,
    ir: fr_constraint::ir::Ir,
    base: Snapshot,
}

fn fx() -> Fx {
    let e = embed(0);
    let ir = compile::compile(&e.carrier_pack).unwrap();
    let base = carrier_snapshot(&e, [20.0, 10.0]);
    Fx { e, ir, base }
}

fn files(ir: &fr_constraint::ir::Ir) -> Vec<BaseFile> {
    let a = edit::authorization(ir).unwrap();
    a.binding["files"].as_array().unwrap().iter().map(|f| BaseFile { path: f["path"].as_str().unwrap().into(), sha256: f["sha256"].as_str().map(str::to_string) }).collect()
}

fn check(f: &Fx, changes: Value) -> edit::EditCheck {
    let a = edit::authorization(&f.ir).unwrap();
    let d = json!({"format": "fastroute-hcp-edit-draft", "draft_version": "1.0", "title": "t",
        "pack": {"id": f.ir.pack.id, "version": f.ir.pack.version, "sha256": f.ir.pack.sha256}, "base": {"id": a.base_id, "binding_sha256": a.binding_sha256}, "changes": changes});
    edit::check_edit(&f.ir, &d, Some(&f.base), Some(&files(&f.ir))).unwrap_or_else(|e| panic!("{e:#?}"))
}

fn errs(c: &edit::EditCheck) -> Vec<String> {
    c.findings.iter().filter(|f| f.severity == "error").map(|f| f.code.clone()).collect()
}

/// A free area that is big enough for a part, and a point well inside it
fn free_spot(f: &Fx, nth: usize) -> (String, [f64; 2]) {
    let mut v: Vec<(String, [f64; 2])> = f.e.carrier_pack["regions"].as_array().unwrap().iter().filter(|r| r["kind"] == "editable").map(|r| {
        let p = r["polygon"].as_array().unwrap();
        let (x0, y0, x1, y1) = (p[0][0].as_f64().unwrap(), p[0][1].as_f64().unwrap(), p[2][0].as_f64().unwrap(), p[2][1].as_f64().unwrap());
        (r["id"].as_str().unwrap().to_string(), [(x0 + x1) / 2.0, (y0 + y1) / 2.0])
    }).collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v.swap_remove(nth)
}

#[test]
fn a_legal_extension_of_the_carrier_is_accepted() {
    let f = fx();
    let (r, at) = free_spot(&f, 0);
    let c = check(&f, json!([
        {"id": "CH-1", "op": "add_component", "region": r, "block": "BLK-SENSOR", "role": "sensor", "ref": "S1", "value": "TMP", "at": at},
        {"id": "CH-2", "op": "connect", "region": r, "ref": "S1", "pin": "1", "net": "SDA"},
        {"id": "CH-3", "op": "connect", "region": r, "ref": "U1", "pin": "J1.4", "net": "SDA"},
        {"id": "CH-4", "op": "add_track", "region": r, "net": "SDA", "layer": "F.Cu", "width_mm": 0.2, "points": [at, [at[0] + 1.0, at[1]]]}
    ]));
    assert!(errs(&c).is_empty(), "{:#?}", c.findings);
}

#[test]
fn every_way_of_overreaching_is_rejected() {
    let f = fx();
    let (r0, at0) = free_spot(&f, 0);
    let (r1, at1) = free_spot(&f, 1);
    let body = region(&f.e, "REG-BOARD-BODY")["polygon"].clone();
    let inside_body = [(body[0][0].as_f64().unwrap() + body[2][0].as_f64().unwrap()) / 2.0, (body[0][1].as_f64().unwrap() + body[2][1].as_f64().unwrap()) / 2.0];
    let ant = region(&f.e, "REG-DB-ANT")["polygon"].clone();
    let inside_ant = [(ant[0][0].as_f64().unwrap() + ant[2][0].as_f64().unwrap()) / 2.0, (ant[0][1].as_f64().unwrap() + ant[2][1].as_f64().unwrap()) / 2.0];
    let add = |region: &str, block: &str, at: [f64; 2]| json!({"id": "CH-1", "op": "add_component", "region": region, "block": block, "role": "sensor", "ref": "S1", "value": "X", "at": at});
    let has = |c: &edit::EditCheck, code: &str| errs(c).iter().any(|x| x == code);

    // 1. a pin of the chip that the board does not bring out: not in the pool (the board has no such pin at all)
    let c = check(&f, json!([add(&r0, "BLK-SENSOR", at0), {"id": "CH-2", "op": "connect", "region": r0, "ref": "U1", "pin": "GPIO37", "net": "X"}]));
    assert!(has(&c, "pin_not_in_pool"), "{:#?}", c.findings);
    // a pin of the board that is reserved, on the board, unknown or protected
    for pin in ["J1.6", "J3.4", "J3.5", "J1.3", "J1.1"] {
        let c = check(&f, json!([{"id": "CH-1", "op": "connect", "region": r0, "ref": "U1", "pin": pin, "net": "X"}]));
        assert!(has(&c, "pin_not_in_pool") || has(&c, "reserved_pin_used"), "{pin}: {:#?}", c.findings);
    }
    // 2. the boot button and the reset circuit are on the board: not added again
    for blk in ["BLK-ONBOARD-BOOT", "BLK-ONBOARD-RESET"] {
        let c = check(&f, json!([add(&r0, blk, at0)]));
        assert!(has(&c, "unauthorized_block"), "{blk}: {:#?}", c.findings);
    }
    // 3. the board is a fixed component: not moved or removed
    for ch in [json!({"id": "CH-1", "op": "move_component", "region": r0, "ref": "U1", "to": [30.0, 30.0]}), json!({"id": "CH-1", "op": "remove_component", "region": r0, "ref": "U1"})] {
        let c = check(&f, json!([ch]));
        assert!(!errs(&c).is_empty(), "{ch}: {:#?}", c.findings);
    }
    // 4. into the body of the board, and into the antenna keep-out
    let c = check(&f, json!([add(&r0, "BLK-SENSOR", inside_body)]));
    assert!(has(&c, "outside_region"), "{:#?}", c.findings);
    let c = check(&f, json!([add(&r0, "BLK-SENSOR", inside_ant)]));
    assert!(!errs(&c).is_empty(), "{:#?}", c.findings);
    let c = check(&f, json!([{"id": "CH-1", "op": "add_track", "region": r0, "net": "X", "layer": "F.Cu", "width_mm": 0.2, "points": [at0, inside_ant]}]));
    assert!(!errs(&c).is_empty(), "{:#?}", c.findings);
    let c = check(&f, json!([{"id": "CH-1", "op": "add_track", "region": r0, "net": "X", "layer": "F.Cu", "width_mm": 0.2, "points": [at0, inside_body]}]));
    assert!(!errs(&c).is_empty(), "{:#?}", c.findings);
    // 5. a track from one free area to another, across the board
    let c = check(&f, json!([{"id": "CH-1", "op": "add_track", "region": r0, "net": "X", "layer": "F.Cu", "width_mm": 0.2, "points": [at0, at1]}]));
    assert!(has(&c, "outside_region") || has(&c, "crosses_region"), "{r0} -> {r1}: {:#?}", c.findings);
    // 6. an operation that is not allowed in the area
    let c = check(&f, json!([{"id": "CH-1", "op": "add_via", "region": r0, "net": "X", "at": at0, "diameter_mm": 0.6}]));
    assert!(has(&c, "op_not_allowed"), "{:#?}", c.findings);
    // 7. the outline of the carrier is not an area
    let c = check(&f, json!([add("REG-CARRIER-OUTLINE", "BLK-SENSOR", at0)]));
    assert!(has(&c, "region_not_editable"), "{:#?}", c.findings);
}

#[test]
fn the_report_says_what_is_fixed_what_may_be_used_and_what_is_unknown() {
    let f = fx();
    let zh = report(&f.e.result, &f.e.carrier_pack, true);
    for needle in ["开发板固定部分", "可用排针", "J1.4", "J3.5", "开发板已板载", "BLK-ONBOARD-BOOT", "载板待连接的外设", "temperature sensor", "未知", "证据位置", "REG-BOARD-BODY"] {
        assert!(zh.contains(needle), "{needle}\n{zh}");
    }
    let en = report(&f.e.result, &f.e.carrier_pack, false);
    assert!(en.contains("What the board fixes") && en.contains("Already on the board") && en.contains("unknown"), "{en}");
}
