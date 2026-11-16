//! `sources`: adding documents to the inventory, syncing them into the cache, checking the cache.

use crate::error::{Error, Result};
use crate::fetch;
use crate::index::{cached_file, read_meta, Meta};
use crate::inventory::{Doc, Inventory, LEVELS};
use crate::workspace::{today, write_atomic, Workspace};
use fr_constraint::hcp::sha256_hex;
use serde::Serialize;
use std::collections::BTreeMap;

pub fn add(ws: &Workspace, doc: Doc) -> Result<()> {
    if !doc.document_id.starts_with("DOC-") || doc.document_id.len() < 5 || !doc.document_id[4..].chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-' || c == '_') {
        return Err(Error::usage(format!("'{}' is not a document id (DOC-NAME, upper case letters, digits, - and _)", doc.document_id)));
    }
    if !LEVELS.contains(&doc.level.as_str()) {
        return Err(Error::usage(format!("'{}' is not a source level ({})", doc.level, LEVELS.join(", "))));
    }
    if doc.url.is_none() && doc.local_path.is_none() {
        return Err(Error::usage("a document needs --url or --file"));
    }
    if doc.applies_to.is_empty() {
        return Err(Error::usage("a document names the device model(s) it is for (--applies-to PART[:REVISION])"));
    }
    let mut inv = if ws.inventory_path().exists() { Inventory::load(ws)? } else { Inventory::empty() };
    if inv.get(&doc.document_id).is_some() {
        return Err(Error::usage(format!("{} is already in the inventory", doc.document_id)));
    }
    inv.documents.push(doc);
    ws.init()?;
    inv.save(ws)
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncResult {
    pub document_id: String,
    /// `new`, `unchanged`, `changed` (the document at the source differs from the pinned digest),
    /// `accepted` (a change was accepted), `unreachable`
    pub status: &'static str,
    pub sha256: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncReport {
    pub format: &'static str,
    pub format_version: &'static str,
    pub results: Vec<SyncResult>,
    pub warnings: Vec<String>,
}

/// Fetches the documents (all, or the named ones) into the cache and pins their digests. A digest
/// that differs from the pinned one is reported as `changed` and not adopted unless `accept`.
pub fn sync(ws: &Workspace, only: &[String], accept: bool, timeout_secs: u64) -> Result<SyncReport> {
    let mut inv = Inventory::load(ws)?;
    for id in only {
        if inv.get(id).is_none() {
            return Err(Error::usage(format!("{id} is not in the inventory")));
        }
    }
    let base = ws.root.clone();
    let mut results = Vec::new();
    for i in 0..inv.documents.len() {
        let doc = inv.documents[i].clone();
        if !only.is_empty() && !only.contains(&doc.document_id) {
            continue;
        }
        match fetch::fetch(&doc, &base, timeout_secs) {
            Err(e) => {
                let had_cache = read_meta(ws, &doc.document_id).is_some();
                inv.documents[i].last_error = Some(e.message.clone());
                if had_cache && doc.url.is_some() {
                    inv.documents[i].status = Some("dead".into());
                }
                results.push(SyncResult { document_id: doc.document_id, status: "unreachable", sha256: None, message: format!("{}{}", e.message, if had_cache { " (the cached copy is kept)" } else { "" }) });
            }
            Ok(f) => {
                let sha = sha256_hex(&f.bytes);
                let dir = ws.cache_dir(&doc.document_id);
                let file = format!("{sha}.{}", f.ext);
                let pinned = doc.sha256.clone();
                let adopt = |inv: &mut Inventory, status: &'static str, message: String| -> Result<SyncResult> {
                    write_atomic(&dir.join(&file), &f.bytes)?;
                    let meta = Meta { document_id: doc.document_id.clone(), sha256: sha.clone(), file: file.clone(), bytes: f.bytes.len() as u64, retrieved: today(), origin: f.origin.clone() };
                    write_atomic(&ws.meta_path(&doc.document_id), &serde_json::to_vec_pretty(&meta).expect("serialises"))?;
                    let d = &mut inv.documents[i];
                    d.sha256 = Some(sha.clone());
                    d.retrieved = Some(meta.retrieved.clone());
                    d.status = Some("active".into());
                    d.last_error = None;
                    Ok(SyncResult { document_id: doc.document_id.clone(), status, sha256: Some(sha.clone()), message })
                };
                match pinned {
                    Some(p) if p == sha => {
                        let cached = read_meta(ws, &doc.document_id).is_some_and(|m| cached_file(ws, &m).exists());
                        if cached {
                            inv.documents[i].status = Some("active".into());
                            inv.documents[i].last_error = None;
                            results.push(SyncResult { document_id: doc.document_id.clone(), status: "unchanged", sha256: Some(sha), message: "the digest is the pinned one".into() });
                        } else {
                            results.push(adopt(&mut inv, "new", "cached again (the pinned digest matches)".into())?);
                        }
                    }
                    Some(p) if !accept => {
                        // keep the candidate for review, do not adopt it
                        write_atomic(&dir.join(&file), &f.bytes)?;
                        inv.documents[i].status = Some("changed".into());
                        inv.documents[i].last_error = Some(format!("digest changed from {p} to {sha}"));
                        results.push(SyncResult {
                            document_id: doc.document_id.clone(),
                            status: "changed",
                            sha256: Some(sha.clone()),
                            message: format!("the document at the source is not the pinned one ({}...): review it, then `sources sync {} --accept`; evidence quoted from the old text may be stale", &p[..12.min(p.len())], doc.document_id),
                        });
                    }
                    Some(_) => results.push(adopt(&mut inv, "accepted", "the changed document was accepted; rebuild the index and verify the evidence".into())?),
                    None => results.push(adopt(&mut inv, "new", format!("cached from {}", f.origin))?),
                }
            }
        }
    }
    inv.save(ws)?;
    // the same bytes under two ids is worth saying
    let mut by_sha: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for d in &inv.documents {
        if let Some(s) = d.sha256.as_deref() {
            by_sha.entry(s).or_default().push(d.document_id.as_str());
        }
    }
    let warnings = by_sha.values().filter(|v| v.len() > 1).map(|v| format!("duplicate content: {} are the same file", v.join(" and "))).collect();
    Ok(SyncReport { format: "hcp-manager-sync", format_version: "1.0", results, warnings })
}

#[derive(Debug, Clone, Serialize)]
pub struct CacheProblem {
    pub document_id: String,
    pub problem: String,
}

/// Verifies that every cached file is what the metadata and the inventory say (corruption, edits).
pub fn check(ws: &Workspace) -> Result<Vec<CacheProblem>> {
    let inv = Inventory::load(ws)?;
    let mut out = Vec::new();
    for d in &inv.documents {
        let Some(meta) = read_meta(ws, &d.document_id) else {
            if d.sha256.is_some() {
                out.push(CacheProblem { document_id: d.document_id.clone(), problem: "pinned in the inventory but not cached: run `hcp-manager sources sync`".into() });
            }
            continue;
        };
        let file = cached_file(ws, &meta);
        match std::fs::read(&file) {
            Err(_) => out.push(CacheProblem { document_id: d.document_id.clone(), problem: format!("{} is missing", file.display()) }),
            Ok(b) => {
                let sha = sha256_hex(&b);
                if sha != meta.sha256 {
                    out.push(CacheProblem { document_id: d.document_id.clone(), problem: format!("the cached file has digest {sha}, its metadata says {}", meta.sha256) });
                }
                if d.sha256.as_deref().is_some_and(|p| p != sha) {
                    out.push(CacheProblem { document_id: d.document_id.clone(), problem: "the cached file is not the one pinned in the inventory".into() });
                }
            }
        }
    }
    Ok(out)
}
