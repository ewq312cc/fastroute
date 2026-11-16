//! The derived index: text blocks of every cached document. It is rebuilt from the cache, can be
//! deleted at any time, and says when it is stale. There is no vector index in this version; the
//! status says so.

use crate::error::{Error, Result};
use crate::inventory::Inventory;
use crate::text::{chunk_pages, extract_pages, Chunk};
use crate::workspace::{write_atomic, Workspace};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const BUILT_WITH: &str = "keyword-blocks-v1";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Meta {
    pub document_id: String,
    pub sha256: String,
    pub file: String,
    pub bytes: u64,
    pub retrieved: String,
    pub origin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexState {
    pub built_with: String,
    /// document id -> digest it was built from, and the number of blocks
    pub documents: BTreeMap<String, IndexedDoc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedDoc {
    pub sha256: String,
    pub chunks: usize,
}

pub fn read_meta(ws: &Workspace, doc: &str) -> Option<Meta> {
    serde_json::from_slice(&std::fs::read(ws.meta_path(doc)).ok()?).ok()
}

fn read_state(ws: &Workspace) -> IndexState {
    std::fs::read(ws.index_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn cached_file(ws: &Workspace, meta: &Meta) -> std::path::PathBuf {
    ws.cache_dir(&meta.document_id).join(&meta.file)
}

/// Builds the blocks of the given documents (all cached ones if none are named). Returns, per
/// document, the number of blocks or why there are none.
pub fn build(ws: &Workspace, inv: &Inventory, only: &[String]) -> Result<Vec<(String, std::result::Result<usize, String>)>> {
    let mut state = read_state(ws);
    state.built_with = BUILT_WITH.into();
    let mut results = Vec::new();
    for d in inv.documents.iter().filter(|d| only.is_empty() || only.contains(&d.document_id)) {
        let Some(meta) = read_meta(ws, &d.document_id) else {
            results.push((d.document_id.clone(), Err("not cached (run `hcp-manager sources sync`)".to_string())));
            continue;
        };
        let file = cached_file(ws, &meta);
        let ext = file.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        match extract_pages(&file, &ext) {
            Err(e) => {
                state.documents.remove(&d.document_id);
                let _ = std::fs::remove_file(ws.chunks_path(&d.document_id));
                results.push((d.document_id.clone(), Err(e)));
            }
            Ok(pages) => {
                let chunks = chunk_pages(&d.document_id, &meta.sha256, &pages);
                let mut text = String::new();
                for c in &chunks {
                    text.push_str(&serde_json::to_string(c).expect("serialises"));
                    text.push('\n');
                }
                write_atomic(&ws.chunks_path(&d.document_id), text.as_bytes())?;
                state.documents.insert(d.document_id.clone(), IndexedDoc { sha256: meta.sha256.clone(), chunks: chunks.len() });
                results.push((d.document_id.clone(), Ok(chunks.len())));
            }
        }
    }
    let bytes = serde_json::to_vec_pretty(&state).map_err(|e| Error::failed(e.to_string()))?;
    write_atomic(&ws.index_path(), &bytes)?;
    Ok(results)
}

/// Deletes everything derived and builds it again from the cache.
pub fn rebuild(ws: &Workspace, inv: &Inventory) -> Result<Vec<(String, std::result::Result<usize, String>)>> {
    let d = ws.derived_dir();
    if d.exists() {
        std::fs::remove_dir_all(&d).map_err(|e| Error::output(format!("{}: {e}", d.display())))?;
    }
    build(ws, inv, &[])
}

#[derive(Debug, Clone, Serialize)]
pub struct DocStatus {
    pub document_id: String,
    /// `indexed`, `stale` (the cached file changed), `not_indexed`, `not_cached`
    pub state: &'static str,
    pub chunks: usize,
    pub cached_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub format: &'static str,
    pub format_version: &'static str,
    pub keyword_index: &'static str,
    pub vector_index: &'static str,
    pub documents: Vec<DocStatus>,
}

pub fn status(ws: &Workspace, inv: &Inventory) -> Status {
    let state = read_state(ws);
    let documents: Vec<DocStatus> = inv
        .documents
        .iter()
        .map(|d| {
            let meta = read_meta(ws, &d.document_id);
            let indexed = state.documents.get(&d.document_id);
            let st = match (&meta, indexed) {
                (None, _) => "not_cached",
                (Some(_), None) => "not_indexed",
                (Some(m), Some(i)) if m.sha256 == i.sha256 && ws.chunks_path(&d.document_id).exists() => "indexed",
                (Some(_), Some(_)) => "stale",
            };
            DocStatus { document_id: d.document_id.clone(), state: st, chunks: indexed.map(|i| i.chunks).unwrap_or(0), cached_sha256: meta.map(|m| m.sha256) }
        })
        .collect();
    let all_ok = !documents.is_empty() && documents.iter().all(|d| d.state == "indexed");
    Status {
        format: "hcp-manager-index-status",
        format_version: "1.0",
        keyword_index: if all_ok { "ready" } else if documents.iter().any(|d| d.state == "indexed") { "partial" } else { "empty" },
        // the vector index is another task (#19); everything here works without it
        vector_index: "not available",
        documents,
    }
}

pub fn read_chunks(ws: &Workspace, doc: &str) -> Vec<Chunk> {
    std::fs::read_to_string(ws.chunks_path(doc)).map(|t| t.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()).unwrap_or_default()
}
