//! Checking the evidence of a pack against the cached documents: the source exists, its version and
//! digest are the ones in the pack, the block id (if given) is not stale, and the quotation really is
//! on the cited page. This is the step that sends a reader "back to the original text".

use crate::error::{Error, Result};
use crate::index::{chunks_for_evidence, read_meta};
use crate::inventory::Inventory;
use crate::text::normalise;
use crate::workspace::Workspace;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct Problem {
    /// `unknown_document`, `version_mismatch`, `digest_mismatch`, `level_mismatch`, `chunk_stale`,
    /// `chunk_unknown`, `quote_not_found`, `page_not_found`
    pub code: &'static str,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Unverifiable {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvidenceReport {
    pub format: &'static str,
    pub format_version: &'static str,
    pub pack: String,
    pub evidence_checked: usize,
    pub verified: usize,
    pub problems: Vec<Problem>,
    /// Evidence that could not be checked because the text is not here (not cached, not indexed).
    pub unverifiable: Vec<Unverifiable>,
}

impl EvidenceReport {
    /// 0 all verified, 1 a problem, 5 nothing wrong but some evidence could not be checked.
    pub fn exit_code(&self) -> u8 {
        if !self.problems.is_empty() {
            1
        } else if !self.unverifiable.is_empty() {
            5
        } else {
            0
        }
    }
}

fn all_evidence<'a>(pack: &'a Value, path: String, out: &mut Vec<(String, &'a Value)>) {
    match pack {
        Value::Object(m) => {
            for (k, v) in m {
                if k == "evidence" {
                    if let Some(a) = v.as_array() {
                        for (i, e) in a.iter().enumerate() {
                            out.push((format!("{path}.evidence[{i}]"), e));
                        }
                    }
                } else {
                    all_evidence(v, format!("{path}.{k}"), out);
                }
            }
        }
        Value::Array(a) => {
            for (i, v) in a.iter().enumerate() {
                all_evidence(v, format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

pub fn verify(ws: &Workspace, inv: &Inventory, pack: &Value, pack_name: &str) -> Result<EvidenceReport> {
    let sources: Vec<&Value> = pack.get("sources").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    if sources.is_empty() {
        return Err(Error::input(format!("{pack_name}: not a pack (no sources)")));
    }
    let mut problems = Vec::new();
    let mut unverifiable = Vec::new();
    // the sources of the pack against the inventory and the cache
    let mut source_ok: std::collections::BTreeMap<String, Option<String>> = std::collections::BTreeMap::new(); // source_id -> document id when usable
    for (i, s) in sources.iter().enumerate() {
        let (sid, did) = (s["source_id"].as_str().unwrap_or(""), s["document_id"].as_str().unwrap_or(""));
        let path = format!("$.sources[{i}]");
        let Some(doc) = inv.get(did) else {
            problems.push(Problem { code: "unknown_document", path, message: format!("{sid}: the document {did} is not in this workspace's inventory") });
            source_ok.insert(sid.to_string(), None);
            continue;
        };
        let mut ok = true;
        if s["version"].as_str().is_some_and(|v| v != doc.version) {
            problems.push(Problem { code: "version_mismatch", path: format!("{path}.version"), message: format!("{sid}: the pack says version {}, the inventory has {}", s["version"].as_str().unwrap_or(""), doc.version) });
            ok = false;
        }
        if s["level"].as_str().is_some_and(|l| l != doc.level) {
            problems.push(Problem { code: "level_mismatch", path: format!("{path}.level"), message: format!("{sid}: the pack says {}, the inventory has {}", s["level"].as_str().unwrap_or(""), doc.level) });
            ok = false;
        }
        match read_meta(ws, did) {
            None => {
                unverifiable.push(Unverifiable { path: path.clone(), reason: format!("{did} is not cached: run `hcp-manager sources sync`") });
                ok = false;
            }
            Some(m) => {
                if s["sha256"].as_str().is_some_and(|d| d != m.sha256) {
                    problems.push(Problem { code: "digest_mismatch", path: format!("{path}.sha256"), message: format!("{sid}: the pack cites digest {}, the cached document has {}: the evidence is about another file", s["sha256"].as_str().unwrap_or(""), m.sha256) });
                    ok = false;
                }
            }
        }
        source_ok.insert(sid.to_string(), if ok { Some(did.to_string()) } else { None });
    }
    // the evidence entries against the text
    let mut list = Vec::new();
    all_evidence(pack, "$".into(), &mut list);
    let mut verified = 0;
    for (path, e) in &list {
        let sid = e["source_id"].as_str().unwrap_or("");
        let Some(Some(did)) = source_ok.get(sid) else {
            if !problems.iter().any(|p| p.message.starts_with(sid)) && !unverifiable.iter().any(|u| u.reason.contains(sid)) {
                unverifiable.push(Unverifiable { path: path.clone(), reason: format!("the source {sid} could not be verified (see above)") });
            } else if !source_ok.contains_key(sid) {
                problems.push(Problem { code: "unknown_document", path: path.clone(), message: format!("the evidence cites {sid}, which is not a source of the pack") });
            }
            continue;
        };
        // only the blocks of a current index: text of an older version of the document is not the text of the cached one
        let chunks = match inv.get(did).map(|d| chunks_for_evidence(ws, d)) {
            Some(Ok(c)) if !c.is_empty() => c,
            Some(Err(why)) => {
                unverifiable.push(Unverifiable { path: path.clone(), reason: format!("{did}: {why} (and `pdftotext` must be installed for PDFs)") });
                continue;
            }
            _ => {
                unverifiable.push(Unverifiable { path: path.clone(), reason: format!("{did} has no text blocks: run `hcp-manager index build` (and `pdftotext` must be installed for PDFs)") });
                continue;
            }
        };
        let mut ok = true;
        if let Some(cid) = e["chunk_id"].as_str() {
            match chunks.iter().find(|c| c.chunk_id == cid) {
                Some(_) => {}
                None => {
                    // an id of the right document with another digest is a stale one, anything else is unknown
                    let name = did.trim_start_matches("DOC-");
                    let stale = cid.starts_with(&format!("CHK-{name}@"));
                    problems.push(Problem {
                        code: if stale { "chunk_stale" } else { "chunk_unknown" },
                        path: format!("{path}.chunk_id"),
                        message: if stale { format!("{cid} was made from another version of {did}: the document changed since the evidence was written") } else { format!("{cid} is not a block of {did}") },
                    });
                    ok = false;
                }
            }
        }
        let quote = normalise(e["quote"].as_str().unwrap_or(""));
        let page = e["locator"]["page"].as_u64();
        let haystack: String = match page {
            Some(p) => {
                let on_page: Vec<&str> = chunks.iter().filter(|c| c.page as u64 == p).map(|c| c.text.as_str()).collect();
                if on_page.is_empty() {
                    problems.push(Problem { code: "page_not_found", path: format!("{path}.locator.page"), message: format!("{did} has no text on page {p}") });
                    ok = false;
                }
                normalise(&on_page.join("\n\n"))
            }
            None => normalise(&chunks.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join("\n\n")),
        };
        if !quote.is_empty() && !haystack.to_lowercase().contains(&quote.to_lowercase()) && !(page.is_some() && haystack.is_empty()) {
            problems.push(Problem {
                code: "quote_not_found",
                path: format!("{path}.quote"),
                message: format!("the quotation is not on {}{}: \"{}\"", page.map(|p| format!("page {p} of ")).unwrap_or_default(), did, e["quote"].as_str().unwrap_or("")),
            });
            ok = false;
        }
        if ok {
            verified += 1;
        }
    }
    Ok(EvidenceReport { format: "hcp-manager-evidence", format_version: "1.0", pack: pack_name.to_string(), evidence_checked: list.len(), verified, problems, unverifiable })
}
