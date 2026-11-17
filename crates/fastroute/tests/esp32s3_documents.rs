//! The ESP32-S3-WROOM-1-N8R8 pack against the locked documents: sources -> index -> search -> evidence -> schema -> compile ->
//! validate -> report, with hcp-manager and (through the Skill's script) the whole flow in one run.
//!
//! The documents are not in the repository. The tests use the cache of the pack's workspace when it has one and otherwise download
//! the documents (hcp-manager checks every file against the digest pinned in `workspace/sources.yaml`); with neither a network nor
//! a cache, or without a built `hcp-manager`, they say so and pass. The tests in `esp32s3_pack.rs` need none of this.
#![cfg(unix)]

mod common;
use common::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const DEVICE: &str = "ESP32-S3-WROOM-1-N8R8";

fn pack_dir() -> PathBuf {
    manifest_dir().join("../../hardware-packs/espressif/esp32-s3-wroom-1-n8r8")
}

fn manager_bin() -> Option<PathBuf> {
    let p = manifest_dir().join("../../target/debug/hcp-manager");
    p.is_file().then_some(p)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let t = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), t).unwrap();
        }
    }
}

struct Ws {
    root: PathBuf,
    ws: PathBuf,
    manager: PathBuf,
}

impl Ws {
    /// hcp-manager with the pack's inventory in a fresh workspace; the documents are copied from the pack's own cache or fetched.
    fn new(name: &str) -> Option<Ws> {
        let manager = manager_bin().or_else(|| {
            eprintln!("hcp-manager is not built (`cargo build -p hcp-manager`): skipped");
            None
        })?;
        let root = tmp(name);
        let ws = root.join("workspace");
        std::fs::create_dir_all(root.join("example")).unwrap();
        std::fs::copy(pack_dir().join("example/carrier-notes.md"), root.join("example/carrier-notes.md")).unwrap();
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::copy(pack_dir().join("workspace/sources.yaml"), ws.join("sources.yaml")).unwrap();
        let w = Ws { root, ws, manager };
        let cached = pack_dir().join("workspace/cache");
        if cached.is_dir() {
            copy_dir(&cached, &w.ws.join("cache"));
        }
        let o = w.hcp(&["--json", "sources", "sync"]);
        if code(&o) != 0 {
            eprintln!("the documents are neither cached nor reachable (sync exit {}): skipped\n{}", code(&o), stderr(&o));
            return None;
        }
        let r: Value = serde_json::from_slice(&o.stdout).unwrap();
        for d in r["results"].as_array().unwrap() {
            assert!(matches!(d["status"].as_str(), Some("new" | "unchanged" | "ok")), "{d}");
        }
        Some(w)
    }

    fn hcp(&self, args: &[&str]) -> Output {
        Command::new(&self.manager).arg("--workspace").arg(&self.ws).args(args).output().expect("run hcp-manager")
    }

    fn json(&self, args: &[&str]) -> (i32, Value) {
        let o = self.hcp(args);
        (code(&o), serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{e}: {}{}", stdout(&o), stderr(&o))))
    }

    fn search(&self, query: &str, level: Option<&str>, device: Option<&str>) -> Value {
        self.search_mode(query, level, device, "keyword")
    }

    fn search_mode(&self, query: &str, level: Option<&str>, device: Option<&str>, mode: &str) -> Value {
        let mut a = vec!["--json", "search", "--limit", "10", "--mode", mode];
        if let Some(d) = device {
            a.extend(["--device", d]);
        }
        if let Some(l) = level {
            a.extend(["--source-level", l]);
        }
        a.push(query);
        let (c, v) = self.json(&a);
        assert_eq!(c, 0, "{v}");
        v
    }
}

fn eval_set() -> Value {
    fr_constraint::hcp::parse(&std::fs::read_to_string(pack_dir().join("eval/retrieval-eval.yaml")).unwrap()).unwrap()
}

fn answers(w: &Ws) -> Vec<Value> {
    let e = eval_set();
    let mut out: Vec<Value> = Vec::new();
    for mode in ["keyword", "vector", "hybrid"] {
        out.extend(e["cases"].as_array().unwrap().iter().map(|c| w.search_mode(c["query"].as_str().unwrap(), c["source_level"].as_str(), Some(DEVICE), mode)));
        for i in e["isolation"].as_array().unwrap() {
            out.push(w.search_mode(i["query"].as_str().unwrap(), None, Some(DEVICE), mode));
        }
    }
    out
}

#[test]
fn the_documents_are_pinned_indexed_and_found_in_the_version_that_was_read() {
    let Some(w) = Ws::new("esp-docs") else { return };
    // sources: every document is the file whose digest is pinned
    let (c, r) = w.json(&["sources", "check", "--json"]);
    assert_eq!(c, 0, "{r}");
    let (c, idx) = w.json(&["index", "build", "--json"]);
    assert_eq!(c, 0, "{idx}");
    let (_, st) = w.json(&["index", "status", "--json"]);
    assert_eq!((st["keyword_index"].as_str(), st["vector_index"].as_str()), (Some("ready"), Some("ready")), "{st}");
    // the evaluation set: the right page of the right version, ranked as the set says, by keywords and by keywords with vectors
    let e = eval_set();
    let (mut hits, mut cases) = (0, 0);
    for mode in ["keyword", "hybrid"] {
        for c in e["cases"].as_array().unwrap() {
            cases += 1;
            let r = w.search_mode(c["query"].as_str().unwrap(), c["source_level"].as_str(), Some(DEVICE), mode);
            assert_eq!(r["mode_used"], mode);
            let ex = &c["expect"];
            let k = ex["top_k"].as_u64().unwrap() as usize;
            let hit = r["hits"].as_array().unwrap().iter().take(k).find(|h| h["document_id"] == ex["document_id"] && (ex["page"].is_null() || h["page"] == ex["page"]));
            let id = c["id"].as_str().unwrap();
            let h = hit.unwrap_or_else(|| panic!("{mode} {id}: {} not in the first {k}: {:?}", ex["document_id"], r["hits"].as_array().unwrap().iter().map(|h| (h["document_id"].clone(), h["page"].clone())).collect::<Vec<_>>()));
            if !ex["version"].is_null() {
                assert_eq!(h["document_version"], ex["version"], "{id}: a hit names the version of the document it comes from");
            }
            // a hit goes back to the text: its block id carries the digest of the document it was built from, and the block can be shown
            assert!(h["chunk_id"].as_str().unwrap().contains('@'), "{id}");
            hits += 1;
        }
    }
    assert_eq!(hits, cases);
    // a hit goes back to the page: the block of a hit is the text of that page of the cached document
    let r = w.search_mode("Do not leave the EN pin floating", None, Some(DEVICE), "hybrid");
    let (c, shown) = w.json(&["--json", "chunk", "show", r["hits"][0]["chunk_id"].as_str().unwrap()]);
    assert_eq!(c, 0, "{shown}");
    assert_eq!((shown["chunk"]["document_id"].as_str(), shown["chunk"]["page"].as_u64(), shown["chunk"]["document_version"].as_str()), (Some("DOC-ESP32S3-WROOM1-DS"), Some(11), Some("v1.8")));
    assert!(shown["chunk"]["text"].as_str().unwrap().contains("Do not leave the EN pin floating"));
    assert_eq!(shown["current"], json!(true));
    // paraphrased questions (no wording of the document): a floor that guards against breakage, not a claim about quality; the measured
    // numbers are in acceptance.md
    let (mut kw_found, mut hy_found, mut total) = (0, 0, 0);
    for p in e["paraphrases"].as_array().unwrap() {
        total += 1;
        let find = |mode: &str| {
            let r = w.search_mode(p["query"].as_str().unwrap(), p["source_level"].as_str(), Some(DEVICE), mode);
            r["hits"].as_array().unwrap().iter().take(3).any(|h| h["document_id"] == p["expect"]["document_id"] && (p["expect"]["page"].is_null() || h["page"] == p["expect"]["page"]))
        };
        kw_found += find("keyword") as usize;
        hy_found += find("hybrid") as usize;
    }
    eprintln!("paraphrases found in the first three hits: keyword {kw_found}, hybrid {hy_found} of {total}");
    assert!(kw_found * 3 >= total * 2 && hy_found * 3 >= total * 2, "keyword {kw_found}, hybrid {hy_found} of {total}");
    // a similar module is isolated: never searched for this device in any mode, searched when there is no device
    for i in e["isolation"].as_array().unwrap() {
        let q = i["query"].as_str().unwrap();
        for mode in ["keyword", "vector", "hybrid"] {
            let with = w.search_mode(q, None, Some(DEVICE), mode);
            for never in i["never_return"].as_array().unwrap() {
                assert!(!with["hits"].as_array().unwrap().iter().any(|h| h["document_id"] == *never), "{mode}: {never} was returned");
                let ex = with["documents_excluded"].as_array().unwrap().iter().find(|d| d["document_id"] == *never).unwrap_or_else(|| panic!("{never} is not listed as excluded: {with}"));
                assert!(ex["reason"].as_str().unwrap().contains("not for ESP32-S3-WROOM-1-N8R8"), "{ex}");
                let without = w.search_mode(q, None, None, mode);
                assert!(without["documents_considered"].as_array().unwrap().iter().any(|d| d == never), "without the device filter {never} is searched too: {without}");
            }
        }
    }
    // the vectors are not needed: without them the hybrid mode says it fell back to keywords, and the answers are the keyword answers
    let kw = w.search_mode("Do not leave the EN pin floating", None, Some(DEVICE), "keyword");
    assert_eq!(code(&w.hcp(&["index", "rebuild", "--no-vector"])), 0);
    let h = w.search_mode("Do not leave the EN pin floating", None, Some(DEVICE), "hybrid");
    assert_eq!(h["mode_used"], "keyword");
    assert!(h["degraded"].as_str().unwrap().contains("no vector ranking"), "{h}");
    assert_eq!(h["hits"], kw["hits"], "without vectors a hybrid search is a keyword search");
    assert_eq!(code(&w.hcp(&["index", "build"])), 0, "the vectors are made again, the blocks are kept");
    // every quotation of the pack is on the page it cites, in the version and with the digest of the pack
    let (c, ev) = w.json(&["--json", "evidence", "verify", &s(&pack_dir().join("pack.yaml"))]);
    assert_eq!(c, 0, "{ev}");
    assert_eq!((ev["evidence_checked"].as_u64(), ev["verified"].as_u64()), (Some(84), Some(84)), "{}", ev["problems"]);
    assert!(ev["problems"].as_array().unwrap().is_empty() && ev["unverifiable"].as_array().unwrap().is_empty());
}

#[test]
fn every_quotation_of_the_pack_can_be_found_again_by_a_search() {
    let Some(w) = Ws::new("esp-recall") else { return };
    assert_eq!(code(&w.hcp(&["index", "build"])), 0);
    let pack = fr_constraint::hcp::parse(&std::fs::read_to_string(pack_dir().join("pack.yaml")).unwrap()).unwrap();
    let level = |id: &str| pack["sources"].as_array().unwrap().iter().find(|s| s["source_id"] == id).unwrap().clone();
    fn walk<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
        match v {
            Value::Object(m) => {
                for (k, x) in m {
                    if k == "evidence" {
                        out.extend(x.as_array().unwrap());
                    } else {
                        walk(x, out);
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut ev = Vec::new();
    walk(&pack, &mut ev);
    assert_eq!(ev.len(), 84);
    let ev_again = ev.clone();
    let mut ranks = [0usize; 3]; // found first, found second, not in the first two
    for e in ev {
        let src = level(e["source_id"].as_str().unwrap());
        let lvl = (src["level"] != "official_manual").then(|| src["level"].as_str().unwrap().to_string());
        let r = w.search(e["quote"].as_str().unwrap(), lvl.as_deref(), Some(DEVICE));
        let page = e["locator"]["page"].as_u64();
        let pos = r["hits"].as_array().unwrap().iter().position(|h| h["document_id"] == src["document_id"] && page.is_none_or(|p| h["page"].as_u64() == Some(p)));
        ranks[pos.filter(|p| *p < 2).unwrap_or(2)] += 1;
    }
    // measured on the locked documents (keyword): 67 at the first place, 17 at the second, none missing
    assert_eq!(ranks[2], 0, "quotations that a search does not bring back among the first two hits: {ranks:?}");
    assert!(ranks[0] >= 60, "{ranks:?}");
    // with the vectors added, a quotation is still brought back (measured: 66 first, all 84 among the first three)
    let mut found = 0;
    for e in &ev_again {
        let src = level(e["source_id"].as_str().unwrap());
        let lvl = (src["level"] != "official_manual").then(|| src["level"].as_str().unwrap().to_string());
        let r = w.search_mode(e["quote"].as_str().unwrap(), lvl.as_deref(), Some(DEVICE), "hybrid");
        let page = e["locator"]["page"].as_u64();
        found += r["hits"].as_array().unwrap().iter().take(3).any(|h| h["document_id"] == src["document_id"] && page.is_none_or(|p| h["page"].as_u64() == Some(p))) as usize;
    }
    assert!(found >= 82, "hybrid search brings back {found} of 84 quotations among the first three hits");
}

#[test]
fn deleting_the_index_and_building_it_again_changes_nothing() {
    let Some(w) = Ws::new("esp-rebuild") else { return };
    assert_eq!(code(&w.hcp(&["index", "build"])), 0);
    let verify = || w.json(&["--json", "evidence", "verify", &s(&pack_dir().join("pack.yaml"))]).1.to_string();
    let compile = || {
        let o = run(&["constraints", "compile", &s(&pack_dir().join("pack.yaml")), "-o", "-", "-q"]);
        assert_eq!(code(&o), 0);
        o.stdout
    };
    let validate = || {
        let o = run(&["constraints", "validate", "--ir", &s(&pack_dir().join("ir.json")), "--kicad-pcb", &s(&pack_dir().join("example/carrier.kicad_pcb")), "--json", "-q"]);
        assert_eq!(code(&o), 5);
        o.stdout
    };
    let before = (answers(&w), verify(), compile(), validate());
    // the derived text is gone: a search finds nothing and says why, the pack and its checks do not need it
    std::fs::remove_dir_all(w.ws.join("derived")).unwrap();
    let gone = w.search("Do not leave the EN pin floating", None, Some(DEVICE));
    assert!(gone["hits"].as_array().unwrap().is_empty(), "{gone}");
    assert!(gone["documents_excluded"].to_string().contains("index build"), "{gone}");
    let (c, st) = w.json(&["index", "status", "--json"]);
    assert_eq!((c, st["keyword_index"].as_str()), (0, Some("empty")), "{st}");
    assert_eq!((compile(), validate()), (before.2.clone(), before.3.clone()), "compiling and validating do not use the index");
    // built again from the cache: the same answers, block by block, and the same verdicts
    assert_eq!(code(&w.hcp(&["index", "rebuild"])), 0);
    let after = (answers(&w), verify(), compile(), validate());
    assert_eq!(before.0, after.0, "the search answers are not the same after a rebuild");
    assert_eq!(before.1, after.1, "the evidence check is not the same after a rebuild");
    assert_eq!((before.2, before.3), (after.2, after.3));
}

#[test]
fn the_skill_repeats_the_whole_flow_and_gets_the_same_files() {
    let Some(w) = Ws::new("esp-skill") else { return };
    let python = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).map(|d| d.join("python3")).find(|p| p.is_file());
    let Some(python) = python else {
        eprintln!("python3 not found: skipped");
        return;
    };
    assert_eq!(code(&w.hcp(&["index", "build"])), 0);
    let script = manifest_dir().join("../../skills/hardware-constraint-design/scripts/hcp_cli.py");
    let out = w.root.join("pipeline");
    let o = Command::new(python)
        .arg("-I")
        .arg(&script)
        .args(["--fastroute", env!("CARGO_BIN_EXE_fastroute"), "--manager", &s(&w.manager), "--workspace", &s(&w.ws), "pipeline"])
        .arg(pack_dir().join("pack.yaml"))
        .arg("--out")
        .arg(&out)
        .arg("--kicad-pcb")
        .arg(pack_dir().join("example/carrier.kicad_pcb"))
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert_eq!(code(&o), 5, "{}{}", stdout(&o), stderr(&o));
    let st = json(&out.join("pipeline-status.json"));
    let status = |n: &str| st["steps"].as_array().unwrap().iter().find(|s| s["step"] == n).unwrap()["status"].as_str().unwrap().to_string();
    assert_eq!((status("check").as_str(), status("compile").as_str(), status("report").as_str(), status("evidence").as_str(), status("validate").as_str()), ("ok", "ok", "ok", "ok", "needs_human"), "{st}");
    // what a person must still confirm: the draft pack, the unknown values and the unchecked rules
    let humans = st["needs_human"].to_string();
    for want in ["draft", "C-ESP-EN-RC-VALUES", "C-ESP-USB-SKEW", "not checked or are unknown"] {
        assert!(humans.contains(want), "{want} in {humans}");
    }
    // the files of the flow are the committed ones
    assert_eq!(std::fs::read(out.join("ir.json")).unwrap(), std::fs::read(pack_dir().join("ir.json")).unwrap());
    assert_eq!(std::fs::read(out.join("pack-report.md")).unwrap(), std::fs::read(pack_dir().join("reports/pack-report.zh.md")).unwrap());
    let (a, b) = (json(&out.join("validation.json")), json(&pack_dir().join("reports/validation-legal.json")));
    assert_eq!(a["findings"], b["findings"]);
    assert_eq!(json(&out.join("evidence.json"))["verified"], json!(84));
}

#[test]
fn the_application_manages_the_documents_and_the_pack_without_the_skill() {
    let Some(manager) = manager_bin() else {
        eprintln!("hcp-manager is not built (`cargo build -p hcp-manager`): skipped");
        return;
    };
    let root = tmp("esp-packs");
    let ws = root.join("workspace");
    let hcp = |args: &[&str]| Command::new(&manager).arg("--workspace").arg(&ws).args(args).output().unwrap();
    assert_eq!(code(&hcp(&["init"])), 0);
    std::fs::copy(pack_dir().join("workspace/sources.yaml"), ws.join("sources.yaml")).unwrap();
    // the documents for the device, best source first; the similar module is named as excluded
    let o = hcp(&["--json", "sources", "resolve", "--device", DEVICE]);
    let r: Value = serde_json::from_slice(&o.stdout).unwrap();
    let ids: Vec<&str> = r["documents"].as_array().unwrap().iter().map(|d| d["document_id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), 5, "{r}");
    assert!(!ids.contains(&"DOC-ESP32S3-WROOM2-DS"));
    assert_eq!(r["documents"][0]["level"], "official_manual");
    assert_eq!(r["documents"][4]["level"], "open_board", "the open board comes last");
    assert!(r["similar_models_excluded"].to_string().contains("DOC-ESP32S3-WROOM2-DS"), "{r}");
    assert!(r["missing"].as_array().unwrap().is_empty());
    // the pack: shown, and exported with the inventory entries it cites (and no others)
    let pack = pack_dir().join("pack.yaml");
    let o = hcp(&["--json", "packs", "show", &s(&pack)]);
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!((v["info"]["valid"].as_bool(), v["info"]["review"].as_str(), v["rules_by_modifiability"]["unknown"].as_u64()), (Some(true), Some("draft"), Some(3)), "{v}");
    let dest = root.join("export");
    let o = hcp(&["packs", "export", &s(&pack), "--to", &s(&dest)]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let used = std::fs::read_to_string(dest.join("sources-used.yaml")).unwrap();
    for id in ["DOC-ESP32S3-WROOM1-DS", "DOC-ESP32S3-DS", "DOC-ESP32S3-HDG-SCH", "DOC-ESP32S3-HDG-PCB", "DOC-ESP32S3-CARRIER"] {
        assert!(used.contains(id), "{id}");
    }
    assert!(!used.contains("WROOM2"), "the pack does not cite the similar module");
    assert_eq!(std::fs::read(dest.join("pack.yaml")).unwrap(), std::fs::read(&pack).unwrap());
}
