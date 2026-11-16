//! hcp-manager end to end on fixture documents (text, markdown, HTML): inventory, cache and digests,
//! text blocks with stable ids, keyword search that filters before it ranks, evidence verification,
//! and the guarantee that everything derived can be deleted and rebuilt. No network is used.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_hcp-manager")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hcp-manager-it-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(ws: &Path, args: &[&str]) -> Output {
    Command::new(bin()).arg("--workspace").arg(ws).args(args).env("HCP_TODAY", "2026-11-16").output().unwrap()
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn jsn(o: &Output) -> Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("not JSON ({e}): {}\n{}", out(o), err(o)))
}

const DS_PAGES: [&str; 3] = [
    "Acme FM-100 Datasheet\n\n1 Overview\n\nThe FM-100 is a fictional module for tests.\n",
    "3.3 Strapping pins\n\nIO0 is sampled at reset; the internal pull-up selects normal boot.\nAn external driver on IO0 changes the boot mode.\n",
    "6.2 Antenna keep-out\n\nKeep the area around the antenna free of copper on all layers.\nThe keep-out is 15 mm from the module edge.\n",
];

const DSU_PAGES: [&str; 2] = [
    "Acme FM-100U Datasheet\n\n1 Overview\n\nThe FM-100U has an external antenna connector.\n",
    "6.2 Antenna keep-out\n\nThe FM-100U needs no antenna keep-out: keep the area around the connector free of components.\n",
];

struct Fx {
    ws: PathBuf,
    dir: PathBuf,
}

/// A workspace with five documents added (not yet synced).
fn fixture(name: &str) -> Fx {
    let dir = tmp(name);
    let ws = dir.join("ws");
    let w = |file: &str, text: &str| {
        let p = dir.join(file);
        std::fs::write(&p, text).unwrap();
        p
    };
    let ds = w("fm100-ds.txt", &DS_PAGES.join("\x0c"));
    let dsu = w("fm100u-ds.txt", &DSU_PAGES.join("\x0c"));
    let hdg = w("hdg.html", "<html><head><style>p{}</style></head><body><h2>4.1 Reset</h2><p>EN needs a 10 kOhm pull-up and a 1 nF capacitor to ground.</p><script>var x=1;</script></body></html>");
    let board = w("openboard.md", "# Open board\n\n## Mechanical drawing\n\nMounting hole 1 at 3,3. Prototyping area on the left.\n");
    assert_eq!(code(&run(&ws, &["init"])), 0);
    let add = |id: &str, title: &str, level: &str, version: &str, file: &Path, applies: &str| {
        let o = run(&ws, &["sources", "add", "--id", id, "--title", title, "--level", level, "--version", version, "--file", file.to_str().unwrap(), "--applies-to", applies]);
        assert_eq!(code(&o), 0, "{}", err(&o));
    };
    add("DOC-FM100-DS", "Acme FM-100 datasheet", "official_manual", "v1.0", &ds, "FM-100:A");
    add("DOC-FM100-HDG", "Acme FM-100 hardware design guide", "official_guide", "v1.0", &hdg, "FM-100");
    add("DOC-FM100U-DS", "Acme FM-100U datasheet", "official_manual", "v1.0", &dsu, "FM-100U");
    add("DOC-OPENBOARD", "Open FM-100 carrier board", "open_board", "abc", &board, "FM-100");
    Fx { ws, dir }
}

fn synced(name: &str) -> Fx {
    let f = fixture(name);
    let o = run(&f.ws, &["sources", "sync"]);
    assert_eq!(code(&o), 0, "{}{}", out(&o), err(&o));
    let o = run(&f.ws, &["index", "build"]);
    assert_eq!(code(&o), 0, "{}{}", out(&o), err(&o));
    f
}

#[test]
fn info_states_the_contract_and_what_the_vector_layer_is() {
    let d = tmp("info");
    let v = jsn(&run(&d, &["--json", "info"]));
    assert_eq!((v["interface"].as_str(), v["name"].as_str()), (Some("1"), Some("hcp-manager")));
    assert_eq!(v["capabilities"]["keyword_search"], json!(true));
    assert_eq!(v["capabilities"]["vector_index"], json!(true));
    assert_eq!(v["capabilities"]["default_embedding_provider"], json!("hashing"));
    // honest about what it is: no model is bundled, and nothing is sent anywhere unless that is configured and authorised
    assert_eq!((v["capabilities"]["semantic_model_bundled"].clone(), v["capabilities"]["remote_embedding_by_default"].clone()), (json!(false), json!(false)));
    assert_eq!(v["search_modes"], json!(["keyword", "vector", "hybrid"]));
    assert_eq!(v["exit_codes"]["partial"], json!(5));
}

#[test]
fn sync_pins_digests_and_a_second_sync_changes_nothing() {
    let f = fixture("sync");
    let o = run(&f.ws, &["--json", "sources", "sync"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let r = jsn(&o);
    assert!(r["results"].as_array().unwrap().iter().all(|x| x["status"] == "new" && x["sha256"].as_str().unwrap().len() == 64), "{r}");
    let inv = std::fs::read_to_string(f.ws.join("sources.yaml")).unwrap();
    assert!(inv.contains("sha256:") && inv.contains("retrieved: 2026-11-16") && inv.contains("status: active"), "{inv}");
    // the cached file is named by its digest
    let sha = r["results"].as_array().unwrap().iter().find(|x| x["document_id"] == "DOC-FM100-DS").unwrap()["sha256"].as_str().unwrap().to_string();
    assert!(f.ws.join("cache/DOC-FM100-DS").join(format!("{sha}.txt")).is_file());
    let r2 = jsn(&run(&f.ws, &["--json", "sources", "sync"]));
    assert!(r2["results"].as_array().unwrap().iter().all(|x| x["status"] == "unchanged"), "{r2}");
    assert_eq!(code(&run(&f.ws, &["sources", "check"])), 0);
}

#[test]
fn a_changed_document_is_not_adopted_silently() {
    let f = synced("changed");
    let ds = f.dir.join("fm100-ds.txt");
    let old_inv = std::fs::read_to_string(f.ws.join("sources.yaml")).unwrap();
    std::fs::write(&ds, DS_PAGES.join("\x0c").replace("15 mm", "20 mm")).unwrap();
    let o = run(&f.ws, &["--json", "sources", "sync", "DOC-FM100-DS"]);
    assert_eq!(code(&o), 5, "{}", err(&o));
    let r = jsn(&o);
    assert_eq!(r["results"][0]["status"], "changed");
    assert!(r["results"][0]["message"].as_str().unwrap().contains("--accept"));
    let inv = std::fs::read_to_string(f.ws.join("sources.yaml")).unwrap();
    assert!(inv.contains("status: changed") && inv.contains("digest changed from"), "{inv}");
    // the pinned digest and the cache are what they were, and the changed document is not searched
    assert!(old_inv.contains(&jsn(&run(&f.ws, &["--json", "index", "status"]))["documents"][0]["cached_sha256"].as_str().unwrap().to_string()));
    let s = jsn(&run(&f.ws, &["--json", "search", "keep-out", "--device", "FM-100"]));
    assert!(s["documents_excluded"].as_array().unwrap().iter().any(|e| e["document_id"] == "DOC-FM100-DS" && e["reason"] == "status changed"), "{s}");
    // adopting it is a decision; the index is then stale until it is rebuilt
    let o = run(&f.ws, &["--json", "sources", "sync", "DOC-FM100-DS", "--accept"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert_eq!(jsn(&o)["results"][0]["status"], "accepted");
    let st = jsn(&run(&f.ws, &["--json", "index", "status"]));
    assert_eq!(st["documents"].as_array().unwrap().iter().find(|d| d["document_id"] == "DOC-FM100-DS").unwrap()["state"], "stale");
    assert_eq!(code(&run(&f.ws, &["index", "build"])), 0);
    let st = jsn(&run(&f.ws, &["--json", "index", "status"]));
    assert_eq!(st["keyword_index"], "ready");
}

#[test]
fn an_unreachable_document_keeps_its_cache_and_is_reported() {
    let f = synced("dead");
    std::fs::remove_file(f.dir.join("fm100-ds.txt")).unwrap();
    let o = run(&f.ws, &["--json", "sources", "sync", "DOC-FM100-DS"]);
    assert_eq!(code(&o), 5);
    assert_eq!(jsn(&o)["results"][0]["status"], "unreachable");
    // the cached copy is still there and still searchable
    let s = jsn(&run(&f.ws, &["--json", "search", "antenna", "--device", "FM-100"]));
    assert!(!s["hits"].as_array().unwrap().is_empty());
    // an unknown document id is a usage error
    assert_eq!(code(&run(&f.ws, &["sources", "sync", "DOC-NOPE"])), 2);
    // a URL that is not http(s) is an input error, not a crash
    let o = run(&f.ws, &["sources", "add", "--id", "DOC-BAD", "--title", "t", "--level", "community", "--version", "1", "--url", "ftp://x/y.pdf", "--applies-to", "FM-100"]);
    assert_eq!(code(&o), 0);
    let o = run(&f.ws, &["--json", "sources", "sync", "DOC-BAD"]);
    assert_eq!(jsn(&o)["results"][0]["status"], "unreachable");
}

#[test]
fn search_ranks_official_sources_first_and_returns_what_evidence_needs() {
    let f = synced("search");
    let s = jsn(&run(&f.ws, &["--json", "search", "antenna", "keep-out", "copper", "--device", "FM-100", "--revision", "A"]));
    assert_eq!(s["mode_used"], "keyword");
    let hits = s["hits"].as_array().unwrap();
    assert_eq!(hits[0]["document_id"], "DOC-FM100-DS");
    assert_eq!(hits[0]["page"], 3);
    assert_eq!(hits[0]["section"], "6.2 Antenna keep-out");
    assert_eq!(hits[0]["official"], json!(true));
    assert_eq!(hits[0]["document_version"], "v1.0");
    assert_eq!(hits[0]["locator"], json!({"page": 3, "section": "6.2 Antenna keep-out"}));
    let id = hits[0]["chunk_id"].as_str().unwrap();
    let sha8 = &jsn(&run(&f.ws, &["--json", "index", "status"]))["documents"][0]["cached_sha256"].as_str().unwrap()[..8].to_string();
    assert!(id.starts_with("CHK-FM100-DS@") && id.ends_with(":p3.1"), "{id}");
    assert!(id.contains(&format!("@{sha8}:")), "the id carries the digest of the document it was made from: {id}");
    assert!(hits[0]["snippet"].as_str().unwrap().contains("free of copper"));
    // priority comes before score: a guide's block never outranks the manual's on the same topic
    let s = jsn(&run(&f.ws, &["--json", "search", "pull-up", "--device", "FM-100"]));
    let docs: Vec<&str> = s["hits"].as_array().unwrap().iter().map(|h| h["document_id"].as_str().unwrap()).collect();
    assert_eq!(docs, vec!["DOC-FM100-DS", "DOC-FM100-HDG"], "{s}");
    // HTML was read without its markup, scripts or styles
    let h = jsn(&run(&f.ws, &["--json", "search", "EN pull-up capacitor", "--device", "FM-100", "--source-level", "official_guide"]));
    let text = h["hits"][0]["snippet"].as_str().unwrap();
    assert!(text.contains("10 kOhm") && !text.contains("<p>") && !text.contains("var x"), "{text}");
    assert_eq!(h["hits"][0]["section"], "4.1 Reset");
    // an exact level or `official`
    let o = jsn(&run(&f.ws, &["--json", "search", "mounting hole", "--device", "FM-100", "--source-level", "official"]));
    assert!(o["hits"].as_array().unwrap().is_empty(), "the open board is not official");
    let b = jsn(&run(&f.ws, &["--json", "search", "mounting hole", "--device", "FM-100", "--source-level", "open_board"]));
    assert_eq!(b["hits"][0]["document_id"], "DOC-OPENBOARD");
}

#[test]
fn a_similar_model_is_filtered_out_before_ranking() {
    let f = synced("similar");
    // the FM-100U manual says the opposite about the antenna keep-out; it must not be found for the FM-100
    let s = jsn(&run(&f.ws, &["--json", "search", "antenna keep-out", "--device", "FM-100"]));
    assert!(s["hits"].as_array().unwrap().iter().all(|h| h["document_id"] != "DOC-FM100U-DS"), "{s}");
    let ex = s["documents_excluded"].as_array().unwrap();
    assert!(ex.iter().any(|e| e["document_id"] == "DOC-FM100U-DS" && e["reason"].as_str().unwrap().starts_with("not for FM-100")), "{ex:?}");
    assert!(!s["documents_considered"].as_array().unwrap().iter().any(|d| d == "DOC-FM100U-DS"));
    // and the other way round
    let u = jsn(&run(&f.ws, &["--json", "search", "antenna keep-out", "--device", "FM-100U"]));
    assert!(u["hits"].as_array().unwrap().iter().all(|h| h["document_id"] == "DOC-FM100U-DS"));
    // a revision that the document does not have excludes it
    let r = jsn(&run(&f.ws, &["--json", "search", "antenna", "--device", "FM-100", "--revision", "B"]));
    assert!(r["documents_excluded"].as_array().unwrap().iter().any(|e| e["document_id"] == "DOC-FM100-DS"));
    // a version that is not there finds nothing (it does not fall back to another version)
    let v = jsn(&run(&f.ws, &["--json", "search", "antenna", "--device", "FM-100", "--version", "v9.9"]));
    assert!(v["hits"].as_array().unwrap().is_empty());
    // without a device filter the mix is possible, and said
    let m = jsn(&run(&f.ws, &["--json", "search", "antenna keep-out"]));
    assert!(m["warnings"][0].as_str().unwrap().contains("--device"));
    let docs: std::collections::BTreeSet<&str> = m["hits"].as_array().unwrap().iter().map(|h| h["document_id"].as_str().unwrap()).collect();
    assert!(docs.contains("DOC-FM100U-DS") && docs.contains("DOC-FM100-DS"));
}

#[test]
fn hybrid_mode_uses_the_vectors_and_degrades_to_keyword_without_them() {
    let f = synced("hybrid");
    let s = jsn(&run(&f.ws, &["--json", "search", "antenna", "--device", "FM-100", "--mode", "hybrid"]));
    assert_eq!((s["mode_requested"].as_str(), s["mode_used"].as_str()), (Some("hybrid"), Some("hybrid")));
    assert!(s["degraded"].is_null() && !s["hits"].as_array().unwrap().is_empty());
    // without the blocks' vectors the same request is answered by keywords, and says so
    assert_eq!(code(&run(&f.ws, &["--json", "index", "rebuild", "--no-vector"])), 0);
    let s = jsn(&run(&f.ws, &["--json", "search", "antenna", "--device", "FM-100", "--mode", "hybrid"]));
    assert_eq!((s["mode_requested"].as_str(), s["mode_used"].as_str()), (Some("hybrid"), Some("keyword")));
    assert!(s["degraded"].as_str().unwrap().contains("no vector ranking"), "{s}");
    assert!(!s["hits"].as_array().unwrap().is_empty());
    assert_eq!(code(&run(&f.ws, &["search", "x", "--mode", "magic"])), 2);
}

#[test]
fn resolve_lists_by_priority_and_says_what_is_missing_without_using_a_similar_model() {
    let f = fixture("resolve");
    let o = run(&f.ws, &["--json", "sources", "resolve", "--device", "FM-100", "--revision", "A"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let r = jsn(&o);
    let ids: Vec<&str> = r["documents"].as_array().unwrap().iter().map(|d| d["document_id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["DOC-FM100-DS", "DOC-FM100-HDG", "DOC-OPENBOARD"]);
    assert!(r["documents"][2]["notes"].to_string().contains("never overrides"));
    assert_eq!(r["similar_models_excluded"].as_array().unwrap().len(), 1);
    assert!(r["similar_models_excluded"][0].as_str().unwrap().contains("DOC-FM100U-DS"));
    // the revision B of the datasheet is not for revision B: no official manual, and the similar model is not used
    let o = run(&f.ws, &["--json", "sources", "resolve", "--device", "FM-100", "--revision", "B"]);
    assert_eq!(code(&o), 5);
    let r = jsn(&o);
    assert!(r["missing"].as_array().unwrap().iter().any(|m| m == "official_manual"));
    assert!(r["warnings"][0].as_str().unwrap().contains("do not generate fixed or required rules"));
    // a version mismatch is listed but unusable
    let r = jsn(&run(&f.ws, &["--json", "sources", "resolve", "--device", "FM-100", "--version", "v2.0"]));
    assert!(r["documents"].as_array().unwrap().iter().all(|d| d["usable"] == json!(false)));
    // a model that has no document at all
    let o = run(&f.ws, &["--json", "sources", "resolve", "--device", "FM-200"]);
    assert_eq!(code(&o), 5);
    assert!(jsn(&o)["documents"].as_array().unwrap().is_empty());
}

#[test]
fn the_index_can_be_deleted_and_rebuilt_with_the_same_ids_and_results() {
    let f = synced("rebuild");
    let q = |ws: &Path| jsn(&run(ws, &["--json", "search", "strapping IO0 reset boot", "--device", "FM-100"]));
    let before = q(&f.ws);
    let ids = |v: &Value| v["hits"].as_array().unwrap().iter().map(|h| (h["chunk_id"].clone(), h["score"].clone(), h["page"].clone())).collect::<Vec<_>>();
    // delete everything derived: search has nothing to search, and says why
    std::fs::remove_dir_all(f.ws.join("derived")).unwrap();
    let gone = q(&f.ws);
    assert!(gone["hits"].as_array().unwrap().is_empty());
    assert!(gone["documents_excluded"].to_string().contains("no text blocks"));
    assert_eq!(jsn(&run(&f.ws, &["--json", "index", "status"]))["keyword_index"], "empty");
    // rebuild: the same blocks with the same ids
    assert_eq!(code(&run(&f.ws, &["index", "rebuild"])), 0);
    let after = q(&f.ws);
    assert_eq!(ids(&before), ids(&after));
    assert!(!ids(&before).is_empty());
    // the inventory and the cache were not touched by any of it
    assert_eq!(code(&run(&f.ws, &["sources", "check"])), 0);
    // the cache can be deleted too: the inventory pins the digests, sync fetches the files again
    std::fs::remove_dir_all(f.ws.join("cache")).unwrap();
    std::fs::remove_dir_all(f.ws.join("derived")).unwrap();
    assert_eq!(code(&run(&f.ws, &["sources", "sync"])), 0);
    assert_eq!(code(&run(&f.ws, &["index", "build"])), 0);
    assert_eq!(ids(&before), ids(&q(&f.ws)));
    // the vectors are rebuilt with the blocks
    assert_eq!(jsn(&run(&f.ws, &["--json", "index", "status"]))["vector_index"], "ready");
}

fn pack_for(f: &Fx, quote: &str, page: u64, chunk: Option<&str>) -> PathBuf {
    let inv = std::fs::read_to_string(f.ws.join("sources.yaml")).unwrap();
    let sha = inv.lines().skip_while(|l| !l.contains("DOC-FM100-DS")).find(|l| l.trim_start().starts_with("sha256:") || l.trim_start().starts_with("- sha256:")).map(|l| l.split(':').nth(1).unwrap().trim().trim_matches('"').to_string()).unwrap();
    let mut ev = json!({"source_id": "SRC-DS", "locator": {"page": page, "section": "6.2 Antenna keep-out"}, "quote": quote, "basis": "explicit"});
    if let Some(c) = chunk {
        ev["chunk_id"] = json!(c);
    }
    let pack = json!({
        "hcp_version": "1.0",
        "pack": {"id": "test/fm-100", "title": "t", "version": "0.1", "created": "2026-11-16", "extractor": {"kind": "human"}},
        "sources": [{"source_id": "SRC-DS", "document_id": "DOC-FM100-DS", "title": "Acme FM-100 datasheet", "version": "v1.0", "level": "official_manual", "retrieved": "2026-11-16", "sha256": sha, "applies_to": ["DEV-FM100"]}],
        "devices": [{"id": "DEV-FM100", "vendor": "Acme", "part_number": "FM-100", "package": "module", "pins": [{"number": "1", "name": "GND", "class": "ground"}]}],
        "rules": [{"id": "C-RES", "kind": "reserved_pin", "modifiability": "fixed", "subject": {"device": "DEV-FM100"}, "params": {"pins": ["1"]}, "evidence": [ev]}]
    });
    let p = f.dir.join("pack.json");
    std::fs::write(&p, serde_json::to_string_pretty(&pack).unwrap()).unwrap();
    p
}

#[test]
fn evidence_is_checked_against_the_cited_page_of_the_cached_document() {
    let f = synced("evidence");
    let good = pack_for(&f, "Keep the area around the antenna free of copper on all layers.", 3, None);
    let o = run(&f.ws, &["--json", "evidence", "verify", good.to_str().unwrap()]);
    assert_eq!(code(&o), 0, "{}{}", out(&o), err(&o));
    let r = jsn(&o);
    assert_eq!((r["evidence_checked"].as_u64(), r["verified"].as_u64()), (Some(1), Some(1)));
    // a quotation split over a line break still matches (whitespace is normalised)
    let wrapped = pack_for(&f, "IO0 is sampled at reset;   the internal pull-up\nselects normal boot.", 2, None);
    assert_eq!(code(&run(&f.ws, &["evidence", "verify", wrapped.to_str().unwrap()])), 0);
    // a quotation that is not on the page, or on another page, is refused
    let wrong = pack_for(&f, "Keep the area around the antenna free of copper on all layers.", 2, None);
    let o = run(&f.ws, &["--json", "evidence", "verify", wrong.to_str().unwrap()]);
    assert_eq!(code(&o), 1);
    assert_eq!(jsn(&o)["problems"][0]["code"], "quote_not_found");
    let invented = pack_for(&f, "The antenna keep-out is 3 mm.", 3, None);
    assert_eq!(jsn(&run(&f.ws, &["--json", "evidence", "verify", invented.to_str().unwrap()]))["problems"][0]["code"], "quote_not_found");
    let nopage = pack_for(&f, "Mounting hole 1 at 3,3.", 40, None);
    assert_eq!(jsn(&run(&f.ws, &["--json", "evidence", "verify", nopage.to_str().unwrap()]))["problems"][0]["code"], "page_not_found");
}

#[test]
fn evidence_notices_a_changed_document_a_stale_block_id_and_a_missing_cache() {
    let f = synced("evidence2");
    // a block id from the current build is fine
    let s = jsn(&run(&f.ws, &["--json", "search", "antenna keep-out", "--device", "FM-100"]));
    let cid = s["hits"][0]["chunk_id"].as_str().unwrap().to_string();
    let q = "Keep the area around the antenna free of copper on all layers.";
    let ok = pack_for(&f, q, 3, Some(&cid));
    assert_eq!(code(&run(&f.ws, &["evidence", "verify", ok.to_str().unwrap()])), 0);
    // an id made from other bytes is stale, an id of nothing is unknown
    let stale = pack_for(&f, q, 3, Some("CHK-FM100-DS@deadbeef:p3.1"));
    let r = jsn(&run(&f.ws, &["--json", "evidence", "verify", stale.to_str().unwrap()]));
    assert_eq!(r["problems"][0]["code"], "chunk_stale");
    let unknown = pack_for(&f, q, 3, Some("CHK-OTHER@deadbeef:p3.1"));
    assert_eq!(jsn(&run(&f.ws, &["--json", "evidence", "verify", unknown.to_str().unwrap()]))["problems"][0]["code"], "chunk_unknown");
    // the pack was written about a document with another digest
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&ok).unwrap()).unwrap();
    v["sources"][0]["sha256"] = json!("0".repeat(64));
    std::fs::write(&ok, serde_json::to_string(&v).unwrap()).unwrap();
    let r = jsn(&run(&f.ws, &["--json", "evidence", "verify", ok.to_str().unwrap()]));
    assert!(r["problems"].as_array().unwrap().iter().any(|p| p["code"] == "digest_mismatch"), "{r}");
    // a version or level that differs from the inventory
    v["sources"][0]["version"] = json!("v2.0");
    v["sources"][0]["level"] = json!("community");
    std::fs::write(&ok, serde_json::to_string(&v).unwrap()).unwrap();
    let codes: Vec<String> = jsn(&run(&f.ws, &["--json", "evidence", "verify", ok.to_str().unwrap()]))["problems"].as_array().unwrap().iter().map(|p| p["code"].as_str().unwrap().to_string()).collect();
    assert!(codes.contains(&"version_mismatch".to_string()) && codes.contains(&"level_mismatch".to_string()), "{codes:?}");
    // a document that is not in the workspace
    v["sources"][0]["document_id"] = json!("DOC-NOT-HERE");
    std::fs::write(&ok, serde_json::to_string(&v).unwrap()).unwrap();
    assert_eq!(jsn(&run(&f.ws, &["--json", "evidence", "verify", ok.to_str().unwrap()]))["problems"][0]["code"], "unknown_document");
    // nothing cached: the evidence cannot be checked, which is not the same as being right (exit 5)
    let g = pack_for(&f, q, 3, None);
    std::fs::remove_dir_all(f.ws.join("cache")).unwrap();
    std::fs::remove_dir_all(f.ws.join("derived")).unwrap();
    let o = run(&f.ws, &["--json", "evidence", "verify", g.to_str().unwrap()]);
    assert_eq!(code(&o), 5, "{}", out(&o));
    assert!(!jsn(&o)["unverifiable"].as_array().unwrap().is_empty());
}

#[test]
fn packs_are_listed_shown_and_exported_with_the_sources_they_cite() {
    let f = synced("packs");
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/hardware-constraint-design/assets/example-device.yaml");
    let dest = f.ws.join("packs/acme/fm-100");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::copy(&example, dest.join("pack.yaml")).unwrap();
    let l = jsn(&run(&f.ws, &["--json", "packs", "list"]));
    assert_eq!(l["packs"][0]["id"], "example/acme-fm-100");
    assert_eq!(l["packs"][0]["valid"], json!(true));
    let s = jsn(&run(&f.ws, &["packs", "show", "example/acme-fm-100"]));
    assert_eq!(s["counts"]["rules"], json!(12));
    assert_eq!(s["rules_by_modifiability"]["unknown"], json!(1));
    // export: the pack, the inventory entries it cites (the example cites DOC-FM100-DS, -HDG and -OPENBOARD) and digests
    let out_dir = f.dir.join("exported");
    let o = run(&f.ws, &["--json", "packs", "export", "example/acme-fm-100", "--to", out_dir.to_str().unwrap()]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(out_dir.join("export-manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["files"]["pack.yaml"].as_str().unwrap().len(), 64);
    assert_eq!(manifest["missing_from_inventory"], json!([]));
    assert!(std::fs::read_to_string(out_dir.join("sources-used.yaml")).unwrap().contains("DOC-FM100-DS"));
    // it does not overwrite a non-empty directory without --force
    assert_eq!(code(&run(&f.ws, &["packs", "export", "example/acme-fm-100", "--to", out_dir.to_str().unwrap()])), 4);
    assert_eq!(code(&run(&f.ws, &["packs", "export", "example/acme-fm-100", "--to", out_dir.to_str().unwrap(), "--force"])), 0);
    assert_eq!(code(&run(&f.ws, &["packs", "show", "no/such-pack"])), 3);
}

#[test]
fn import_adds_an_inventory_file_and_bad_input_is_refused() {
    let dir = tmp("import");
    let ws = dir.join("ws");
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/hardware-constraint-design/assets/example-sources.yaml");
    let o = run(&ws, &["sources", "import", example.to_str().unwrap()]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    assert!(out(&o).contains("imported 5"), "{}", out(&o));
    assert_eq!(code(&run(&ws, &["sources", "import", example.to_str().unwrap()])), 0, "importing twice adds nothing");
    let l = jsn(&run(&ws, &["--json", "sources", "list"]));
    assert_eq!(l["documents"].as_array().unwrap().len(), 5);
    // the inventory file is checked against its schema
    let bad = dir.join("bad.yaml");
    std::fs::write(&bad, "sources_version: \"1.0\"\ndocuments:\n  - {document_id: DOC-X, title: t, level: wiki, version: \"1\", applies_to: [{part_number: A}], url: http://x}\n").unwrap();
    let o = run(&ws, &["sources", "import", bad.to_str().unwrap()]);
    assert_eq!(code(&o), 3);
    assert!(err(&o).contains("level"), "{}", err(&o));
    // usage: a document id that is not one, a level that is not one, no source, a duplicate
    let add = |args: &[&str]| {
        let mut a = vec!["sources", "add", "--title", "t", "--version", "1", "--applies-to", "P"];
        a.extend_from_slice(args);
        code(&run(&ws, &a))
    };
    assert_eq!(add(&["--id", "doc-x", "--level", "community", "--url", "http://x"]), 2);
    assert_eq!(add(&["--id", "DOC-NEW", "--level", "wiki", "--url", "http://x"]), 2);
    assert_eq!(add(&["--id", "DOC-NEW", "--level", "community"]), 2);
    assert_eq!(add(&["--id", "DOC-FM100-DS", "--level", "community", "--url", "http://x"]), 2);
    assert_eq!(code(&run(&ws, &["frobnicate"])), 2);
    // no inventory in this workspace: an input problem with the way out
    let o = run(&dir.join("empty"), &["sources", "list"]);
    assert_eq!(code(&o), 3);
    assert!(err(&o).contains("hcp-manager init"), "{}", err(&o));
}
