//! Knowledge packs in the workspace: listing, summaries and a self-contained export. The pack file
//! (HCP YAML) is the fact; nothing here reads an index.

use crate::error::{Error, Result};
use crate::inventory::{priority, Inventory};
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// the packs it inherits from, `id@version`
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub upstream: Vec<String>,
}

/// The HCP documents under a directory: `hcp.yaml` (a releasable pack, whose `pack.yaml` is the manifest), else the older
/// `pack.yaml` / `pack.json` / `*.hcp.yaml` that is the HCP document itself.
fn find_pack_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    entries.sort();
    let has_hcp_yaml = entries.iter().any(|p| p.file_name().is_some_and(|n| n == "hcp.yaml"));
    for p in entries {
        if p.is_dir() {
            find_pack_files(&p, out);
        } else if p.file_name().is_some_and(|n| n == "hcp.yaml" || (!has_hcp_yaml && (n == "pack.yaml" || n == "pack.json")) || n.to_string_lossy().ends_with(".hcp.yaml")) {
            out.push(p);
        }
    }
}

pub fn load(path: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::input(format!("{}: {e}", path.display())))?;
    hcp::parse(&text).map_err(|e| Error::input(format!("{}: {e}", path.display())))
}

/// The pack as every consumer reads it: when it inherits (`pack.upstream`), merged with the exact upstream packs of the workspace
/// (id and version, pinned digest); otherwise the pack itself. The errors are readable lines.
pub fn resolved_doc(ws: &Workspace, v: &Value) -> std::result::Result<Value, Vec<String>> {
    if v["pack"]["upstream"].as_array().is_none_or(|a| a.is_empty()) {
        return Ok(v.clone());
    }
    let mut files = Vec::new();
    find_pack_files(&ws.packs_dir(), &mut files);
    let ups: Vec<Value> = files.iter().filter_map(|f| load(f).ok()).collect();
    fr_constraint::resolve::resolve(v, &ups).map_err(|ds| ds.into_iter().map(|d| format!("{} {}: {}", d.code, d.path, d.message)).collect())
}

fn is_valid(ws: &Workspace, v: &Value) -> bool {
    match resolved_doc(ws, v) {
        Ok(flat) => hcp::check(&flat).iter().all(|d| d.severity != fr_constraint::diag::Severity::Error),
        Err(_) => false,
    }
}

fn info(ws: &Workspace, path: &Path, v: &Value) -> PackInfo {
    PackInfo {
        id: v["pack"]["id"].as_str().unwrap_or("").to_string(),
        title: v["pack"]["title"].as_str().unwrap_or("").to_string(),
        version: v["pack"]["version"].as_str().unwrap_or("").to_string(),
        review: v["pack"]["review"]["status"].as_str().unwrap_or("draft").to_string(),
        path: path.display().to_string(),
        devices: v["devices"].as_array().map(|a| a.iter().filter_map(|d| d["part_number"].as_str().map(str::to_string)).collect()).unwrap_or_default(),
        valid: is_valid(ws, v),
        kind: v["pack"]["kind"].as_str().map(str::to_string),
        upstream: v["pack"]["upstream"].as_array().map(|a| a.iter().filter_map(|u| Some(format!("{}@{}", u["id"].as_str()?, u["version"].as_str()?))).collect()).unwrap_or_default(),
    }
}

pub fn list(ws: &Workspace) -> Vec<PackInfo> {
    let mut files = Vec::new();
    find_pack_files(&ws.packs_dir(), &mut files);
    files.iter().filter_map(|f| Some(info(ws, f, &load(f).ok()?))).collect()
}

/// Compares version strings part by part, numbers as numbers (`0.10.0` is after `0.9.0`).
pub fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |s: &str| s.split(['.', '-', '+']).map(|p| p.parse::<u64>().map(|n| (0, n, String::new())).unwrap_or((1, 0, p.to_string()))).collect::<Vec<_>>();
    parts(a).cmp(&parts(b))
}

/// A pack by path, by id (the highest version of it) or by `id@version`.
pub fn find(ws: &Workspace, id_or_path: &str) -> Result<(PathBuf, Value)> {
    let p = Path::new(id_or_path);
    if p.is_file() {
        return Ok((p.to_path_buf(), load(p)?));
    }
    let (id, want) = match id_or_path.rsplit_once('@') {
        Some((i, v)) => (i, Some(v)),
        None => (id_or_path, None),
    };
    let mut files = Vec::new();
    find_pack_files(&ws.packs_dir(), &mut files);
    let mut found: Vec<(String, PathBuf, Value)> = Vec::new();
    for f in files {
        if let Ok(v) = load(&f) {
            if v["pack"]["id"] == id && want.is_none_or(|w| v["pack"]["version"] == w) {
                found.push((v["pack"]["version"].as_str().unwrap_or("").to_string(), f, v));
            }
        }
    }
    found.sort_by(|a, b| version_cmp(&a.0, &b.0));
    found.pop().map(|(_, f, v)| (f, v)).ok_or_else(|| Error::input(format!("no pack '{id_or_path}' in {} (and it is not a file)", ws.packs_dir().display())))
}

pub fn show(ws: &Workspace, path: &Path, v: &Value) -> Value {
    let count = |section: &str| v[section].as_array().map(Vec::len).unwrap_or(0);
    let mut by_mod: std::collections::BTreeMap<String, usize> = Default::default();
    for r in v["rules"].as_array().into_iter().flatten() {
        *by_mod.entry(r["modifiability"].as_str().unwrap_or("").to_string()).or_insert(0) += 1;
    }
    let flat = resolved_doc(ws, v);
    let diag = match &flat {
        Ok(f) => hcp::check(f),
        Err(_) => hcp::check_with(v, true),
    };
    json!({
        "format": "hcp-manager-pack",
        "format_version": "1.0",
        "info": info(ws, path, v),
        "canonical_sha256": fr_constraint::resolve::canonical_sha256(v),
        "unresolved": flat.as_ref().err().cloned().unwrap_or_default(),
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


// ---- create and import ----------------------------------------------------------------------------------------------------

/// Where a pack of this id and version lives in the workspace: `packs/<id>/<version>/pack.yaml`. Packs are isolated by id and
/// version; two versions of one pack are two files.
pub fn pack_path(ws: &Workspace, id: &str, version: &str) -> PathBuf {
    ws.packs_dir().join(id).join(version).join("pack.yaml")
}

fn safe_part(s: &str) -> bool {
    !s.is_empty() && s.split('/').all(|p| !p.is_empty() && p != "." && p != ".." && p.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)))
}

#[derive(Debug, Clone)]
pub struct CreateSpec {
    pub id: String,
    pub version: String,
    pub title: Option<String>,
    pub vendor: String,
    pub part_number: String,
    pub revision: Option<String>,
    pub language: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateReport {
    pub format: &'static str,
    pub format_version: &'static str,
    pub path: String,
    pub sources: Vec<String>,
    /// Documents for the device that were not put in: no digest yet (not synced)
    pub not_included: Vec<String>,
    /// A new pack is a skeleton: it says what is missing and is not valid until the facts are written
    pub valid: bool,
    pub errors: Vec<String>,
}

/// A skeleton pack in the workspace: the metadata, the device, and the documents of the inventory that are for this exact device
/// and have a digest. It holds no pins and no rules: it is not valid until a person (or an AI that read the documents) has written
/// them, and it says so.
pub fn create(ws: &Workspace, inv: &Inventory, spec: &CreateSpec) -> Result<(PathBuf, CreateReport)> {
    if !safe_part(&spec.id) {
        return Err(Error::usage(format!("the pack id '{}' must be like vendor/device (letters, digits, . _ -)", spec.id)));
    }
    if !safe_part(&spec.version) || spec.version.contains('/') {
        return Err(Error::usage(format!("the version '{}' is not usable as a directory name", spec.version)));
    }
    let path = pack_path(ws, &spec.id, &spec.version);
    if path.exists() {
        return Err(Error::output(format!("{} exists: a pack of this id and version is never overwritten (use another version)", path.display())));
    }
    let dev_id = format!("DEV-{}", spec.part_number.to_ascii_uppercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>().trim_matches('-'));
    let mut sources = Vec::new();
    let mut ids = Vec::new();
    let mut not_included = Vec::new();
    let mut docs: Vec<&crate::inventory::Doc> = inv.documents.iter().filter(|d| d.applies(&spec.part_number, spec.revision.as_deref())).collect();
    docs.sort_by(|a, b| (priority(&a.level), &a.document_id).cmp(&(priority(&b.level), &b.document_id)));
    for d in docs {
        match &d.sha256 {
            Some(sha) => {
                let sid = format!("SRC-{}", d.document_id.trim_start_matches("DOC-"));
                let mut v = json!({"source_id": sid, "document_id": d.document_id, "title": d.title, "version": d.version, "level": d.level,
                    "retrieved": d.retrieved.clone().unwrap_or_else(crate::workspace::today), "sha256": sha, "applies_to": [dev_id]});
                if let Some(u) = &d.url {
                    v["url"] = json!(u);
                }
                ids.push(sid);
                sources.push(v);
            }
            None => not_included.push(format!("{} (not synced: no digest)", d.document_id)),
        }
    }
    let mut device = json!({"id": dev_id, "vendor": spec.vendor, "part_number": spec.part_number, "package": "unknown (fill in from the datasheet)", "pins": []});
    if let Some(r) = &spec.revision {
        device["revision"] = json!(r);
    }
    let pack = json!({
        "hcp_version": "1.0",
        "pack": {
            "id": spec.id,
            "title": spec.title.clone().unwrap_or_else(|| format!("{} constraint pack", spec.part_number)),
            "version": spec.version,
            "language": spec.language,
            "created": crate::workspace::today(),
            "extractor": {"kind": "human", "tool": "hcp-manager packs create"},
            "review": {"status": "draft", "note": "A skeleton made by hcp-manager: the device has no pins and there are no rules yet. Nothing in it is a fact about the part."}
        },
        "sources": sources,
        "devices": [device],
    });
    let text = format!(
        "# A skeleton made by `hcp-manager packs create`. It has no pins and no rules: read the documents (hcp-manager search / sources resolve),\n# then write them with their evidence (see skills/hardware-constraint-design/references/hcp-format.md). `fastroute constraints check` says what is missing.\n{}",
        serde_yaml::to_string(&pack).map_err(|e| Error::failed(e.to_string()))?
    );
    write_atomic(&path, text.as_bytes())?;
    let diag = hcp::check(&pack);
    let errors: Vec<String> = diag.iter().filter(|d| d.severity == fr_constraint::diag::Severity::Error).map(|d| format!("{} {}: {}", d.code, d.path, d.message)).collect();
    Ok((path.clone(), CreateReport { format: "hcp-manager-pack-create", format_version: "1.0", path: path.display().to_string(), sources: ids, not_included, valid: errors.is_empty(), errors }))
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportReport {
    pub format: &'static str,
    pub format_version: &'static str,
    pub pack_id: String,
    pub version: String,
    pub path: String,
    pub inventory_added: Vec<String>,
    pub inventory_already_there: Vec<String>,
    /// Cited by the pack but not in the inventory that came with it
    pub missing_from_inventory: Vec<String>,
    pub already_imported: bool,
}

/// Imports a pack file, or a directory written by `packs export` (the pack, `sources-used.yaml` and a manifest whose digests must
/// match). The pack must be valid. The documents it cites are added to the inventory; a document that is already there under the
/// same id with another version, level, devices or digest is a conflict and nothing is imported: the inventory is a set of facts and
/// is not changed by importing.
pub fn import(ws: &Workspace, from: &Path, force: bool) -> Result<ImportReport> {
    let (pack_file, used_file, manifest): (PathBuf, Option<PathBuf>, Option<Value>) = if from.is_dir() {
        let mf = from.join("export-manifest.json");
        let manifest: Option<Value> = if mf.is_file() { Some(serde_json::from_slice(&std::fs::read(&mf).map_err(|e| Error::input(format!("{}: {e}", mf.display())))?).map_err(|e| Error::input(format!("{}: {e}", mf.display())))?) } else { None };
        // an export names its pack file in the manifest (it keeps the name of the file it was made from)
        let named = manifest.as_ref().and_then(|m| m["files"].as_object()).and_then(|f| f.keys().find(|k| k.as_str() != "sources-used.yaml").map(|k| from.join(k)));
        let pack = match named {
            Some(p) => p,
            None => {
                let mut files = Vec::new();
                find_pack_files(from, &mut files);
                files.into_iter().next().ok_or_else(|| Error::input(format!("{}: no pack.yaml (or pack.json, *.hcp.yaml, or an export manifest) in the directory", from.display())))?
            }
        };
        let used = from.join("sources-used.yaml");
        (pack, used.is_file().then_some(used), manifest)
    } else if from.is_file() {
        (from.to_path_buf(), None, None)
    } else {
        return Err(Error::input(format!("{}: not a file or a directory", from.display())));
    };
    if let Some(m) = &manifest {
        for (name, want) in m["files"].as_object().into_iter().flatten() {
            let f = from.join(name);
            let bytes = std::fs::read(&f).map_err(|e| Error::input(format!("{}: listed in the manifest but {e}", f.display())))?;
            if want.as_str().is_some_and(|w| w != sha256_hex(&bytes)) {
                return Err(Error::input(format!("{}: its digest is not the one in export-manifest.json: the export was changed after it was written", f.display())));
            }
        }
    }
    let pack = load(&pack_file)?;
    let errors: Vec<String> = hcp::check(&pack).iter().filter(|d| d.severity == fr_constraint::diag::Severity::Error).take(5).map(|d| format!("{} {}: {}", d.code, d.path, d.message)).collect();
    if !errors.is_empty() {
        return Err(Error::input(format!("{}: the pack is not valid, it is not imported:\n  {}", pack_file.display(), errors.join("\n  "))));
    }
    let (id, version) = (pack["pack"]["id"].as_str().unwrap_or("").to_string(), pack["pack"]["version"].as_str().unwrap_or("").to_string());
    if !safe_part(&id) || !safe_part(&version) || version.contains('/') {
        return Err(Error::input(format!("{}: the pack id or version cannot be a directory name", pack_file.display())));
    }
    ws.init()?;
    let mut inv = if ws.inventory_path().exists() { Inventory::load(ws)? } else { Inventory::empty() };
    let theirs: Inventory = match &used_file {
        Some(f) => Inventory::parse(&std::fs::read_to_string(f).map_err(|e| Error::input(format!("{}: {e}", f.display())))?, &f.display().to_string())?,
        None => Inventory::empty(),
    };
    let (mut added, mut there, mut conflicts) = (Vec::new(), Vec::new(), Vec::new());
    for d in &theirs.documents {
        match inv.get(&d.document_id) {
            None => added.push(d.clone()),
            Some(mine) => {
                let same = mine.version == d.version && mine.level == d.level && mine.applies_to == d.applies_to && (mine.sha256.is_none() || d.sha256.is_none() || mine.sha256 == d.sha256);
                if same {
                    there.push(d.document_id.clone());
                } else {
                    conflicts.push(format!("{}: the workspace has version {} ({}), the import has {} ({})", d.document_id, mine.version, mine.sha256.as_deref().map(|s| &s[..8.min(s.len())]).unwrap_or("no digest"), d.version, d.sha256.as_deref().map(|s| &s[..8.min(s.len())]).unwrap_or("no digest")));
                }
            }
        }
    }
    if !conflicts.is_empty() {
        return Err(Error::input(format!("not imported: the inventory of the workspace disagrees with the documents of the pack (the inventory is not changed by an import):\n  {}", conflicts.join("\n  "))));
    }
    let dest = pack_path(ws, &id, &version);
    let new_bytes = std::fs::read(&pack_file).map_err(|e| Error::input(format!("{}: {e}", pack_file.display())))?;
    let already = dest.is_file() && std::fs::read(&dest).map(|b| b == new_bytes).unwrap_or(false);
    if dest.exists() && !already && !force {
        return Err(Error::output(format!("{} exists with other content (use --force to replace it, or give the pack another version)", dest.display())));
    }
    let added_ids: Vec<String> = added.iter().map(|d| d.document_id.clone()).collect();
    inv.documents.extend(added);
    if !added_ids.is_empty() {
        inv.save(ws)?;
    }
    if !already {
        write_atomic(&dest, &new_bytes)?;
    }
    let cited: Vec<String> = pack["sources"].as_array().into_iter().flatten().filter_map(|s| s["document_id"].as_str().map(str::to_string)).collect();
    let missing: Vec<String> = cited.into_iter().filter(|c| inv.get(c).is_none()).collect();
    Ok(ImportReport { format: "hcp-manager-pack-import", format_version: "1.0", pack_id: id, version, path: dest.display().to_string(), inventory_added: added_ids, inventory_already_there: there, missing_from_inventory: missing, already_imported: already })
}
