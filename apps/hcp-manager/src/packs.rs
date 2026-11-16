//! Knowledge packs in the workspace: listing, summaries and a self-contained export. The pack file
//! (HCP YAML) is the fact; nothing here reads an index.

use crate::error::{Error, Result};
use crate::inventory::Inventory;
use crate::workspace::{write_atomic, Workspace};
use fr_constraint::hcp::{self, sha256_hex};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct PackInfo {
    pub id: String,
    pub title: String,
    pub version: String,
    pub review: String,
    pub path: String,
    pub devices: Vec<String>,
    pub valid: bool,
}

fn find_pack_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            find_pack_files(&p, out);
        } else if p.file_name().is_some_and(|n| n == "pack.yaml" || n == "pack.json" || n.to_string_lossy().ends_with(".hcp.yaml")) {
            out.push(p);
        }
    }
}

pub fn load(path: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::input(format!("{}: {e}", path.display())))?;
    hcp::parse(&text).map_err(|e| Error::input(format!("{}: {e}", path.display())))
}

fn info(path: &Path, v: &Value) -> PackInfo {
    PackInfo {
        id: v["pack"]["id"].as_str().unwrap_or("").to_string(),
        title: v["pack"]["title"].as_str().unwrap_or("").to_string(),
        version: v["pack"]["version"].as_str().unwrap_or("").to_string(),
        review: v["pack"]["review"]["status"].as_str().unwrap_or("draft").to_string(),
        path: path.display().to_string(),
        devices: v["devices"].as_array().map(|a| a.iter().filter_map(|d| d["part_number"].as_str().map(str::to_string)).collect()).unwrap_or_default(),
        valid: hcp::check(v).iter().all(|d| d.severity != fr_constraint::diag::Severity::Error),
    }
}

pub fn list(ws: &Workspace) -> Vec<PackInfo> {
    let mut files = Vec::new();
    find_pack_files(&ws.packs_dir(), &mut files);
    files.iter().filter_map(|f| Some(info(f, &load(f).ok()?))).collect()
}

/// A pack by id (or a path to one).
pub fn find(ws: &Workspace, id_or_path: &str) -> Result<(PathBuf, Value)> {
    let p = Path::new(id_or_path);
    if p.is_file() {
        return Ok((p.to_path_buf(), load(p)?));
    }
    let mut files = Vec::new();
    find_pack_files(&ws.packs_dir(), &mut files);
    for f in files {
        if let Ok(v) = load(&f) {
            if v["pack"]["id"] == id_or_path {
                return Ok((f, v));
            }
        }
    }
    Err(Error::input(format!("no pack '{id_or_path}' in {} (and it is not a file)", ws.packs_dir().display())))
}

pub fn show(path: &Path, v: &Value) -> Value {
    let count = |section: &str| v[section].as_array().map(Vec::len).unwrap_or(0);
    let mut by_mod: std::collections::BTreeMap<String, usize> = Default::default();
    for r in v["rules"].as_array().into_iter().flatten() {
        *by_mod.entry(r["modifiability"].as_str().unwrap_or("").to_string()).or_insert(0) += 1;
    }
    let diag = hcp::check(v);
    json!({
        "format": "hcp-manager-pack",
        "format_version": "1.0",
        "info": info(path, v),
        "counts": {"sources": count("sources"), "devices": count("devices"), "peripheral_blocks": count("peripheral_blocks"), "rules": count("rules"), "regions": count("regions")},
        "rules_by_modifiability": by_mod,
        "diagnostics": {"errors": diag.iter().filter(|d| d.severity == fr_constraint::diag::Severity::Error).count(), "warnings": diag.iter().filter(|d| d.severity == fr_constraint::diag::Severity::Warning).count()},
    })
}

/// Writes a self-contained directory: the pack, the inventory entries it cites (with digests) and a
/// manifest of digests. No cache, no index: those are rebuilt from the inventory.
pub fn export(ws: &Workspace, path: &Path, v: &Value, dest: &Path, force: bool) -> Result<Vec<String>> {
    if dest.exists() && !force && std::fs::read_dir(dest).map(|mut r| r.next().is_some()).unwrap_or(false) {
        return Err(Error::output(format!("{} exists and is not empty (use --force)", dest.display())));
    }
    let inv = Inventory::load(ws).unwrap_or_else(|_| Inventory::empty());
    let cited: Vec<String> = v["sources"].as_array().into_iter().flatten().filter_map(|s| s["document_id"].as_str().map(str::to_string)).collect();
    let mut used = Inventory::empty();
    for id in &cited {
        if let Some(d) = inv.get(id) {
            used.documents.push(d.clone());
        }
    }
    used.documents.sort_by(|a, b| a.document_id.cmp(&b.document_id));
    let pack_bytes = std::fs::read(path).map_err(|e| Error::input(format!("{}: {e}", path.display())))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "pack.yaml".into());
    let sources_yaml = serde_yaml::to_string(&used).map_err(|e| Error::failed(e.to_string()))?;
    write_atomic(&dest.join(&name), &pack_bytes)?;
    write_atomic(&dest.join("sources-used.yaml"), sources_yaml.as_bytes())?;
    let manifest = json!({
        "format": "hcp-manager-export",
        "format_version": "1.0",
        "pack_id": v["pack"]["id"],
        "files": {name.clone(): sha256_hex(&pack_bytes), "sources-used.yaml": sha256_hex(sources_yaml.as_bytes())},
        "missing_from_inventory": cited.iter().filter(|c| inv.get(c).is_none()).collect::<Vec<_>>(),
        "note": "The cache, the text blocks and any index are derived: rebuild them with `hcp-manager sources sync` and `hcp-manager index build`."
    });
    write_atomic(&dest.join("export-manifest.json"), serde_json::to_string_pretty(&manifest).expect("serialises").as_bytes())?;
    Ok(vec![name, "sources-used.yaml".into(), "export-manifest.json".into()])
}
