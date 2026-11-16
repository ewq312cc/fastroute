//! Text of a cached document and the blocks it is cut into. A block has a stable id that contains
//! the first 8 hex digits of the document's SHA-256 (`CHK-<DOC>@<sha8>:p<page>.<n>`): rebuilding the
//! blocks of the same bytes gives the same ids, a changed document gives new ones, so evidence that
//! points at an old block is recognised as stale.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chunk {
    pub chunk_id: String,
    pub document_id: String,
    pub page: u32,
    pub ordinal: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    pub text: String,
}

pub fn pdftotext_available() -> bool {
    Command::new("pdftotext").arg("-v").output().is_ok()
}

fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let (mut in_tag, mut skip) = (false, 0usize);
    let lower = html.to_ascii_lowercase();
    let bytes: Vec<char> = html.chars().collect();
    let lower_chars: Vec<char> = lower.chars().collect();
    let starts = |i: usize, pat: &str| lower_chars[i..].iter().take(pat.len()).collect::<String>() == pat;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if !in_tag && c == '<' {
            if starts(i, "<script") || starts(i, "<style") {
                skip += 1;
            }
            if starts(i, "</script") || starts(i, "</style") {
                skip = skip.saturating_sub(1);
            }
            in_tag = true;
            if starts(i, "<p") || starts(i, "<br") || starts(i, "<h") || starts(i, "<li") || starts(i, "<tr") || starts(i, "<div") || starts(i, "</") {
                out.push('\n');
            }
        } else if in_tag && c == '>' {
            in_tag = false;
        } else if !in_tag && skip == 0 {
            out.push(c);
        }
        i += 1;
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ").replace("&quot;", "\"").replace("&#39;", "'")
}

/// The pages of a document as text. PDFs need `pdftotext`; HTML and text are read directly. A
/// document without pages (text, HTML) is one page.
pub fn extract_pages(file: &Path, ext: &str) -> Result<Vec<String>, String> {
    let pages: Vec<String> = match ext {
        "pdf" => {
            if !pdftotext_available() {
                return Err("`pdftotext` (poppler-utils) is not installed: the PDF cannot be read as text".into());
            }
            let out = Command::new("pdftotext").args(["-layout", "-enc", "UTF-8"]).arg(file).arg("-").output().map_err(|e| e.to_string())?;
            if !out.status.success() {
                return Err(format!("pdftotext failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
            }
            String::from_utf8_lossy(&out.stdout).split('\x0c').map(str::to_string).collect()
        }
        "html" | "htm" => vec![strip_html(&String::from_utf8_lossy(&std::fs::read(file).map_err(|e| e.to_string())?))],
        "bin" => return Err("the file type is unknown: give the url or local_path a file extension (pdf, html, txt, md)".into()),
        _ => String::from_utf8_lossy(&std::fs::read(file).map_err(|e| e.to_string())?).split('\x0c').map(str::to_string).collect(),
    };
    // pdftotext ends the last page with a form feed: drop a trailing empty page
    let mut pages = pages;
    while pages.len() > 1 && pages.last().is_some_and(|p| p.trim().is_empty()) {
        pages.pop();
    }
    Ok(pages)
}

fn heading(line: &str) -> Option<String> {
    let t = line.trim();
    if t.len() < 3 || t.chars().count() > 90 {
        return None;
    }
    let mut it = t.splitn(2, char::is_whitespace);
    let (num, rest) = (it.next()?, it.next()?.trim());
    let numbered = num.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())) && num.chars().next()?.is_ascii_digit();
    let starts_word = rest.chars().next().is_some_and(|c| c.is_alphabetic());
    if numbered && starts_word && !t.ends_with('.') && !t.ends_with(',') {
        Some(t.to_string())
    } else {
        None
    }
}

/// Cuts pages into blocks of about 400..1500 characters at paragraph boundaries.
pub fn chunk_pages(document_id: &str, sha256: &str, pages: &[String]) -> Vec<Chunk> {
    let name = document_id.trim_start_matches("DOC-");
    let sha8 = &sha256[..8.min(sha256.len())];
    let mut chunks = Vec::new();
    let mut section: Option<String> = None;
    for (pi, page) in pages.iter().enumerate() {
        let page_no = pi as u32 + 1;
        let mut ordinal = 0u32;
        let mut current = String::new();
        let mut current_section = section.clone();
        let flush = |text: &mut String, sec: &Option<String>, ordinal: &mut u32, out: &mut Vec<Chunk>| {
            let t = text.trim().to_string();
            if !t.is_empty() {
                *ordinal += 1;
                out.push(Chunk { chunk_id: format!("CHK-{name}@{sha8}:p{page_no}.{ordinal}"), document_id: document_id.to_string(), page: page_no, ordinal: *ordinal, section: sec.clone(), text: t });
            }
            text.clear();
        };
        for para in page.split("\n\n") {
            for line in para.lines() {
                if let Some(h) = heading(line) {
                    section = Some(h.clone());
                    if current.trim().len() > 300 {
                        flush(&mut current, &current_section.clone(), &mut ordinal, &mut chunks);
                    }
                    current_section = Some(h);
                }
            }
            if para.trim().is_empty() {
                continue;
            }
            if !current.is_empty() && current.len() + para.len() > 1500 {
                flush(&mut current, &current_section.clone(), &mut ordinal, &mut chunks);
                current_section = section.clone();
            }
            current.push_str(para.trim_end());
            current.push_str("\n\n");
            if current.len() >= 800 {
                flush(&mut current, &current_section.clone(), &mut ordinal, &mut chunks);
                current_section = section.clone();
            }
        }
        flush(&mut current, &current_section.clone(), &mut ordinal, &mut chunks);
    }
    chunks
}

/// Whitespace collapsed, soft hyphens and line-end hyphenation removed: how quotes are compared.
pub fn normalise(s: &str) -> String {
    let s = s.replace('\u{00ad}', "").replace("-\n", "").replace('\u{2019}', "'").replace('\u{201c}', "\"").replace('\u{201d}', "\"");
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
