//! The two stores behind the derived index, `sqlite` (default) and `jsonl` (a directory of plain files): each in its own place, the same
//! candidates and the same way back to the page for the same query, damage and old layouts found and repaired by a rebuild and never
//! migrated, and nothing in a pack or an IR reading either. The embeddings are fixed fakes (the deterministic local provider, and a mock
//! service for the OpenAI-compatible one), so the two backends can be compared exactly.

mod common;
use common::*;
use fr_constraint::hcp;
use serde_json::{json, Value};
use std::path::Path;

const QUERIES: [&str; 5] = ["antenna keep-out copper", "IO0 sampled at reset boot mode", "decoupling capacitor supply", "EN pull-up resistor reset", "wireless module connector"];

fn built_both(name: &str, yaml: Option<&str>) -> Fx {
    let f = fixture(name);
    if let Some(y) = yaml {
        write_embedding(&f.ws, y);
    }
    for b in ["sqlite", "jsonl"] {
        let o = run(&f.ws, &["--backend", b, "--json", "index", "build"]);
        assert_eq!(code(&o), 0, "{b}: {}{}", out(&o), err(&o));
        assert_eq!(jsn(&o)["backend"], b);
    }
    f
}

fn hits(ws: &Path, backend: &str, q: &str, mode: &str, extra: &[&str]) -> Value {
    let mut a = vec!["--backend", backend, "--json", "search", q, "--mode", mode, "--limit", "10"];
    a.extend_from_slice(extra);
    let o = run(ws, &a);
    assert_eq!(code(&o), 0, "{backend} {mode} {q}: {}{}", out(&o), err(&o));
    jsn(&o)
}

fn compare(f: &Fx, expect_vector: bool) {
    let mut compared = 0;
    for q in QUERIES {
        for mode in ["keyword", "vector", "hybrid"] {
            for filter in [vec!["--device", "AC-100"], vec![], vec!["--device", "AC-100U"], vec!["--device", "AC-100", "--source-level", "official_guide"]] {
                let (a, b) = (hits(&f.ws, "sqlite", q, mode, &filter), hits(&f.ws, "jsonl", q, mode, &filter));
                assert_eq!(a["mode_used"], b["mode_used"], "{q} {mode} {filter:?}");
                assert_eq!(a["hits"], b["hits"], "the same candidates, scores and references on both backends: {q} {mode} {filter:?}");
                if expect_vector && mode == "vector" && filter.is_empty() {
                    assert!(a["hits"].as_array().is_some_and(|h| !h.is_empty()) && a["mode_used"] == "vector", "{a}");
                }
                // every hit goes back to the original: document, version, page, section, digest, and the text itself
                for h in a["hits"].as_array().unwrap() {
                    let id = h["chunk_id"].as_str().unwrap();
                    let show = |backend: &str| jsn(&run(&f.ws, &["--backend", backend, "--json", "chunk", "show", id]));
                    let (x, y) = (show("sqlite"), show("jsonl"));
                    assert_eq!(x["chunk"], y["chunk"], "{id}");
                    assert_eq!(x["current"], json!(true));
                    assert_eq!(x["chunk"]["document_id"], h["document_id"]);
                    assert_eq!(x["chunk"]["page"], h["page"]);
                    compared += 1;
                }
            }
        }
    }
    assert!(compared > 50, "{compared} hits compared");
    // the status of both says the same about every document
    let (a, b) = (jsn(&run(&f.ws, &["--backend", "sqlite", "--json", "index", "status"])), jsn(&run(&f.ws, &["--backend", "jsonl", "--json", "index", "status"])));
    assert_eq!(a["documents"], b["documents"]);
    assert_eq!(a["embedder"]["generations"], b["embedder"]["generations"]);
    assert_eq!((a["store"]["backend"].as_str(), b["store"]["backend"].as_str()), (Some("sqlite"), Some("jsonl")));
}

#[test]
fn each_backend_keeps_its_own_files_and_a_build_for_one_leaves_the_other_alone() {
    let f = fixture("b-locations");
    assert_eq!(code(&run(&f.ws, &["index", "build"])), 0);
    assert!(f.ws.join("derived/index.db").is_file() && !f.ws.join("derived/jsonl").exists(), "the default is sqlite and makes nothing else");
    let before = std::fs::read(f.ws.join("derived/index.db")).unwrap();
    assert_eq!(code(&run(&f.ws, &["--backend", "jsonl", "index", "build"])), 0);
    for file in ["store.json", "documents.jsonl", "chunks.jsonl", "embedders.jsonl", "doc_vectors.jsonl", "vectors.jsonl"] {
        assert!(f.ws.join("derived/jsonl").join(file).is_file(), "{file}");
    }
    assert_eq!(std::fs::read(f.ws.join("derived/index.db")).unwrap(), before, "the sqlite file was not touched by the jsonl build");
    let m: Value = serde_json::from_slice(&std::fs::read(f.ws.join("derived/jsonl/store.json")).unwrap()).unwrap();
    assert_eq!((m["format"].as_str(), m["backend"].as_str(), m["store_version"].as_i64()), (Some("hcp-manager-vector-store"), Some("jsonl"), Some(1)));
    assert!(m["files"]["vectors.jsonl"]["sha256"].as_str().unwrap().len() == 64 && m["files"]["vectors.jsonl"]["lines"].as_u64().unwrap() > 0);
    // the files are plain JSON Lines: a person can read them
    let first = std::fs::read_to_string(f.ws.join("derived/jsonl/chunks.jsonl")).unwrap();
    let c: Value = serde_json::from_str(first.lines().next().unwrap()).unwrap();
    assert!(c["chunk_id"].as_str().unwrap().starts_with("CHK-") && c["text"].is_string() && c["page"].is_number(), "{c}");
    let v: Value = serde_json::from_str(std::fs::read_to_string(f.ws.join("derived/jsonl/vectors.jsonl")).unwrap().lines().next().unwrap()).unwrap();
    assert!(v["embedder_key"].as_str().unwrap().starts_with("hashing:") && v["vec"].as_array().unwrap().len() == 384);
    // the status names the backend and where it lives
    let st = jsn(&run(&f.ws, &["--backend", "jsonl", "--json", "index", "status"]));
    assert_eq!(st["store"]["backend"], "jsonl");
    assert!(st["store"]["path"].as_str().unwrap().ends_with("derived/jsonl"), "{st}");
    // an unknown backend is a usage error
    assert_eq!(code(&run(&f.ws, &["--backend", "cloud", "index", "status"])), 2);
}

#[test]
fn the_backend_comes_from_the_flag_or_from_the_configuration() {
    let f = fixture("b-choice");
    write_embedding(&f.ws, "provider: hashing\nstore:\n  backend: jsonl\n");
    assert_eq!(code(&run(&f.ws, &["index", "build"])), 0);
    assert!(f.ws.join("derived/jsonl/store.json").is_file() && !f.ws.join("derived/index.db").exists(), "the configuration chose jsonl");
    assert_eq!(jsn(&run(&f.ws, &["--json", "embedding", "show"]))["store_backend"], "jsonl");
    // the flag wins; the other backend is empty until it is built
    let st = jsn(&run(&f.ws, &["--backend", "sqlite", "--json", "index", "status"]));
    assert_eq!((st["store"]["state"].as_str(), st["vector_index"].as_str()), (Some("missing"), Some("empty")), "{st}");
    // only `store:` in the file: the default provider with that store
    write_embedding(&f.ws, "store:\n  backend: jsonl\n");
    assert_eq!(jsn(&run(&f.ws, &["--json", "embedding", "show"]))["embedder"]["provider"], "hashing");
}

#[test]
fn both_backends_give_the_same_candidates_and_the_same_way_back_to_the_page() {
    let f = built_both("b-same-hashing", None);
    compare(&f, true);
}

#[test]
fn the_same_holds_with_the_vectors_of_an_openai_compatible_service() {
    if !curl_here() {
        eprintln!("curl not found: skipped");
        return;
    }
    let mock = Mock::start("127.0.0.1", fake_service(true));
    let f = built_both("b-same-openai", Some(&format!("provider: openai\nbase_url: http://127.0.0.1:{}\nmodel: m8\ndim: 8\nretry_backoff_ms: 5\n", mock.port)));
    compare(&f, true);
    // the service was asked once per backend for the same texts
    assert!(mock.count() >= 2);
}

#[test]
fn deleting_either_vector_store_changes_no_pack_no_ir_and_not_the_other_store() {
    let f = built_both("b-delete", None);
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/hardware-constraint-design/assets/example-device.yaml");
    let o = run(&f.ws, &["--json", "packs", "import", example.to_str().unwrap()]);
    assert_eq!(code(&o), 5, "imported; its fictional documents are not in the inventory, which is said (exit 5): {}", err(&o));
    let snapshot = |ws: &Path| {
        let pack = hcp::parse(&std::fs::read_to_string(&example).unwrap()).unwrap();
        let ir = serde_json::to_string(&fr_constraint::compile::compile(&pack).unwrap()).unwrap();
        let shown = jsn(&run(ws, &["--json", "packs", "list"]));
        (ir, shown, std::fs::read(ws.join("sources.yaml")).unwrap(), std::fs::read(ws.join("packs/example/acme-fm-100/0.1.0/pack.yaml")).unwrap())
    };
    let before = snapshot(&f.ws);
    let jsonl_before: Vec<_> = all_files(&f.ws.join("derived/jsonl")).into_iter().map(|(p, b)| (p, hcp::sha256_hex(&b))).collect();
    // delete the sqlite store: the pack, the IR, the inventory are as they were, and so is the other backend
    std::fs::remove_file(f.ws.join("derived/index.db")).unwrap();
    assert_eq!(before, snapshot(&f.ws));
    let jsonl_after: Vec<_> = all_files(&f.ws.join("derived/jsonl")).into_iter().map(|(p, b)| (p, hcp::sha256_hex(&b))).collect();
    assert_eq!(jsonl_before, jsonl_after);
    assert_eq!(search(&f.ws, "antenna keep-out copper", &["--backend", "jsonl", "--device", "AC-100", "--mode", "hybrid"])["mode_used"], "hybrid");
    // delete the jsonl store too: still nothing of the facts is gone; keyword search says why there are no blocks, and a rebuild brings them back
    std::fs::remove_dir_all(f.ws.join("derived")).unwrap();
    assert_eq!(before, snapshot(&f.ws));
    let o = run(&f.ws, &["--json", "search", "antenna", "--device", "AC-100"]);
    assert!(jsn(&o)["hits"].as_array().unwrap().is_empty(), "no index, no hits, and it says why: {}", out(&o));
    assert_eq!(code(&run(&f.ws, &["index", "build"])), 0);
    assert_eq!(code(&run(&f.ws, &["--backend", "jsonl", "index", "build"])), 0);
    assert_eq!(before, snapshot(&f.ws));
}

#[test]
fn rebuild_removes_only_the_backend_in_force() {
    let f = built_both("b-rebuild", None);
    let sqlite_before = std::fs::read(f.ws.join("derived/index.db")).unwrap();
    assert_eq!(code(&run(&f.ws, &["--backend", "jsonl", "index", "rebuild"])), 0);
    assert_eq!(std::fs::read(f.ws.join("derived/index.db")).unwrap(), sqlite_before, "the sqlite store is not part of a jsonl rebuild");
    assert!(f.ws.join("derived/jsonl/store.json").is_file());
    // and a rebuild gives the same store: the same bytes of every file
    let files = |ws: &Path| all_files(&ws.join("derived/jsonl")).into_iter().map(|(p, b)| (p, hcp::sha256_hex(&b))).collect::<Vec<_>>();
    let one = files(&f.ws);
    assert_eq!(code(&run(&f.ws, &["--backend", "jsonl", "index", "rebuild"])), 0);
    assert_eq!(one, files(&f.ws), "a rebuilt jsonl store is byte for byte the same");
    assert_eq!(code(&run(&f.ws, &["--backend", "sqlite", "index", "rebuild"])), 0);
    assert!(f.ws.join("derived/jsonl/store.json").is_file(), "the jsonl store is not part of a sqlite rebuild");
}

#[test]
fn damage_is_found_by_the_checksums_and_by_the_counts_and_a_build_repairs_it() {
    let f = built_both("b-damage", None);
    // jsonl: a changed byte in a file does not match the manifest
    let docs = f.ws.join("derived/jsonl/documents.jsonl");
    let mut b = std::fs::read(&docs).unwrap();
    b[10] ^= 1;
    std::fs::write(&docs, b).unwrap();
    let st = jsn(&run(&f.ws, &["--backend", "jsonl", "--json", "index", "status"]));
    assert_eq!(st["store"]["state"], "unreadable", "{st}");
    assert!(st["store"]["reason"].as_str().unwrap().contains("checksum"), "{st}");
    assert_eq!(search(&f.ws, "antenna", &["--backend", "jsonl", "--device", "AC-100"])["hits"].as_array().unwrap().len(), 0, "a damaged store is not read");
    let b = jsn(&run(&f.ws, &["--backend", "jsonl", "--json", "index", "build"]));
    assert!(b["store_note"].as_str().unwrap().contains("made again"), "{b}");
    assert_eq!(jsn(&run(&f.ws, &["--backend", "jsonl", "--json", "index", "status"]))["vector_index"], "ready");
    // sqlite: a file that is not a database
    std::fs::write(f.ws.join("derived/index.db"), b"this is not a database at all, but long enough to be read as a header....").unwrap();
    let st = jsn(&run(&f.ws, &["--json", "index", "status"]));
    assert_eq!(st["store"]["state"], "unreadable", "{st}");
    assert_eq!(code(&run(&f.ws, &["index", "build"])), 0);
    assert_eq!(jsn(&run(&f.ws, &["--json", "index", "status"]))["vector_index"], "ready");

    // a store that is consistent but holds fewer vectors than it recorded: the vectors of that document are stale, the blocks are not
    let conn = rusqlite::Connection::open(f.ws.join("derived/index.db")).unwrap();
    let removed = conn.execute("DELETE FROM vectors WHERE rowid IN (SELECT rowid FROM vectors LIMIT 2)", []).unwrap();
    drop(conn);
    assert_eq!(removed, 2);
    let st = jsn(&run(&f.ws, &["--json", "index", "status"]));
    assert_eq!((st["keyword_index"].as_str(), st["vector_index"].as_str()), (Some("ready"), Some("partial")), "{st}");
    let bad: Vec<&Value> = st["documents"].as_array().unwrap().iter().filter(|d| d["vector"]["state"] == "stale").collect();
    assert_eq!(bad.len(), 1, "{st}");
    assert!(bad[0]["vector"]["reason"].as_str().unwrap().contains("damaged"), "{bad:?}");
    let b = jsn(&run(&f.ws, &["--json", "index", "build"]));
    let rebuilt: Vec<_> = b["documents"].as_array().unwrap().iter().filter(|d| d["vectors"]["state"] == "built").collect();
    assert_eq!(rebuilt.len(), 1, "only the damaged document gets new vectors: {b}");
    assert_eq!(jsn(&run(&f.ws, &["--json", "index", "status"]))["vector_index"], "ready");

    // the same on jsonl: drop two vector lines and make the manifest agree with the file, as a store would be after a partial copy
    let dir = f.ws.join("derived/jsonl");
    let vf = dir.join("vectors.jsonl");
    let text = std::fs::read_to_string(&vf).unwrap();
    let kept: String = text.lines().skip(2).map(|l| format!("{l}\n")).collect();
    std::fs::write(&vf, &kept).unwrap();
    let mut m: Value = serde_json::from_slice(&std::fs::read(dir.join("store.json")).unwrap()).unwrap();
    m["files"]["vectors.jsonl"] = json!({"sha256": hcp::sha256_hex(kept.as_bytes()), "lines": kept.lines().count()});
    std::fs::write(dir.join("store.json"), serde_json::to_string_pretty(&m).unwrap()).unwrap();
    let st = jsn(&run(&f.ws, &["--backend", "jsonl", "--json", "index", "status"]));
    assert_eq!(st["store"]["state"], "ready", "the checksums are consistent: {st}");
    assert!(st["documents"].as_array().unwrap().iter().any(|d| d["vector"]["state"] == "stale" && d["vector"]["reason"].as_str().unwrap().contains("damaged")), "{st}");
    assert_eq!(code(&run(&f.ws, &["--backend", "jsonl", "index", "build"])), 0);
    assert_eq!(jsn(&run(&f.ws, &["--backend", "jsonl", "--json", "index", "status"]))["vector_index"], "ready");
}

#[test]
fn another_layout_version_is_rebuilt_never_migrated() {
    let f = built_both("b-version", None);
    // jsonl
    let p = f.ws.join("derived/jsonl/store.json");
    let mut m: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    m["store_version"] = json!(99);
    std::fs::write(&p, serde_json::to_string_pretty(&m).unwrap()).unwrap();
    let st = jsn(&run(&f.ws, &["--backend", "jsonl", "--json", "index", "status"]));
    assert_eq!(st["store"]["state"], "other_version", "{st}");
    assert!(st["store"]["reason"].as_str().unwrap().contains("no in-place migration"), "{st}");
    let b = jsn(&run(&f.ws, &["--backend", "jsonl", "--json", "index", "build"]));
    assert!(b["store_note"].as_str().unwrap().contains("layout version 99"), "{b}");
    // sqlite
    let conn = rusqlite::Connection::open(f.ws.join("derived/index.db")).unwrap();
    conn.pragma_update(None, "user_version", 99).unwrap();
    drop(conn);
    let st = jsn(&run(&f.ws, &["--json", "index", "status"]));
    assert_eq!(st["store"]["state"], "other_version", "{st}");
    let b = jsn(&run(&f.ws, &["--json", "index", "build"]));
    assert!(b["store_note"].as_str().unwrap().contains("layout version 99"), "{b}");
    assert_eq!(jsn(&run(&f.ws, &["--json", "index", "status"]))["vector_index"], "ready");
}
