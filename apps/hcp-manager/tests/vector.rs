//! The embedded index and the vector layer of hcp-manager, on fixture documents: what a block keeps, filters before ranking, hybrid
//! search and its labels, stale blocks and vectors and the selective rebuild, deleting and damaging the store, the embedding
//! providers (a local command, a service over http, the refusals for a remote one), the default that makes no network call, and
//! packs created, exported and imported. No network is used (the one service is a mock on this machine).

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_hcp-manager")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hcp-manager-vec-{}-{name}", std::process::id()));
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

const DS: [&str; 4] = [
    "Acme AC-100 Datasheet\n\n1 Overview\n\nThe AC-100 is a fictional wireless module for tests.\n",
    "3.3 Strapping pins\n\nIO0 is sampled at reset; the internal pull-up selects normal boot.\nAn external driver on IO0 changes the boot mode.\n",
    "5.1 Power supply\n\nAdd a bulk capacitor of at least 10 uF at the supply input. Decoupling capacitors of 100 nF belong next to each supply pin.\n",
    "6.2 Antenna keep-out\n\nKeep the area around the antenna free of copper on all layers.\n",
];
const DSU: [&str; 4] = [
    "Acme AC-100U Datasheet\n\n1 Overview\n\nThe AC-100U is a fictional wireless module with an external antenna connector.\n",
    "3.3 Strapping pins\n\nIO0 is sampled at reset; the internal pull-up selects normal boot.\nAn external driver on IO0 changes the boot mode.\n",
    "5.1 Power supply\n\nAdd a bulk capacitor of at least 10 uF at the supply input. Decoupling capacitors of 100 nF belong next to each supply pin.\n",
    "6.2 Antenna keep-out\n\nThe AC-100U needs no antenna keep-out: keep the area around the connector free of components.\n",
];
const DS2: [&str; 1] = ["6.2 Antenna keep-out\n\nThe keep-out is 20 mm from the module edge. Keep the area free of copper.\n"];
const HDG: &str = "<html><body><h2>4.1 Reset</h2><p>The EN pin needs a pull-up resistor and a capacitor to ground for a clean reset.</p><h2>4.2 Supply</h2><p>Place decoupled capacitors close to the module and keep the supply traces short.</p></body></html>";
const BOARD: &str = "# Open board\n\n## Mechanical drawing\n\nMounting hole 1 at 3,3. Antenna keep-out is drawn on the top layer.\n";

struct Fx {
    ws: PathBuf,
    dir: PathBuf,
}

fn fixture(name: &str) -> Fx {
    let dir = tmp(name);
    let ws = dir.join("ws");
    let w = |file: &str, text: &str| {
        let p = dir.join(file);
        std::fs::write(&p, text).unwrap();
        p
    };
    let files = [
        ("ac100-ds.txt", DS.join("\x0c")),
        ("ac100u-ds.txt", DSU.join("\x0c")),
        ("ac100-ds2.txt", DS2.join("\x0c")),
        ("hdg.html", HDG.to_string()),
        ("board.md", BOARD.to_string()),
    ];
    for (f, t) in &files {
        w(f, t);
    }
    assert_eq!(code(&run(&ws, &["init"])), 0);
    let add = |id: &str, level: &str, version: &str, file: &str, applies: &str| {
        let o = run(&ws, &["sources", "add", "--id", id, "--title", id, "--level", level, "--version", version, "--file", dir.join(file).to_str().unwrap(), "--applies-to", applies]);
        assert_eq!(code(&o), 0, "{}", err(&o));
    };
    add("DOC-AC-100-DS", "official_manual", "v1.0", "ac100-ds.txt", "AC-100");
    add("DOC-AC-100U-DS", "official_manual", "v1.0", "ac100u-ds.txt", "AC-100U");
    add("DOC-AC-100-DS2", "official_manual", "v2.0", "ac100-ds2.txt", "AC-100");
    add("DOC-AC-100-HDG", "official_guide", "v1.0", "hdg.html", "AC-100");
    add("DOC-AC-100-BOARD", "open_board", "abc", "board.md", "AC-100");
    Fx { ws, dir }
}

fn built(name: &str) -> Fx {
    let f = fixture(name);
    let o = run(&f.ws, &["sources", "sync"]);
    assert_eq!(code(&o), 0, "{}{}", out(&o), err(&o));
    let o = run(&f.ws, &["--json", "index", "build"]);
    assert_eq!(code(&o), 0, "{}{}", out(&o), err(&o));
    f
}

fn search(ws: &Path, q: &str, extra: &[&str]) -> Value {
    let mut a = vec!["--json", "search", q];
    a.extend_from_slice(extra);
    let o = run(ws, &a);
    assert_eq!(code(&o), 0, "{}{}", out(&o), err(&o));
    jsn(&o)
}

fn docs_of(v: &Value) -> Vec<String> {
    v["hits"].as_array().unwrap().iter().map(|h| h["document_id"].as_str().unwrap().to_string()).collect()
}

fn ids_of(v: &Value) -> Vec<String> {
    v["hits"].as_array().unwrap().iter().map(|h| h["chunk_id"].as_str().unwrap().to_string()).collect()
}

fn sha(bytes: &[u8]) -> String {
    fr_constraint::hcp::sha256_hex(bytes)
}

// ---- what the index keeps ---------------------------------------------------------------------------------------------------

#[test]
fn every_block_keeps_where_it_came_from_and_a_hit_goes_back_to_the_page() {
    let f = built("keeps");
    assert!(f.ws.join("derived/index.db").is_file(), "the embedded store is one file");
    let st = jsn(&run(&f.ws, &["--json", "index", "status"]));
    assert_eq!((st["keyword_index"].as_str(), st["vector_index"].as_str(), st["store"]["state"].as_str()), (Some("ready"), Some("ready"), Some("ready")));
    assert_eq!(st["chunker"], "paragraph-400-1500-v1");
    let key = st["embedder"]["configured"]["key"].as_str().unwrap();
    assert!(key.starts_with("hashing:hash-ngram:1:384:"), "{key}");
    assert!(st["documents"].as_array().unwrap().iter().all(|d| d["state"] == "indexed" && d["vector"]["state"] == "current"), "{st}");

    let r = search(&f.ws, "antenna keep-out copper", &["--device", "AC-100", "--mode", "hybrid"]);
    assert!(!r["hits"].as_array().unwrap().is_empty());
    let chunk_schema: Value = serde_json::from_str(&std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/hardware-constraint-design/schemas/chunk-v1.schema.json")).unwrap()).unwrap();
    for h in r["hits"].as_array().unwrap() {
        // the hit says what a hit says, and where it is from
        for k in ["chunk_id", "document_id", "document_version", "level", "applies_to", "page", "content_sha256", "matched_by", "locator", "snippet"] {
            assert!(!h[k].is_null(), "{k} in {h}");
        }
        // and the block it points to is the original text of that page of the cached file
        let o = run(&f.ws, &["--json", "chunk", "show", h["chunk_id"].as_str().unwrap()]);
        assert_eq!(code(&o), 0, "{}", err(&o));
        let c = jsn(&o);
        assert!(fr_constraint::jsonschema::validate(&chunk_schema, &c["chunk"]).is_empty(), "{:#?}", fr_constraint::jsonschema::validate(&chunk_schema, &c["chunk"]));
        assert_eq!(c["chunk"]["text_sha256"], h["content_sha256"], "the digest of the block is the digest of its text");
        assert_eq!(sha(c["chunk"]["text"].as_str().unwrap().as_bytes()), c["chunk"]["text_sha256"].as_str().unwrap());
        assert_eq!(c["current"], json!(true));
        let cached = std::fs::read(c["source"]["cached_file"].as_str().unwrap()).unwrap();
        assert_eq!(sha(&cached), c["source"]["sha256"].as_str().unwrap(), "the digest of the document the block was cut from");
        assert!(c["chunk"]["chunk_id"].as_str().unwrap().contains(&format!("@{}", &c["source"]["sha256"].as_str().unwrap()[..8])));
        // the words of the block are on the cited page of the cached text
        let page = h["page"].as_u64().unwrap() as usize;
        let text = String::from_utf8(cached).unwrap();
        let on_page = text.split('\x0c').nth(page - 1).unwrap_or(&text);
        let first_line = c["chunk"]["text"].as_str().unwrap().lines().next().unwrap().trim();
        assert!(on_page.contains(first_line) || text.contains(first_line), "{first_line} is not in the document");
    }
    // an unknown or stale block id is refused, not guessed
    let o = run(&f.ws, &["chunk", "show", "CHK-AC-100-DS@00000000:p1.1"]);
    assert_eq!(code(&o), 3);
    assert!(err(&o).contains("digest of the document"), "{}", err(&o));
}

// ---- filters come first ---------------------------------------------------------------------------------------------------

#[test]
fn vector_and_hybrid_search_filter_by_device_version_and_level_before_they_rank() {
    let f = built("filters");
    // the text of the AC-100U is the text of the AC-100 (a similar model): without a device both are searched
    for mode in ["keyword", "vector", "hybrid"] {
        let all = search(&f.ws, "IO0 is sampled at reset the internal pull-up selects normal boot", &["--mode", mode]);
        assert!(docs_of(&all).contains(&"DOC-AC-100U-DS".to_string()), "{mode}: the similar model is there to be found when nothing says which device: {:?}", docs_of(&all));
        // with the device it is never returned, however alike the words are
        let one = search(&f.ws, "IO0 is sampled at reset the internal pull-up selects normal boot", &["--mode", mode, "--device", "AC-100"]);
        assert!(!docs_of(&one).contains(&"DOC-AC-100U-DS".to_string()), "{mode}: {:?}", docs_of(&one));
        assert!(!one["hits"].as_array().unwrap().is_empty(), "{mode}");
        assert!(one["documents_excluded"].to_string().contains("not for AC-100"), "{mode}");
        // a version, and a level
        let v2 = search(&f.ws, "antenna keep-out", &["--mode", mode, "--device", "AC-100", "--version", "v2.0"]);
        assert!(!v2["hits"].as_array().unwrap().is_empty() && docs_of(&v2).iter().all(|d| d == "DOC-AC-100-DS2"), "{mode}: {:?}", docs_of(&v2));
        let guide = search(&f.ws, "capacitor supply", &["--mode", mode, "--device", "AC-100", "--source-level", "official_guide"]);
        assert!(!guide["hits"].as_array().unwrap().is_empty() && docs_of(&guide).iter().all(|d| d == "DOC-AC-100-HDG"), "{mode}: {:?}", docs_of(&guide));
    }
    // the filters are applied before the vectors are looked at: a vector search over the excluded documents is not even asked
    let r = search(&f.ws, "antenna", &["--mode", "vector", "--device", "AC-100"]);
    assert!(!r["vector"]["documents_with_vectors"].to_string().contains("AC-100U"), "{}", r["vector"]);
}

#[test]
fn hybrid_orders_by_source_priority_then_score_and_labels_what_matched() {
    let f = built("hybrid");
    let r = search(&f.ws, "antenna keep-out copper", &["--device", "AC-100", "--mode", "hybrid", "--limit", "10"]);
    assert_eq!(r["mode_used"], "hybrid");
    let levels: Vec<u64> = r["hits"].as_array().unwrap().iter().map(|h| match h["level"].as_str().unwrap() { "official_manual" => 1, "official_guide" => 2, _ => 4 }).collect();
    assert!(levels.windows(2).all(|w| w[0] <= w[1]), "official manuals first, the open board last: {levels:?}");
    assert!(r["hit_semantics"].as_str().unwrap().contains("not the strength of any evidence"), "{}", r["hit_semantics"]);
    for h in r["hits"].as_array().unwrap() {
        assert!(h["matched_by"].as_array().unwrap().iter().all(|m| m == "keyword" || m == "vector"));
    }
    // a spelling variant: no word of the query is a word of the text ("decouple capacitance" against "Decoupling capacitors"), so
    // keywords find nothing; the character parts do
    let kw = search(&f.ws, "decouple capacitance", &["--device", "AC-100", "--source-level", "official_manual", "--mode", "keyword"]);
    let hy = search(&f.ws, "decouple capacitance", &["--device", "AC-100", "--source-level", "official_manual", "--mode", "hybrid"]);
    assert!(kw["hits"].as_array().unwrap().is_empty(), "{kw}");
    let top = &hy["hits"][0];
    assert_eq!((top["document_id"].as_str(), top["matched_by"].clone()), (Some("DOC-AC-100-DS"), json!(["vector"])), "{hy}");
    assert!(top["snippet"].as_str().unwrap().contains("Decoupling"), "{top}");
    assert!(top["vector_similarity"].as_f64().unwrap() >= 0.15 && top["keyword_score"].is_null());
    // keyword mode is not changed by the vector layer: the same answer with the vectors there and gone
    let before = search(&f.ws, "antenna keep-out copper", &["--device", "AC-100"]);
    assert_eq!(code(&run(&f.ws, &["index", "rebuild", "--no-vector"])), 0);
    let after = search(&f.ws, "antenna keep-out copper", &["--device", "AC-100"]);
    assert_eq!(before["hits"], after["hits"], "keyword hits do not depend on the vectors");
}

// ---- stale blocks and stale vectors ---------------------------------------------------------------------------------------

fn built_keys(v: &Value) -> Vec<(String, String, String)> {
    v["documents"].as_array().unwrap().iter().map(|d| (d["document_id"].as_str().unwrap().to_string(), d["keyword"].as_str().unwrap().to_string(), d["vectors"]["state"].as_str().unwrap().to_string())).collect()
}

#[test]
fn a_changed_document_makes_its_old_index_stale_and_only_that_document_is_rebuilt() {
    let f = built("stale-doc");
    let ids = |ws: &Path| ids_of(&search(ws, "bulk capacitor supply input", &["--device", "AC-100", "--source-level", "official_manual"]));
    let old_ids = ids(&f.ws);
    assert!(!old_ids.is_empty());
    std::fs::write(f.dir.join("ac100-ds.txt"), DS.join("\x0c").replace("at least 10 uF", "at least 22 uF")).unwrap();
    assert_eq!(code(&run(&f.ws, &["sources", "sync", "--accept", "DOC-AC-100-DS"])), 0);
    // recognised: the status says so and why, search and the evidence check do not use the old text
    let st = jsn(&run(&f.ws, &["--json", "index", "status"]));
    let d = st["documents"].as_array().unwrap().iter().find(|d| d["document_id"] == "DOC-AC-100-DS").unwrap();
    assert_eq!(d["state"], "stale");
    assert!(d["stale_reason"].as_str().unwrap().contains("cached document changed"), "{d}");
    assert_eq!(d["vector"]["state"], "stale");
    assert_eq!(st["keyword_index"], "partial");
    let r = search(&f.ws, "bulk capacitor supply input", &["--device", "AC-100", "--source-level", "official_manual", "--mode", "hybrid"]);
    assert!(r["documents_excluded"].to_string().contains("stale index"), "{r}");
    assert!(!docs_of(&r).contains(&"DOC-AC-100-DS".to_string()));
    // rebuilt: only the document that changed, the others are kept
    let b = jsn(&run(&f.ws, &["--json", "index", "build"]));
    let keys = built_keys(&b);
    assert!(keys.contains(&("DOC-AC-100-DS".into(), "built".into(), "built".into())), "{keys:?}");
    assert!(keys.iter().filter(|k| k.0 != "DOC-AC-100-DS").all(|k| k.1 == "kept" && k.2 == "kept"), "{keys:?}");
    let new_ids = ids(&f.ws);
    assert!(!new_ids.is_empty() && new_ids.iter().all(|n| !old_ids.contains(n)), "a changed document has new block ids: {old_ids:?} -> {new_ids:?}");
    // the old block id is gone for good
    let o = run(&f.ws, &["chunk", "show", &old_ids[0]]);
    assert_eq!(code(&o), 3);
    // a second build changes nothing
    let again = built_keys(&jsn(&run(&f.ws, &["--json", "index", "build"])));
    assert!(again.iter().all(|k| k.1 == "kept" && k.2 == "kept"), "{again:?}");
}

fn set_inventory_version(ws: &Path, doc: &str, version: &str) {
    let p = ws.join("sources.yaml");
    let mut y: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    for d in y["documents"].as_sequence_mut().unwrap() {
        if d["document_id"].as_str() == Some(doc) {
            d["version"] = serde_yaml::Value::String(version.into());
        }
    }
    std::fs::write(&p, serde_yaml::to_string(&y).unwrap()).unwrap();
}

#[test]
fn a_changed_inventory_entry_makes_the_blocks_stale_because_they_carry_its_version() {
    let f = built("stale-inv");
    set_inventory_version(&f.ws, "DOC-AC-100-HDG", "v1.1");
    let st = jsn(&run(&f.ws, &["--json", "index", "status"]));
    let d = st["documents"].as_array().unwrap().iter().find(|d| d["document_id"] == "DOC-AC-100-HDG").unwrap();
    assert_eq!(d["state"], "stale");
    assert!(d["stale_reason"].as_str().unwrap().contains("inventory entry"), "{d}");
    let b = built_keys(&jsn(&run(&f.ws, &["--json", "index", "build"])));
    assert!(b.contains(&("DOC-AC-100-HDG".into(), "built".into(), "built".into())), "{b:?}");
    assert_eq!(b.iter().filter(|k| k.1 == "built").count(), 1, "{b:?}");
    // and the hits now say the new version
    let r = search(&f.ws, "capacitor supply", &["--device", "AC-100", "--source-level", "official_guide"]);
    assert_eq!(r["hits"][0]["document_version"], "v1.1");
}

fn write_embedding(ws: &Path, yaml: &str) {
    std::fs::write(ws.join("embedding.yaml"), yaml).unwrap();
}

#[test]
fn another_embedding_configuration_makes_the_vectors_stale_and_keywords_carry_on() {
    let f = built("stale-embed");
    let r0 = search(&f.ws, "antenna keep-out copper", &["--device", "AC-100", "--mode", "hybrid"]);
    assert_eq!(r0["mode_used"], "hybrid");
    write_embedding(&f.ws, "provider: hashing\ndim: 128\n");
    let st = jsn(&run(&f.ws, &["--json", "index", "status"]));
    assert_eq!((st["keyword_index"].as_str(), st["vector_index"].as_str()), (Some("ready"), Some("stale")), "{st}");
    let d = &st["documents"][0];
    assert!(d["vector"]["reason"].as_str().unwrap().contains("another embedding configuration"), "{d}");
    // the same request is answered by keywords and says why; vectors of the old model are not mixed in
    let r1 = search(&f.ws, "antenna keep-out copper", &["--device", "AC-100", "--mode", "hybrid"]);
    assert_eq!(r1["mode_used"], "keyword");
    assert!(r1["degraded"].as_str().unwrap().contains("no document has current vectors"), "{r1}");
    assert_eq!(code(&run(&f.ws, &["search", "antenna", "--device", "AC-100", "--mode", "vector"])), 3);
    // rebuilt: the blocks are kept, the vectors are made again
    let keys = built_keys(&jsn(&run(&f.ws, &["--json", "index", "build"])));
    assert!(keys.iter().all(|k| k.1 == "kept" && k.2 == "built"), "{keys:?}");
    let st = jsn(&run(&f.ws, &["--json", "index", "status"]));
    assert_eq!(st["vector_index"], "ready");
    // the configuration in force has its own generation of vectors; the old one is kept (a build never deletes it), until `index prune`
    let gens = st["embedder"]["generations"].as_array().unwrap();
    assert_eq!(gens.len(), 2, "{st}");
    let now = gens.iter().find(|g| g["in_force"] == true).unwrap();
    assert!(now["embedder_key"].as_str().unwrap().starts_with("hashing:hash-ngram:1:128:"), "{st}");
    assert!(st["embedder"]["stored"].as_str().unwrap().starts_with("hashing:hash-ngram:1:384:"), "the older generation is the one that is reported as another: {st}");
    assert_eq!(search(&f.ws, "antenna keep-out copper", &["--device", "AC-100", "--mode", "hybrid"])["vector"]["embedder"], now["embedder_key"]);
    let pruned = jsn(&run(&f.ws, &["--json", "index", "prune"]));
    assert_eq!(pruned["removed_generations"], 1, "{pruned}");
    let st = jsn(&run(&f.ws, &["--json", "index", "status"]));
    assert_eq!((st["embedder"]["generations"].as_array().unwrap().len(), st["vector_index"].as_str()), (1, Some("ready")));
    // a broken configuration is said, and nothing but the vectors is lost
    write_embedding(&f.ws, "provider: carrier-pigeon\n");
    let st = jsn(&run(&f.ws, &["--json", "index", "status"]));
    assert_eq!(st["vector_index"], "unavailable");
    assert!(st["embedder"]["problem"].as_str().unwrap().contains("carrier-pigeon") || st["embedder"]["problem"].as_str().unwrap().contains("not a valid"), "{st}");
    assert_eq!(search(&f.ws, "antenna", &["--device", "AC-100", "--mode", "hybrid"])["mode_used"], "keyword");
}

// ---- the store can go ---------------------------------------------------------------------------------------------------------

#[test]
fn deleting_or_damaging_the_store_loses_no_fact_and_a_rebuild_gives_the_same_blocks_and_answers() {
    let f = built("store");
    let q = |ws: &Path| search(ws, "antenna keep-out copper", &["--device", "AC-100", "--mode", "hybrid", "--limit", "10"]);
    let before = q(&f.ws);
    let chunk_texts = |ws: &Path, r: &Value| -> Vec<Value> {
        ids_of(r).iter().map(|id| jsn(&run(ws, &["--json", "chunk", "show", id]))["chunk"].clone()).collect()
    };
    let blocks_before = chunk_texts(&f.ws, &before);
    // the facts are the inventory, the packs and the cache: none of them is in the store
    let inv_before = std::fs::read(f.ws.join("sources.yaml")).unwrap();
    std::fs::remove_dir_all(f.ws.join("derived")).unwrap();
    assert_eq!(jsn(&run(&f.ws, &["--json", "index", "status"]))["store"]["state"], "missing");
    let gone = q(&f.ws);
    assert!(gone["hits"].as_array().unwrap().is_empty() && gone["documents_excluded"].to_string().contains("index build"));
    assert_eq!(std::fs::read(f.ws.join("sources.yaml")).unwrap(), inv_before);
    assert_eq!(code(&run(&f.ws, &["sources", "check"])), 0);
    assert_eq!(code(&run(&f.ws, &["index", "rebuild"])), 0);
    let after = q(&f.ws);
    assert_eq!(before["hits"], after["hits"], "the same hits, scores and all");
    assert_eq!(blocks_before, chunk_texts(&f.ws, &after), "the same blocks");
    // a damaged file is not an error that stops anything: it is said, and replaced by a build
    std::fs::write(f.ws.join("derived/index.db"), b"this is not a database").unwrap();
    assert_eq!(jsn(&run(&f.ws, &["--json", "index", "status"]))["store"]["state"], "unreadable");
    assert!(q(&f.ws)["hits"].as_array().unwrap().is_empty());
    let b = jsn(&run(&f.ws, &["--json", "index", "build"]));
    assert!(b["store_note"].as_str().unwrap().contains("could not be read"), "{b}");
    assert_eq!(before["hits"], q(&f.ws)["hits"]);
    // a store of another layout is replaced the same way
    {
        let c = rusqlite::Connection::open(f.ws.join("derived/index.db")).unwrap();
        c.pragma_update(None, "user_version", 99).unwrap();
    }
    assert_eq!(jsn(&run(&f.ws, &["--json", "index", "status"]))["store"]["state"], "other_version");
    let b = jsn(&run(&f.ws, &["--json", "index", "build"]));
    assert!(b["store_note"].as_str().unwrap().contains("layout version 99"), "{b}");
    assert_eq!(before["hits"], q(&f.ws)["hits"]);
}

// ---- nothing leaves the machine by default ---------------------------------------------------------------------------------

#[test]
fn the_default_needs_no_network_no_tool_and_no_model() {
    let f = fixture("offline");
    // an empty PATH (no curl, no pdftotext) and a proxy that nothing listens on: every step still works on local documents
    let go = |args: &[&str]| {
        Command::new(bin())
            .arg("--workspace")
            .arg(&f.ws)
            .args(args)
            .env_clear()
            .env("PATH", "")
            .env("HTTPS_PROXY", "http://127.0.0.1:9")
            .env("HTTP_PROXY", "http://127.0.0.1:9")
            .env("ALL_PROXY", "http://127.0.0.1:9")
            .output()
            .unwrap()
    };
    assert_eq!(code(&go(&["sources", "sync"])), 0);
    let b = go(&["--json", "index", "build"]);
    assert_eq!(code(&b), 0, "{}{}", out(&b), err(&b));
    let emb = jsn(&go(&["--json", "embedding", "show"]));
    assert_eq!((emb["text_leaves_this_machine"].clone(), emb["config_file_present"].clone(), emb["embedder"]["provider"].clone()), (json!(false), json!(false), json!("hashing")));
    let s = jsn(&go(&["--json", "search", "antenna keep-out", "--device", "AC-100", "--mode", "hybrid"]));
    assert_eq!(s["mode_used"], "hybrid");
    assert!(!s["hits"].as_array().unwrap().is_empty());
    let t = jsn(&go(&["--json", "embedding", "test"]));
    assert!(t["similarity"]["related_sentences"].as_f64().unwrap() > t["similarity"]["unrelated_sentences"].as_f64().unwrap() + 0.1, "{t}");
    // reading, listing and exporting packs does not touch the index at all
    std::fs::remove_dir_all(f.ws.join("derived")).unwrap();
    assert_eq!(code(&go(&["packs", "list"])), 0);
}

// ---- providers ------------------------------------------------------------------------------------------------------------------

fn python() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join("python3")).find(|p| p.is_file())
}

const TOY: &str = r#"import json, sys
d = json.load(sys.stdin)
TOPICS = [["strapping", "sampled", "boot", "pull-up", "reset", "startup", "configuration"], ["antenna", "keep-out", "copper"], ["capacitor", "decoupling", "decoupled", "supply"], ["connector", "mounting"]]
def vec(text):
    words = text.lower().replace(";", " ").replace(".", " ").split()
    v = [float(sum(w in t for w in words)) + 0.01 for t in TOPICS]
    return v + [0.01] * 4
sys.stdout.write(json.dumps({"model": "toy", "version": "1", "dim": 8, "vectors": [vec(t) for t in d["texts"]]}))
"#;

#[test]
fn a_local_program_can_be_the_embedder_and_finds_what_the_words_do_not() {
    let Some(py) = python() else {
        eprintln!("python3 not found: skipped");
        return;
    };
    let f = built("command");
    std::fs::write(f.ws.join("toy_embed.py"), TOY).unwrap();
    write_embedding(&f.ws, &format!("provider: command\nmodel: toy\nversion: \"1\"\ndim: 8\ncommand: [\"{}\", \"toy_embed.py\"]\n", py.display()));
    let emb = jsn(&run(&f.ws, &["--json", "embedding", "show"]));
    assert_eq!(emb["text_leaves_this_machine"], json!(false), "a program on this machine is local");
    let b = jsn(&run(&f.ws, &["--json", "index", "build"]));
    let keys = built_keys(&b);
    assert!(keys.iter().all(|k| k.1 == "kept" && k.2 == "built"), "{keys:?}");
    assert!(b["embedder"]["key"].as_str().unwrap().starts_with("command:toy:1:8:"), "{b}");
    // "startup configuration" is not a word of any document: keywords find nothing, the plugged-in model finds the strapping page
    let kw = search(&f.ws, "startup configuration", &["--device", "AC-100", "--mode", "keyword"]);
    assert!(kw["hits"].as_array().unwrap().is_empty());
    let hy = search(&f.ws, "startup configuration", &["--device", "AC-100", "--mode", "vector"]);
    assert_eq!(hy["hits"][0]["document_id"], "DOC-AC-100-DS");
    assert!(hy["hits"][0]["snippet"].as_str().unwrap().contains("Strapping") || hy["hits"][0]["snippet"].as_str().unwrap().contains("sampled"), "{hy}");
    assert_eq!(hy["hits"][0]["matched_by"], json!(["vector"]));
    // what the program says it is must be what the configuration says
    write_embedding(&f.ws, &format!("provider: command\nmodel: toy\nversion: \"1\"\ndim: 16\ncommand: [\"{}\", \"toy_embed.py\"]\n", py.display()));
    let o = run(&f.ws, &["--json", "index", "build"]);
    assert_eq!(code(&o), 5, "{}{}", out(&o), err(&o));
    assert!(jsn(&o)["documents"][0]["vectors"]["reason"].as_str().unwrap().contains("reports dim 8"), "{}", out(&o));
    // a program that fails costs the vectors and nothing else
    write_embedding(&f.ws, &format!("provider: command\nmodel: toy\nversion: \"1\"\ndim: 8\ncommand: [\"{}\", \"-c\", \"import sys; sys.exit(3)\"]\n", py.display()));
    let o = run(&f.ws, &["--json", "index", "rebuild"]);
    assert_eq!(code(&o), 5);
    let b = jsn(&o);
    assert!(b["documents"].as_array().unwrap().iter().all(|d| d["keyword"] == "built" && d["vectors"]["state"] == "skipped"), "{b}");
    let s = search(&f.ws, "antenna", &["--device", "AC-100", "--mode", "hybrid"]);
    assert_eq!((s["mode_used"].as_str(), s["hits"].as_array().unwrap().is_empty()), (Some("keyword"), false));
    assert_eq!(code(&run(&f.ws, &["search", "antenna", "--device", "AC-100", "--mode", "vector"])), 3);
    // a program that does not exist
    write_embedding(&f.ws, "provider: command\nmodel: toy\nversion: \"1\"\ndim: 8\ncommand: [\"/nonexistent/embedder\"]\n");
    let o = run(&f.ws, &["--json", "index", "build"]);
    assert_eq!(code(&o), 5);
    assert!(jsn(&o)["documents"][0]["vectors"]["reason"].as_str().unwrap().contains("cannot start"), "{}", out(&o));
}

/// A service on this machine: records every request body and answers with an 8-number vector per text.
struct Mock {
    port: u16,
    bodies: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Mock {
    fn start(bind: &str) -> Mock {
        let listener = TcpListener::bind(format!("{bind}:0")).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (bodies, stop) = (Arc::new(Mutex::new(Vec::new())), Arc::new(AtomicBool::new(false)));
        let (b2, s2) = (bodies.clone(), stop.clone());
        let handle = std::thread::spawn(move || {
            while !s2.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut sock, _)) => {
                        sock.set_nonblocking(false).unwrap();
                        let mut buf = Vec::new();
                        let mut chunk = [0u8; 4096];
                        let (mut header_end, mut want) = (None, 0usize);
                        loop {
                            let n = sock.read(&mut chunk).unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            buf.extend_from_slice(&chunk[..n]);
                            if header_end.is_none() {
                                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                    header_end = Some(p + 4);
                                    let head = String::from_utf8_lossy(&buf[..p]).to_ascii_lowercase();
                                    want = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                                }
                            }
                            if let Some(h) = header_end {
                                if buf.len() >= h + want {
                                    break;
                                }
                            }
                        }
                        let h = header_end.unwrap_or(buf.len());
                        let body: Value = serde_json::from_slice(&buf[h..]).unwrap_or(Value::Null);
                        let texts: Vec<String> = body["input"].as_array().map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_string)).collect()).unwrap_or_default();
                        b2.lock().unwrap().push(body);
                        let data: Vec<Value> = texts
                            .iter()
                            .enumerate()
                            .map(|(i, t)| {
                                let mut v = [0.01f64; 8];
                                for b in t.bytes() {
                                    v[(b % 8) as usize] += 1.0;
                                }
                                json!({"index": i, "embedding": v})
                            })
                            .collect();
                        let payload = serde_json::to_string(&json!({"data": data})).unwrap();
                        let _ = write!(sock, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len());
                    }
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
                }
            }
        });
        Mock { port, bodies, stop, handle: Some(handle) }
    }

    fn requests(&self) -> usize {
        self.bodies.lock().unwrap().len()
    }

    fn texts(&self) -> Vec<String> {
        self.bodies.lock().unwrap().iter().flat_map(|b| b["input"].as_array().cloned().unwrap_or_default()).filter_map(|t| t.as_str().map(str::to_string)).collect()
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn curl_here() -> bool {
    Command::new("curl").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

/// An address of this machine that is not the loopback one (None where there is none).
fn lan_address() -> Option<String> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then(|| ip.to_string())
}

fn run_noproxy(ws: &Path, args: &[&str]) -> Output {
    Command::new(bin()).arg("--workspace").arg(ws).args(args).env("NO_PROXY", "*").env("no_proxy", "*").env_remove("HTTP_PROXY").env_remove("HTTPS_PROXY").env_remove("http_proxy").env_remove("https_proxy").env_remove("ALL_PROXY").env("HCP_TODAY", "2026-11-16").output().unwrap()
}

#[test]
fn a_service_on_this_machine_is_local_and_gets_the_text_it_is_asked_to_embed() {
    if !curl_here() {
        eprintln!("curl not found: skipped");
        return;
    }
    let f = built("http-local");
    let mock = Mock::start("127.0.0.1");
    write_embedding(&f.ws, &format!("provider: http\nurl: http://127.0.0.1:{}/v1/embeddings\nmodel: mock\ndim: 8\n", mock.port));
    let emb = jsn(&run(&f.ws, &["--json", "embedding", "show"]));
    assert_eq!(emb["text_leaves_this_machine"], json!(false), "a service on localhost is local: no authorisation is asked for");
    let b = jsn(&run_noproxy(&f.ws, &["--json", "index", "build"]));
    assert!(built_keys(&b).iter().all(|k| k.2 == "built"), "{b}");
    assert!(mock.requests() > 0);
    assert!(mock.bodies.lock().unwrap().iter().all(|b| b["model"] == "mock"), "the model of the configuration is asked for");
    let s = jsn(&run_noproxy(&f.ws, &["--json", "search", "antenna keep-out", "--device", "AC-100", "--mode", "hybrid"]));
    assert_eq!((s["mode_used"].as_str(), s["vector"]["embedder"].as_str().map(|k| k.starts_with("http:mock:1:8:"))), (Some("hybrid"), Some(true)), "{s}");
    assert_eq!(mock.texts().last().unwrap(), "antenna keep-out", "the query is what was sent");
}

#[test]
fn a_remote_service_receives_nothing_unless_the_configuration_the_command_and_the_document_all_say_so() {
    // never a call to curl before the refusals: an empty PATH would make a call fail with a different message
    let f = built("remote");
    let refuse = |args: &[&str]| Command::new(bin()).arg("--workspace").arg(&f.ws).args(args).env("PATH", "").env("HCP_TODAY", "2026-11-16").output().unwrap();
    write_embedding(&f.ws, "provider: http\nurl: https://embeddings.example.com/v1/embeddings\nmodel: m\ndim: 8\nallow_remote: false\n");
    let emb = jsn(&refuse(&["--json", "embedding", "show"]));
    assert_eq!(emb["text_leaves_this_machine"], json!(true));
    assert_eq!(emb["remote_requires"].as_array().unwrap().len(), 3);
    // 1. the configuration does not allow it
    let o = refuse(&["--json", "index", "build", "--authorize-remote"]);
    assert_eq!(code(&o), 5, "{}{}", out(&o), err(&o));
    assert!(jsn(&o)["embedder_problem"].as_str().unwrap().contains("allow_remote"), "{}", out(&o));
    // 2. the configuration allows it, the command does not
    write_embedding(&f.ws, "provider: http\nurl: https://embeddings.example.com/v1/embeddings\nmodel: m\ndim: 8\nallow_remote: true\n");
    let o = refuse(&["--json", "index", "build"]);
    assert_eq!(code(&o), 5);
    assert!(jsn(&o)["embedder_problem"].as_str().unwrap().contains("--authorize-remote"), "{}", out(&o));
    // the query of a search is not sent either
    let s = jsn(&refuse(&["--json", "search", "antenna", "--device", "AC-100", "--mode", "hybrid"]));
    assert_eq!(s["mode_used"], "keyword");
    assert!(s["degraded"].as_str().unwrap().contains("remote service"), "{s}");
    assert_eq!(code(&refuse(&["search", "antenna", "--device", "AC-100", "--mode", "vector"])), 3);
    // 3. both authorise it, but no document says it may be sent: nothing is embedded, nothing is sent
    let o = refuse(&["--json", "index", "build", "--authorize-remote"]);
    assert_eq!(code(&o), 5);
    let b = jsn(&o);
    assert!(b["documents"].as_array().unwrap().iter().all(|d| d["vectors"]["state"] == "skipped" && d["vectors"]["reason"].as_str().unwrap().contains("embed_remote")), "{b}");
    // the keyword index is complete all the while
    assert_eq!(jsn(&refuse(&["--json", "index", "status"]))["keyword_index"], "ready");
}

#[test]
fn a_remote_service_gets_only_the_documents_that_are_marked_and_only_when_authorised() {
    if !curl_here() {
        eprintln!("curl not found: skipped");
        return;
    }
    let Some(ip) = lan_address() else {
        eprintln!("this machine has no address other than the loopback one: skipped");
        return;
    };
    let f = built("remote-send");
    let mock = Mock::start("0.0.0.0");
    write_embedding(&f.ws, &format!("provider: http\nurl: http://{ip}:{}/v1/embeddings\nmodel: mock\ndim: 8\nallow_remote: true\n", mock.port));
    let emb = jsn(&run_noproxy(&f.ws, &["--json", "embedding", "show"]));
    assert_eq!(emb["text_leaves_this_machine"], json!(true), "{emb}");
    // not authorised on the command line, or no document marked: nothing arrives
    assert_eq!(code(&run_noproxy(&f.ws, &["index", "build"])), 5);
    assert_eq!(code(&run_noproxy(&f.ws, &["index", "build", "--authorize-remote"])), 5);
    assert_eq!(mock.requests(), 0, "no request was made");
    // one document is marked
    let p = f.ws.join("sources.yaml");
    let mut y: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    for d in y["documents"].as_sequence_mut().unwrap() {
        if d["document_id"].as_str() == Some("DOC-AC-100-DS") {
            d["embed_remote"] = serde_yaml::Value::Bool(true);
        }
    }
    std::fs::write(&p, serde_yaml::to_string(&y).unwrap()).unwrap();
    let o = run_noproxy(&f.ws, &["--json", "index", "build", "--authorize-remote"]);
    assert_eq!(code(&o), 5, "the unmarked documents are skipped, so the build is not complete");
    let b = jsn(&o);
    let state = |d: &str| b["documents"].as_array().unwrap().iter().find(|x| x["document_id"] == d).unwrap()["vectors"]["state"].as_str().unwrap().to_string();
    assert_eq!((state("DOC-AC-100-DS").as_str(), state("DOC-AC-100-HDG").as_str()), ("built", "skipped"), "{b}");
    let sent = mock.texts();
    assert!(!sent.is_empty());
    // what was sent is the text of the marked document and of no other
    for forbidden in ["decoupled", "Mounting hole", "external antenna connector", "20 mm"] {
        assert!(!sent.iter().any(|t| t.contains(forbidden)), "'{forbidden}' was sent: it is not in the marked document");
    }
    assert!(sent.iter().any(|t| t.contains("Strapping pins")));
    // the query is sent only when the search is authorised
    let n = mock.requests();
    let s = jsn(&run_noproxy(&f.ws, &["--json", "search", "antenna", "--device", "AC-100", "--mode", "hybrid"]));
    assert_eq!((s["mode_used"].as_str(), mock.requests()), (Some("keyword"), n));
    let s = jsn(&run_noproxy(&f.ws, &["--json", "search", "antenna", "--device", "AC-100", "--mode", "hybrid", "--authorize-remote"]));
    assert_eq!(s["mode_used"], "hybrid");
    assert_eq!(mock.requests(), n + 1);
    assert_eq!(mock.texts().last().unwrap(), "antenna");
    // only the marked document is searched by vector; the rest by keyword, and the answer says so
    assert_eq!(s["vector"]["documents_with_vectors"], json!(["DOC-AC-100-DS"]));
    assert!(s["warnings"].to_string().contains("searched by keyword only"), "{s}");
}

// ---- packs: create, export, import -----------------------------------------------------------------------------------------

#[test]
fn a_pack_skeleton_is_made_from_the_inventory_and_is_not_valid_until_the_facts_are_written() {
    let f = built("packs-create");
    let o = run(&f.ws, &["--json", "packs", "create", "--id", "acme/ac-100", "--device", "AC-100", "--vendor", "Acme", "--title", "Acme AC-100"]);
    assert_eq!(code(&o), 0, "{}{}", out(&o), err(&o));
    let r = jsn(&o);
    assert_eq!(r["valid"], json!(false), "a skeleton holds no pins: it says so");
    assert!(r["errors"].to_string().contains("pins"), "{r}");
    // the documents of this device and of no other model, with the digests the cache has
    let srcs: Vec<&str> = r["sources"].as_array().unwrap().iter().map(|s| s.as_str().unwrap()).collect();
    assert_eq!(srcs.len(), 4, "{srcs:?}");
    assert!(!srcs.iter().any(|s| s.contains("100U")));
    let path = f.ws.join("packs/acme/ac-100/0.1.0/pack.yaml");
    let pack = fr_constraint::hcp::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(pack["pack"]["review"]["status"], "draft");
    assert_eq!(pack["devices"][0]["pins"], json!([]));
    let inv: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(f.ws.join("sources.yaml")).unwrap()).unwrap();
    for s in pack["sources"].as_array().unwrap() {
        let d = inv["documents"].as_sequence().unwrap().iter().find(|d| d["document_id"].as_str() == s["document_id"].as_str()).unwrap();
        assert_eq!(d["sha256"].as_str(), s["sha256"].as_str());
    }
    // never overwritten; another version is another file; the highest version is the default
    assert_eq!(code(&run(&f.ws, &["packs", "create", "--id", "acme/ac-100", "--device", "AC-100", "--vendor", "Acme"])), 4);
    assert_eq!(code(&run(&f.ws, &["packs", "create", "--id", "acme/ac-100", "--pack-version", "0.10.0", "--device", "AC-100", "--vendor", "Acme"])), 0);
    assert_eq!(code(&run(&f.ws, &["packs", "create", "--id", "acme/ac-100", "--pack-version", "0.9.0", "--device", "AC-100", "--vendor", "Acme"])), 0);
    let l = jsn(&run(&f.ws, &["--json", "packs", "list"]));
    assert_eq!(l["packs"].as_array().unwrap().len(), 3);
    assert_eq!(jsn(&run(&f.ws, &["--json", "packs", "show", "acme/ac-100"]))["info"]["version"], "0.10.0");
    assert_eq!(jsn(&run(&f.ws, &["--json", "packs", "show", "acme/ac-100@0.9.0"]))["info"]["version"], "0.9.0");
    // a document that is not synced is not cited with a digest it does not have
    let g = fixture("packs-create-unsynced");
    let r = jsn(&run(&g.ws, &["--json", "packs", "create", "--id", "acme/x", "--device", "AC-100", "--vendor", "Acme"]));
    assert!(r["sources"].as_array().unwrap().is_empty() && r["not_included"].as_array().unwrap().len() == 4, "{r}");
    assert_eq!(code(&run(&f.ws, &["packs", "create", "--id", "../escape", "--device", "AC-100", "--vendor", "Acme"])), 2);
}

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/hardware-constraint-design/assets").join(name)
}

#[test]
fn a_pack_is_exported_and_imported_into_another_workspace_with_the_documents_it_cites() {
    let a = tmp("packs-a");
    let wsa = a.join("ws");
    assert_eq!(code(&run(&wsa, &["init"])), 0);
    assert_eq!(code(&run(&wsa, &["sources", "import", example("example-sources.yaml").to_str().unwrap()])), 0);
    let dir = a.join("export");
    let o = run(&wsa, &["packs", "export", example("example-device.yaml").to_str().unwrap(), "--to", dir.to_str().unwrap()]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    // another workspace: empty, then imported
    let wsb = tmp("packs-b").join("ws");
    let o = run(&wsb, &["--json", "packs", "import", dir.to_str().unwrap()]);
    assert_eq!(code(&o), 0, "{}{}", out(&o), err(&o));
    let r = jsn(&o);
    assert_eq!((r["pack_id"].as_str(), r["version"].as_str()), (Some("example/acme-fm-100"), Some("0.1.0")));
    assert_eq!(r["inventory_added"].as_array().unwrap().len(), 3, "the documents the pack cites: {r}");
    assert!(wsb.join("packs/example/acme-fm-100/0.1.0/pack.yaml").is_file());
    let l = jsn(&run(&wsb, &["--json", "packs", "list"]));
    assert_eq!(l["packs"][0]["valid"], json!(true));
    assert_eq!(code(&run(&wsb, &["packs", "show", "example/acme-fm-100"])), 0);
    // again: nothing to do, and said so
    let r = jsn(&run(&wsb, &["--json", "packs", "import", dir.to_str().unwrap()]));
    assert_eq!((r["already_imported"].clone(), r["inventory_added"].clone()), (json!(true), json!([])));
    // an export that was changed after it was written is refused
    let bad = a.join("export-bad");
    std::fs::create_dir_all(&bad).unwrap();
    for e in std::fs::read_dir(&dir).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), bad.join(e.file_name())).unwrap();
    }
    let manifest: Value = serde_json::from_slice(&std::fs::read(bad.join("export-manifest.json")).unwrap()).unwrap();
    let pack_name = manifest["files"].as_object().unwrap().keys().find(|k| *k != "sources-used.yaml").unwrap().clone();
    let pack_file = bad.join(&pack_name);
    std::fs::write(&pack_file, std::fs::read_to_string(&pack_file).unwrap().replace("0.1.0", "0.1.1")).unwrap();
    let o = run(&tmp("packs-c").join("ws"), &["packs", "import", bad.to_str().unwrap()]);
    assert_eq!(code(&o), 3);
    assert!(err(&o).contains("export was changed"), "{}", err(&o));
    // a workspace whose inventory says something else about a document does not take the pack, and its inventory stays as it was
    let wsd = tmp("packs-d").join("ws");
    assert_eq!(code(&run(&wsd, &["init"])), 0);
    assert_eq!(code(&run(&wsd, &["sources", "add", "--id", "DOC-FM100-DS", "--title", "x", "--level", "official_manual", "--version", "v9.9", "--url", "https://example.invalid/x.pdf", "--applies-to", "FM-100"])), 0);
    let inv_before = std::fs::read(wsd.join("sources.yaml")).unwrap();
    let o = run(&wsd, &["packs", "import", dir.to_str().unwrap()]);
    assert_eq!(code(&o), 3);
    assert!(err(&o).contains("DOC-FM100-DS") && err(&o).contains("v9.9"), "{}", err(&o));
    assert_eq!(std::fs::read(wsd.join("sources.yaml")).unwrap(), inv_before);
    assert!(!wsd.join("packs").join("example").exists());
    // a pack that is not valid is not imported
    let invalid = a.join("invalid.yaml");
    std::fs::write(&invalid, "hcp_version: \"1.0\"\npack: {id: x/y, title: t, version: \"1\", created: \"2026-11-16\", extractor: {kind: human}}\nsources: []\ndevices: []\n").unwrap();
    let o = run(&tmp("packs-e").join("ws"), &["packs", "import", invalid.to_str().unwrap()]);
    assert_eq!(code(&o), 3);
    assert!(err(&o).contains("not valid"), "{}", err(&o));
}

// ---- the evaluation set ----------------------------------------------------------------------------------------------------

#[test]
fn a_small_evaluation_set_shows_what_each_mode_finds_and_what_none_may_return() {
    let f = built("eval");
    // (query, expected document, expected page, the words are in the text: verbatim)
    let cases = [
        ("IO0 is sampled at reset", "DOC-AC-100-DS", 2, true),
        ("bulk capacitor at the supply input", "DOC-AC-100-DS", 3, true),
        ("free of copper on all layers", "DOC-AC-100-DS", 4, true),
        ("pull-up resistor and a capacitor to ground", "DOC-AC-100-HDG", 1, true),
        ("decouple capacitance", "DOC-AC-100-DS", 3, false),
        ("strapping pin sampling", "DOC-AC-100-DS", 2, false),
    ];
    let rank = |r: &Value, doc: &str, page: u64| r["hits"].as_array().unwrap().iter().position(|h| h["document_id"] == doc && (page == 1 && doc.contains("HDG") || h["page"].as_u64() == Some(page)));
    let (mut found_kw, mut found_hy) = (0, 0);
    for (q, doc, page, verbatim) in cases {
        let kw = search(&f.ws, q, &["--device", "AC-100", "--source-level", if doc.contains("HDG") { "official_guide" } else { "official_manual" }, "--mode", "keyword"]);
        let hy = search(&f.ws, q, &["--device", "AC-100", "--source-level", if doc.contains("HDG") { "official_guide" } else { "official_manual" }, "--mode", "hybrid", "--limit", "5"]);
        let (rk, rh) = (rank(&kw, doc, page), rank(&hy, doc, page));
        if verbatim {
            assert!(rk.is_some_and(|r| r < 3), "keyword: '{q}' -> {rk:?}");
            assert!(rh.is_some_and(|r| r < 3), "hybrid: '{q}' -> {rh:?}");
        }
        found_kw += rk.is_some_and(|r| r < 3) as usize;
        found_hy += rh.is_some_and(|r| r < 3) as usize;
        // the similar model is never among the answers
        assert!(!docs_of(&hy).contains(&"DOC-AC-100U-DS".to_string()), "{q}");
    }
    assert!(found_hy >= found_kw, "hybrid finds at least what keywords find ({found_hy} against {found_kw})");
    assert!(found_hy > found_kw, "and the variant spelling is what the vectors add ({found_hy} against {found_kw})");
}
