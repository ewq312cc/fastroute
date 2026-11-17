//! The board pack of the ESP32-S3-DevKitC-1 v1.1 (N16R8): `espressif/esp32-s3/esp32-s3-devkitc-1/n16r8-v1.1`, written from the user guide,
//! the schematic and the DXF of Espressif and built on the module pack `espressif/esp32-s3/esp32-s3-wroom-1/n16r8` (which is built on the
//! chip pack). What is tested here is the pack as it is committed, with no network, no model and no index:
//!
//! * the pack resolves with its upstream packs, checks clean, and every header pin says why it may or may not be used;
//! * the board, as the DXF measures it, and the pins that the module pack protects;
//! * the embedding in a carrier at all four rotations, its refusals (another revision, another variant, a changed pack), and the edit boundary
//!   of #17 applied to the carrier pack: legal and illegal edits, including the ones that every real board makes possible;
//! * the committed example (`hardware-packs/espressif/esp32-s3-devkitc-1-example/`) is what the command writes today.
//!
//! The quotations of the pack are checked against the documents by `hcp-manager evidence verify` when the documents are cached (208 of 208
//! when the pack was written); that needs the documents and is not part of this offline run.

mod common;
use common::{bin, code, err, tmp};
use fr_constraint::board;
use fr_constraint::compile;
use fr_constraint::diag::Severity;
use fr_constraint::edit::{self, BaseFile};
use fr_constraint::hcp;
use fr_constraint::resolve::canonical_sha256;
use fr_constraint::snapshot::Snapshot;
use hcp_manager::release::{self, Pack};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

const BOARD: &str = "espressif/esp32-s3/esp32-s3-devkitc-1/n16r8-v1.1";
const MODULE: &str = "espressif/esp32-s3/esp32-s3-wroom-1/n16r8";
const N8R8: &str = "espressif/esp32-s3/esp32-s3-wroom-1/n8r8";
const DEV: &str = "DEV-DEVKITC1-N16R8-V11";

fn packs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../hardware-packs")
}

fn example_dir() -> PathBuf {
    packs_dir().join("espressif/esp32-s3-devkitc-1-example")
}

fn tree() -> Vec<Pack> {
    let (packs, problems) = release::load_tree(&packs_dir()).expect("the tree is readable");
    assert!(problems.is_empty(), "{problems:#?}");
    packs
}

fn pack<'a>(all: &'a [Pack], id: &str) -> &'a Pack {
    all.iter().find(|p| p.id == id).unwrap_or_else(|| panic!("no pack {id}"))
}

fn flat(all: &[Pack], id: &str) -> Value {
    release::resolved(pack(all, id), all).unwrap_or_else(|e| panic!("{id}: {e:#?}"))
}

fn errors(doc: &Value) -> Vec<String> {
    hcp::check(doc).into_iter().filter(|d| d.severity == Severity::Error).map(|d| format!("{} {}: {}", d.code, d.path, d.message)).collect()
}

/// header pins of the board: (header id, pin number, name, usability, the pin object)
fn pins(b: &Value) -> Vec<(String, String, String, String, Value)> {
    b["board_integration"]["exposed_headers"].as_array().unwrap().iter().flat_map(|h| {
        let id = h["id"].as_str().unwrap().to_string();
        h["pins"].as_array().unwrap().iter().map(move |p| (id.clone(), p["number"].as_str().unwrap().to_string(), p["name"].as_str().unwrap().to_string(), p["usability"].as_str().unwrap().to_string(), p.clone()))
    }).collect()
}

fn by_name<'a>(all: &'a [(String, String, String, String, Value)], name: &str) -> &'a (String, String, String, String, Value) {
    all.iter().find(|p| p.2 == name).unwrap_or_else(|| panic!("no header pin {name}"))
}

fn request(all: &[Pack], rot: i64, at: [f64; 2]) -> Value {
    json!({
        "request_version": "1.0",
        "board": {"id": BOARD, "version": "0.1.0", "hardware_revision": "v1.1", "canonical_sha256": canonical_sha256(&pack(all, BOARD).hcp)},
        "carrier": {"name": "sensor-carrier", "outline": {"x_mm": 0, "y_mm": 0, "width_mm": 100, "height_mm": 120}},
        "placement": {"ref": "U1", "origin_mm": at, "rotation_deg": rot},
        "targets": [{"name": "temperature sensor (I2C)", "block": "BLK-DK-PERIPHERAL", "pins": ["J1.4", "J1.5"]}]
    })
}

fn embed(all: &[Pack], rot: i64, at: [f64; 2]) -> board::Embedding {
    let b = flat(all, BOARD);
    board::embed(&b, &canonical_sha256(&pack(all, BOARD).hcp), &request(all, rot, at)).unwrap_or_else(|e| panic!("{e:#?}"))
}

// ---- the pack ----------------------------------------------------------------------------------------------------------------

#[test]
fn the_board_pack_resolves_with_its_upstream_checks_clean_and_every_pin_says_why() {
    let all = tree();
    let b = flat(&all, BOARD);
    assert!(errors(&b).is_empty(), "{:#?}", errors(&b));
    // board -> module (N16R8, not N8R8) -> chip
    let up: Vec<&str> = pack(&all, BOARD).hcp["pack"]["upstream"].as_array().unwrap().iter().map(|u| u["id"].as_str().unwrap()).collect();
    assert_eq!(up, [MODULE]);
    assert_eq!(pack(&all, MODULE).hcp["pack"]["upstream"][0]["id"], json!("espressif/esp32-s3/esp32-s3/qfn56"));
    assert_eq!(b["board_integration"]["hardware_revision"], json!("v1.1"));
    assert_eq!(b["devices"].as_array().unwrap().iter().find(|d| d["id"] == DEV).unwrap()["part_number"], json!("ESP32-S3-DevKitC-1-N16R8"));

    let all_pins = pins(&b);
    assert_eq!(all_pins.len(), 44, "two headers of 22 pins");
    let count = |u: &str| all_pins.iter().filter(|p| p.3 == u).count();
    assert_eq!((count("usable"), count("reserved"), count("onboard"), count("unknown")), (24, 10, 7, 3));
    for (h, n, name, usability, p) in &all_pins {
        if usability == "usable" {
            assert!(p["evidence"].as_array().is_some_and(|e| e.len() >= 2), "{h}.{n} {name}: a usable pin has the row of the guide and the statement that the pins are available");
        } else {
            assert!(p["reason"].as_str().is_some_and(|r| r.len() > 15), "{h}.{n} {name}: {usability} without a reason");
        }
    }
    // what the documents say, pin by pin
    for (name, usability) in [("GPIO0", "onboard"), ("RST", "onboard"), ("TX", "onboard"), ("RX", "onboard"), ("GPIO19", "onboard"), ("GPIO20", "onboard"), ("GPIO38", "onboard"),
                              ("GPIO35", "reserved"), ("GPIO36", "reserved"), ("GPIO37", "reserved"), ("3V3", "reserved"), ("5V", "reserved"),
                              ("GPIO3", "unknown"), ("GPIO45", "unknown"), ("GPIO46", "unknown"), ("GPIO4", "usable"), ("GPIO48", "usable"), ("GPIO1", "usable")] {
        assert_eq!(by_name(&all_pins, name).3, usability, "{name}");
    }
    // the RGB LED is on GPIO38 in this revision, so GPIO48 is free: the other revision is another pack
    assert!(pack(&all, BOARD).hcp["pack"]["review"]["note"].as_str().unwrap().contains("GPIO48"));
}

#[test]
fn the_board_is_what_the_dxf_measures_and_the_headers_are_where_the_guide_numbers_them() {
    let all = tree();
    let b = flat(&all, BOARD);
    let region = |id: &str| b["regions"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap_or_else(|| panic!("no region {id}")).clone();
    let rect = |r: &Value| -> (f64, f64, f64, f64) {
        let p = r["polygon"].as_array().unwrap();
        let xs: Vec<f64> = p.iter().map(|q| q[0].as_f64().unwrap()).collect();
        let ys: Vec<f64> = p.iter().map(|q| q[1].as_f64().unwrap()).collect();
        (xs.iter().cloned().fold(f64::MAX, f64::min), ys.iter().cloned().fold(f64::MAX, f64::min), xs.iter().cloned().fold(f64::MIN, f64::max), ys.iter().cloned().fold(f64::MIN, f64::max))
    };
    let (x0, y0, x1, y1) = rect(&region("REG-DK-OUTLINE"));
    assert_eq!((x0, y0, x1, y1), (0.0, 0.0, 25.4, 62.865));
    // the antenna part of the module sticks out beyond the top edge: 18 x 6.3 mm
    let (ax0, ay0, ax1, ay1) = rect(&region("REG-DK-ANT"));
    assert!((ax1 - ax0 - 18.0).abs() < 1e-6 && (ay1 - ay0 - 6.31).abs() < 0.01 && ay1 > y1 + 6.0, "{:?}", (ax0, ay0, ax1, ay1));
    assert!(ax0 > x0 && ax1 < x1, "the module is inside the width of the board");
    // both USB connectors stick out beyond the bottom edge
    for id in ["REG-DK-USB-J2", "REG-DK-USB-J4"] {
        let (_, cy0, _, cy1) = rect(&region(id));
        assert!(cy0 < 0.0 && cy1 == 0.0, "{id}");
    }
    // 2 x 22 pins at a 2.54 mm pitch, 1.27 mm from the edges, pin 1 at the antenna end
    for (h, x) in [("J1", 1.27), ("J3", 24.13)] {
        let hdr = b["board_integration"]["exposed_headers"].as_array().unwrap().iter().find(|g| g["id"] == h).unwrap();
        let ps = hdr["pins"].as_array().unwrap();
        assert_eq!(ps.len(), 22);
        for (i, p) in ps.iter().enumerate() {
            let (px, py) = (p["position"][0].as_f64().unwrap(), p["position"][1].as_f64().unwrap());
            assert!((px - x).abs() < 1e-9, "{h}.{}: x {px}", i + 1);
            assert!((py - (61.3 - 2.54 * i as f64)).abs() < 1e-6, "{h}.{}: y {py}", i + 1);
        }
    }
    // the module pins that the header pins are wired to are the module's own pin numbers
    let m = flat(&all, MODULE);
    let module_pin = |name: &str| m["devices"].as_array().unwrap().iter().flat_map(|d| d["pins"].as_array().unwrap().iter()).find(|p| p["name"] == name).unwrap()["number"].as_str().unwrap().to_string();
    let all_pins = pins(&b);
    for (hdr_name, mod_name) in [("RST", "EN"), ("TX", "TXD0"), ("RX", "RXD0"), ("GPIO4", "IO4"), ("GPIO0", "IO0"), ("GPIO35", "IO35"), ("GPIO48", "IO48")] {
        assert_eq!(by_name(&all_pins, hdr_name).4["module_pin"], json!(module_pin(mod_name)), "{hdr_name}");
    }
}

#[test]
fn a_pin_the_board_calls_usable_but_the_module_pack_protects_is_refused_for_the_real_board() {
    let all = tree();
    let ok = flat(&all, BOARD);
    // the pins that the module pack protects: strapping (0, 3, 45, 46), reset (EN) and the octal PSRAM (35, 36, 37)
    for name in ["GPIO0", "GPIO3", "GPIO45", "GPIO46", "RST", "GPIO35", "GPIO36", "GPIO37"] {
        let mut b = ok.clone();
        for h in b["board_integration"]["exposed_headers"].as_array_mut().unwrap() {
            for p in h["pins"].as_array_mut().unwrap() {
                if p["name"] == name {
                    p["usability"] = json!("usable");
                    p.as_object_mut().unwrap().remove("reason");
                    p["evidence"] = json!([{"source_id": "SRC-DK-UG", "locator": {"section": "Header Block"}, "quote": "x", "basis": "explicit"}]);
                }
            }
        }
        let e = errors(&b);
        assert!(e.iter().any(|x| x.starts_with("board_pin_conflicts_with_upstream")), "{name} usable must conflict with the module pack: {e:#?}");
    }
    // a pin nobody protects may be made usable without a conflict (here: GPIO4 was usable already; GPIO38 is the RGB LED, which the board implements)
    assert!(errors(&ok).is_empty());
}

#[test]
fn the_n8r8_module_is_not_the_module_of_this_board() {
    let all = tree();
    // resolved against the N8R8 module pack the board is refused: the exact upstream is the N16R8 one, a similar pack is never taken instead
    let up = pack(&all, N8R8).hcp.clone();
    let err = fr_constraint::resolve::resolve(&pack(&all, BOARD).hcp, &[up]).unwrap_err();
    assert!(err.iter().any(|d| d.code == "upstream_missing"), "{err:#?}");
}

// ---- the embedding -------------------------------------------------------------------------------------------------------------

#[test]
fn the_board_embeds_at_every_rotation_with_the_same_pool_and_says_what_is_open() {
    let all = tree();
    for (rot, at) in [(0, [30.0, 40.0]), (90, [90.0, 20.0]), (180, [70.0, 110.0]), (270, [10.0, 110.0])] {
        let e = embed(&all, rot, at);
        let pool: Vec<&str> = e.result["pin_pool"].as_array().unwrap().iter().map(|p| p.as_str().unwrap()).collect();
        assert_eq!(pool.len(), 24, "rot {rot}");
        for not in ["J1.3", "J1.13", "J1.14", "J3.2", "J3.3", "J3.10", "J3.11", "J3.12", "J3.13", "J3.14", "J3.15", "J3.19", "J3.20"] {
            assert!(!pool.contains(&not), "rot {rot}: {not} must not be in the pool");
        }
        assert!(pool.contains(&"J1.4") && pool.contains(&"J1.5") && pool.contains(&"J3.16"), "rot {rot}");
        // header pin J1.1 moved with the board: it was at (1.27, 61.3)
        let p = &e.result["board_in_carrier"]["headers"][0]["pins"][0]["position_mm"];
        let want = match rot {
            0 => [at[0] + 1.27, at[1] + 61.3],
            90 => [at[0] - 61.3, at[1] + 1.27],
            180 => [at[0] - 1.27, at[1] - 61.3],
            _ => [at[0] + 61.3, at[1] - 1.27],
        };
        assert!((p[0].as_f64().unwrap() - want[0]).abs() < 1e-6 && (p[1].as_f64().unwrap() - want[1]).abs() < 1e-6, "rot {rot}: {p} vs {want:?}");
        // the open questions are listed, not hidden: three strapping pins and the RF clearance of the antenna
        assert_eq!(e.result["status"], json!("needs_attention"), "rot {rot}");
        let unknown: Vec<String> = e.result["unknown"].as_array().unwrap().iter().map(|u| u.as_str().unwrap().to_string()).collect();
        assert_eq!(unknown.len(), 4, "{unknown:#?}");
        assert!(unknown.iter().any(|u| u.starts_with("rule C-DK-ANT-CLEARANCE")), "{unknown:#?}");
        for pin in ["J1.13", "J1.14", "J3.15"] {
            assert!(unknown.iter().any(|u| u.starts_with(pin)), "{pin}: {unknown:#?}");
        }
        // the carrier pack is valid, compiles, and is the same bytes every time
        assert!(errors(&e.carrier_pack).is_empty(), "rot {rot}: {:#?}", errors(&e.carrier_pack));
        let again = embed(&all, rot, at);
        assert_eq!(hcp::canonical_json(&e.carrier_pack), hcp::canonical_json(&again.carrier_pack));
        assert!(compile::compile(&e.carrier_pack).is_ok());
    }
}

#[test]
fn another_revision_another_variant_or_a_changed_pack_is_refused() {
    let all = tree();
    let b = flat(&all, BOARD);
    let digest = canonical_sha256(&pack(&all, BOARD).hcp);
    let codes = |r: &Value| -> Vec<String> { board::embed(&b, &digest, r).err().unwrap_or_default().into_iter().map(|d| d.code).collect() };
    let ok = request(&all, 0, [30.0, 40.0]);
    assert!(codes(&ok).is_empty());
    // the initial revision of the board drives its RGB LED from GPIO48: its pool is another one
    let mut r = ok.clone();
    r["board"]["hardware_revision"] = json!("v1.0");
    assert!(codes(&r).contains(&"board_revision_mismatch".to_string()), "{:?}", codes(&r));
    // another variant (the N8R8 module is not on this board) is another pack id
    let mut r = ok.clone();
    r["board"]["id"] = json!("espressif/esp32-s3/esp32-s3-devkitc-1/n8r8-v1.1");
    assert!(codes(&r).contains(&"board_mismatch".to_string()), "{:?}", codes(&r));
    // another version, and a pack that is not the one the request was made for
    let mut r = ok.clone();
    r["board"]["version"] = json!("0.2.0");
    assert!(codes(&r).contains(&"board_mismatch".to_string()));
    let mut r = ok.clone();
    r["board"]["canonical_sha256"] = json!("0".repeat(64));
    assert!(codes(&r).contains(&"board_pack_changed".to_string()), "{:?}", codes(&r));
}

// ---- the edit boundary of the carrier -----------------------------------------------------------------------------------------------

struct Fx {
    e: board::Embedding,
    ir: fr_constraint::ir::Ir,
    base: Snapshot,
}

fn fx() -> Fx {
    let all = tree();
    let at = [30.0, 40.0];
    let e = embed(&all, 0, at);
    let ir = compile::compile(&e.carrier_pack).unwrap();
    let pins: Vec<Value> = e.result["board_in_carrier"]["headers"].as_array().unwrap().iter().flat_map(|h| h["pins"].as_array().unwrap().iter()).map(|p| json!({"ref": "U1", "pin": p["id"], "x": p["position_mm"][0], "y": p["position_mm"][1]})).collect();
    let base: Snapshot = serde_json::from_value(json!({
        "snapshot_version": "1.0", "project": {"name": "sensor-carrier"},
        "components": [{"ref": "U1", "device": DEV, "value": "ESP32-S3-DevKitC-1", "x": at[0], "y": at[1], "rotation_deg": 0.0}],
        "pins": pins, "tracks": [], "outline": [[0.0, 0.0], [100.0, 0.0], [100.0, 120.0], [0.0, 120.0]], "mount_holes": []
    }))
    .unwrap();
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

fn region<'a>(f: &'a Fx, id: &str) -> &'a Value {
    f.e.carrier_pack["regions"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap_or_else(|| panic!("no region {id}"))
}

fn centre(poly: &Value) -> [f64; 2] {
    let p = poly.as_array().unwrap();
    let (xs, ys): (Vec<f64>, Vec<f64>) = (p.iter().map(|q| q[0].as_f64().unwrap()).collect(), p.iter().map(|q| q[1].as_f64().unwrap()).collect());
    [(xs.iter().cloned().fold(f64::MAX, f64::min) + xs.iter().cloned().fold(f64::MIN, f64::max)) / 2.0, (ys.iter().cloned().fold(f64::MAX, f64::min) + ys.iter().cloned().fold(f64::MIN, f64::max)) / 2.0]
}

/// The two biggest free areas of the carrier: the left and the right strip beside the board (the board is in the middle)
fn strips(f: &Fx) -> (String, [f64; 2], String, [f64; 2]) {
    let mut v: Vec<(f64, String, [f64; 2])> = f.e.carrier_pack["regions"].as_array().unwrap().iter().filter(|r| r["kind"] == "editable").map(|r| {
        let p = r["polygon"].as_array().unwrap();
        let area = (p[2][0].as_f64().unwrap() - p[0][0].as_f64().unwrap()) * (p[2][1].as_f64().unwrap() - p[0][1].as_f64().unwrap());
        (area, r["id"].as_str().unwrap().to_string(), centre(&r["polygon"]))
    }).collect();
    v.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    (v[0].1.clone(), v[0].2, v[1].1.clone(), v[1].2)
}

#[test]
fn a_legal_extension_of_the_carrier_is_accepted() {
    let f = fx();
    let (r, at, _, _) = strips(&f);
    let c = check(&f, json!([
        {"id": "CH-1", "op": "add_component", "region": r, "block": "BLK-DK-PERIPHERAL", "role": "peripheral", "ref": "S1", "value": "TMP117", "at": at},
        {"id": "CH-2", "op": "connect", "region": r, "ref": "S1", "pin": "1", "net": "SDA"},
        {"id": "CH-3", "op": "connect", "region": r, "ref": "U1", "pin": "J1.4", "net": "SDA"},
        {"id": "CH-4", "op": "connect", "region": r, "ref": "U1", "pin": "J1.5", "net": "SCL"},
        {"id": "CH-5", "op": "add_track", "region": r, "net": "SDA", "layer": "F.Cu", "width_mm": 0.2, "points": [at, [at[0] + 1.0, at[1]]]}
    ]));
    assert!(errs(&c).is_empty(), "{:#?}", c.findings);
}

#[test]
fn every_way_of_overreaching_the_real_board_is_rejected() {
    let f = fx();
    let (r0, at0, r1, at1) = strips(&f);
    let has = |c: &edit::EditCheck, code: &str| errs(c).iter().any(|x| x == code);
    let add = |region: &str, block: &str, at: [f64; 2]| json!({"id": "CH-1", "op": "add_component", "region": region, "block": block, "role": "peripheral", "ref": "S1", "value": "X", "at": at});
    let connect = |pin: &str| json!([{"id": "CH-1", "op": "connect", "region": r0, "ref": "U1", "pin": pin, "net": "X"}]);

    // 1. a pin of the module that the board does not bring out (the module's IO26 is the flash, GPIO26 is on no header): not in the pool
    for pin in ["GPIO26", "J9.1", "IO26"] {
        let c = check(&f, connect(pin));
        assert!(has(&c, "pin_not_in_pool"), "{pin}: {:#?}", c.findings);
    }
    // 2. header pins that may not be used, one reason each: on the board (boot button GPIO0, reset, UART0 bridge, USB, RGB LED), reserved
    //    (octal PSRAM, power, ground) and unknown (strapping pins that the board documents leave open)
    for pin in ["J3.14", "J1.3", "J3.2", "J3.3", "J3.19", "J3.20", "J3.10", "J3.11", "J3.12", "J3.13", "J1.1", "J1.21", "J1.22", "J1.13", "J1.14", "J3.15"] {
        let c = check(&f, connect(pin));
        // (a pin that a rule protects, like the boot pin or the PSRAM pins, is also named by that rule: `reserved_pin_used`)
        assert!(has(&c, "pin_not_in_pool") || has(&c, "reserved_pin_used"), "{pin}: {:#?}", c.findings);
    }
    // 3. the boot button, the reset circuit, the USB-to-UART bridge and the regulator are on the board: not added again
    for blk in ["BLK-DK-BOOT", "BLK-DK-RESET", "BLK-DK-USB-UART", "BLK-DK-POWER"] {
        let c = check(&f, json!([add(&r0, blk, at0)]));
        assert!(has(&c, "unauthorized_block"), "{blk}: {:#?}", c.findings);
    }
    // 4. the board is a fixed component: not moved, not removed
    for ch in [json!({"id": "CH-1", "op": "move_component", "region": r0, "ref": "U1", "to": [50.0, 50.0]}), json!({"id": "CH-1", "op": "remove_component", "region": r0, "ref": "U1"})] {
        let c = check(&f, json!([ch]));
        assert!(!errs(&c).is_empty(), "{ch}: {:#?}", c.findings);
    }
    // 5. into the body of the board, into the antenna part that sticks out of it, over a USB connector
    let body = centre(&region(&f, "REG-BOARD-BODY")["polygon"]);
    let c = check(&f, json!([add(&r0, "BLK-DK-PERIPHERAL", body)]));
    assert!(has(&c, "outside_region"), "{:#?}", c.findings);
    for id in ["REG-DK-ANT", "REG-DK-USB-J2", "REG-DK-USB-J4"] {
        let spot = centre(&region(&f, id)["polygon"]);
        let c = check(&f, json!([add(&r0, "BLK-DK-PERIPHERAL", spot)]));
        assert!(!errs(&c).is_empty(), "{id}: {:#?}", c.findings);
        let c = check(&f, json!([{"id": "CH-1", "op": "add_track", "region": r0, "net": "X", "layer": "F.Cu", "width_mm": 0.2, "points": [at0, spot]}]));
        assert!(!errs(&c).is_empty(), "a track into {id}: {:#?}", c.findings);
    }
    // 6. a track from the area on one side of the board to the area on the other side, across the board
    let c = check(&f, json!([{"id": "CH-1", "op": "add_track", "region": r0, "net": "X", "layer": "F.Cu", "width_mm": 0.2, "points": [at0, at1]}]));
    assert!(has(&c, "outside_region") || has(&c, "crosses_region"), "{r0} -> {r1}: {:#?}", c.findings);
    // 7. an operation that the area does not allow (vias), and the outline of the carrier as an area
    let c = check(&f, json!([{"id": "CH-1", "op": "add_via", "region": r0, "net": "X", "at": at0, "diameter_mm": 0.6}]));
    assert!(has(&c, "op_not_allowed"), "{:#?}", c.findings);
    let c = check(&f, json!([add("REG-CARRIER-OUTLINE", "BLK-DK-PERIPHERAL", at0)]));
    assert!(has(&c, "region_not_editable"), "{:#?}", c.findings);
}

// ---- the committed example -------------------------------------------------------------------------------------------------------

#[test]
fn the_committed_example_is_what_the_command_writes_today_with_no_path_and_no_network() {
    let all = tree();
    let digest = canonical_sha256(&pack(&all, BOARD).hcp);
    let req_text = std::fs::read_to_string(example_dir().join("request.json")).expect("the example request is committed");
    let mut req: Value = serde_json::from_str(&req_text).unwrap();
    if std::env::var("HCP_WRITE_SAMPLE").is_ok() && req["board"]["canonical_sha256"] != json!(digest) {
        // the pack was rebuilt: the request is bound to the new digest
        req["board"]["canonical_sha256"] = json!(digest);
        std::fs::write(example_dir().join("request.json"), format!("{}\n", serde_json::to_string_pretty(&req).unwrap())).unwrap();
    }
    assert_eq!(req["board"]["canonical_sha256"], json!(digest), "the pack changed: regenerate the example (HCP_WRITE_SAMPLE=1 cargo test -p hcp-manager --test devkitc example)");

    // install the board and what it needs from a release built from the tree, into an empty workspace, then embed: no PATH, a dead proxy
    let t = tmp("devkitc");
    let rel = t.join("release");
    release::build(&packs_dir(), &rel, true).unwrap_or_else(|e| panic!("{}", e.message));
    let ws = t.join("ws");
    let run = |args: &[&str]| Command::new(bin()).arg("--workspace").arg(&ws).args(args).env("PATH", "").env("HTTPS_PROXY", "http://127.0.0.1:9").env("HCP_TODAY", "2026-11-17").output().unwrap();
    let o = run(&["packs", "builtin", "--release", rel.to_str().unwrap(), "install", BOARD]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let mut outputs = vec![];
    for (lang, file) in [("en", "report.en.md"), ("zh", "report.zh.md")] {
        let out = t.join(format!("out-{lang}"));
        let o = run(&["--json", "packs", "embed", "--board", BOARD, "--request", example_dir().join("request.json").to_str().unwrap(), "--out", out.to_str().unwrap(), "--lang", lang]);
        assert_eq!(code(&o), 5, "needs attention (the open questions are named): {}{}", String::from_utf8_lossy(&o.stdout), err(&o));
        outputs.push((file, std::fs::read_to_string(out.join(file)).unwrap(), out));
    }
    let embedding = std::fs::read_to_string(outputs[0].2.join("embedding.json")).unwrap();
    if std::env::var("HCP_WRITE_SAMPLE").is_ok() {
        let out = example_dir().join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("embedding.json"), &embedding).unwrap();
        for (file, text, _) in &outputs {
            std::fs::write(out.join(file), text).unwrap();
        }
    }
    let committed = |name: &str| std::fs::read_to_string(example_dir().join("out").join(name)).unwrap_or_else(|_| panic!("out/{name} is not committed: HCP_WRITE_SAMPLE=1 cargo test -p hcp-manager --test devkitc example"));
    assert_eq!(committed("embedding.json"), embedding, "out/embedding.json is not what the command writes today");
    for (file, text, _) in &outputs {
        assert_eq!(&committed(file), text, "out/{file} is not what the command writes today");
    }
    // the open questions are in the reports, in the reader's language
    assert!(outputs[0].1.contains("C-DK-ANT-CLEARANCE") && outputs[0].1.contains("GPIO45"));
    assert!(outputs[1].1.contains("C-DK-ANT-CLEARANCE") && outputs[1].1.contains("开发板固定部分"));
}
