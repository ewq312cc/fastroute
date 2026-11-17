//! The official Espressif packs of `hardware-packs/` (chip and module for ESP32, ESP32-S3 and ESP32-C3): the tree verifies, the release builds
//! and installs offline, chips and modules keep their own facts, a legal design for each module passes and each kind of mistake fails at the rule
//! that is made for it, what no document gives stays unknown, and coverage.yaml says exactly what the packs cover.
//!
//! The designs are made from the packs themselves (a module, the parts of its blocks, the nets of its pins): nothing in this file is a
//! number from a datasheet. No network, no model and no index is used; the documents are not needed (they were checked when the packs were
//! built, and `hcp-manager evidence verify` repeats that when they are cached).

use fr_constraint::compile;
use fr_constraint::diag::Severity;
use fr_constraint::hcp;
use fr_constraint::resolve::canonical_sha256;
use fr_constraint::snapshot::Snapshot;
use fr_constraint::validate::{validate, Validation, Verdict};
use hcp_manager::release::{self, Pack, Release};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

const CHIPS: [&str; 3] = ["espressif/esp32-s3/esp32-s3/qfn56", "espressif/esp32/esp32/d0wd-v3", "espressif/esp32-c3/esp32-c3/qfn32"];
const MODULES: [&str; 3] = ["espressif/esp32-s3/esp32-s3-wroom-1/n8r8", "espressif/esp32/esp32-wroom-32e/n4", "espressif/esp32-c3/esp32-c3-wroom-02/n4"];

fn packs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../hardware-packs")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hcp-esp-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
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

// ---- the tree and the release ------------------------------------------------------------------------------------------------

#[test]
fn the_six_packs_verify_and_the_catalogue_is_the_lock() {
    let all = tree();
    let ids: Vec<&str> = all.iter().map(|p| p.id.as_str()).collect();
    for id in CHIPS.iter().chain(MODULES.iter()) {
        assert!(ids.contains(id), "{id} is not in the tree: {ids:?}");
    }
    for c in CHIPS {
        let p = pack(&all, c);
        assert_eq!((p.kind.as_str(), p.dependencies().len()), ("chip", 0), "{c}");
    }
    for m in MODULES {
        let p = pack(&all, m);
        assert_eq!(p.kind, "module");
        let deps = p.dependencies();
        assert_eq!(deps.len(), 1, "{m}: a module depends on exactly its chip");
        assert_eq!(pack(&all, &deps[0].0).kind, "chip");
        assert_eq!(p.manifest["review"], "draft", "nobody has reviewed these packs");
    }
    // the same identity scheme as any other vendor: vendor/family/device/variant
    for p in &all {
        assert_eq!(p.id.split('/').count(), 4);
        assert_eq!(p.manifest["identity"]["vendor"], "espressif");
    }
    let o = Command::new(env!("CARGO_BIN_EXE_hcp-manager")).args(["release", "index", "--check", "--packs"]).arg(packs_dir()).output().unwrap();
    assert_eq!(o.status.code(), Some(0), "hardware-packs/index.yaml is stale: run `hcp-manager release index`\n{}", String::from_utf8_lossy(&o.stderr));
}

#[test]
fn the_release_builds_verifies_and_installs_offline_and_is_reproducible() {
    let d = tmp("release");
    let (a, b) = (d.join("a"), d.join("b"));
    release::build(&packs_dir(), &a, true).unwrap_or_else(|e| panic!("{}", e.message));
    release::build(&packs_dir(), &b, true).unwrap();
    let files = |p: &Path| release::read_dir_files(p).unwrap().into_iter().map(|(k, v)| (k, hcp::sha256_hex(&v))).collect::<Vec<_>>();
    assert_eq!(files(&a), files(&b), "the same tree gives the same release");
    let rel = Release::open(&a).unwrap();
    assert!(rel.entries().len() >= 6);
    let (verified, problems) = rel.verify().unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    assert!(verified.len() >= 6);
    // installed with no PATH and a dead proxy: the modules bring their chips
    let ws = d.join("ws");
    let run = |args: &[&str]| Command::new(env!("CARGO_BIN_EXE_hcp-manager")).arg("--workspace").arg(&ws).args(args).env("PATH", "").env("HTTPS_PROXY", "http://127.0.0.1:9").output().unwrap();
    for m in MODULES {
        let o = run(&["--json", "packs", "builtin", "--release", a.to_str().unwrap(), "install", m]);
        assert_eq!(o.status.code(), Some(0), "{m}: {}", String::from_utf8_lossy(&o.stderr));
    }
    let listed: Value = serde_json::from_slice(&run(&["--json", "packs", "list"]).stdout).unwrap();
    assert_eq!(listed["packs"].as_array().unwrap().len(), 6);
    assert!(listed["packs"].as_array().unwrap().iter().all(|p| p["valid"] == true), "{listed}");
    // compile again from the installed packs: the same IR as from the tree, with the index deleted or never made
    for m in MODULES {
        let o = run(&["packs", "resolve", m]);
        assert_eq!(o.status.code(), Some(0));
        let from_install = hcp::parse(&String::from_utf8_lossy(&o.stdout)).unwrap();
        let all = tree();
        let from_tree = flat(&all, m);
        let ir_a = serde_json::to_string(&compile::compile(&from_install).unwrap()).unwrap();
        let ir_b = serde_json::to_string(&compile::compile(&from_tree).unwrap()).unwrap();
        // (the merged documents differ only in where they came from: x_ keys are not part of what is compiled)
        assert_eq!(ir_a, ir_b, "{m}: the installed copies compile to the IR of the tree");
    }
    std::fs::create_dir_all(ws.join("derived")).unwrap();
    std::fs::write(ws.join("derived/index.db"), b"gone").unwrap();
    std::fs::remove_dir_all(ws.join("derived")).unwrap();
    assert_eq!(run(&["packs", "resolve", MODULES[0]]).status.code(), Some(0), "no index is needed");
}

#[test]
fn chips_and_modules_keep_their_own_facts() {
    let all = tree();
    for c in CHIPS {
        let p = pack(&all, c);
        let doc = &p.hcp;
        // a chip pack knows one device, no module, no antenna, no flash of a module, no board outline
        assert_eq!(doc["devices"].as_array().unwrap().len(), 1, "{c}");
        let dev = &doc["devices"][0];
        assert!(!dev["part_number"].as_str().unwrap().contains("WROOM"), "{c}: {}", dev["part_number"]);
        if let Some(r) = doc["regions"].as_array().and_then(|a| a.first()) {
            panic!("{c} has a region {}: regions (keep-outs, outline, holes) belong to modules and boards", r["id"]);
        }
        for s in doc["sources"].as_array().unwrap() {
            let d = s["document_id"].as_str().unwrap();
            assert!(!d.contains("WROOM"), "{c} cites a module document: {d}");
        }
        for r in doc["rules"].as_array().unwrap() {
            let sub = &r["subject"];
            assert!(sub.get("device").is_none() || sub["device"] == dev["id"], "{c}: rule {} is about another device", r["id"]);
            assert!(r["kind"] != "keepout" && r["kind"] != "mechanical_fixed", "{c}: {}", r["id"]);
        }
    }
    for m in MODULES {
        let p = pack(&all, m);
        let own: Vec<&str> = p.hcp["devices"].as_array().unwrap().iter().map(|d| d["id"].as_str().unwrap()).collect();
        assert_eq!(own.len(), 1, "{m}: the module's own document has the module device only; the chip's comes by inheritance");
        // every rule of the module's own document is about the module (or a block of it, or nothing in particular)
        for r in p.hcp["rules"].as_array().unwrap() {
            let sub = &r["subject"];
            assert!(sub.get("device").is_none() || sub["device"] == own[0], "{m}: rule {} is about another device", r["id"]);
        }
        // what the chip pack says about the chip comes in by name, and is not written again
        let up = &p.hcp["pack"]["upstream"][0];
        let inherited: Vec<&str> = up["inherit"]["rules"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert!(!inherited.is_empty());
        let module_ids: Vec<&str> = p.hcp["rules"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert!(inherited.iter().all(|i| !module_ids.contains(i)), "{m}: an inherited rule is also written in the module pack");
        // nothing of the module is in the chip pack that it depends on
        let chip = pack(&all, &p.dependencies()[0].0);
        let chip_dev = chip.hcp["devices"][0]["part_number"].as_str().unwrap();
        assert!(p.manifest["identity"]["part_number"].as_str().unwrap() != chip_dev);
        let ids_in_chip: Vec<String> = chip.hcp["rules"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap().to_string()).collect();
        assert!(module_ids.iter().all(|i| !ids_in_chip.contains(&i.to_string())));
    }
    // different families share nothing: no id, no source id, no document
    for (i, a) in MODULES.iter().enumerate() {
        for b in &MODULES[i + 1..] {
            let (x, y) = (pack(&all, a), pack(&all, b));
            let ids = |p: &Pack, sec: &str, key: &str| -> Vec<String> { p.hcp[sec].as_array().into_iter().flatten().filter_map(|o| o[key].as_str().map(str::to_string)).collect() };
            for (sec, key) in [("rules", "id"), ("peripheral_blocks", "id"), ("devices", "id"), ("sources", "document_id")] {
                let (xa, ya) = (ids(x, sec, key), ids(y, sec, key));
                assert!(xa.iter().all(|i| !ya.contains(i)), "{a} and {b} share a {sec} id");
            }
        }
    }
}

// ---- designs made from the packs ----------------------------------------------------------------------------------------------

struct Fx {
    flat: Value,
    ir: fr_constraint::ir::Ir,
    dev: String,
}

fn fx(all: &[Pack], id: &str) -> Fx {
    let flat = flat(all, id);
    let part = pack(all, id).manifest["identity"]["part_number"].as_str().unwrap().to_string();
    let dev = flat["devices"].as_array().unwrap().iter().find(|d| d["part_number"] == part.as_str()).unwrap()["id"].as_str().unwrap().to_string();
    let ir = compile::compile(&flat).unwrap_or_else(|e| panic!("{id}: {e:#?}"));
    Fx { flat, ir, dev }
}

fn parse_nf(spec: &Value) -> Option<String> {
    spec["min_value_nf"].as_f64().map(|n| format!("{}nF", n.max(1.0)))
}

/// A design that has the module, the parts of every block of the pack on the nets of their pins, ground on the ground pins and the supply on the supply pin.
fn legal(f: &Fx) -> Value {
    let dev = f.flat["devices"].as_array().unwrap().iter().find(|d| d["id"] == f.dev.as_str()).unwrap();
    let mut pin_net: std::collections::BTreeMap<String, String> = Default::default();
    for p in dev["pins"].as_array().unwrap() {
        let n = p["number"].as_str().unwrap().to_string();
        match p["class"].as_str().unwrap() {
            "power" => {
                pin_net.insert(n, "3V3".into());
            }
            "ground" => {
                pin_net.insert(n, "GND".into());
            }
            _ => {}
        }
    }
    let mut components = vec![json!({"ref": "U1", "device": f.dev, "value": "module", "x": 50.0, "y": 50.0, "rotation_deg": 0.0})];
    let mut parts: Vec<Value> = vec![];
    let mut counters: std::collections::BTreeMap<&str, u32> = Default::default();
    for b in f.flat["peripheral_blocks"].as_array().into_iter().flatten().filter(|b| b["device"] == f.dev.as_str()) {
        let Some(specs) = b["components"].as_array() else { continue };
        let conns = b["connects"].as_array().cloned().unwrap_or_default();
        for c in &conns {
            let pin = c["pin"].as_str().unwrap().to_string();
            pin_net.entry(pin.clone()).or_insert_with(|| format!("N_{}_{pin}", b["id"].as_str().unwrap().replace("BLK-", "")));
        }
        let Some(first) = conns.first() else { continue };
        let net = pin_net[first["pin"].as_str().unwrap()].clone();
        for s in specs {
            for _ in 0..s["min_count"].as_u64().unwrap_or(1) {
                let (prefix, value) = match s["kind"].as_str().unwrap() {
                    "capacitor" => ("C", parse_nf(s).unwrap_or_else(|| "100nF".into())),
                    "resistor" => ("R", s["value_ohm"].as_f64().map(|o| if o >= 1000.0 { format!("{}k", o / 1000.0) } else { format!("{o}") }).unwrap_or_else(|| "10k".into())),
                    "switch" => ("SW", "button".into()),
                    "connector" => ("J", "header".into()),
                    _ => ("X", "part".into()),
                };
                let n = counters.entry(prefix).or_insert(0);
                *n += 1;
                let r = format!("{prefix}{n}");
                let (x, y) = (100.0 + 5.0 * *n as f64, 100.0 + 8.0 * counters.len() as f64);
                components.push(json!({"ref": r, "value": value, "x": x, "y": y}));
                parts.push(json!({"ref": r, "pin": "1", "net": net, "x": x, "y": y}));
                parts.push(json!({"ref": r, "pin": "2", "net": "GND", "x": x + 1.0, "y": y}));
            }
        }
    }
    let mut pins: Vec<Value> = dev["pins"].as_array().unwrap().iter().enumerate().map(|(i, p)| {
        let n = p["number"].as_str().unwrap();
        let mut v = json!({"ref": "U1", "pin": n, "x": 40.0 + (i % 10) as f64, "y": 40.0 + (i / 10) as f64});
        if let Some(net) = pin_net.get(n) {
            v["net"] = json!(net);
        }
        v
    }).collect();
    pins.extend(parts);
    json!({"snapshot_version": "1.0", "project": {"name": "minimal"}, "components": components, "pins": pins, "tracks": [], "outline": [[0.0, 0.0], [200.0, 0.0], [200.0, 200.0], [0.0, 200.0]], "mount_holes": []})
}

fn run(f: &Fx, design: &Value) -> Validation {
    let snap: Snapshot = serde_json::from_value(design.clone()).unwrap();
    validate(&f.ir, &snap, "minimal", None).unwrap()
}

fn failed(v: &Validation) -> Vec<String> {
    let mut f: Vec<String> = v.findings.iter().filter(|x| x.verdict == Verdict::Fail).map(|x| x.rule_id.clone()).collect();
    f.sort();
    f.dedup();
    f
}

fn rule_of<'a>(f: &'a Fx, kind: &str, pred: impl Fn(&Value) -> bool) -> &'a str {
    f.flat["rules"].as_array().unwrap().iter().find(|r| r["kind"] == kind && r["modifiability"] != "unknown" && pred(r)).unwrap_or_else(|| panic!("no {kind} rule")).get("id").and_then(Value::as_str).unwrap()
}

fn set_net(d: &mut Value, pin: &str, net: Option<&str>) {
    for p in d["pins"].as_array_mut().unwrap() {
        if p["ref"] == "U1" && p["pin"] == pin {
            match net {
                Some(n) => p["net"] = json!(n),
                None => {
                    p.as_object_mut().unwrap().remove("net");
                }
            }
        }
    }
}

#[test]
fn a_legal_design_passes_for_each_module_and_says_what_it_did_not_check() {
    let all = tree();
    for m in MODULES {
        let f = fx(&all, m);
        let v = run(&f, &legal(&f));
        assert_eq!(v.result, "pass", "{m}: {:#?}", v.findings.iter().filter(|x| x.verdict == Verdict::Fail).collect::<Vec<_>>());
        assert!(!v.complete, "{m}: unknown and unsupported rules are never a complete pass");
        // the documents' gaps are findings, not passes
        let unknown: Vec<&str> = v.findings.iter().filter(|x| x.verdict == Verdict::Unknown).map(|x| x.rule_id.as_str()).collect();
        assert!(unknown.iter().any(|r| r.contains("EN-RC-VALUES")), "{m}: {unknown:?}");
        let ant = f.ir.rules.iter().find(|r| r.kind == "keepout").unwrap();
        let verdict = v.findings.iter().filter(|x| x.rule_id == ant.rule_id).map(|x| x.verdict).min().unwrap();
        assert!(matches!(verdict, Verdict::Pass | Verdict::Unknown | Verdict::NotApplicable), "{m}: {verdict:?}");
        // the electrical limits are reported, not checked
        let el = f.ir.rules.iter().find(|r| r.kind == "electrical").unwrap();
        assert!(v.findings.iter().filter(|x| x.rule_id == el.rule_id).all(|x| matches!(x.verdict, Verdict::Unchecked | Verdict::Unsupported | Verdict::Pass)), "{m}");
    }
}

#[test]
fn each_mistake_fails_at_the_rule_made_for_it() {
    let all = tree();
    for m in MODULES {
        let f = fx(&all, m);
        let base = legal(&f);
        let dev = f.flat["devices"].as_array().unwrap().iter().find(|d| d["id"] == f.dev.as_str()).unwrap().clone();
        let pins_of = |class: &str| -> Vec<String> { dev["pins"].as_array().unwrap().iter().filter(|p| p["class"] == class).map(|p| p["number"].as_str().unwrap().to_string()).collect() };
        let mut checked = vec![];

        // a ground pin on the supply
        let gnd = pins_of("ground");
        let mut d = base.clone();
        set_net(&mut d, &gnd[0], Some("3V3"));
        let rule = rule_of(&f, "fixed_net", |r| r["params"]["pin"] == gnd[0].as_str());
        assert!(failed(&run(&f, &d)).contains(&rule.to_string()), "{m}: a ground pin on the supply must fail {rule}");
        checked.push("ground on the supply");

        // RST: the reset pin floating, or held low for ever
        let rst = rule_of(&f, "reset_pin", |r| r["subject"]["device"] == f.dev.as_str());
        let rst_pin = f.flat["rules"].as_array().unwrap().iter().find(|r| r["id"] == rst).unwrap()["subject"]["pin"].as_str().unwrap().to_string();
        for net in [None, Some("GND")] {
            let mut d = base.clone();
            set_net(&mut d, &rst_pin, net);
            assert!(failed(&run(&f, &d)).contains(&rst.to_string()), "{m}: the reset pin on {net:?} must fail {rst}");
        }
        checked.push("RST conflict");

        // BOOT: the download pin tied to ground, or to the supply
        let boot = rule_of(&f, "boot_pin", |r| r["subject"]["device"] == f.dev.as_str());
        let boot_pin = f.flat["rules"].as_array().unwrap().iter().find(|r| r["id"] == boot).unwrap()["subject"]["pin"].as_str().unwrap().to_string();
        for net in ["GND", "3V3"] {
            let mut d = base.clone();
            set_net(&mut d, &boot_pin, Some(net));
            assert!(failed(&run(&f, &d)).contains(&boot.to_string()), "{m}: the boot pin on {net} must fail {boot}");
        }
        checked.push("BOOT conflict");

        // a required circuit deleted: the reset circuit, the boot button (their parts are taken out)
        for kind in ["reset_circuit", "boot_button"] {
            let block = f.flat["peripheral_blocks"].as_array().unwrap().iter().find(|b| b["kind"] == kind && b["device"] == f.dev.as_str()).unwrap()["id"].as_str().unwrap().to_string();
            let rule = rule_of(&f, "required_block", |r| r["subject"]["block"] == block.as_str());
            // the parts of the block are the components that are not the module and hang on its nets: remove by net name
            let net_prefix = format!("N_{}_", block.replace("BLK-", ""));
            let mut d = base.clone();
            let gone: Vec<String> = d["pins"].as_array().unwrap().iter().filter(|p| p["ref"] != "U1" && p["net"].as_str().is_some_and(|n| n.starts_with(&net_prefix))).map(|p| p["ref"].as_str().unwrap().to_string()).collect();
            assert!(!gone.is_empty(), "{m}: {block} has parts in the design");
            d["components"].as_array_mut().unwrap().retain(|c| !gone.contains(&c["ref"].as_str().unwrap().to_string()));
            d["pins"].as_array_mut().unwrap().retain(|p| !gone.contains(&p["ref"].as_str().unwrap().to_string()));
            let got = run(&f, &d);
            let verdicts: Vec<Verdict> = got.findings.iter().filter(|x| x.rule_id == rule).map(|x| x.verdict).collect();
            assert!(verdicts.contains(&Verdict::Fail), "{m}: deleting {block} must fail {rule}: {verdicts:?}");
        }
        checked.push("a required circuit deleted");

        // a reserved or not-connected pin used (where the module has them)
        let reserved = f.flat["rules"].as_array().unwrap().iter().find(|r| (r["kind"] == "reserved_pin" || r["kind"] == "nc_pin") && r["modifiability"] != "unknown" && r["subject"]["device"] == f.dev.as_str());
        if let Some(r) = reserved {
            let pin = r["params"]["pins"][0].as_str().unwrap().to_string();
            let mut d = base.clone();
            // a pin is used when something is connected to it: a resistor on the net of the pin
            set_net(&mut d, &pin, Some("SOMETHING"));
            d["components"].as_array_mut().unwrap().push(json!({"ref": "R90", "value": "1k", "x": 120.0, "y": 150.0}));
            d["pins"].as_array_mut().unwrap().push(json!({"ref": "R90", "pin": "1", "net": "SOMETHING", "x": 120.0, "y": 150.0}));
            assert!(failed(&run(&f, &d)).contains(&r["id"].as_str().unwrap().to_string()), "{m}: pin {pin} used must fail {}", r["id"]);
            checked.push("a reserved or not-connected pin used");
        }

        // the antenna keep-out: where the pack has a shape it is checked; where it has none, nothing is claimed
        let ko = f.ir.rules.iter().find(|r| r.kind == "keepout").unwrap();
        let has_shape = f.flat["regions"].as_array().into_iter().flatten().any(|g| g["kind"] == "antenna_keepout");
        if has_shape {
            let g = f.flat["regions"].as_array().unwrap().iter().find(|g| g["kind"] == "antenna_keepout").unwrap();
            let p = g["polygon"].as_array().unwrap();
            let (cx, cy) = ((p[0][0].as_f64().unwrap() + p[2][0].as_f64().unwrap()) / 2.0, (p[0][1].as_f64().unwrap() + p[2][1].as_f64().unwrap()) / 2.0);
            let mut d = base.clone();
            d["components"].as_array_mut().unwrap().push(json!({"ref": "C99", "value": "100nF", "x": 50.0 + cx, "y": 50.0 + cy}));
            d["pins"].as_array_mut().unwrap().push(json!({"ref": "C99", "pin": "1", "net": "GND", "x": 50.0 + cx, "y": 50.0 + cy}));
            assert!(failed(&run(&f, &d)).contains(&ko.rule_id), "{m}: a part in the keep-out must fail {}", ko.rule_id);
            checked.push("in the keep-out");
        } else {
            let v = run(&f, &base);
            let verdict = v.findings.iter().filter(|x| x.rule_id == ko.rule_id).map(|x| x.verdict).min().unwrap();
            assert_eq!(verdict, Verdict::Unknown, "{m}: the size of the keep-out is not in the documents read, so the rule says unknown and no design passes it");
            assert!(!v.complete);
            checked.push("keep-out unknown (not claimed)");
        }
        eprintln!("{m}: {}", checked.join("; "));
        assert!(checked.len() >= 5);
    }
}

#[test]
fn a_high_speed_number_without_evidence_is_not_accepted_and_the_ones_the_documents_do_not_give_stay_unknown() {
    let all = tree();
    for m in MODULES {
        let mut doc = flat(&all, m);
        let usb: Vec<&Value> = doc["rules"].as_array().unwrap().iter().filter(|r| r["kind"] == "high_speed").collect();
        if usb.is_empty() {
            continue; // this series has no USB in its documents here: there is no high-speed rule to invent
        }
        // the skew is unknown in the pack: it is not a number, and it says what is missing
        let skew = usb.iter().find(|r| r["params"].get("skew_mm").is_some()).unwrap();
        assert_eq!(skew["modifiability"], "unknown", "{m}");
        assert!(skew["params"]["skew_mm"].is_null() && skew["missing"].as_str().unwrap().contains("no number"), "{m}");
        // writing a value for it, without a document, is refused by the pack check
        let id = skew["id"].as_str().unwrap().to_string();
        let r = doc["rules"].as_array_mut().unwrap().iter_mut().find(|r| r["id"] == id.as_str()).unwrap();
        r["params"]["skew_mm"] = json!(0.5);
        assert!(hcp::check(&doc).iter().any(|d| d.code == "unknown_value" && d.severity == Severity::Error), "{m}: a number in an unknown rule");
        let r = doc["rules"].as_array_mut().unwrap().iter_mut().find(|r| r["id"] == id.as_str()).unwrap();
        r["modifiability"] = json!("configurable");
        r.as_object_mut().unwrap().remove("missing");
        assert!(hcp::check(&doc).iter().any(|d| d.severity == Severity::Error && (d.code == "schema" || d.code == "weak_evidence")), "{m}: a configurable rule without evidence");
    }
}

#[test]
fn rules_of_one_chip_or_module_are_not_evidence_for_another() {
    let all = tree();
    // 1. a rule of the ESP32 module that cites the datasheet of the C3 module: that document applies to another device
    let mut doc = pack(&all, MODULES[1]).hcp.clone();
    let c3 = flat(&all, MODULES[2]);
    let foreign = c3["sources"].as_array().unwrap().iter().find(|s| s["document_id"] == "DOC-ESP32C3-WROOM02-DS").unwrap().clone();
    doc["sources"].as_array_mut().unwrap().push(foreign);
    doc["rules"][0]["evidence"][0]["source_id"] = json!("SRC-ESP32C3-WROOM02-DS");
    let up = pack(&all, CHIPS[1]).hcp.clone();
    let err = resolve_with(&doc, &[up]).unwrap_err();
    assert!(err.iter().any(|d| d.contains("source_not_applicable")), "{err:?}");
    // 2. a module resolved against the chip of another family: the exact upstream is not there
    let mut wrong = pack(&all, MODULES[1]).hcp.clone();
    let err = resolve_with(&wrong, &[pack(&all, CHIPS[2]).hcp.clone()]).unwrap_err();
    assert!(err.iter().any(|d| d.contains("upstream_missing")), "{err:?}");
    // 3. the right chip, but another version or a changed document: not adopted
    let mut chip = pack(&all, CHIPS[1]).hcp.clone();
    chip["pack"]["version"] = json!("0.2.0");
    assert!(resolve_with(&wrong, &[chip]).unwrap_err().iter().any(|d| d.contains("upstream_missing")));
    let mut chip = pack(&all, CHIPS[1]).hcp.clone();
    chip["rules"][0]["params"]["default_level"] = json!("low");
    assert!(resolve_with(&wrong, &[chip]).unwrap_err().iter().any(|d| d.contains("upstream_digest_mismatch")));
    // 4. a module that claims another revision of its chip: the release has no such pack
    wrong["pack"]["upstream"][0]["version"] = json!("0.9.0");
    assert!(resolve_with(&wrong, &[pack(&all, CHIPS[1]).hcp.clone()]).unwrap_err().iter().any(|d| d.contains("upstream_missing")));
    // 5. a similar variant is another pack: the exact id is needed
    let d = tmp("variant");
    release::build(&packs_dir(), &d, true).unwrap();
    let rel = Release::open(&d).unwrap();
    for near in ["espressif/esp32/esp32-wroom-32e/n8", "espressif/esp32/esp32-wroom-32e", "espressif/esp32/esp32-wroom-32ue/n4", "espressif/esp32-c3/esp32-c3-wroom-02/n8"] {
        assert!(rel.find(near).is_err(), "{near}");
    }
    // 6. a module whose pack lists the chip of another series as a dependency fails the set check
    let mut packs = tree();
    let i = packs.iter().position(|p| p.id == MODULES[1]).unwrap();
    let c3_chip = packs.iter().find(|p| p.id == CHIPS[2]).unwrap().clone();
    packs[i].manifest["dependencies"] = json!([{"id": CHIPS[2], "kind": "chip", "version": "0.1.0", "canonical_sha256": c3_chip.canonical_sha256}]);
    let codes: Vec<String> = release::verify_set(&packs).into_iter().map(|p| p.code).collect();
    assert!(codes.contains(&"pack_invalid".to_string()) || codes.contains(&"upstream_mismatch".to_string()) || codes.contains(&"dependency_digest_mismatch".to_string()), "{codes:?}");
}

fn resolve_with(doc: &Value, ups: &[Value]) -> Result<Value, Vec<String>> {
    fr_constraint::resolve::resolve(doc, ups).map_err(|ds| ds.into_iter().map(|d| format!("{} {}", d.code, d.message)).collect())
}

// ---- no vector, no network -----------------------------------------------------------------------------------------------------

#[test]
fn nothing_of_the_vector_layer_is_in_a_pack_or_the_release_and_the_packs_compile_the_same_without_it() {
    let all = tree();
    for p in &all {
        let text = serde_json::to_string(&p.hcp).unwrap().to_ascii_lowercase();
        for word in ["embedding", "vector", "similarity", "cosine"] {
            assert!(!text.contains(&format!("\"{word}")), "{}: {word}", p.id);
        }
        assert!(hcp::check(&p.hcp).iter().all(|d| d.code != "vector_field"));
    }
    for m in MODULES {
        let a = serde_json::to_string(&fx(&all, m).ir).unwrap();
        let b = serde_json::to_string(&fx(&tree(), m).ir).unwrap();
        assert_eq!(a, b, "{m}: the same IR every time");
        assert_eq!(canonical_sha256(&flat(&all, m)), canonical_sha256(&flat(&all, m)));
    }
}

// ---- coverage.yaml ---------------------------------------------------------------------------------------------------------------

#[test]
fn coverage_says_exactly_what_each_pack_covers_and_never_more() {
    let text = std::fs::read_to_string(packs_dir().join("espressif/coverage.yaml")).unwrap();
    let cov = hcp::parse(&text).unwrap();
    let all = tree();
    let topics = ["power_supply", "ground", "decoupling", "required_external_parts", "boot_strapping", "reset", "download_interface", "clock", "fixed_reserved_pins", "configurable_pins", "high_speed", "mechanical", "module_rf"];
    assert_eq!(cov["series"].as_array().unwrap().len(), 3);
    let mut supported = 0;
    for s in cov["series"].as_array().unwrap() {
        for kind in ["chip", "module"] {
            let id = s[kind].as_str().unwrap();
            let f = flat(&all, id);
            let rules: std::collections::BTreeMap<&str, &Value> = f["rules"].as_array().unwrap().iter().map(|r| (r["id"].as_str().unwrap(), r)).collect();
            for t in topics {
                let e = &s["topics"][t][kind];
                let status = e["status"].as_str().unwrap_or_else(|| panic!("{id}: no status for {t}"));
                assert!(["supported", "unknown", "not_applicable", "conflict"].contains(&status), "{id} {t}: {status}");
                match status {
                    "supported" => {
                        let named: Vec<&str> = e["rules"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
                        assert!(!named.is_empty(), "{id} {t}: supported without a rule");
                        for r in named {
                            let rule = rules.get(r).unwrap_or_else(|| panic!("{id} {t}: no rule {r}"));
                            assert!(rule["modifiability"] != "unknown" && rule["modifiability"] != "conflict", "{id} {t}: {r} is {}", rule["modifiability"]);
                            assert!(rule.get("evidence").is_some_and(|e| !e.as_array().unwrap().is_empty()), "{id} {t}: {r} has no evidence");
                            supported += 1;
                        }
                    }
                    _ => assert!(e["reason"].as_str().is_some_and(|r| r.len() > 15), "{id} {t}: {status} without a reason"),
                }
            }
            // every `unknown` rule of the pack is mentioned where it belongs: an unknown rule is not hidden by a `supported` topic's name
            for (rid, r) in &rules {
                if r["modifiability"] == "unknown" {
                    let said = text.contains(rid);
                    assert!(said, "{id}: the unknown rule {rid} is not mentioned in coverage.yaml");
                }
            }
        }
    }
    assert!(supported > 50, "coverage.yaml names suspiciously few supported rules: {supported}");
    assert!(cov["conflicts"].as_array().unwrap().is_empty(), "no conflict is recorded: if the documents disagree, a `conflict` rule and an entry here are needed");
}
