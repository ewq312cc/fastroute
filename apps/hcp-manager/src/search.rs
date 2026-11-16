//! Keyword search over the blocks, **filtered before ranked**: only blocks of documents that are for
//! the asked device (exact part number, and revision / version if given) and source level are
//! considered, so that a document of a similar model cannot be found by accident. Results are
//! ordered by source priority first (official manual before guide before board), then by score.
//! The answer carries everything an evidence entry needs (document, version, page, section, block id).

use crate::error::{Error, Result};
use crate::index::read_chunks;
use crate::inventory::{is_official, priority, Inventory};
use crate::workspace::Workspace;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default)]
pub struct Query {
    pub text: String,
    pub device: Option<String>,
    pub revision: Option<String>,
    pub version: Option<String>,
    /// an exact level, or `official`
    pub source_level: Option<String>,
    pub limit: usize,
    /// `keyword` (the only one that exists) or `hybrid` (falls back, and says so)
    pub mode: String,
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
    pub score: f64,
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
    /// Why the mode used is not the one asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degraded: Option<String>,
    pub documents_considered: Vec<String>,
    pub documents_excluded: Vec<Excluded>,
    pub hits: Vec<Hit>,
    pub warnings: Vec<String>,
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

fn tokens(s: &str) -> Vec<String> {
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

pub fn search(ws: &Workspace, inv: &Inventory, q: &Query) -> Result<SearchResult> {
    if q.text.trim().is_empty() {
        return Err(Error::usage("the query is empty"));
    }
    if !["keyword", "hybrid"].contains(&q.mode.as_str()) {
        return Err(Error::usage(format!("unknown mode '{}' (keyword or hybrid)", q.mode)));
    }
    let mut warnings = Vec::new();
    if q.device.is_none() {
        warnings.push("no --device: the results may mix documents of different models; give the exact part number".to_string());
    }
    let mut considered = Vec::new();
    let mut excluded = Vec::new();
    let mut pool: Vec<(crate::text::Chunk, &crate::inventory::Doc)> = Vec::new();
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
        let chunks = read_chunks(ws, &d.document_id);
        if chunks.is_empty() {
            excluded.push(Excluded { document_id: d.document_id.clone(), reason: "no text blocks (not cached or not indexed: `hcp-manager index build`)".into() });
            continue;
        }
        considered.push(d.document_id.clone());
        pool.extend(chunks.into_iter().map(|c| (c, d)));
    }
    let terms: Vec<String> = {
        let mut t = tokens(&q.text);
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
    let phrase = q.text.trim().to_lowercase();
    let mut hits: Vec<Hit> = Vec::new();
    for ((c, d), tf) in pool.iter().zip(&doc_tokens) {
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
        let snippet = snippet(&c.text, &terms);
        hits.push(Hit {
            chunk_id: c.chunk_id.clone(),
            document_id: d.document_id.clone(),
            document_version: d.version.clone(),
            level: d.level.clone(),
            official: is_official(&d.level),
            applies_to: d.applies_to.iter().map(|a| match &a.revision { Some(r) => format!("{} {r}", a.part_number), None => a.part_number.clone() }).collect(),
            page: c.page,
            section: c.section.clone(),
            score: (score * 1000.0).round() / 1000.0,
            snippet,
            locator: match &c.section {
                Some(s) => serde_json::json!({"page": c.page, "section": s}),
                None => serde_json::json!({"page": c.page}),
            },
        });
    }
    hits.sort_by(|a, b| {
        (priority(&a.level), std::cmp::Reverse((a.score * 1e6) as i64), &a.document_id, a.page, &a.chunk_id).cmp(&(priority(&b.level), std::cmp::Reverse((b.score * 1e6) as i64), &b.document_id, b.page, &b.chunk_id))
    });
    hits.truncate(if q.limit == 0 { 10 } else { q.limit });
    let degraded = (q.mode == "hybrid").then(|| "there is no vector index (hcp-manager has none in this version): keyword search only; nothing downstream depends on it".to_string());
    let used: BTreeSet<&str> = hits.iter().map(|h| h.document_id.as_str()).collect();
    let _ = used;
    Ok(SearchResult {
        format: "hcp-manager-search",
        format_version: "1.0",
        query: q.text.clone(),
        mode_requested: q.mode.clone(),
        mode_used: "keyword",
        degraded,
        documents_considered: considered,
        documents_excluded: excluded,
        hits,
        warnings,
    })
}
