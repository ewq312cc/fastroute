//! The ESP32-S3-WROOM-1-N8R8 example pack (hardware-packs/espressif/esp32-s3-wroom-1-n8r8): written from the locked Espressif
//! documents, compiled, validated on a legal carrier board and on boards that break one rule each, used to confine an AI to a free
//! area, and handed to the router. The tests that need the documents (download or a cache) are in `esp32s3_documents.rs`.

mod common;
use common::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn pack_dir() -> PathBuf {
    manifest_dir().join("../../hardware-packs/espressif/esp32-s3-wroom-1-n8r8")
}

fn pack() -> PathBuf {
    pack_dir().join("pack.yaml")
}

fn ir() -> PathBuf {
    pack_dir().join("ir.json")
}

fn carrier() -> PathBuf {
    pack_dir().join("example/carrier.kicad_pcb")
}

/// The carrier board as a design snapshot, for the tests to break.
fn snapshot() -> Value {
    let d = tmp("esp-snapshot");
    let f = d.join("carrier.json");
    let o = run(&["constraints", "extract-kicad", &s(&carrier()), "-o", &s(&f), "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    json(&f)
}

fn validate(snap: &Value) -> (i32, Value) {
    let d = tmp("esp-validate");
    let f = d.join("design.json");
    std::fs::write(&f, serde_json::to_string(snap).unwrap()).unwrap();
    let o = run(&["constraints", "validate", "--ir", &s(&ir()), "--design", &s(&f), "--json", "-q"]);
    let v: Value = serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{e}: {}", stderr(&o)));
    (code(&o), v)
}

fn failed(v: &Value) -> Vec<String> {
    let mut ids: Vec<String> = v["findings"].as_array().unwrap().iter().filter(|f| f["verdict"] == "fail").map(|f| f["rule_id"].as_str().unwrap().to_string()).collect();
    ids.sort();
    ids.dedup();
    ids
}

// ---- the pack and what is derived from it -----------------------------------------------------------------------------------

#[test]
fn the_pack_is_valid_and_everything_derived_from_it_is_current() {
    let o = run(&["constraints", "check", &s(&pack()), "--json", "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let c: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!((c["valid"].as_bool(), c["errors"].as_u64(), c["warnings"].as_u64()), (Some(true), Some(0), Some(0)));
    let d = tmp("esp-derived");
    // the committed IR and the Chinese report are what the programs of this repository write today
    let (out_ir, out_md) = (d.join("ir.json"), d.join("report.md"));
    assert_eq!(code(&run(&["constraints", "compile", &s(&pack()), "-o", &s(&out_ir), "-q"])), 0);
    assert_eq!(std::fs::read(&out_ir).unwrap(), std::fs::read(ir()).unwrap(), "ir.json is out of date: run hardware-packs/espressif/esp32-s3-wroom-1-n8r8/tools/regenerate.sh");
    assert_eq!(code(&run(&["constraints", "report", &s(&pack()), "-o", &s(&out_md), "--lang", "zh", "-q"])), 0);
    assert_eq!(std::fs::read(&out_md).unwrap(), std::fs::read(pack_dir().join("reports/pack-report.zh.md")).unwrap(), "the report is out of date: run tools/regenerate.sh");
    let (vj, vm) = (d.join("v.json"), d.join("v.md"));
    let o = run(&["constraints", "validate", "--ir", &s(&ir()), "--kicad-pcb", &s(&carrier()), "-o", &s(&vj), "--report", &s(&vm), "--lang", "zh", "-q"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    assert_eq!(std::fs::read(&vj).unwrap(), std::fs::read(pack_dir().join("reports/validation-legal.json")).unwrap(), "validation-legal.json is out of date: run tools/regenerate.sh");
    assert_eq!(std::fs::read(&vm).unwrap(), std::fs::read(pack_dir().join("reports/validation-legal.zh.md")).unwrap());
}

#[test]
fn the_pack_and_the_inventory_agree_on_every_document_and_the_local_files_match_their_digests() {
    let p = fr_constraint::hcp::parse(&std::fs::read_to_string(pack()).unwrap()).unwrap();
    let inv = fr_constraint::hcp::parse(&std::fs::read_to_string(pack_dir().join("workspace/sources.yaml")).unwrap()).unwrap();
    for src in p["sources"].as_array().unwrap() {
        let doc = inv["documents"].as_array().unwrap().iter().find(|d| d["document_id"] == src["document_id"]).unwrap_or_else(|| panic!("{} is not in the inventory", src["document_id"]));
        assert_eq!((doc["version"].as_str(), doc["sha256"].as_str(), doc["level"].as_str()), (src["version"].as_str(), src["sha256"].as_str(), src["level"].as_str()), "{}", src["document_id"]);
    }
    let sha = |f: &Path| fr_constraint::hcp::sha256_hex(&std::fs::read(f).unwrap());
    // the notes that are the open-board source, and the board the pack is bound to
    let notes = inv["documents"].as_array().unwrap().iter().find(|d| d["document_id"] == "DOC-ESP32S3-CARRIER").unwrap();
    assert_eq!(notes["sha256"].as_str().unwrap(), sha(&pack_dir().join("example/carrier-notes.md")), "carrier-notes.md changed: pin its digest in workspace/sources.yaml and pack.template.yaml");
    assert_eq!(p["base_design"]["binding"]["files"][0]["sha256"].as_str().unwrap(), sha(&carrier()), "carrier.kicad_pcb changed: run tools/regenerate.sh");
    // the second document of another model is in the inventory and not in the pack
    assert!(inv["documents"].as_array().unwrap().iter().any(|d| d["document_id"] == "DOC-ESP32S3-WROOM2-DS"));
    assert!(!p["sources"].as_array().unwrap().iter().any(|d| d["document_id"] == "DOC-ESP32S3-WROOM2-DS"));
}

#[test]
fn nothing_is_filled_in_for_what_the_documents_do_not_say() {
    let ir = json(&ir());
    let status = |id: &str| ir["rules"].as_array().unwrap().iter().find(|r| r["rule_id"] == id).unwrap_or_else(|| panic!("{id}")).clone();
    for id in ["C-ESP-EN-RC-VALUES", "C-ESP-USB-SKEW", "C-ESP-USB-GAP"] {
        let r = status(id);
        assert_eq!(r["status"], "unknown", "{id}");
        assert!(r["missing"].as_str().unwrap().len() > 40, "{id} says what is missing");
        assert!(!r["params"].to_string().chars().any(|c| c.is_ascii_digit()) || r["params"]["skew_mm"].is_null() || r["params"]["gap_mm"].is_null(), "{id}: no number in an unknown rule");
    }
    // the numbers that are in the pack are in a quotation
    let usb = status("C-ESP-USB");
    assert_eq!(usb["params"]["impedance_ohm"], 90);
    assert!(usb["evidence"][0]["quote"].as_str().unwrap().contains("90 Ω"));
    // the reserved pins are those of the quoted note, and nothing else is reserved
    assert_eq!(status("C-ESP-RESERVED-PSRAM")["params"]["pins"], json!(["28", "29", "30"]));
    // the report tells the four kinds apart
    let md = std::fs::read_to_string(pack_dir().join("reports/pack-report.zh.md")).unwrap();
    for h in ["## 1. 不可修改项（固定）", "## 3. 可配置项、可选项与推导项", "## 4. AI 可编辑区域（白名单）", "## 6. 冲突与缺口", "待确认的依据"] {
        assert!(md.contains(h), "{h}");
    }
}

// ---- the legal board and the boards that break one rule each --------------------------------------------------------------------

#[test]
fn the_legal_carrier_passes_and_what_could_not_be_checked_is_named() {
    let (c, v) = validate(&snapshot());
    assert_eq!(c, 5, "{v}");
    assert_eq!((v["result"].as_str(), v["complete"].as_bool()), (Some("pass"), Some(false)));
    assert!(failed(&v).is_empty());
    let by = |verdict: &str| -> Vec<String> {
        let mut r: Vec<String> = v["findings"].as_array().unwrap().iter().filter(|f| f["verdict"] == verdict).map(|f| f["rule_id"].as_str().unwrap().to_string()).collect();
        r.sort();
        r
    };
    assert_eq!(by("unknown"), ["C-ESP-EN-RC-VALUES", "C-ESP-USB-GAP", "C-ESP-USB-SKEW"]);
    assert_eq!(by("unsupported"), ["C-ESP-ELECTRICAL-VDD33"]);
    assert_eq!(by("unchecked"), ["C-ESP-USB", "PINMUX-ESP32S3-WROOM1-N8R8"]);
    assert!(by("pass").len() >= 18);
}

#[test]
fn each_mistake_fails_at_the_expected_rule_with_the_evidence_of_that_rule() {
    struct Case {
        what: &'static str,
        edit: fn(&mut Value),
        rules: &'static [&'static str],
        quote: &'static str,
    }
    fn del_comp(v: &mut Value, r: &str) {
        v["components"].as_array_mut().unwrap().retain(|c| c["ref"] != r);
        v["pins"].as_array_mut().unwrap().retain(|p| p["ref"] != r);
    }
    fn set_net(v: &mut Value, r: &str, pin: &str, net: Option<&str>) {
        for p in v["pins"].as_array_mut().unwrap() {
            if p["ref"] == r && p["pin"] == pin {
                match net {
                    Some(n) => p["net"] = json!(n),
                    None => {
                        p.as_object_mut().unwrap().remove("net");
                    }
                }
            }
        }
    }
    let cases = [
        Case { what: "the supply capacitor of the power entrance is missing", edit: |v| del_comp(v, "C1"), rules: &["C-ESP-NEED-POWER"], quote: "at least 10 μF capacitor" },
        Case { what: "GPIO0 is tied to ground (BOOT conflict)", edit: |v| set_net(v, "U1", "27", Some("GND")), rules: &["C-ESP-BOOT-IO0"], quote: "GPIO0 and GPIO46 control the boot mode" },
        Case { what: "GPIO0 is tied to the supply (download mode cannot be entered)", edit: |v| set_net(v, "U1", "27", Some("3V3")), rules: &["C-ESP-BOOT-IO0", "C-ESP-NEED-BOOT"], quote: "GPIO0 and GPIO46 control the boot mode" },
        Case { what: "IO36 (PSRAM) is used", edit: |v| set_net(v, "U1", "29", Some("UART_TX")), rules: &["C-ESP-RESERVED-PSRAM"], quote: "Octal SPI PSRAM" },
        Case { what: "EN is left floating", edit: |v| set_net(v, "U1", "3", None), rules: &["C-ESP-EN", "C-ESP-NEED-RESET"], quote: "Do not leave the EN pin floating" },
        Case { what: "a ground pin is on the supply", edit: |v| set_net(v, "U1", "40", Some("3V3")), rules: &["C-ESP-GND-40"], quote: "GND 40 P GND" },
        Case {
            what: "a track runs through the antenna keep-out",
            edit: |v| v["tracks"].as_array_mut().unwrap().push(json!({"net": "GND", "layer": "F.Cu", "width_mm": 0.25, "x1": 20.0, "y1": 8.0, "x2": 30.0, "y2": 8.0})),
            rules: &["C-ESP-ANT-KEEPOUT"],
            quote: "cut off the base board",
        },
        Case {
            what: "a component stands in the antenna keep-out",
            edit: |v| v["components"].as_array_mut().unwrap().push(json!({"ref": "C9", "value": "100nF", "x": 20.0, "y": 8.0})),
            rules: &["C-ESP-ANT-KEEPOUT"],
            quote: "keepout zone for module’s antenna",
        },
        Case { what: "a mounting hole moved", edit: |v| v["mount_holes"][1]["x"] = json!(5.0), rules: &["C-ESP-MOUNT-H2"], quote: "Mounting hole H2 at (4, 36) mm" },
        Case { what: "the board outline changed", edit: |v| v["outline"][3] = json!([47.0, 10.0]), rules: &["C-ESP-OUTLINE"], quote: "Board outline" },
    ];
    for c in cases {
        let mut v = snapshot();
        (c.edit)(&mut v);
        let (code, r) = validate(&v);
        assert_eq!(code, 1, "{}: {r}", c.what);
        assert_eq!(r["result"], "fail", "{}", c.what);
        assert_eq!(failed(&r), c.rules, "{}: {:#?}", c.what, failed(&r));
        let f = r["findings"].as_array().unwrap().iter().find(|f| f["verdict"] == "fail" && f["rule_id"] == c.rules[0]).unwrap();
        assert!(f["evidence"].as_array().unwrap().iter().any(|e| e["quote"].as_str().unwrap().contains(c.quote)), "{}: the evidence of {} quotes '{}': {}", c.what, c.rules[0], c.quote, f["evidence"]);
        assert!(f["message"].as_str().unwrap().len() > 20 && f["object"].is_string(), "{}: {f}", c.what);
    }
}

#[test]
fn the_check_is_offline_and_gives_the_same_answer_every_time() {
    // a proxy that nothing listens on, no PATH: whatever runs must not need a network, a model or an index
    let go = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_fastroute"))
            .args(["constraints", "validate", "--ir", &s(&ir()), "--kicad-pcb", &s(&carrier()), "--json", "-q"])
            .env("PATH", "")
            .env("HTTPS_PROXY", "http://127.0.0.1:9")
            .env("HTTP_PROXY", "http://127.0.0.1:9")
            .env("ALL_PROXY", "http://127.0.0.1:9")
            .output()
            .unwrap()
    };
    let (a, b) = (go(), go());
    assert_eq!((code(&a), code(&b)), (5, 5));
    assert_eq!(a.stdout, b.stdout);
    assert_eq!(a.stdout, std::fs::read(pack_dir().join("reports/validation-legal.json")).unwrap(), "same answer as the committed one");
}

#[test]
fn evidence_from_the_datasheet_of_a_similar_module_is_refused() {
    let d = tmp("esp-similar");
    let mut p = fr_constraint::hcp::parse(&std::fs::read_to_string(pack()).unwrap()).unwrap();
    // the WROOM-2 datasheet (in the inventory, for another module) cited for a fact about the WROOM-1-N8R8
    p["devices"].as_array_mut().unwrap().push(json!({
        "id": "DEV-ESP32S3-WROOM2-N16R8V", "vendor": "Espressif", "part_number": "ESP32-S3-WROOM-2-N16R8V", "package": "Module",
        "pins": [{"number": "1", "name": "GND", "class": "ground"}]
    }));
    p["sources"].as_array_mut().unwrap().push(json!({
        "source_id": "SRC-ESP32S3-WROOM2-DS", "document_id": "DOC-ESP32S3-WROOM2-DS", "title": "ESP32-S3-WROOM-2 Datasheet", "version": "v1.7", "level": "official_manual",
        "url": "https://www.espressif.com/sites/default/files/documentation/esp32-s3-wroom-2_datasheet_en.pdf", "retrieved": "2026-11-16",
        "sha256": "71701928d62519658cd40e4e5f001c4e16bef8ab634e5c9ada56f3535896ae1c", "applies_to": ["DEV-ESP32S3-WROOM2-N16R8V"]
    }));
    for r in p["rules"].as_array_mut().unwrap() {
        if r["id"] == "C-ESP-RESERVED-PSRAM" {
            r["evidence"][1]["source_id"] = json!("SRC-ESP32S3-WROOM2-DS");
        }
    }
    let f = d.join("similar.json");
    std::fs::write(&f, serde_json::to_string(&p).unwrap()).unwrap();
    let o = run(&["constraints", "check", &s(&f), "--json", "-q"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    let hit = v["diagnostics"].as_array().unwrap().iter().find(|x| x["code"] == "source_not_applicable").unwrap_or_else(|| panic!("{v}"));
    assert_eq!(hit["object"], "C-ESP-RESERVED-PSRAM");
}

// ---- the router and the base design ---------------------------------------------------------------------------------------------

#[test]
fn what_the_router_can_use_is_exported_and_the_rest_is_listed_with_the_reason() {
    let d = tmp("esp-export");
    let out = d.join("out");
    let o = run(&["constraints", "export-fastroute", "--ir", &s(&ir()), "--kicad-pcb", &s(&carrier()), "-o", &s(&out), "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    // the guideline gives the impedance of the USB pair and no spacing or skew: there is nothing to hand to the router, and the
    // unknown values are not made up to have something to hand over
    for f in ["pairs.txt", "tune.txt", "rules.kicad_dru", "route-args.txt"] {
        assert!(!out.join(f).exists(), "{f} was written although no rule can be executed by the router");
    }
    let r = json(&out.join("export-report.json"));
    assert!(r["exported"].as_array().unwrap().is_empty());
    let not_exported = |id: &str| r["not_exported"].as_array().unwrap().iter().find(|x| x["rule_id"] == id).cloned().unwrap_or_else(|| panic!("{id} is not in the list of what was not handed over: {r}"));
    for id in ["C-ESP-USB-SKEW", "C-ESP-USB-GAP", "C-ESP-EN-RC-VALUES"] {
        assert_eq!(not_exported(id)["status"], "unknown", "{id}: unknown rules are never exported");
    }
    assert!(not_exported("C-ESP-USB")["reason"].as_str().unwrap().contains("no spacing, skew or length"), "{r}");
    assert!(not_exported("C-ESP-ANT-KEEPOUT")["reason"].as_str().unwrap().contains("keepouts come from the design"), "{r}");
    // the rules that the validator checks and the router has nothing to do with are said so
    assert!(r["checked_by_the_validator_not_the_router"].as_array().unwrap().iter().any(|x| x == "C-ESP-NEED-POWER"), "{r}");
}

struct Edit {
    dir: PathBuf,
    request: Value,
}

fn edit_env(name: &str) -> Edit {
    let dir = tmp(name);
    let req = dir.join("request.json");
    let o = run(&["constraints", "edit-request", "--pack", &s(&pack()), "--requirement", "add a status LED on a free pin", "-o", &s(&req), "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    Edit { dir, request: json(&req) }
}

impl Edit {
    fn check(&self, changes: Value, extra: impl FnOnce(&mut Value)) -> (i32, Value) {
        let mut d = json!({
            "format": "fastroute-hcp-edit-draft", "draft_version": "1.0",
            "pack": {"id": self.request["pack"]["id"], "version": self.request["pack"]["version"], "sha256": self.request["pack"]["sha256"]},
            "base": {"id": self.request["base"]["id"], "binding_sha256": self.request["base"]["binding_sha256"]},
            "changes": changes
        });
        extra(&mut d);
        let f = self.dir.join("draft.json");
        std::fs::write(&f, serde_json::to_string(&d).unwrap()).unwrap();
        let o = run(&["constraints", "check-edit", &s(&f), "--pack", &s(&pack()), "--kicad-pcb", &s(&carrier()), "--base-dir", &s(&pack_dir().join("example")), "--json", "-q"]);
        (code(&o), serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{e}: {}", stderr(&o))))
    }
}

fn led() -> Value {
    json!([
        {"id": "CH-1", "op": "add_component", "region": "REG-ESP-USER", "block": "BLK-ESP-LED", "role": "led", "ref": "D1", "value": "LED", "at": [10.0, 20.0]},
        {"id": "CH-2", "op": "add_component", "region": "REG-ESP-USER", "block": "BLK-ESP-LED", "role": "series", "ref": "R2", "value": "330", "at": [14.0, 20.0]},
        {"id": "CH-3", "op": "connect", "region": "REG-ESP-USER", "ref": "U1", "pin": "4", "net": "LED_DRIVE"},
        {"id": "CH-4", "op": "connect", "region": "REG-ESP-USER", "ref": "R2", "pin": "1", "net": "LED_DRIVE"},
        {"id": "CH-5", "op": "connect", "region": "REG-ESP-USER", "ref": "R2", "pin": "2", "net": "LED_A"},
        {"id": "CH-6", "op": "connect", "region": "REG-ESP-USER", "ref": "D1", "pin": "1", "net": "LED_A"},
        {"id": "CH-7", "op": "connect", "region": "REG-ESP-USER", "ref": "D1", "pin": "2", "net": "GND"},
        {"id": "CH-8", "op": "add_track", "region": "REG-ESP-USER", "net": "LED_DRIVE", "layer": "F.Cu", "width_mm": 0.25, "points": [[14.0, 20.0], [18.0, 20.0], [18.0, 28.0]]}
    ])
}

#[test]
fn the_ai_may_add_the_led_in_the_free_area_and_nothing_else() {
    let e = edit_env("esp-edit-ok");
    // the whitelist in the request: one region, one block, two free pins, and the protected pins with the rule that protects each
    let a = &e.request["authorization"];
    assert_eq!(a["regions"].as_array().unwrap().len(), 1);
    assert_eq!(a["regions"][0]["pin_pool"][0]["pins"], json!(["4", "5"]));
    let protected: Vec<String> = a["protected_pins"].as_array().unwrap().iter().map(|p| format!("{}:{}", p["pin"].as_str().unwrap(), p["rule"].as_str().unwrap())).collect();
    for want in ["28:C-ESP-RESERVED-PSRAM", "27:C-ESP-BOOT-IO0", "3:C-ESP-EN", "1:C-ESP-GND-1"] {
        assert!(protected.iter().any(|p| p == want), "{want} in {protected:?}");
    }
    let (c, v) = e.check(led(), |_| {});
    assert_eq!(c, 0, "{v}");
    assert_eq!((v["status"].as_str(), v["complete"].as_bool(), v["base"]["verified"].as_bool()), (Some("accepted"), Some(true), Some(true)));
    assert_eq!(v["validation"]["new_failures"].as_array().unwrap().len(), 0, "{}", v["validation"]);
}

#[test]
fn every_kind_of_overreach_is_rejected_at_the_boundary_it_crosses() {
    let e = edit_env("esp-edit-reject");
    let one = |ch: Value| e.check(json!([ch]), |_| {});
    let codes = |v: &Value| -> Vec<String> { v["findings"].as_array().unwrap().iter().filter(|f| f["severity"] == "error").map(|f| f["code"].as_str().unwrap().to_string()).collect() };
    let cases: Vec<(&str, Value, &str)> = vec![
        ("a part outside the free area, in the antenna keep-out", json!({"id": "CH-1", "op": "add_component", "region": "REG-ESP-USER", "block": "BLK-ESP-LED", "role": "led", "ref": "D1", "at": [30.0, 8.0]}), "outside_region"),
        ("copper that leaves the free area", json!({"id": "CH-1", "op": "add_track", "region": "REG-ESP-USER", "net": "SIG", "layer": "F.Cu", "width_mm": 0.25, "points": [[10.0, 20.0], [30.0, 20.0]]}), "crosses_region"),
        ("the PSRAM pin IO35", json!({"id": "CH-1", "op": "connect", "region": "REG-ESP-USER", "ref": "U1", "pin": "28", "net": "SIG"}), "reserved_pin_used"),
        ("the boot pin IO0", json!({"id": "CH-1", "op": "connect", "region": "REG-ESP-USER", "ref": "U1", "pin": "27", "net": "SIG"}), "reserved_pin_used"),
        ("a free pin that is not in the pool (IO6)", json!({"id": "CH-1", "op": "connect", "region": "REG-ESP-USER", "ref": "U1", "pin": "6", "net": "SIG"}), "pin_not_in_pool"),
        ("an unauthorised peripheral (the supply capacitor block)", json!({"id": "CH-1", "op": "add_component", "region": "REG-ESP-USER", "block": "BLK-ESP-POWER", "role": "bulk", "ref": "C9", "at": [10.0, 20.0]}), "unauthorized_block"),
        ("moving the module", json!({"id": "CH-1", "op": "move_component", "region": "REG-ESP-USER", "ref": "U1", "to": [20.0, 20.0]}), "op_not_allowed"),
        ("the antenna keep-out as an editable region", json!({"id": "CH-1", "op": "add_via", "region": "REG-ESP-ANT", "net": "GND", "at": [20.0, 8.0], "diameter_mm": 0.6}), "region_not_editable"),
        ("a net that carries the reset pin", json!({"id": "CH-1", "op": "connect", "region": "REG-ESP-USER", "ref": "R2", "pin": "1", "net": "EN"}), "protected_net"),
    ];
    for (what, ch, expected) in cases {
        let (c, v) = one(ch);
        assert_eq!(c, 1, "{what}: {v}");
        assert_eq!(v["status"], "rejected", "{what}");
        assert!(codes(&v).iter().any(|x| x == expected), "{what}: expected {expected}, got {:?}", codes(&v));
        assert_eq!(v["diff"]["not_applied"].as_array().unwrap().len(), 1, "{what}");
    }
    // the reserved pin is refused with the quotation of the rule that protects it
    let (_, v) = one(json!({"id": "CH-1", "op": "connect", "region": "REG-ESP-USER", "ref": "U1", "pin": "28", "net": "SIG"}));
    let f = v["findings"].as_array().unwrap().iter().find(|f| f["code"] == "reserved_pin_used").unwrap();
    assert!(f["message"].as_str().unwrap().contains("C-ESP-RESERVED-PSRAM") && f["evidence"].to_string().contains("Octal SPI PSRAM"), "{f}");
    // the same for a part that the draft adds
    let mut ch = led();
    ch[6] = json!({"id": "CH-7", "op": "connect", "region": "REG-ESP-USER", "ref": "D1", "pin": "2", "net": "EN"});
    let (c, v) = e.check(ch, |_| {});
    assert_eq!(c, 1);
    assert!(codes(&v).contains(&"protected_net".to_string()), "{:?}", codes(&v));
}

#[test]
fn a_block_that_a_vector_search_recalled_does_not_widen_the_authority() {
    let e = edit_env("esp-edit-recall");
    let change = json!([{"id": "CH-1", "op": "add_component", "region": "REG-ESP-USER", "block": "BLK-ESP-POWER", "role": "bulk", "ref": "C9", "at": [10.0, 20.0]}]);
    let (c1, plain) = e.check(change.clone(), |_| {});
    let (c2, recalled) = e.check(change, |d| d["recall"] = json!([{"kind": "block", "id": "BLK-ESP-POWER", "source": "vector", "score": 0.97}]));
    assert_eq!((c1, c2), (1, 1));
    let errors = |v: &Value| -> Vec<String> { v["findings"].as_array().unwrap().iter().filter(|f| f["severity"] == "error").map(|f| f["code"].as_str().unwrap().to_string()).collect() };
    assert_eq!(errors(&plain), errors(&recalled));
    assert!(recalled["findings"].to_string().contains("a search result does not authorise anything"));
    // and a draft that claims the block, the pin or the region for itself is refused as such
    let (c, v) = e.check(led(), |d| d["claims"] = json!({"blocks": ["BLK-ESP-LED", "BLK-ESP-POWER"], "pins": [{"device": "DEV-ESP32S3-WROOM1-N8R8", "pins": ["4", "28"]}], "regions": ["REG-ESP-USER", "REG-ESP-ANT"]}));
    assert_eq!(c, 1);
    let msg = v["findings"].as_array().unwrap().iter().find(|f| f["code"] == "widened_authorization").unwrap()["message"].as_str().unwrap().to_string();
    for want in ["BLK-ESP-POWER", "DEV-ESP32S3-WROOM1-N8R8.28", "REG-ESP-ANT"] {
        assert!(msg.contains(want), "{msg}");
    }
}

#[test]
fn a_changed_base_board_needs_a_review_and_the_original_is_never_touched() {
    let e = edit_env("esp-edit-base");
    let d = tmp("esp-edit-base-copy");
    std::fs::copy(carrier(), d.join("carrier.kicad_pcb")).unwrap();
    let before = std::fs::read(carrier()).unwrap();
    // the open board was updated after the pack was made
    let mut text = std::fs::read_to_string(d.join("carrier.kicad_pcb")).unwrap();
    text.push_str("\n");
    std::fs::write(d.join("carrier.kicad_pcb"), text).unwrap();
    let draft = e.dir.join("draft.json");
    let (_c, _v) = e.check(led(), |_| {});
    let o = run(&["constraints", "check-edit", &s(&draft), "--pack", &s(&pack()), "--kicad-pcb", &s(&d.join("carrier.kicad_pcb")), "--base-dir", &s(&d), "--json", "-q"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["status"], "needs_review");
    assert!(v["findings"].to_string().contains("base_changed"), "{v}");
    assert_eq!(std::fs::read(carrier()).unwrap(), before);
}

#[test]
fn the_carrier_has_no_error_in_kicads_own_drc() {
    // KiCad 8 or newer, when there is one (the other tests run with an empty PATH, this one looks for the tool by hand)
    let path = std::env::var_os("PATH").unwrap_or_default();
    let Some(kicad_cli) = std::env::split_paths(&path).map(|d| d.join("kicad-cli")).find(|p| p.is_file()) else {
        eprintln!("kicad-cli not found: skipped");
        return;
    };
    let v = std::process::Command::new(&kicad_cli).arg("--version").output().unwrap();
    let major: u32 = String::from_utf8_lossy(&v.stdout).trim().split('.').next().and_then(|x| x.parse().ok()).unwrap_or(0);
    if major < 8 {
        eprintln!("kicad-cli {major} has no `pcb drc`: skipped");
        return;
    }
    let d = tmp("esp-drc");
    let out = d.join("drc.json");
    let board = d.join("carrier.kicad_pcb");
    std::fs::copy(carrier(), &board).unwrap();
    let o = std::process::Command::new(&kicad_cli).args(["pcb", "drc", "--severity-error", "--format", "json", "-o"]).arg(&out).arg(&board).output().unwrap();
    assert!(out.is_file(), "{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    let r = json(&out);
    let errors: Vec<&Value> = r["violations"].as_array().unwrap().iter().filter(|x| x["severity"] == "error").collect();
    assert!(errors.is_empty(), "KiCad's DRC finds errors on the carrier: {errors:#?}");
}
