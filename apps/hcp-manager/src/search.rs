//! Search over the blocks of the index: keyword, vector, or both, **filtered before ranked**: only blocks of documents that are for
//! the asked device (exact part number, and revision / version if given) and source level are considered, so that a document of a
//! similar model cannot be found by accident, whatever its text looks like. Results are ordered by source priority first
//! (official manual before guide before board), then by score. The answer carries everything an evidence entry needs (document,
//! version, page, section, source level, devices, block id and the digest of the text), and goes back to the original with
//! `hcp-manager chunk show`.
//!
//! A hit is a lead. The score of a keyword search, the similarity of a vector and the fused score of a hybrid one say how well a
//! block matched the words of the query. They are not the strength of any evidence and no check reads them: a pack cites a page and
//! the quoted words, and `evidence verify` looks at those.

use crate::embed;
use crate::error::{Error, Result};
use crate::index::current_records;
use crate::inventory::{is_official, priority, Inventory};
use crate::store::{ChunkRecord, Store, VectorIndex};
use crate::workspace::Workspace;
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};

/// Below this cosine similarity a block is not a vector hit (a floor for noise, not a measure of relevance).
pub const MIN_VECTOR_SIMILARITY: f32 = 0.15;

pub const HIT_SEMANTICS: &str = "discovery only: a score or similarity says how well a block matched the query; it is not the strength of any evidence and no check or verdict reads it. Read the page, then cite it.";

#[derive(Debug, Clone, Default)]
pub struct Query {
    pub text: String,
    pub device: Option<String>,
    pub revision: Option<String>,
    pub version: Option<String>,
    /// an exact level, or `official`
    pub source_level: Option<String>,
    pub limit: usize,
    /// `keyword`, `vector` (an error when there are no usable vectors) or `hybrid` (keyword and vector; keyword alone, and says so,
    /// when there are no usable vectors)
    pub mode: String,
    /// The caller authorises that a remote embedding provider receives the text of the query
    pub authorize_remote: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub chunk_id: String,
    pub document_id: String,
    pub document_version: String,
    pub level: String,
    pub official: bool,
    pub applies_to: Vec<String>,
    pub page: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// The ranking score of the mode used: the keyword score, the cosine similarity, or for a hybrid search the keyword score
    /// relative to the best keyword hit plus `vector_weight` times the similarity above the noise floor
    pub score: f64,
    /// SHA-256 of the text of the block
    pub content_sha256: String,
    /// `keyword` and / or `vector`
    pub matched_by: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyword_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_similarity: Option<f64>,
    pub snippet: String,
    /// ready to paste into an HCP evidence entry (after reading the text!)
    pub locator: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub format: &'static str,
    pub format_version: &'static str,
    pub query: String,
    pub mode_requested: String,
    pub mode_used: &'static str,
    /// What the scores are (and are not).
    pub hit_semantics: &'static str,
    /// Why the mode used is not the one asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degraded: Option<String>,
    pub documents_considered: Vec<String>,
    pub documents_excluded: Vec<Excluded>,
    /// The vector side of the search, when there was one
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector: Option<VectorInfo>,
    pub hits: Vec<Hit>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VectorInfo {
    pub embedder: String,
    pub remote: bool,
    pub documents_with_vectors: Vec<String>,
    /// Considered documents that have no current vectors: they were searched by keyword only
    pub documents_without_vectors: Vec<Excluded>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Excluded {
    pub document_id: String,
    pub reason: String,
}

fn find_sub(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == *needle)
}

/// About 240 characters of the block, starting a little before the first query word.
fn snippet(text: &str, terms: &[String]) -> String {
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = chars.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    let first = terms.iter().filter_map(|t| find_sub(&lower, &t.chars().collect::<Vec<_>>())).min().unwrap_or(0);
    chars[first.saturating_sub(80)..].iter().take(240).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() && c.is_ascii() {
            word.push(c.to_ascii_lowercase());
        } else {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            if c.is_alphabetic() {
                out.push(c.to_string()); // (one token per CJK character)
            }
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// Keyword scores of the blocks of the pool (those that share at least one word with the query).
fn keyword_scores(pool: &[(ChunkRecord, &crate::inventory::Doc)], q: &str) -> (Vec<(usize, f64)>, Vec<String>) {
    let terms: Vec<String> = {
        let mut t = tokens(q);
        t.sort();
        t.dedup();
        t
    };
    // document frequency over the filtered pool only
    let mut df: BTreeMap<&str, usize> = BTreeMap::new();
    let doc_tokens: Vec<BTreeMap<String, usize>> = pool
        .iter()
        .map(|(c, _)| {
            let mut tf = BTreeMap::new();
            for t in tokens(&c.text) {
                *tf.entry(t).or_insert(0) += 1;
            }
            tf
        })
        .collect();
    for t in &terms {
        df.insert(t.as_str(), doc_tokens.iter().filter(|tf| tf.contains_key(t)).count());
    }
    let n = pool.len().max(1) as f64;
    let phrase = q.trim().to_lowercase();
    let mut out = Vec::new();
    for (i, ((c, _), tf)) in pool.iter().zip(&doc_tokens).enumerate() {
        let mut score = 0.0;
        let mut matched = 0;
        for t in &terms {
            if let Some(f) = tf.get(t) {
                matched += 1;
                let idf = (1.0 + n / (1.0 + df[t.as_str()] as f64)).ln();
                score += (1.0 + (*f as f64).ln()) * idf;
            }
        }
        if matched == 0 {
            continue;
        }
        score *= matched as f64 / terms.len().max(1) as f64; // blocks with all the words first
        if c.text.to_lowercase().contains(&phrase) {
            score *= 1.5;
        }
        out.push((i, score));
    }
    (out, terms)
}

pub fn search(ws: &Workspace, inv: &Inventory, q: &Query) -> Result<SearchResult> {
    if q.text.trim().is_empty() {
        return Err(Error::usage("the query is empty"));
    }
    if !["keyword", "vector", "hybrid"].contains(&q.mode.as_str()) {
        return Err(Error::usage(format!("unknown mode '{}' (keyword, vector or hybrid)", q.mode)));
    }
    let mut warnings = Vec::new();
    if q.device.is_none() {
        warnings.push("no --device: the results may mix documents of different models; give the exact part number".to_string());
    }
    let store = Store::open_read(ws);
    let mut considered = Vec::new();
    let mut excluded = Vec::new();
    let mut pool: Vec<(ChunkRecord, &crate::inventory::Doc)> = Vec::new();
    for d in &inv.documents {
        // filter first: device, version, level
        if let Some(dev) = &q.device {
            if !d.applies(dev, q.revision.as_deref()) {
                excluded.push(Excluded { document_id: d.document_id.clone(), reason: format!("not for {dev}{}", q.revision.as_ref().map(|r| format!(" revision {r}")).unwrap_or_default()) });
                continue;
            }
        }
        if let Some(v) = &q.version {
            if &d.version != v {
                excluded.push(Excluded { document_id: d.document_id.clone(), reason: format!("version {} (asked for {v})", d.version) });
                continue;
            }
        }
        if let Some(l) = &q.source_level {
            let ok = if l == "official" { is_official(&d.level) } else { &d.level == l };
            if !ok {
                excluded.push(Excluded { document_id: d.document_id.clone(), reason: format!("level {} (asked for {l})", d.level) });
                continue;
            }
        }
        if matches!(d.status.as_deref(), Some("superseded") | Some("changed")) {
            excluded.push(Excluded { document_id: d.document_id.clone(), reason: format!("status {}", d.status.clone().unwrap_or_default()) });
            continue;
        }
        match current_records(store.as_ref(), ws, d) {
            Err(why) => {
                excluded.push(Excluded { document_id: d.document_id.clone(), reason: why });
                continue;
            }
            Ok(chunks) if chunks.is_empty() => {
                excluded.push(Excluded { document_id: d.document_id.clone(), reason: "no text blocks (not cached or not indexed: `hcp-manager index build`)".into() });
                continue;
            }
            Ok(chunks) => {
                considered.push(d.document_id.clone());
                pool.extend(chunks.into_iter().map(|c| (c, d)));
            }
        }
    }

    // the vector side: only over the blocks that survived the filters, and only when there are current vectors
    let mut vector_note: Option<String> = None; // why there is no vector ranking
    let mut vector_info: Option<VectorInfo> = None;
    let mut vector_hits: Vec<(usize, f32)> = Vec::new();
    let mut vector_weight = 1.0f64;
    if q.mode != "keyword" {
        let prepared = (|| -> std::result::Result<(), String> {
            let cfg = embed::load_config(ws).map_err(|e| e.message)?;
            let info = embed::describe(&cfg).map_err(|e| e.message)?;
            // authorisation first: a remote provider is refused before anything else is looked at
            let embedder = embed::make(&cfg, q.authorize_remote, &ws.root).map_err(|e| e.message)?;
            vector_weight = cfg.effective_vector_weight();
            let st = store.as_ref().ok_or_else(|| "there is no index store: `hcp-manager index build`".to_string())?;
            let mut with = Vec::new();
            let mut without = Vec::new();
            for id in &considered {
                let d = inv.get(id).expect("considered");
                let meta = crate::index::read_meta(ws, id);
                match st.vec_row(id, &info.key).map_err(|e| e.message)? {
                    Some(v) if meta.as_ref().is_some_and(|m| v.sha256 == m.sha256) && v.chunker == crate::store::CHUNKER => with.push(id.clone()),
                    Some(_) => without.push(Excluded { document_id: id.clone(), reason: "the vectors were made from other text blocks: `hcp-manager index build`".into() }),
                    None => without.push(Excluded {
                        document_id: id.clone(),
                        reason: if info.remote && d.embed_remote != Some(true) { "not authorised for remote embedding".into() } else { format!("no vectors for the embedding configuration {}: `hcp-manager index build`", info.key) },
                    }),
                }
            }
            if with.is_empty() {
                return Err(format!("no document has current vectors for the embedding configuration {} (`hcp-manager index status`)", info.key));
            }
            let qv = embedder.embed(std::slice::from_ref(&q.text)).map_err(|e| e.message)?.pop().ok_or("the embedder returned nothing")?;
            let candidates: HashSet<String> = pool.iter().filter(|(c, _)| with.contains(&c.document_id)).map(|(c, _)| c.chunk_id.clone()).collect();
            let near = st.nearest(&info.key, &qv, &candidates, (q.limit.max(10)) * 5).map_err(|e| e.message)?;
            let index_of: BTreeMap<&str, usize> = pool.iter().enumerate().map(|(i, (c, _))| (c.chunk_id.as_str(), i)).collect();
            vector_hits = near.into_iter().filter(|(_, s)| *s >= MIN_VECTOR_SIMILARITY).filter_map(|(id, s)| index_of.get(id.as_str()).map(|i| (*i, s))).collect();
            vector_info = Some(VectorInfo { embedder: info.key.clone(), remote: info.remote, documents_with_vectors: with, documents_without_vectors: without });
            Ok(())
        })();
        if let Err(why) = prepared {
            if q.mode == "vector" {
                return Err(Error::input(format!("vector search is not possible: {why}")));
            }
            vector_note = Some(format!("no vector ranking ({why}): keyword search only; nothing downstream depends on it"));
        }
        if let Some(v) = &vector_info {
            for x in &v.documents_without_vectors {
                warnings.push(format!("{} has no current vectors: it was searched by keyword only ({})", x.document_id, x.reason));
            }
        }
    }

    let (kw, terms) = if q.mode == "vector" && vector_note.is_none() { (Vec::new(), tokens(&q.text)) } else { keyword_scores(&pool, &q.text) };
    let kw_score: BTreeMap<usize, f64> = kw.iter().cloned().collect();
    let vec_sim: BTreeMap<usize, f32> = vector_hits.iter().cloned().collect();
    let both = q.mode == "hybrid" && vector_note.is_none();
    let kw_max = kw_score.values().cloned().fold(f64::MIN_POSITIVE, f64::max);
    let mode_used: &'static str = if q.mode == "vector" { "vector" } else if both { "hybrid" } else { "keyword" };
    let mut indices: Vec<usize> = kw_score.keys().chain(vec_sim.keys()).cloned().collect();
    indices.sort();
    indices.dedup();
    let mut hits: Vec<Hit> = Vec::new();
    for i in indices {
        let (c, d) = &pool[i];
        let (ks, vs) = (kw_score.get(&i).copied(), vec_sim.get(&i).copied());
        let score = match mode_used {
            "vector" => vs.unwrap_or(0.0) as f64,
            // keyword score relative to the best keyword hit, plus a weighted share for the similarity above the noise floor:
            // a block the words found stays ahead of one that only looks similar, and agreement between the two lifts a block
            "hybrid" => ks.map(|k| k / kw_max).unwrap_or(0.0) + vector_weight * vs.map(|v| ((v - MIN_VECTOR_SIMILARITY) / (1.0 - MIN_VECTOR_SIMILARITY)).max(0.0) as f64).unwrap_or(0.0),
            _ => ks.unwrap_or(0.0),
        };
        let mut matched_by = Vec::new();
        if ks.is_some() && mode_used != "vector" {
            matched_by.push("keyword");
        }
        if vs.is_some() && mode_used != "keyword" {
            matched_by.push("vector");
        }
        if matched_by.is_empty() {
            continue;
        }
        let round = |x: f64, k: f64| (x * k).round() / k;
        hits.push(Hit {
            chunk_id: c.chunk_id.clone(),
            document_id: d.document_id.clone(),
            document_version: d.version.clone(),
            level: d.level.clone(),
            official: is_official(&d.level),
            applies_to: c.applies_to.clone(),
            page: c.page,
            section: c.section.clone(),
            score: round(score, 1e4),
            content_sha256: c.text_sha256.clone(),
            matched_by,
            keyword_score: ks.filter(|_| mode_used != "vector").map(|x| round(x, 1e3)),
            vector_similarity: vs.filter(|_| mode_used != "keyword").map(|x| round(x as f64, 1e3)),
            snippet: snippet(&c.text, &terms),
            locator: match &c.section {
                Some(s) => serde_json::json!({"page": c.page, "section": s}),
                None => serde_json::json!({"page": c.page}),
            },
        });
    }
    hits.sort_by(|a, b| {
        (priority(&a.level), std::cmp::Reverse((a.score * 1e9) as i64), &a.document_id, a.page, &a.chunk_id).cmp(&(priority(&b.level), std::cmp::Reverse((b.score * 1e9) as i64), &b.document_id, b.page, &b.chunk_id))
    });
    hits.truncate(if q.limit == 0 { 10 } else { q.limit });
    Ok(SearchResult {
        format: "hcp-manager-search",
        format_version: "1.1",
        query: q.text.clone(),
        mode_requested: q.mode.clone(),
        mode_used,
        hit_semantics: HIT_SEMANTICS,
        degraded: if q.mode == "hybrid" { vector_note } else { None },
        documents_considered: considered,
        documents_excluded: excluded,
        vector: vector_info,
        hits,
        warnings,
    })
}
