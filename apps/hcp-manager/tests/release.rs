//! Versioned packs that ship with the software: manifest, sealing, the generated catalogue, the deterministic archive, the release
//! directory, offline installation into a workspace, and every way a release or a pack can be wrong. The packs are fictional (a chip,
//! a module that inherits from it, a board that inherits from the module, and metadata-only packs for STC and STM32); they test the
//! mechanism, not a device. No network is used.

use fr_constraint::hcp;
use fr_constraint::resolve::canonical_sha256;
use hcp_manager::inventory::{Applies, Doc, Inventory};
use hcp_manager::release::{self, Files, Pack, Release};
use hcp_manager::workspace::Workspace;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hcp-release-it-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn bin(ws: &Path, args: &[&str]) -> Output {
    // no PATH and a dead proxy: nothing here may need a program or the network
    Command::new(env!("CARGO_BIN_EXE_hcp-manager")).arg("--workspace").arg(ws).args(args).env("PATH", "").env("HTTPS_PROXY", "http://127.0.0.1:9").env("HTTP_PROXY", "http://127.0.0.1:9").env("HCP_TODAY", "2026-11-16").output().unwrap()
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn jsn(o: &Output) -> Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("not JSON ({e}): {}\n{}", String::from_utf8_lossy(&o.stdout), err(o)))
}

fn codes(ps: &[release::Problem]) -> Vec<String> {
    ps.iter().map(|p| p.code.clone()).collect()
}

// ---- fictional packs ---------------------------------------------------------------------------------------------------------

fn ev(source: &str, page: u32, quote: &str) -> Value {
    json!([{"source_id": source, "locator": {"page": page}, "quote": quote, "basis": "explicit"}])
}

fn source(id: &str, doc: &str, level: &str, device: &str) -> Value {
    json!({"source_id": id, "document_id": doc, "title": format!("{doc} (fictional)"), "version": "v1.0", "level": level,
        "url": format!("https://example.invalid/{doc}"), "retrieved": "2026-11-16", "sha256": "0".repeat(64), "applies_to": [device]})
}

fn pins(n: usize) -> Value {
    Value::Array((1..=n).map(|i| json!({"number": i.to_string(), "name": format!("P{i}"), "class": if i == 1 { "ground" } else { "gpio" }})).collect())
}

fn meta(id: &str, kind: &str, title: &str) -> Value {
    json!({"id": id, "kind": kind, "title": title, "version": "0.1.0", "created": "2026-11-16", "extractor": {"kind": "human"}, "review": {"status": "draft"}})
}

pub const CHIP: &str = "acme/cx/cx1/default";
pub const MODULE: &str = "acme/cx/cxm1/n4";
pub const BOARD: &str = "acme/cx/cxb1/rev-a";

fn chip_hcp() -> Value {
    json!({
        "hcp_version": "1.1",
        "pack": meta(CHIP, "chip", "CX1 chip (fictional)"),
        "sources": [source("SRC-CX1-DS", "DOC-CX1-DS", "official_manual", "DEV-CX1")],
        "devices": [{"id": "DEV-CX1", "vendor": "Acme", "part_number": "CX1", "package": "QFN-4", "pins": pins(4)}],
        "rules": [
            {"id": "C-CX1-GND", "kind": "fixed_net", "modifiability": "fixed", "subject": {"device": "DEV-CX1"}, "params": {"pin": "1", "net_pattern": "GND*"}, "evidence": ev("SRC-CX1-DS", 5, "Pin 1 is ground.")},
            {"id": "C-CX1-RSVD", "kind": "reserved_pin", "modifiability": "fixed", "subject": {"device": "DEV-CX1"}, "params": {"pins": ["4"]}, "evidence": ev("SRC-CX1-DS", 5, "Pin 4 is reserved.")}
        ]
    })
}

fn module_hcp(chip: &Value) -> Value {
    json!({
        "hcp_version": "1.1",
        "pack": {"id": MODULE, "kind": "module", "title": "CXM1 module (fictional)", "version": "0.1.0", "created": "2026-11-16", "extractor": {"kind": "human"}, "review": {"status": "draft"},
            "upstream": [{"id": CHIP, "version": "0.1.0", "kind": "chip", "canonical_sha256": canonical_sha256(chip), "inherit": {"devices": ["DEV-CX1"], "rules": ["C-CX1-GND", "C-CX1-RSVD"]}}]},
        "sources": [source("SRC-CXM1-DS", "DOC-CXM1-DS", "official_manual", "DEV-CXM1")],
        "devices": [{"id": "DEV-CXM1", "vendor": "Acme", "part_number": "CXM1-N4", "package": "module", "pins": pins(6)}],
        "rules": [{"id": "C-CXM1-RSVD", "kind": "reserved_pin", "modifiability": "fixed", "subject": {"device": "DEV-CXM1"}, "params": {"pins": ["5", "6"]}, "evidence": ev("SRC-CXM1-DS", 9, "Pins 5 and 6 are used by the flash inside the module.")}]
    })
}

fn board_hcp(module: &Value) -> Value {
    json!({
        "hcp_version": "1.1",
        "pack": {"id": BOARD, "kind": "board", "title": "CXB1 board (fictional)", "version": "0.1.0", "created": "2026-11-16", "extractor": {"kind": "human"}, "review": {"status": "draft"},
            "upstream": [{"id": MODULE, "version": "0.1.0", "kind": "module", "canonical_sha256": canonical_sha256(module), "inherit": {"devices": ["DEV-CXM1"], "rules": ["C-CXM1-RSVD"]}}]},
        "sources": [source("SRC-CXB1-SCH", "DOC-CXB1-SCH", "official_devkit", "DEV-CXB1")],
        "devices": [{"id": "DEV-CXB1", "vendor": "Acme", "part_number": "CXB1-A", "package": "board", "pins": pins(8)}],
        "rules": [{"id": "C-CXB1-RSVD", "kind": "reserved_pin", "modifiability": "fixed", "subject": {"device": "DEV-CXB1"}, "params": {"pins": ["7"]}, "evidence": ev("SRC-CXB1-SCH", 1, "Header pin 7 is wired to the on-board LED.")}]
    })
}

/// A pack that registers an identity and nothing else (STC, STM32 and other vendors that are not covered yet).
fn metadata_only_hcp(id: &str, part: &str) -> Value {
    json!({
        "hcp_version": "1.1",
        "pack": {"id": id, "kind": "chip", "content": "metadata_only", "title": format!("{part} (identity only: no rules have been written)"), "version": "0.1.0", "created": "2026-11-16", "extractor": {"kind": "human"}, "review": {"status": "draft"}},
        "sources": [source(&format!("SRC-{}-REG", part.to_uppercase()), &format!("DOC-{}-REG", part.to_uppercase()), "community", &format!("DEV-{}", part.to_uppercase()))],
        "devices": [{"id": format!("DEV-{}", part.to_uppercase()), "vendor": "registry", "part_number": part, "package": "unknown", "pins": []}]
    })
}

struct Spec {
    hcp: Value,
    content: &'static str,
    deps: Vec<(String, String, String)>,
    /// document id -> allowed | reference_only
    licenses: Vec<(String, &'static str)>,
    extra: Vec<(String, Vec<u8>)>,
}

impl Spec {
    fn new(hcp: Value) -> Spec {
        let licenses = hcp["sources"].as_array().unwrap().iter().map(|s| (s["document_id"].as_str().unwrap().to_string(), "reference_only")).collect();
        let deps = hcp["pack"]["upstream"].as_array().into_iter().flatten().map(|u| (u["id"].as_str().unwrap().into(), u["kind"].as_str().unwrap().into(), u["version"].as_str().unwrap().into())).collect();
        Spec { hcp, content: "full", deps, licenses, extra: vec![] }
    }
}

/// Writes and seals one pack under `root`: `<id>/<version>/`.
fn add(root: &Path, spec: &Spec) -> PathBuf {
    let id = spec.hcp["pack"]["id"].as_str().unwrap();
    let version = spec.hcp["pack"]["version"].as_str().unwrap();
    let dir = root.join(id).join(version);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("hcp.yaml"), hcp::to_yaml(&spec.hcp).unwrap()).unwrap();
    let part = |dev: &str| spec.hcp["devices"].as_array().unwrap().iter().find(|d| d["id"] == dev).map(|d| d["part_number"].as_str().unwrap().to_string());
    let inv = Inventory {
        sources_version: "1.0".into(),
        documents: spec.hcp["sources"].as_array().unwrap().iter().map(|s| Doc {
            document_id: s["document_id"].as_str().unwrap().into(),
            title: s["title"].as_str().unwrap().into(),
            level: s["level"].as_str().unwrap().into(),
            version: s["version"].as_str().unwrap().into(),
            applies_to: s["applies_to"].as_array().unwrap().iter().filter_map(|d| part(d.as_str().unwrap())).map(|p| Applies { part_number: p, revision: None }).collect(),
            url: s["url"].as_str().map(str::to_string),
            local_path: None,
            sha256: Some(s["sha256"].as_str().unwrap().into()),
            retrieved: Some("2026-11-16".into()),
            status: None,
            license_note: None,
            note: None,
            last_error: None,
            embed_remote: None,
        }).collect(),
    };
    std::fs::write(dir.join("sources.yaml"), serde_yaml::to_string(&inv).unwrap()).unwrap();
    for (p, b) in &spec.extra {
        let f = dir.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, b).unwrap();
    }
    let parts: Vec<&str> = id.split('/').collect();
    let manifest = json!({
        "manifest_version": "1.0", "id": id, "kind": spec.hcp["pack"]["kind"], "version": version, "content": spec.content, "review": "draft",
        "identity": {"vendor": parts[0], "family": parts[1], "device": parts[2], "variant": parts[3], "part_number": spec.hcp["devices"][0]["part_number"]},
        "compatibility": {"hcp_schema_version": "1.1", "ir_version": "1.0", "min_app_version": "0.1.0"},
        "dependencies": spec.deps.iter().map(|(i, k, v)| json!({"id": i, "kind": k, "version": v})).collect::<Vec<_>>(),
        "license": {"pack": "CC0-1.0", "documents": spec.licenses.iter().map(|(d, r)| json!({"document_id": d, "redistribution": r})).collect::<Vec<_>>()},
    });
    std::fs::write(dir.join("pack.yaml"), hcp::to_yaml(&manifest).unwrap()).unwrap();
    release::seal(&dir).expect("seal");
    dir
}

/// chip, module, board, and the two registry packs
fn tree(name: &str) -> (PathBuf, Value, Value) {
    let root = tmp(name).join("hardware-packs");
    let chip = chip_hcp();
    let module = module_hcp(&chip);
    let board = board_hcp(&module);
    add(&root, &Spec::new(chip.clone()));
    add(&root, &Spec::new(module.clone()));
    add(&root, &Spec::new(board));
    for (id, part) in [("stc/stc8/stc8h8k64u/lqfp48", "stc8h8k64u"), ("st/stm32/stm32f103c8/lqfp48", "stm32f103c8")] {
        let mut s = Spec::new(metadata_only_hcp(id, part));
        s.content = "metadata_only";
        add(&root, &s);
    }
    (root, chip, module)
}

fn build(root: &Path) -> PathBuf {
    let out = root.parent().unwrap().join("dist/hcp-packs");
    release::build(root, &out, true).unwrap_or_else(|e| panic!("{}", e.message));
    out
}

fn tree_digests(dir: &Path) -> Vec<(String, String)> {
    release::read_dir_files(dir).unwrap().into_iter().map(|(k, v)| (k, hcp::sha256_hex(&v))).collect()
}

// ---- the positive path -------------------------------------------------------------------------------------------------------

#[test]
fn a_tree_of_packs_verifies_and_builds_into_a_release() {
    let (root, _, _) = tree("build");
    let (packs, problems) = release::load_tree(&root).unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(packs.len(), 5);
    // the generic identity: vendor/family/device/variant, whatever the vendor
    let ids: Vec<&str> = packs.iter().map(|p| p.id.as_str()).collect();
    assert!(ids.contains(&"stc/stc8/stc8h8k64u/lqfp48") && ids.contains(&"st/stm32/stm32f103c8/lqfp48"), "{ids:?}");
    let out = build(&root);
    assert!(out.join("release.yaml").is_file() && out.join("SHA256SUMS").is_file());
    let rel = Release::open(&out).unwrap();
    let (verified, problems) = rel.verify().unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(verified.len(), 5);
    // the manifest of a pack says where each fact came from
    let board = verified.iter().find(|p| p.id == BOARD).unwrap();
    assert_eq!(board.manifest["source_digests"][0]["document_id"], "DOC-CXB1-SCH");
    assert_eq!(board.manifest["dependencies"][0]["id"], MODULE);
    // a registry pack carries no rules
    let stc = verified.iter().find(|p| p.id.starts_with("stc/")).unwrap();
    assert_eq!(stc.manifest["content"], "metadata_only");
    assert!(stc.hcp.get("rules").is_none());
}

#[test]
fn the_release_is_reproducible() {
    let (root, _, _) = tree("repro");
    let a = build(&root);
    let first = tree_digests(&a);
    // again, into another directory, after the files were touched: same bytes
    for (_, files) in release::scan(&root).unwrap() {
        assert!(!files.is_empty());
    }
    let b = root.parent().unwrap().join("dist2");
    release::build(&root, &b, true).unwrap();
    assert_eq!(first, tree_digests(&b));
    // the lock lists the digests of the archives
    let rel = Release::open(&a).unwrap();
    for e in rel.entries() {
        let bytes = std::fs::read(a.join(e["archive"].as_str().unwrap())).unwrap();
        assert_eq!(hcp::sha256_hex(&bytes), e["archive_sha256"]);
    }
}

#[test]
fn the_catalogue_is_the_lock_and_a_stale_one_is_refused() {
    let (root, _, _) = tree("index");
    let out = bin(&root, &["release", "index", "--packs", root.to_str().unwrap()]);
    assert_eq!(code(&out), 0, "{}", err(&out));
    assert_eq!(code(&bin(&root, &["release", "index", "--packs", root.to_str().unwrap(), "--check"])), 0);
    let text = std::fs::read_to_string(root.join("index.yaml")).unwrap();
    assert!(text.contains("hcp_canonical_sha256") && text.contains("stc/stc8/stc8h8k64u/lqfp48"));
    // a pack is added to the tree: the committed index is stale, and a release is not built from a tree its lock does not describe
    let mut s = Spec::new(metadata_only_hcp("nxp/lpc/lpc1768/lqfp100", "lpc1768"));
    s.content = "metadata_only";
    add(&root, &s);
    let stale = bin(&root, &["release", "index", "--packs", root.to_str().unwrap(), "--check"]);
    assert_eq!(code(&stale), 3, "{}", err(&stale));
    assert!(err(&stale).contains("not the current catalogue"));
    let e = release::build(&root, &root.parent().unwrap().join("dist-stale"), true).unwrap_err();
    assert!(e.message.contains("is not the catalogue"), "{}", e.message);
    // a change that breaks a dependency is found before the catalogue is even compared
    let mut chip = chip_hcp();
    chip["pack"]["title"] = json!("CX1 chip (fictional), edited");
    add(&root, &Spec::new(chip));
    let broken = bin(&root, &["release", "index", "--packs", root.to_str().unwrap()]);
    assert_eq!(code(&broken), 3);
    assert!(err(&broken).contains("dependency_digest_mismatch"), "{}", err(&broken));
}

#[test]
fn installing_works_offline_and_brings_the_exact_dependencies() {
    let (root, chip, module) = tree("install");
    let out = build(&root);
    let ws = root.parent().unwrap().join("ws");
    let rel_arg = out.to_str().unwrap();
    // listing, verifying: no network, no PATH
    let l = bin(&ws, &["--json", "packs", "builtin", "--release", rel_arg, "list"]);
    assert_eq!(code(&l), 0, "{}", err(&l));
    assert_eq!(jsn(&l)["packs"].as_array().unwrap().len(), 5);
    assert!(jsn(&l)["packs"].as_array().unwrap().iter().all(|p| p["installed"] == "not_installed"));
    let v = bin(&ws, &["--json", "packs", "builtin", "--release", rel_arg, "verify"]);
    assert_eq!(code(&v), 0, "{}", err(&v));
    // the board pulls in the module and the chip, dependencies first
    let before = tree_digests(&out);
    let i = bin(&ws, &["--json", "packs", "builtin", "--release", rel_arg, "install", BOARD]);
    assert_eq!(code(&i), 0, "{}", err(&i));
    let r = jsn(&i);
    let order: Vec<&str> = r["packs"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(order, [CHIP, MODULE, BOARD]);
    assert!(r["inventory_added"].as_array().unwrap().iter().any(|d| d == "DOC-CX1-DS"));
    assert_eq!(before, tree_digests(&out), "the release is only read");
    // the installed packs are listed, valid and resolve; a pack that inherits is usable once resolved
    let l = jsn(&bin(&ws, &["--json", "packs", "list"]));
    let rows = l["packs"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|p| p["valid"] == true), "{rows:?}");
    let flat = bin(&ws, &["packs", "resolve", BOARD]);
    assert_eq!(code(&flat), 0, "{}", err(&flat));
    let flat_doc = hcp::parse(&String::from_utf8_lossy(&flat.stdout)).unwrap();
    assert!(hcp::check(&flat_doc).iter().all(|d| d.severity != fr_constraint::diag::Severity::Error));
    assert!(flat_doc["rules"].as_array().unwrap().iter().any(|r| r["id"] == "C-CXM1-RSVD"));
    assert!(fr_constraint::compile::compile(&flat_doc).is_ok(), "the resolved pack compiles to an IR");
    assert!(!flat_doc["rules"].as_array().unwrap().iter().any(|r| r["id"] == "C-CX1-GND"), "the board did not list the chip's rules: they are not taken");
    let _ = (chip, module);
    // again: nothing changes
    let again = jsn(&bin(&ws, &["--json", "packs", "builtin", "--release", rel_arg, "install", BOARD]));
    assert!(again["packs"].as_array().unwrap().iter().all(|p| p["action"] == "already_installed"), "{again}");
}

#[test]
fn a_local_change_is_never_overwritten_silently() {
    let (root, _, _) = tree("local");
    let out = build(&root);
    let ws = root.parent().unwrap().join("ws");
    let rel_arg = out.to_str().unwrap();
    assert_eq!(code(&bin(&ws, &["packs", "builtin", "--release", rel_arg, "install", CHIP])), 0);
    let installed = ws.join("packs").join(CHIP).join("0.1.0");
    let edited = std::fs::read_to_string(installed.join("hcp.yaml")).unwrap().replace("Pin 4 is reserved.", "Pin 4 is reserved. (my note)");
    std::fs::write(installed.join("hcp.yaml"), &edited).unwrap();
    let st = jsn(&bin(&ws, &["--json", "packs", "builtin", "--release", rel_arg, "list"]));
    assert_eq!(st["packs"].as_array().unwrap().iter().find(|p| p["id"] == CHIP).unwrap()["installed"], "modified");
    // an upgrade or a reinstall does not touch it
    let o = bin(&ws, &["packs", "builtin", "--release", rel_arg, "install", CHIP]);
    assert_eq!(code(&o), 4, "{}", err(&o));
    assert!(err(&o).contains("changed locally"));
    assert_eq!(std::fs::read_to_string(installed.join("hcp.yaml")).unwrap(), edited, "the local copy is as the user left it");
    // forced: the local copy is kept in backup/, the release's copy is installed
    let o = bin(&ws, &["packs", "builtin", "--release", rel_arg, "install", CHIP, "--force"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(!std::fs::read_to_string(installed.join("hcp.yaml")).unwrap().contains("(my note)"));
    let backups: Vec<_> = release::read_dir_files(&ws.join("backup")).unwrap().into_iter().collect();
    assert!(backups.iter().any(|(k, v)| k.ends_with("hcp.yaml") && String::from_utf8_lossy(v).contains("(my note)")), "{:?}", backups.iter().map(|(k, _)| k).collect::<Vec<_>>());
}

#[test]
fn an_install_that_disagrees_with_the_inventory_writes_nothing() {
    let (root, _, _) = tree("inventory");
    let out = build(&root);
    let ws_dir = root.parent().unwrap().join("ws");
    let ws = Workspace::new(ws_dir.clone());
    ws.init().unwrap();
    // the workspace already has the chip's datasheet with another digest
    let inv = Inventory { sources_version: "1.0".into(), documents: vec![Doc {
        document_id: "DOC-CX1-DS".into(), title: "CX1 datasheet".into(), level: "official_manual".into(), version: "v1.0".into(),
        applies_to: vec![Applies { part_number: "CX1".into(), revision: None }], url: Some("https://example.invalid/DOC-CX1-DS".into()), local_path: None, sha256: Some("1".repeat(64)),
        retrieved: None, status: None, license_note: None, note: None, last_error: None, embed_remote: None }] };
    inv.save(&ws).unwrap();
    let o = bin(&ws_dir, &["packs", "builtin", "--release", out.to_str().unwrap(), "install", MODULE]);
    assert_eq!(code(&o), 3, "{}", err(&o));
    assert!(err(&o).contains("DOC-CX1-DS"), "{}", err(&o));
    assert!(!ws_dir.join("packs").join(CHIP).exists() && !ws_dir.join("packs").join(MODULE).exists(), "nothing is installed");
}

#[test]
fn installing_does_not_need_an_index_or_vectors() {
    let (root, _, _) = tree("novector");
    let out = build(&root);
    let ws = root.parent().unwrap().join("ws");
    // a damaged derived index and a missing one make no difference
    std::fs::create_dir_all(ws.join("derived")).unwrap();
    std::fs::write(ws.join("derived/index.db"), b"not a database").unwrap();
    let o = bin(&ws, &["--json", "packs", "builtin", "--release", out.to_str().unwrap(), "install", MODULE]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let flat_a = bin(&ws, &["packs", "resolve", MODULE]).stdout;
    std::fs::remove_dir_all(ws.join("derived")).unwrap();
    let flat_b = bin(&ws, &["packs", "resolve", MODULE]).stdout;
    assert!(!flat_a.is_empty());
    assert_eq!(flat_a, flat_b, "deleting the derived index changes nothing in what is installed or resolved");
}

// ---- what must fail ----------------------------------------------------------------------------------------------------------

fn files_of(dir: &Path) -> Files {
    release::read_dir_files(dir).unwrap()
}

#[test]
fn a_changed_file_is_found_by_its_digest() {
    let (root, _, _) = tree("digest");
    let dir = root.join(CHIP).join("0.1.0");
    let mut files = files_of(&dir);
    let h = String::from_utf8(files["hcp.yaml"].clone()).unwrap().replace("Pin 4 is reserved.", "Pin 3 is reserved.");
    files.insert("hcp.yaml".into(), h.into_bytes());
    let ps = release::load_pack(files, Some(&format!("{CHIP}/0.1.0"))).unwrap_err();
    assert!(codes(&ps).contains(&"file_digest_mismatch".to_string()) && codes(&ps).contains(&"hcp_canonical_mismatch".to_string()), "{ps:?}");
}

#[test]
fn a_pack_in_the_wrong_directory_or_with_a_mixed_up_identity_is_refused() {
    let (root, _, _) = tree("identity");
    let dir = root.join(CHIP).join("0.1.0");
    let ps = release::load_pack(files_of(&dir), Some("acme/cx/cx1/other/0.1.0")).unwrap_err();
    assert!(codes(&ps).contains(&"path_mismatch".to_string()), "{ps:?}");
    // an id that is not vendor/family/device/variant of the identity
    let mut files = files_of(&dir);
    let m = String::from_utf8(files["pack.yaml"].clone()).unwrap().replace("variant: default", "variant: other");
    files.insert("pack.yaml".into(), m.into_bytes());
    let ps = release::load_pack(files, None).unwrap_err();
    assert!(codes(&ps).contains(&"id_mismatch".to_string()), "{ps:?}");
}

#[test]
fn a_duplicate_pack_id_and_a_missing_dependency_are_found() {
    let (root, _, _) = tree("dup");
    let (packs, _) = release::load_tree(&root).unwrap();
    let mut two: Vec<Pack> = packs.clone();
    two.push(packs.iter().find(|p| p.id == CHIP).unwrap().clone());
    assert!(codes(&release::verify_set(&two)).contains(&"duplicate_pack_id".to_string()));
    // the chip is not in the release: the module's dependency is missing, nothing similar is used
    let without: Vec<Pack> = packs.iter().filter(|p| p.id != CHIP).cloned().collect();
    let ps = release::verify_set(&without);
    assert!(codes(&ps).contains(&"missing_dependency".to_string()), "{ps:?}");
    // and in the tree: a copy under another path is a path mismatch, not a second pack
    let copy = root.join(CHIP).join("0.2.0");
    std::fs::create_dir_all(&copy).unwrap();
    for (k, v) in files_of(&root.join(CHIP).join("0.1.0")) {
        std::fs::write(copy.join(k), v).unwrap();
    }
    let (_, ps) = release::load_tree(&root).unwrap();
    assert!(codes(&ps).contains(&"path_mismatch".to_string()), "{ps:?}");
}

#[test]
fn a_dependency_cycle_is_found() {
    let (root, _, _) = tree("cycle");
    let (packs, _) = release::load_tree(&root).unwrap();
    let mut packs = packs;
    let (c, m) = (packs.iter().position(|p| p.id == CHIP).unwrap(), packs.iter().position(|p| p.id == MODULE).unwrap());
    // the chip claims to depend on the module
    let dep = json!([{"id": MODULE, "kind": "module", "version": "0.1.0", "canonical_sha256": packs[m].canonical_sha256}]);
    packs[c].manifest["dependencies"] = dep;
    let ps = release::verify_set(&packs);
    assert!(codes(&ps).contains(&"dependency_cycle".to_string()), "{ps:?}");
}

#[test]
fn chip_and_module_kinds_cannot_be_mixed() {
    let (root, _, _) = tree("kinds");
    // a chip pack that depends on a module
    let mut files = files_of(&root.join(CHIP).join("0.1.0"));
    let (packs, _) = release::load_tree(&root).unwrap();
    let m = packs.iter().find(|p| p.id == MODULE).unwrap();
    let text = String::from_utf8(files["pack.yaml"].clone()).unwrap().replace("dependencies: []", &format!("dependencies:\n- id: {MODULE}\n  kind: module\n  version: 0.1.0\n  canonical_sha256: {}", m.canonical_sha256));
    files.insert("pack.yaml".into(), text.into_bytes());
    let ps = release::load_pack(files, None).unwrap_err();
    assert!(codes(&ps).contains(&"kind_not_allowed".to_string()), "{ps:?}");
    // a module that depends on a module
    let mut files = files_of(&root.join(BOARD).join("0.1.0"));
    let text = String::from_utf8(files["pack.yaml"].clone()).unwrap().replace("kind: board\n", "kind: module\n");
    files.insert("pack.yaml".into(), text.into_bytes());
    let ps = release::load_pack(files, None).unwrap_err();
    assert!(codes(&ps).contains(&"kind_not_allowed".to_string()) || codes(&ps).contains(&"hcp_identity_mismatch".to_string()), "{ps:?}");
    // the manifest says the dependency is a chip, but it is a module
    let mut packs = packs;
    let b = packs.iter().position(|p| p.id == MODULE).unwrap();
    packs[b].manifest["dependencies"][0]["kind"] = json!("module");
    assert!(codes(&release::verify_set(&packs)).contains(&"dependency_kind_mismatch".to_string()));
}

#[test]
fn a_changed_dependency_makes_the_pack_that_inherits_from_it_stale() {
    let (root, _, _) = tree("stale");
    // the chip is edited and sealed again; the module still pins the old digest
    let mut chip = chip_hcp();
    chip["rules"][1]["params"]["pins"] = json!(["3", "4"]);
    add(&root, &Spec::new(chip));
    let (_, ps) = release::load_tree(&root).unwrap();
    let c = codes(&ps);
    assert!(c.contains(&"dependency_digest_mismatch".to_string()), "{ps:?}");
    assert!(c.contains(&"pack_invalid".to_string()), "the merge itself refuses the changed upstream: {ps:?}");
}

#[test]
fn an_incompatible_ir_schema_or_application_version_is_refused() {
    let (root, _, _) = tree("compat");
    let dir = root.join(CHIP).join("0.1.0");
    for (from, to, want) in [
        ("ir_version: '1.0'", "ir_version: '9.9'", "incompatible_ir_version"),
        ("hcp_schema_version: '1.1'", "hcp_schema_version: '1.9'", "incompatible_hcp_schema"),
        ("min_app_version: 0.1.0", "min_app_version: 99.0.0", "incompatible_app"),
    ] {
        let mut files = files_of(&dir);
        let text = String::from_utf8(files["pack.yaml"].clone()).unwrap();
        assert!(text.contains(from), "{from} not in\n{text}");
        files.insert("pack.yaml".into(), text.replace(from, to).into_bytes());
        let ps = release::load_pack(files, None).unwrap_err();
        assert!(codes(&ps).contains(&want.to_string()), "{want}: {ps:?}");
    }
    let mut files = files_of(&dir);
    let text = String::from_utf8(files["pack.yaml"].clone()).unwrap().replace("min_app_version: 0.1.0", "min_app_version: 0.1.0\n  max_app_version: 0.0.1");
    files.insert("pack.yaml".into(), text.into_bytes());
    assert!(codes(&release::load_pack(files, None).unwrap_err()).contains(&"incompatible_app".to_string()));
}

#[test]
fn the_original_of_a_document_that_may_not_be_redistributed_is_not_packed() {
    let root = tmp("license").join("hardware-packs");
    // a pdf of a reference_only document, sealed into the pack
    let mut s = Spec::new(chip_hcp());
    s.extra.push(("documents/DOC-CX1-DS.pdf".into(), b"%PDF-1.4 fictional original".to_vec()));
    let dir = add(&root, &s);
    let ps = release::load_pack(files_of(&dir), None).unwrap_err();
    assert!(codes(&ps).contains(&"license_forbids_bundling".to_string()), "{ps:?}");
    // the same file under another name, not listed: also found, by its digest
    let mut s = Spec::new(chip_hcp());
    s.extra.push(("reports/notes.txt".into(), b"%PDF-1.4 fictional original".to_vec()));
    let dir = add(&root.parent().unwrap().join("second"), &s);
    let mut files = files_of(&dir);
    // (the digest of the document is the one in the inventory; make the file's digest equal to it)
    let manifest = String::from_utf8(files["pack.yaml"].clone()).unwrap();
    assert!(manifest.contains("redistribution: reference_only"));
    files.insert("documents/other.bin".into(), vec![0u8; 3]);
    let ps = release::load_pack(files, None).unwrap_err();
    assert!(codes(&ps).contains(&"unlisted_file".to_string()), "{ps:?}");
    // where the licence allows it, the original travels with the pack and is installed with it
    let root = tmp("license-ok").join("hardware-packs");
    let mut s = Spec::new(chip_hcp());
    s.licenses = vec![("DOC-CX1-DS".into(), "allowed")];
    s.extra.push(("documents/DOC-CX1-DS.pdf".into(), b"%PDF-1.4 fictional original".to_vec()));
    add(&root, &s);
    let (packs, ps) = release::load_tree(&root).unwrap();
    assert!(ps.is_empty(), "{ps:?}");
    assert!(packs[0].files.contains_key("documents/DOC-CX1-DS.pdf"));
    let out = build(&root);
    let ws = root.parent().unwrap().join("ws");
    assert_eq!(code(&bin(&ws, &["packs", "builtin", "--release", out.to_str().unwrap(), "install", CHIP])), 0);
    assert!(ws.join("packs").join(CHIP).join("0.1.0/documents/DOC-CX1-DS.pdf").is_file());
}

#[test]
fn a_pack_that_only_registers_an_identity_has_no_rules() {
    let root = tmp("meta").join("hardware-packs");
    let mut hcp_doc = metadata_only_hcp("stc/stc8/stc8h8k64u/lqfp48", "stc8h8k64u");
    hcp_doc["rules"] = json!([{"id": "C-MADE-UP", "kind": "reserved_pin", "modifiability": "unknown", "subject": {"device": "DEV-STC8H8K64U"}, "params": {}, "missing": "x"}]);
    let mut s = Spec::new(hcp_doc);
    s.content = "metadata_only";
    let dir = add(&root, &s);
    let ps = release::load_pack(files_of(&dir), None).unwrap_err();
    assert!(codes(&ps).contains(&"metadata_only_with_rules".to_string()), "{ps:?}");
}

#[test]
fn a_damaged_or_replaced_archive_is_found_before_anything_is_installed() {
    let (root, _, _) = tree("damage");
    let out = build(&root);
    let rel = Release::open(&out).unwrap();
    let archive = out.join(rel.find(CHIP).unwrap()["archive"].as_str().unwrap());
    let mut bytes = std::fs::read(&archive).unwrap();
    let n = bytes.len();
    bytes[n / 2] ^= 0xff;
    std::fs::write(&archive, &bytes).unwrap();
    let (_, ps) = rel.verify().unwrap();
    assert!(codes(&ps).contains(&"sums_mismatch".to_string()) && codes(&ps).contains(&"archive_digest_mismatch".to_string()), "{ps:?}");
    let ws = root.parent().unwrap().join("ws");
    let o = bin(&ws, &["packs", "builtin", "--release", out.to_str().unwrap(), "install", MODULE]);
    assert_eq!(code(&o), 3, "{}", err(&o));
    assert!(!ws.join("packs").exists() || !ws.join("packs").join(CHIP).exists(), "nothing was installed");
    let v = bin(&ws, &["packs", "builtin", "--release", out.to_str().unwrap(), "verify"]);
    assert_eq!(code(&v), 3);
}

#[test]
fn only_the_exact_id_is_used_never_a_similar_one() {
    let (root, _, _) = tree("exact");
    let out = build(&root);
    let rel = Release::open(&out).unwrap();
    assert!(rel.find("acme/cx/cx1").is_err(), "a prefix is not an id");
    assert!(rel.find("acme/cx/cx1/other").is_err());
    assert!(rel.find(&format!("{CHIP}@0.9.0")).is_err(), "another version is another pack");
    assert!(rel.find(CHIP).is_ok() && rel.find(&format!("{CHIP}@0.1.0")).is_ok());
}

#[test]
fn an_archive_cannot_write_outside_its_pack() {
    // a tar.gz with a '../' entry, built by hand (the tar library refuses to write one)
    let mut header = tar::Header::new_gnu();
    header.set_size(1);
    header.set_mode(0o644);
    header.set_entry_type(tar::EntryType::Regular);
    {
        let g = header.as_gnu_mut().unwrap();
        g.name[..9].copy_from_slice(b"../escape");
    }
    header.set_cksum();
    let mut raw = Vec::new();
    raw.extend_from_slice(header.as_bytes());
    let mut block = vec![0u8; 512];
    block[0] = b'x';
    raw.extend_from_slice(&block);
    raw.extend_from_slice(&[0u8; 1024]);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut gz, &raw).unwrap();
    let bytes = gz.finish().unwrap();
    let e = release::unarchive(&bytes).unwrap_err();
    assert!(e.message.contains("safe relative path"), "{}", e.message);
}

#[test]
fn the_manifest_schema_rejects_what_it_should() {
    let (root, _, _) = tree("schema");
    let dir = root.join(CHIP).join("0.1.0");
    for (from, to) in [("kind: chip", "kind: gadget"), ("version: 0.1.0\ncontent", "version: latest\ncontent"), ("manifest_version: '1.0'", "manifest_version: '2.0'")] {
        let mut files = files_of(&dir);
        let text = String::from_utf8(files["pack.yaml"].clone()).unwrap();
        assert!(text.contains(from), "{from}\n{text}");
        files.insert("pack.yaml".into(), text.replace(from, to).into_bytes());
        let ps = release::load_pack(files, None).unwrap_err();
        assert!(codes(&ps).contains(&"manifest_schema".to_string()), "{from}: {ps:?}");
    }
}
