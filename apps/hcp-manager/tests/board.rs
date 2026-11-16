//! `hcp-manager packs embed`: a development board pack in the workspace, an embedding request, and what comes out. The board is the
//! same fictional one as in the tests of fr-constraint (`tests/board.rs`); no network, no PATH, no index is needed.

mod common;
use common::*;
use fr_constraint::hcp;
use fr_constraint::resolve::canonical_sha256;
use serde_json::{json, Value};
use std::path::Path;

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

pub fn request(b: &Value, rot: i64, at: [f64; 2]) -> Value {
    json!({
        "request_version": "1.0",
        "board": {"id": ID, "version": "0.1.0", "hardware_revision": "B", "canonical_sha256": canonical_sha256(b)},
        "carrier": {"name": "myboard", "outline": {"x_mm": 0, "y_mm": 0, "width_mm": 100, "height_mm": 100}},
        "placement": {"ref": "U1", "origin_mm": at, "rotation_deg": rot},
        "targets": [{"name": "temperature sensor", "block": "BLK-SENSOR", "pins": ["J1.4", "J1.5"]}]
    })
}


fn bare(ws: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(bin()).arg("--workspace").arg(ws).args(args).env("PATH", "").env("HCP_TODAY", "2026-11-16").output().unwrap()
}

/// The board pack has one pin that the documents leave `unknown`: an embedding says so and needs attention (exit 5). `setup` makes it reserved
/// (said, with a reason), so that a clean embedding can be tested; `setup_with_unknown` keeps it.
fn setup(name: &str) -> (std::path::PathBuf, std::path::PathBuf, Value) {
    setup_with(name, false)
}

fn setup_with(name: &str, keep_unknown: bool) -> (std::path::PathBuf, std::path::PathBuf, Value) {
    let dir = tmp(name);
    let ws = dir.join("ws");
    let mut b = board();
    if !keep_unknown {
        let pin = &mut b["board_integration"]["exposed_headers"][1]["pins"][4];
        pin["usability"] = json!("reserved");
        pin["reason"] = json!("kept free by the board design (stated)");
    }
    let p = ws.join("packs").join(ID).join("0.1.0").join("pack.yaml");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, hcp::to_yaml(&b).unwrap()).unwrap();
    let req = dir.join("request.json");
    std::fs::write(&req, serde_json::to_string_pretty(&request(&b, 0, [20.0, 10.0])).unwrap()).unwrap();
    (ws, req, b)
}

fn files(dir: &Path) -> Vec<(String, String)> {
    all_files(dir).into_iter().map(|(p, b)| (p.strip_prefix(dir).unwrap().display().to_string(), hcp::sha256_hex(&b))).collect()
}

#[test]
fn embed_writes_the_carrier_pack_its_ir_the_differences_and_a_report_without_network_or_index() {
    let (ws, req, b) = setup("embed-ok");
    let out = ws.parent().unwrap().join("out");
    let o = bare(&ws, &["--json", "packs", "embed", "--board", ID, "--request", req.to_str().unwrap(), "--out", out.to_str().unwrap()]);
    assert_eq!(code(&o), 0, "{}{}", out_s(&o), err(&o));
    let r = jsn(&o);
    assert_eq!((r["status"].as_str(), r["pin_pool"].as_u64()), (Some("ok"), Some(2)), "{r}");
    for f in ["embedding.json", "carrier-pack.yaml", "ir.json", "diff-report.json", "report.zh.md", "board-embedding-request.json"] {
        assert!(out.join(f).is_file(), "{f}");
    }
    // the carrier pack is valid and its IR is the one of that pack
    let carrier = hcp::parse(&std::fs::read_to_string(out.join("carrier-pack.yaml")).unwrap()).unwrap();
    assert!(hcp::check(&carrier).iter().all(|d| d.severity != fr_constraint::diag::Severity::Error));
    let ir: fr_constraint::ir::Ir = serde_json::from_slice(&std::fs::read(out.join("ir.json")).unwrap()).unwrap();
    assert!(ir.verify_integrity(), "the IR matches its integrity hash");
    assert_eq!(serde_json::to_string(&fr_constraint::compile::compile(&carrier).unwrap()).unwrap(), serde_json::to_string(&ir).unwrap());
    // an AI gets its authority from the IR: the pool is the usable header pins
    let q = fr_constraint::edit::edit_request(&ir, "add a temperature sensor").unwrap();
    assert!(q["authorization"]["regions"][0]["pin_pool"].to_string().contains("J1.4"), "{q}");
    // the result binds the request to the pack by digest
    let e: Value = serde_json::from_slice(&std::fs::read(out.join("embedding.json")).unwrap()).unwrap();
    assert_eq!(e["board"]["canonical_sha256"], json!(canonical_sha256(&b)));
    let d: Value = serde_json::from_slice(&std::fs::read(out.join("diff-report.json")).unwrap()).unwrap();
    assert!(d["sections"]["regions"]["generated"].to_string().contains("REG-CARRIER-FREE-1") && d["sections"]["devices"]["carried_over"].to_string().contains(DEV), "{d}");
    let zh = std::fs::read_to_string(out.join("report.zh.md")).unwrap();
    assert!(zh.contains("开发板固定部分") && zh.contains("J1.4"));
    // English on request
    let out2 = ws.parent().unwrap().join("out-en");
    assert_eq!(code(&bare(&ws, &["packs", "embed", "--board", ID, "--request", req.to_str().unwrap(), "--out", out2.to_str().unwrap(), "--lang", "en"])), 0);
    assert!(out2.join("report.en.md").is_file() && !out2.join("report.zh.md").exists());
}

#[test]
fn the_same_request_gives_the_same_files_and_deleting_the_index_changes_nothing() {
    let (ws, req, _) = setup("embed-det");
    let run_to = |name: &str| {
        let out = ws.parent().unwrap().join(name);
        assert_eq!(code(&bare(&ws, &["packs", "embed", "--board", ID, "--request", req.to_str().unwrap(), "--out", out.to_str().unwrap()])), 0);
        files(&out)
    };
    let a = run_to("a");
    // an index and vectors exist, then they do not: the same bytes
    std::fs::create_dir_all(ws.join("derived")).unwrap();
    std::fs::write(ws.join("derived/index.db"), b"junk").unwrap();
    assert_eq!(a, run_to("b"));
    std::fs::remove_dir_all(ws.join("derived")).unwrap();
    assert_eq!(a, run_to("c"));
}

#[test]
fn a_request_for_another_revision_a_changed_pack_or_the_wrong_board_is_refused_and_nothing_is_written() {
    let (ws, req, b) = setup("embed-bad");
    let out = ws.parent().unwrap().join("out");
    let try_with = |f: &dyn Fn(&mut Value)| {
        let mut r = request(&b, 0, [20.0, 10.0]);
        f(&mut r);
        let file = ws.parent().unwrap().join("r2.json");
        std::fs::write(&file, serde_json::to_string(&r).unwrap()).unwrap();
        bare(&ws, &["packs", "embed", "--board", ID, "--request", file.to_str().unwrap(), "--out", out.to_str().unwrap()])
    };
    let o = try_with(&|r| r["board"]["hardware_revision"] = json!("A"));
    assert_eq!(code(&o), 3);
    assert!(err(&o).contains("board_revision_mismatch") && err(&o).contains("revision B"), "{}", err(&o));
    let o = try_with(&|r| r["board"]["canonical_sha256"] = json!("2".repeat(64)));
    assert_eq!(code(&o), 3);
    assert!(err(&o).contains("board_pack_changed"), "{}", err(&o));
    // the pack in the workspace changed after the request was made
    let p = ws.join("packs").join(ID).join("0.1.0").join("pack.yaml");
    let edited = std::fs::read_to_string(&p).unwrap().replace("GPIO5 is usable", "GPIO5 is usable").replace("name: GPIO5", "name: GPIO5-x");
    std::fs::write(&p, edited).unwrap();
    let o = bare(&ws, &["packs", "embed", "--board", ID, "--request", req.to_str().unwrap(), "--out", out.to_str().unwrap()]);
    assert_eq!(code(&o), 3, "{}", err(&o));
    assert!(err(&o).contains("board_pack_changed"), "{}", err(&o));
    assert!(!out.exists(), "nothing was written");
    // a pack that is not a board
    let o = bare(&ws, &["packs", "embed", "--board", "acme/none", "--request", req.to_str().unwrap(), "--out", out.to_str().unwrap()]);
    assert_eq!(code(&o), 3);
}

#[test]
fn an_existing_output_is_not_replaced_without_force_and_the_inputs_are_never_overwritten() {
    let (ws, req, _) = setup("embed-out");
    let out = ws.parent().unwrap().join("out");
    let cmd = |extra: &[&str]| {
        let mut a = vec!["packs", "embed", "--board", ID, "--request", req.to_str().unwrap(), "--out", out.to_str().unwrap()];
        a.extend_from_slice(extra);
        bare(&ws, &a)
    };
    assert_eq!(code(&cmd(&[])), 0);
    assert_eq!(code(&cmd(&[])), 4);
    assert_eq!(code(&cmd(&["--force"])), 0);
    // the request lives in the directory that would be written: refused
    let inside = ws.parent().unwrap().join("same");
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::copy(&req, inside.join("request.json")).unwrap();
    let o = bare(&ws, &["packs", "embed", "--board", ID, "--request", inside.join("request.json").to_str().unwrap(), "--out", inside.to_str().unwrap(), "--force"]);
    assert_eq!(code(&o), 4, "{}", err(&o));
    assert_eq!(code(&bare(&ws, &["packs", "embed", "--board", ID, "--request", req.to_str().unwrap(), "--out", out.to_str().unwrap(), "--lang", "fr", "--force"])), 2);
}

#[test]
fn a_target_the_pack_does_not_allow_is_reported_with_the_reason_and_the_exit_is_five() {
    let (ws, _, b) = setup_with("embed-targets", true);
    let mut r = request(&b, 0, [20.0, 10.0]);
    r["targets"] = json!([{"name": "uses the boot pin", "pins": ["J3.4"]}]);
    let file = ws.parent().unwrap().join("r.json");
    std::fs::write(&file, serde_json::to_string(&r).unwrap()).unwrap();
    let out = ws.parent().unwrap().join("out");
    let o = bare(&ws, &["--json", "packs", "embed", "--board", ID, "--request", file.to_str().unwrap(), "--out", out.to_str().unwrap()]);
    assert_eq!(code(&o), 5, "{}", err(&o));
    let e: Value = serde_json::from_slice(&std::fs::read(out.join("embedding.json")).unwrap()).unwrap();
    assert_eq!(e["targets"][0]["status"], "blocked");
    assert!(e["targets"][0]["reasons"].to_string().contains("boot button"), "{e}");
    assert_eq!(e["pin_pool"], json!(["J1.4", "J1.5"]), "asking for it widened nothing");
}

fn out_s(o: &std::process::Output) -> String {
    out(o)
}

#[test]
fn a_pin_the_documents_leave_unknown_needs_attention_and_is_not_in_the_pool() {
    let (ws, req, _) = setup_with("embed-unknown", true);
    let out = ws.parent().unwrap().join("out");
    let o = bare(&ws, &["--json", "packs", "embed", "--board", ID, "--request", req.to_str().unwrap(), "--out", out.to_str().unwrap()]);
    assert_eq!(code(&o), 5, "{}", err(&o));
    let e: Value = serde_json::from_slice(&std::fs::read(out.join("embedding.json")).unwrap()).unwrap();
    assert_eq!(e["status"], "needs_attention");
    assert!(e["unknown"].to_string().contains("J3.5") && !e["pin_pool"].to_string().contains("J3.5"), "{e}");
    assert!(std::fs::read_to_string(out.join("report.zh.md")).unwrap().contains("J3.5"));
}
