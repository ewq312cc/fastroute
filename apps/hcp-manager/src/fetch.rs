//! Getting a document's bytes: from a local file, a `file://` URL, or (with `curl`, which honours
//! the proxy and certificate settings of the machine) from http(s). Nothing is fetched unless
//! `sources sync` is run, and a failure never touches the cache.

use crate::error::{Error, Result};
use crate::inventory::Doc;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Fetched {
    pub bytes: Vec<u8>,
    /// lower-case file extension without the dot (`pdf`, `html`, `txt`, `bin`)
    pub ext: String,
    /// where the bytes came from, for the record
    pub origin: String,
}

fn ext_of(name: &str) -> String {
    let name = name.split(['?', '#']).next().unwrap_or(name);
    Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).filter(|e| e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric())).unwrap_or_else(|| "bin".to_string())
}

pub fn curl_available() -> bool {
    Command::new("curl").arg("--version").output().is_ok_and(|o| o.status.success())
}

pub fn fetch(doc: &Doc, base: &Path, timeout_secs: u64) -> Result<Fetched> {
    if let Some(local) = &doc.local_path {
        let p = if Path::new(local).is_absolute() { PathBuf::from(local) } else { base.join(local) };
        let bytes = std::fs::read(&p).map_err(|e| Error::input(format!("{}: {e}", p.display())))?;
        return Ok(Fetched { bytes, ext: ext_of(local), origin: p.display().to_string() });
    }
    let url = doc.url.as_deref().ok_or_else(|| Error::input(format!("{}: neither a url nor a local_path", doc.document_id)))?;
    if let Some(path) = url.strip_prefix("file://") {
        let bytes = std::fs::read(path).map_err(|e| Error::input(format!("{path}: {e}")))?;
        return Ok(Fetched { bytes, ext: ext_of(path), origin: url.to_string() });
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(Error::input(format!("{}: the url '{url}' is not http(s) or file://", doc.document_id)));
    }
    if !curl_available() {
        return Err(Error::failed("`curl` is not installed: install it, or put the file next to the inventory and use local_path"));
    }
    let out = Command::new("curl")
        .args(["--fail", "--silent", "--show-error", "--location", "--max-time", &timeout_secs.to_string(), "--output", "-", url])
        .output()
        .map_err(|e| Error::failed(format!("cannot run curl: {e}")))?;
    if !out.status.success() {
        return Err(Error::failed(format!("{url}: {}", String::from_utf8_lossy(&out.stderr).trim())));
    }
    if out.stdout.is_empty() {
        return Err(Error::failed(format!("{url}: empty answer")));
    }
    Ok(Fetched { bytes: out.stdout, ext: ext_of(url), origin: url.to_string() })
}
