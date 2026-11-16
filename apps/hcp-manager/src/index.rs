//! The derived index: the text blocks of every cached document and, when an embedding provider is available, a vector for each
//! block, all in the embedded store (`derived/index.db`). It is made from the cache, can be deleted at any time, and says when it is
//! stale. What it is made from is recorded: the digest of the source document, the chunking strategy, the entry of the inventory
//! (version, source level, devices) and the embedding configuration. A change of any of these makes the blocks or the vectors
//! built before stale; `index build` makes again exactly what is stale and keeps what is current.
//!
//! Nothing here is needed to read, export or check a pack. Keyword search works from the blocks alone; the vectors only add a
//! second ranking, and a search says when it had to do without them.

use crate::embed::{self, Embedder, EmbedderInfo};
use crate::error::{Error, Result};
use crate::inventory::{Doc, Inventory};
use crate::store::{ChunkRecord, DocRow, EmbedderRow, Probe, Store, VecRow, CHUNKER};
use crate::text::{chunk_pages, extract_pages, Chunk};
use crate::workspace::Workspace;
use fr_constraint::hcp::sha256_hex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Meta {
    pub document_id: String,
    pub sha256: String,
    pub file: String,
    pub bytes: u64,
    pub retrieved: String,
    pub origin: String,
}

pub fn read_meta(ws: &Workspace, doc: &str) -> Option<Meta> {
    serde_json::from_slice(&std::fs::read(ws.meta_path(doc)).ok()?).ok()
}

pub fn cached_file(ws: &Workspace, meta: &Meta) -> std::path::PathBuf {
    ws.cache_dir(&meta.document_id).join(&meta.file)
}

#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    /// Make everything of the selected documents again, current or not
    pub force: bool,
    /// Build the text blocks only
    pub no_vector: bool,
    /// The caller authorises that a remote embedding provider receives text
    pub authorize_remote: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct VectorOutcome {
    /// `built`, `kept` (current), `skipped` (and why) or `not_requested`
    pub state: &'static str,
    pub vectors: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocBuild {
    pub document_id: String,
    /// `built`, `kept` (current) or `error`
    pub keyword: &'static str,
    pub chunks: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub vectors: VectorOutcome,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildReport {
    pub format: &'static str,
    pub format_version: &'static str,
    pub documents: Vec<DocBuild>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedder: Option<EmbedderInfo>,
    /// Why there are no vectors although they were asked for
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedder_problem: Option<String>,
}

impl BuildReport {
    /// 5 when something that was asked for could not be done.
    pub fn complete(&self) -> bool {
        self.embedder_problem.is_none() && self.documents.iter().all(|d| d.keyword != "error" && d.vectors.state != "skipped")
    }
}

fn record(d: &Doc, c: &Chunk) -> ChunkRecord {
    ChunkRecord {
        chunk_id: c.chunk_id.clone(),
        document_id: c.document_id.clone(),
        document_version: d.version.clone(),
        level: d.level.clone(),
        applies_to: d.applies_to.iter().map(|a| match &a.revision { Some(r) => format!("{} {r}", a.part_number), None => a.part_number.clone() }).collect(),
        page: c.page,
        ordinal: c.ordinal,
        section: c.section.clone(),
        text: c.text.clone(),
        text_sha256: sha256_hex(c.text.as_bytes()),
    }
}

fn embedder_row(i: &EmbedderInfo) -> EmbedderRow {
    EmbedderRow { embedder_key: i.key.clone(), provider: i.provider.clone(), model: i.model.clone(), version: i.version.clone(), dim: i.dim, config_sha256: i.config_sha256.clone(), remote: i.remote }
}

/// Builds the blocks (and vectors) of the given documents (all of the inventory if none are named) that are not current.
pub fn build(ws: &Workspace, inv: &Inventory, only: &[String], opts: &BuildOptions) -> Result<BuildReport> {
    let (mut store, store_note) = Store::open_write(ws)?;
    let (embedder, embedder_problem): (Option<Box<dyn Embedder>>, Option<String>) = if opts.no_vector {
        (None, None)
    } else {
        match embed::load_config(ws).and_then(|c| embed::make(&c, opts.authorize_remote, &ws.root)) {
            Ok(e) => (Some(e), None),
            Err(e) => (None, Some(e.message)),
        }
    };
    if let Some(e) = &embedder {
        store.use_embedder(&embedder_row(e.info()))?;
    }
    let mut rows = store.doc_rows()?;
    if only.is_empty() {
        // documents that left the inventory leave the index
        for id in rows.keys().filter(|id| inv.get(id).is_none()).cloned().collect::<Vec<_>>() {
            store.remove_document(&id)?;
            rows.remove(&id);
        }
    }
    let mut documents = Vec::new();
    let mut vector_failure: Option<String> = None;
    for d in inv.documents.iter().filter(|d| only.is_empty() || only.contains(&d.document_id)) {
        let id = &d.document_id;
        let not_requested = VectorOutcome { state: "not_requested", vectors: 0, reason: None };
        let Some(meta) = read_meta(ws, id) else {
            documents.push(DocBuild { document_id: id.clone(), keyword: "error", chunks: 0, error: Some("not cached (run `hcp-manager sources sync`)".into()), vectors: not_requested });
            continue;
        };
        let sig = d.index_signature();
        let current = !opts.force && rows.get(id).is_some_and(|r| r.sha256 == meta.sha256 && r.chunker == CHUNKER && r.inv_sig == sig && r.chunks > 0);
        let (keyword, chunks) = if current {
            ("kept", rows[id].chunks)
        } else {
            let file = cached_file(ws, &meta);
            let ext = file.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
            match extract_pages(&file, &ext) {
                Err(e) => {
                    store.remove_document(id)?;
                    documents.push(DocBuild { document_id: id.clone(), keyword: "error", chunks: 0, error: Some(e), vectors: not_requested });
                    continue;
                }
                Ok(pages) => {
                    let records: Vec<ChunkRecord> = chunk_pages(id, &meta.sha256, &pages).iter().map(|c| record(d, c)).collect();
                    store.replace_document(&DocRow { document_id: id.clone(), sha256: meta.sha256.clone(), chunker: CHUNKER.into(), inv_sig: sig, chunks: records.len() }, &records)?;
                    ("built", records.len())
                }
            }
        };
        let vectors = match &embedder {
            None => VectorOutcome { state: if opts.no_vector { "not_requested" } else { "skipped" }, vectors: 0, reason: embedder_problem.clone() },
            Some(e) => {
                let info = e.info();
                if let Some(why) = &vector_failure {
                    VectorOutcome { state: "skipped", vectors: 0, reason: Some(why.clone()) }
                } else if info.remote && d.embed_remote != Some(true) {
                    VectorOutcome { state: "skipped", vectors: 0, reason: Some("not authorised for remote embedding: the inventory entry has no `embed_remote: true`".into()) }
                } else {
                    let stored = store.chunks_of(id)?;
                    let fresh = !opts.force && store.vec_row(id, &info.key)?.is_some_and(|v| v.sha256 == meta.sha256 && v.chunker == CHUNKER && v.vectors == stored.len());
                    if fresh {
                        VectorOutcome { state: "kept", vectors: stored.len(), reason: None }
                    } else {
                        let mut out: Vec<(String, Vec<f32>)> = Vec::new();
                        let mut failed = None;
                        for batch in stored.chunks(e.batch().max(1)) {
                            match e.embed(&batch.iter().map(|c| c.text.clone()).collect::<Vec<_>>()) {
                                Ok(v) => out.extend(batch.iter().map(|c| c.chunk_id.clone()).zip(v)),
                                Err(err) => {
                                    failed = Some(err.message);
                                    break;
                                }
                            }
                        }
                        match failed {
                            Some(why) => {
                                vector_failure = Some(why.clone());
                                VectorOutcome { state: "skipped", vectors: 0, reason: Some(why) }
                            }
                            None => {
                                store.replace_doc_vectors(&VecRow { document_id: id.clone(), embedder_key: info.key.clone(), sha256: meta.sha256.clone(), chunker: CHUNKER.into(), vectors: out.len() }, &out)?;
                                VectorOutcome { state: "built", vectors: out.len(), reason: None }
                            }
                        }
                    }
                }
            }
        };
        documents.push(DocBuild { document_id: id.clone(), keyword, chunks, error: None, vectors });
    }
    Ok(BuildReport { format: "hcp-manager-index-build", format_version: "1.1", documents, store_note, embedder: embedder.as_ref().map(|e| e.info().clone()), embedder_problem })
}

/// Deletes everything derived and builds it again from the cache.
pub fn rebuild(ws: &Workspace, inv: &Inventory, opts: &BuildOptions) -> Result<BuildReport> {
    let d = ws.derived_dir();
    if d.exists() {
        std::fs::remove_dir_all(&d).map_err(|e| Error::output(format!("{}: {e}", d.display())))?;
    }
    build(ws, inv, &[], &BuildOptions { force: true, ..opts.clone() })
}

// ---- is the index current? ------------------------------------------------------------------------------------------------

/// `indexed`, `stale` (and why), `not_indexed` or `not_cached`.
pub fn keyword_state(row: Option<&DocRow>, meta: Option<&Meta>, doc: &Doc) -> (&'static str, Option<String>) {
    match (meta, row) {
        (None, _) => ("not_cached", None),
        (Some(_), None) => ("not_indexed", None),
        (Some(m), Some(r)) => {
            if r.sha256 != m.sha256 {
                ("stale", Some(format!("the cached document changed (the blocks are of {}, the cache has {})", &r.sha256[..8.min(r.sha256.len())], &m.sha256[..8.min(m.sha256.len())])))
            } else if r.chunker != CHUNKER {
                ("stale", Some(format!("the chunking strategy changed ({} then, {CHUNKER} now)", r.chunker)))
            } else if r.inv_sig != doc.index_signature() {
                ("stale", Some("the inventory entry (version, source level or devices) changed since the blocks were made".into()))
            } else if r.chunks == 0 {
                ("not_indexed", None)
            } else {
                ("indexed", None)
            }
        }
    }
}

/// The blocks of a document if the index of it is current, else why not. Search and the evidence check use only blocks of a
/// current index: text of an old version of a document is never taken for the text of the cached one.
pub fn current_records(store: Option<&Store>, ws: &Workspace, doc: &Doc) -> std::result::Result<Vec<ChunkRecord>, String> {
    let Some(store) = store else {
        return Err("no text blocks (not indexed: `hcp-manager index build`)".into());
    };
    let meta = read_meta(ws, &doc.document_id);
    let row = store.doc_row(&doc.document_id).map_err(|e| e.message)?;
    match keyword_state(row.as_ref(), meta.as_ref(), doc) {
        ("indexed", _) => store.chunks_of(&doc.document_id).map_err(|e| e.message),
        ("stale", Some(why)) => Err(format!("stale index ({why}): run `hcp-manager index build`")),
        ("not_cached", _) => Err("not cached: run `hcp-manager sources sync`".into()),
        _ => Err("no text blocks (not indexed: `hcp-manager index build`)".into()),
    }
}

/// The blocks for the evidence check.
pub fn chunks_for_evidence(ws: &Workspace, doc: &Doc) -> std::result::Result<Vec<Chunk>, String> {
    let store = Store::open_read(ws);
    current_records(store.as_ref(), ws, doc).map(|v| v.iter().map(ChunkRecord::as_chunk).collect())
}

// ---- status ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct VectorDocStatus {
    /// `current`, `missing`, `stale` (and why), `skipped` (not authorised) or `unavailable` (no usable provider)
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub vectors: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocStatus {
    pub document_id: String,
    /// `indexed`, `stale` (the cached file, the chunking or the inventory entry changed), `not_indexed`, `not_cached`
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_reason: Option<String>,
    pub chunks: usize,
    pub cached_sha256: Option<String>,
    pub vector: VectorDocStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoreStatus {
    pub path: String,
    /// `ready`, `missing`, `unreadable` or `other_version`
    pub state: &'static str,
    pub layout_version: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EmbedderStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configured: Option<EmbedderInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// What the vectors in the store were made with
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub format: &'static str,
    pub format_version: &'static str,
    pub keyword_index: &'static str,
    /// `ready`, `partial`, `empty`, `stale`, or `unavailable` (no usable embedding provider)
    pub vector_index: &'static str,
    pub chunker: &'static str,
    pub store: StoreStatus,
    pub embedder: EmbedderStatus,
    pub documents: Vec<DocStatus>,
}

pub fn status(ws: &Workspace, inv: &Inventory) -> Status {
    let probe = Store::probe(ws);
    let store = Store::open_read(ws);
    let rows = store.as_ref().and_then(|s| s.doc_rows().ok()).unwrap_or_default();
    let (config_info, problem): (Option<EmbedderInfo>, Option<String>) = match embed::load_config(ws).and_then(|c| embed::describe(&c)) {
        Ok(i) => (Some(i), None),
        Err(e) => (None, Some(e.message)),
    };
    let stored = store.as_ref().and_then(|s| s.embedders().ok()).and_then(|e| e.first().map(|r| r.embedder_key.clone()));
    let mut documents: Vec<DocStatus> = Vec::new();
    for d in &inv.documents {
        let meta = read_meta(ws, &d.document_id);
        let row = rows.get(&d.document_id);
        let (state, stale_reason) = keyword_state(row, meta.as_ref(), d);
        let chunks = if state == "indexed" || state == "stale" { row.map(|r| r.chunks).unwrap_or(0) } else { 0 };
        let vector = match (&config_info, state) {
            (None, _) => VectorDocStatus { state: "unavailable", reason: problem.clone(), vectors: 0 },
            (_, "not_cached" | "not_indexed") => VectorDocStatus { state: "missing", reason: None, vectors: 0 },
            (_, "stale") => VectorDocStatus { state: "stale", reason: Some("the text blocks are stale".into()), vectors: 0 },
            (Some(info), _) => match store.as_ref().and_then(|s| s.vec_row(&d.document_id, &info.key).ok().flatten()) {
                Some(v) if meta.as_ref().is_some_and(|m| v.sha256 == m.sha256) && v.chunker == CHUNKER && v.vectors == chunks => VectorDocStatus { state: "current", reason: None, vectors: v.vectors },
                Some(_) => VectorDocStatus { state: "stale", reason: Some("the vectors were made from other text blocks".into()), vectors: 0 },
                None if info.remote && d.embed_remote != Some(true) => VectorDocStatus { state: "skipped", reason: Some("not authorised for remote embedding (embed_remote: true in sources.yaml)".into()), vectors: 0 },
                None => VectorDocStatus {
                    state: if stored.as_deref().is_some_and(|k| k != info.key) { "stale" } else { "missing" },
                    reason: stored.as_deref().filter(|k| *k != info.key).map(|k| format!("the vectors were made with another embedding configuration ({k}); now {}", info.key)),
                    vectors: 0,
                },
            },
        };
        documents.push(DocStatus { document_id: d.document_id.clone(), state, stale_reason, chunks, cached_sha256: meta.map(|m| m.sha256), vector });
    }
    let usable: Vec<&DocStatus> = documents.iter().filter(|d| d.state == "indexed").collect();
    let all_ok = !documents.is_empty() && documents.iter().all(|d| d.state == "indexed");
    let keyword_index = if all_ok { "ready" } else if !usable.is_empty() { "partial" } else { "empty" };
    let current = documents.iter().filter(|d| d.vector.state == "current").count();
    let vector_index = if config_info.is_none() {
        "unavailable"
    } else if !documents.is_empty() && current == documents.len() {
        "ready"
    } else if current > 0 {
        "partial"
    } else if documents.iter().any(|d| d.vector.state == "stale") {
        "stale"
    } else {
        "empty"
    };
    let state = match &probe {
        Probe::Ready => "ready",
        Probe::Missing => "missing",
        Probe::Unreadable(_) => "unreadable",
        Probe::OtherVersion(_) => "other_version",
    };
    Status {
        format: "hcp-manager-index-status",
        format_version: "1.1",
        keyword_index,
        vector_index,
        chunker: CHUNKER,
        store: StoreStatus { path: ws.index_db_path().display().to_string(), state, layout_version: crate::store::STORE_VERSION },
        embedder: EmbedderStatus { configured: config_info, problem, stored },
        documents,
    }
}

/// document id -> chunk ids, for tests and tools that compare two builds.
pub fn chunk_ids(ws: &Workspace) -> BTreeMap<String, Vec<String>> {
    let mut m = BTreeMap::new();
    if let Some(s) = Store::open_read(ws) {
        for id in s.doc_rows().map(|r| r.into_keys().collect::<Vec<_>>()).unwrap_or_default() {
            m.insert(id.clone(), s.chunks_of(&id).map(|v| v.into_iter().map(|c| c.chunk_id).collect()).unwrap_or_default());
        }
    }
    m
}
