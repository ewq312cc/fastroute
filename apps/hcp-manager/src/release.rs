//! Versioned packs that ship with the software: the manifest of a pack (`pack.yaml`), the checks that make a pack directory
//! trustworthy, the deterministic archive, the release directory (`release.yaml`, `SHA256SUMS`, `archives/`) and the offline
//! installation of a pack into a workspace.
//!
//! A releasable pack directory is `<vendor>/<family>/<device>/<variant>/<pack-version>/` with
//!
//! ```text
//! pack.yaml      the manifest: identity, kind, version, dependencies, compatibility, licence, digests of everything else
//! hcp.yaml       the HCP document: the facts and their evidence
//! sources.yaml   the inventory entries of the documents the facts come from (version, digest, url)
//! reports/...    readable reports (optional)
//! documents/...  original documents, only when their licence allows redistribution (optional)
//! ```
//!
//! Nothing here uses the network, an AI model or a vector index; a release is verified and installed from files alone.

use crate::error::{Error, Result};
use crate::inventory::Inventory;
use crate::packs::version_cmp;
use crate::workspace::{write_atomic, Workspace};
use fr_constraint::hcp::{self, sha256_hex};
use fr_constraint::ir::IR_VERSION;
use fr_constraint::jsonschema;
use fr_constraint::resolve::{allowed_upstream_kinds, canonical_sha256};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const MANIFEST_VERSION: &str = "1.0";
pub const MANIFEST_SCHEMA_TEXT: &str = include_str!("../../../hardware-packs/schemas/pack-manifest.schema.json");
pub const RELEASE_FORMAT: &str = "1.0";
/// The version of this program: what `min_app_version` / `max_app_version` of a manifest are compared with.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The files of one pack, by path relative to the pack directory (always with `/`).
pub type Files = BTreeMap<String, Vec<u8>>;

/// A thing that is wrong with a pack or a release, with a stable code.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Problem {
    pub code: String,
    /// `id@version`, or a file of the release
    pub pack: String,
    pub message: String,
}

fn problem(code: &str, pack: &str, message: impl Into<String>) -> Problem {
    Problem { code: code.into(), pack: pack.into(), message: message.into() }
}

pub fn schema() -> Value {
    serde_json::from_str(MANIFEST_SCHEMA_TEXT).expect("the embedded manifest schema is valid JSON")
}

fn yaml_to_value(text: &str) -> std::result::Result<Value, String> {
    hcp::parse(text)
}

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

// ---- reading a pack directory ----------------------------------------------------------------------------------------------

fn walk(dir: &Path, base: &Path, out: &mut Files) -> Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).map_err(|e| Error::input(format!("{}: {e}", dir.display())))?.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let meta = std::fs::symlink_metadata(&p).map_err(|e| Error::input(format!("{}: {e}", p.display())))?;
        if meta.file_type().is_symlink() {
            return Err(Error::input(format!("{}: a symbolic link in a pack directory", p.display())));
        }
        if meta.is_dir() {
            walk(&p, base, out)?;
        } else {
            let rel = p.strip_prefix(base).expect("under base").components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
            out.insert(rel, std::fs::read(&p).map_err(|e| Error::input(format!("{}: {e}", p.display())))?);
        }
    }
    Ok(())
}

pub fn read_dir_files(dir: &Path) -> Result<Files> {
    let mut f = Files::new();
    walk(dir, dir, &mut f)?;
    Ok(f)
}

// ---- verifying one pack ----------------------------------------------------------------------------------------------------

/// A pack whose files and manifest passed every check that needs nothing but its own files.
#[derive(Debug, Clone)]
pub struct Pack {
    pub id: String,
    pub version: String,
    pub kind: String,
    pub manifest: Value,
    pub hcp: Value,
    pub files: Files,
    pub manifest_sha256: String,
    pub hcp_sha256: String,
    pub canonical_sha256: String,
}

impl Pack {
    pub fn label(&self) -> String {
        format!("{}@{}", self.id, self.version)
    }
    pub fn dependencies(&self) -> Vec<(String, String, String)> {
        arr(&self.manifest, "dependencies").iter().map(|d| (s(d, "id").unwrap_or("").to_string(), s(d, "version").unwrap_or("").to_string(), s(d, "kind").unwrap_or("").to_string())).collect()
    }
}

/// Every check that a pack's own files allow: the manifest against its schema, every digest, the identity in the HCP document, the
/// locked sources, the licence rules and the compatibility ranges. `at` is the directory (`id/version`) it was found at, when it
/// has to match.
pub fn load_pack(files: Files, at: Option<&str>) -> std::result::Result<Pack, Vec<Problem>> {
    let mut ps: Vec<Problem> = Vec::new();
    let Some(manifest_bytes) = files.get("pack.yaml") else {
        return Err(vec![problem("manifest_missing", at.unwrap_or("?"), "no pack.yaml (the manifest) in the pack directory")]);
    };
    let manifest = match std::str::from_utf8(manifest_bytes).map_err(|e| e.to_string()).and_then(yaml_to_value) {
        Ok(v) => v,
        Err(e) => return Err(vec![problem("manifest_unparseable", at.unwrap_or("?"), format!("pack.yaml: {e}"))]),
    };
    let id = s(&manifest, "id").unwrap_or("?").to_string();
    let version = s(&manifest, "version").unwrap_or("?").to_string();
    let label = format!("{id}@{version}");
    for e in jsonschema::validate(&schema(), &manifest) {
        ps.push(problem("manifest_schema", &label, format!("{}: {}", e.path, e.message)));
    }
    if !ps.is_empty() {
        return Err(ps);
    }
    let kind = s(&manifest, "kind").unwrap_or("").to_string();

    // identity
    let ident = &manifest["identity"];
    let joined = ["vendor", "family", "device", "variant"].iter().map(|k| s(ident, k).unwrap_or("")).collect::<Vec<_>>().join("/");
    if joined != id {
        ps.push(problem("id_mismatch", &label, format!("the id '{id}' is not vendor/family/device/variant of the identity ('{joined}')")));
    }
    if let Some(at) = at {
        if at != format!("{id}/{version}") {
            ps.push(problem("path_mismatch", &label, format!("the pack is in the directory '{at}' but its id and version say '{id}/{version}'")));
        }
    }

    // files: every one listed, with its digest; nothing unlisted
    let mut listed: BTreeMap<String, String> = BTreeMap::new();
    let fr = &manifest["files"];
    let mut push = |path: &str, sha: &str| {
        listed.insert(path.to_string(), sha.to_string());
    };
    push(s(&fr["hcp"], "path").unwrap_or(""), s(&fr["hcp"], "sha256").unwrap_or(""));
    push(s(&fr["sources"], "path").unwrap_or(""), s(&fr["sources"], "sha256").unwrap_or(""));
    for r in arr(fr, "reports") {
        push(s(r, "path").unwrap_or(""), s(r, "sha256").unwrap_or(""));
    }
    for d in arr(fr, "documents") {
        push(s(d, "path").unwrap_or(""), s(d, "sha256").unwrap_or(""));
    }
    for (path, want) in &listed {
        match files.get(path) {
            None => ps.push(problem("file_missing", &label, format!("{path} is listed in the manifest but is not in the pack"))),
            Some(b) if sha256_hex(b) != *want => ps.push(problem("file_digest_mismatch", &label, format!("{path} has the digest {} but the manifest lists {want}: the file was changed after the pack was sealed", sha256_hex(b)))),
            _ => {}
        }
    }
    for path in files.keys() {
        if path != "pack.yaml" && !listed.contains_key(path) {
            ps.push(problem("unlisted_file", &label, format!("{path} is in the pack but not in the manifest's file list")));
        }
    }

    // the licence of the originals: the manifest must agree with itself, and a document that may not be redistributed is not in the pack
    let lic: BTreeMap<&str, &str> = arr(&manifest["license"], "documents").iter().filter_map(|d| Some((s(d, "document_id")?, s(d, "redistribution")?))).collect();
    let mut reference_only_digests: BTreeMap<String, String> = BTreeMap::new();
    for d in arr(&manifest, "source_digests") {
        let (doc, red) = (s(d, "document_id").unwrap_or(""), s(d, "redistribution").unwrap_or(""));
        if lic.get(doc) != Some(&red) {
            ps.push(problem("license_mismatch", &label, format!("{doc}: source_digests says '{red}', license.documents says '{}'", lic.get(doc).copied().unwrap_or("nothing"))));
        }
        if red == "reference_only" {
            reference_only_digests.insert(s(d, "sha256").unwrap_or("").to_string(), doc.to_string());
        }
    }
    for d in arr(fr, "documents") {
        let doc = s(d, "document_id").unwrap_or("");
        if lic.get(doc) != Some(&"allowed") {
            ps.push(problem("license_forbids_bundling", &label, format!("{} is the original of {doc}, whose licence does not allow redistribution: keep only its URL, version, digest and locators", s(d, "path").unwrap_or(""))));
        }
    }
    for (path, bytes) in &files {
        if let Some(doc) = reference_only_digests.get(&sha256_hex(bytes)) {
            if !ps.iter().any(|p| p.code == "license_forbids_bundling" && p.message.starts_with(path.as_str())) {
                ps.push(problem("license_forbids_bundling", &label, format!("{path} is the original of {doc} (same digest), whose licence does not allow redistribution")));
            }
        }
    }

    // compatibility
    let compat = &manifest["compatibility"];
    if s(compat, "ir_version") != Some(IR_VERSION) {
        ps.push(problem("incompatible_ir_version", &label, format!("the pack is for IR version {}, this program writes {IR_VERSION}", s(compat, "ir_version").unwrap_or("?"))));
    }
    let hv = s(compat, "hcp_schema_version").unwrap_or("");
    match hv.strip_prefix("1.").and_then(|m| m.parse::<u64>().ok()) {
        Some(m) if m <= fr_constraint::hcp::SUPPORTED_MINOR => {}
        _ => ps.push(problem("incompatible_hcp_schema", &label, format!("the pack is for HCP schema {hv}, this program reads 1.0 to 1.{}", fr_constraint::hcp::SUPPORTED_MINOR))),
    }
    if version_cmp(APP_VERSION, s(compat, "min_app_version").unwrap_or("0.0.0")) == std::cmp::Ordering::Less {
        ps.push(problem("incompatible_app", &label, format!("the pack needs hcp-manager {} or newer; this is {APP_VERSION}", s(compat, "min_app_version").unwrap_or("?"))));
    }
    if let Some(max) = s(compat, "max_app_version") {
        if version_cmp(APP_VERSION, max) == std::cmp::Ordering::Greater {
            ps.push(problem("incompatible_app", &label, format!("the pack is declared for hcp-manager up to {max}; this is {APP_VERSION}")));
        }
    }

    // kind and dependencies (what can be said without the other packs)
    let deps = arr(&manifest, "dependencies");
    let allowed = allowed_upstream_kinds(&kind);
    for d in deps {
        let dk = s(d, "kind").unwrap_or("");
        if !allowed.contains(&dk) {
            ps.push(problem("kind_not_allowed", &label, format!("a {kind} pack cannot depend on a {dk} pack ({}); a chip pack depends on nothing, a module pack on chip packs, a board pack on module or chip packs", s(d, "id").unwrap_or("?"))));
        }
    }

    // the HCP document
    let hcp_doc: Option<Value> = files.get(s(&fr["hcp"], "path").unwrap_or("hcp.yaml")).and_then(|b| std::str::from_utf8(b).ok()).and_then(|t| match yaml_to_value(t) {
        Ok(v) => Some(v),
        Err(e) => {
            ps.push(problem("hcp_unparseable", &label, e));
            None
        }
    });
    if let Some(h) = &hcp_doc {
        let digest = canonical_sha256(h);
        if Some(digest.as_str()) != s(&manifest, "hcp_canonical_sha256") {
            ps.push(problem("hcp_canonical_mismatch", &label, format!("the canonical digest of the HCP document is {digest}, the manifest says {}", s(&manifest, "hcp_canonical_sha256").unwrap_or("?"))));
        }
        let hp = &h["pack"];
        if s(hp, "id") != Some(id.as_str()) || s(hp, "version") != Some(version.as_str()) || s(hp, "kind") != Some(kind.as_str()) {
            ps.push(problem("hcp_identity_mismatch", &label, format!("the HCP document says {}@{} kind {}, the manifest {id}@{version} kind {kind}", s(hp, "id").unwrap_or("?"), s(hp, "version").unwrap_or("?"), s(hp, "kind").unwrap_or("(none)"))));
        }
        if s(h, "hcp_version") != Some(hv) {
            ps.push(problem("hcp_version_mismatch", &label, format!("the HCP document is version {}, the manifest declares {hv}", s(h, "hcp_version").unwrap_or("?"))));
        }
        // the locked sources: HCP, manifest and sources.yaml agree on version and digest of every document
        let man: BTreeMap<&str, &Value> = arr(&manifest, "source_digests").iter().filter_map(|d| Some((s(d, "document_id")?, d))).collect();
        for src in arr(h, "sources") {
            let doc = s(src, "document_id").unwrap_or("");
            match man.get(doc) {
                None => ps.push(problem("source_missing", &label, format!("{doc} is a source of the HCP document but not in source_digests"))),
                Some(m) => {
                    if s(m, "sha256") != s(src, "sha256") || s(m, "version") != s(src, "version") {
                        ps.push(problem("source_digest_mismatch", &label, format!("{doc}: the HCP document has version {} and digest {}, source_digests has {} and {}", s(src, "version").unwrap_or("?"), s(src, "sha256").unwrap_or("?"), s(m, "version").unwrap_or("?"), s(m, "sha256").unwrap_or("?"))));
                    }
                }
            }
        }
        // a pack that registers an identity only carries no facts, and says so in both places
        let hcp_content = s(hp, "content").unwrap_or("full");
        if hcp_content != s(&manifest, "content").unwrap_or("") {
            ps.push(problem("content_mismatch", &label, format!("the manifest says content {}, the HCP document says {hcp_content}", s(&manifest, "content").unwrap_or("?"))));
        }
        if s(&manifest, "content") == Some("metadata_only") {
            let has = ["rules", "peripheral_blocks", "regions", "base_design"].iter().any(|k| h.get(*k).is_some_and(|v| v.as_array().map(|a| !a.is_empty()).unwrap_or(true)));
            if has {
                ps.push(problem("metadata_only_with_rules", &label, "a metadata_only pack has no rules, blocks, regions or base design: they would be facts nobody verified"));
            }
        }
        // the dependencies of the manifest are the upstream of the HCP document
        let up: BTreeSet<(String, String, String, String)> = hp.get("upstream").and_then(Value::as_array).into_iter().flatten().map(|u| (s(u, "id").unwrap_or("").into(), s(u, "version").unwrap_or("").into(), s(u, "kind").unwrap_or("").into(), s(u, "canonical_sha256").unwrap_or("").into())).collect();
        let dp: BTreeSet<(String, String, String, String)> = deps.iter().map(|d| (s(d, "id").unwrap_or("").into(), s(d, "version").unwrap_or("").into(), s(d, "kind").unwrap_or("").into(), s(d, "canonical_sha256").unwrap_or("").into())).collect();
        if up != dp {
            ps.push(problem("upstream_mismatch", &label, "the dependencies of the manifest are not the same as pack.upstream of the HCP document (id, version, kind and digest must agree)"));
        }
    }
    // sources.yaml: a valid inventory whose digests are the locked ones
    if let Some(b) = files.get(s(&fr["sources"], "path").unwrap_or("sources.yaml")) {
        match std::str::from_utf8(b).map_err(|e| e.to_string()).and_then(|t| Inventory::parse(t, "sources.yaml").map_err(|e| e.message)) {
            Err(e) => ps.push(problem("sources_unparseable", &label, e)),
            Ok(inv) => {
                let man: BTreeMap<&str, &Value> = arr(&manifest, "source_digests").iter().filter_map(|d| Some((s(d, "document_id")?, d))).collect();
                for d in &inv.documents {
                    match man.get(d.document_id.as_str()) {
                        None => ps.push(problem("source_missing", &label, format!("{} is in sources.yaml but not in source_digests", d.document_id))),
                        Some(m) if d.sha256.as_deref() != s(m, "sha256") || Some(d.version.as_str()) != s(m, "version") => {
                            ps.push(problem("source_digest_mismatch", &label, format!("{}: sources.yaml has version {} and digest {}, source_digests has {} and {}", d.document_id, d.version, d.sha256.as_deref().unwrap_or("none"), s(m, "version").unwrap_or("?"), s(m, "sha256").unwrap_or("?"))))
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    if !ps.is_empty() {
        return Err(ps);
    }
    let hcp = hcp_doc.expect("parsed above");
    Ok(Pack {
        manifest_sha256: sha256_hex(manifest_bytes),
        hcp_sha256: sha256_hex(&files[s(&fr["hcp"], "path").unwrap_or("hcp.yaml")]),
        canonical_sha256: canonical_sha256(&hcp),
        id,
        version,
        kind,
        manifest,
        hcp,
        files,
    })
}

// ---- verifying a set of packs ----------------------------------------------------------------------------------------------

fn key(id: &str, version: &str) -> String {
    format!("{id}@{version}")
}

/// What can only be checked with the other packs: unique identities, dependencies that exist with the pinned digest and kind, no
/// cycles, and the HCP documents (after merging what they inherit) valid.
pub fn verify_set(packs: &[Pack]) -> Vec<Problem> {
    let mut ps = Vec::new();
    let mut by: BTreeMap<String, &Pack> = BTreeMap::new();
    for p in packs {
        if by.insert(p.label(), p).is_some() {
            ps.push(problem("duplicate_pack_id", &p.label(), "two packs have this id and version"));
        }
    }
    // dependencies
    for p in packs {
        for (id, version, kind) in p.dependencies() {
            match by.get(&key(&id, &version)) {
                None => ps.push(problem("missing_dependency", &p.label(), format!("depends on {id}@{version}, which is not in the release (only the exact version counts; a similar pack is never used)"))),
                Some(d) => {
                    if d.kind != kind {
                        ps.push(problem("dependency_kind_mismatch", &p.label(), format!("{id}@{version} is a {} pack, the manifest says {kind}", d.kind)));
                    }
                    let pinned = p.manifest["dependencies"].as_array().into_iter().flatten().find(|x| s(x, "id") == Some(&id) && s(x, "version") == Some(&version)).and_then(|x| s(x, "canonical_sha256")).unwrap_or("");
                    if pinned != d.canonical_sha256 {
                        ps.push(problem("dependency_digest_mismatch", &p.label(), format!("{id}@{version} has the canonical digest {}, the pack pins {pinned}: the dependency changed, so this pack has to be reviewed and sealed again", d.canonical_sha256)));
                    }
                }
            }
        }
    }
    // cycles (depth first over the declared dependencies)
    fn visit(k: &str, by: &BTreeMap<String, &Pack>, stack: &mut Vec<String>, done: &mut BTreeSet<String>, ps: &mut Vec<Problem>) {
        if done.contains(k) {
            return;
        }
        if let Some(pos) = stack.iter().position(|x| x == k) {
            let cycle: Vec<String> = stack[pos..].iter().cloned().chain([k.to_string()]).collect();
            ps.push(problem("dependency_cycle", k, format!("circular dependency: {}", cycle.join(" -> "))));
            return;
        }
        let Some(p) = by.get(k) else { return };
        stack.push(k.to_string());
        for (id, version, _) in p.dependencies() {
            visit(&key(&id, &version), by, stack, done, ps);
        }
        stack.pop();
        done.insert(k.to_string());
    }
    let mut done = BTreeSet::new();
    for k in by.keys() {
        visit(k, &by, &mut Vec::new(), &mut done, &mut ps);
    }
    // the HCP documents, with what they inherit
    if !ps.iter().any(|p| matches!(p.code.as_str(), "dependency_cycle")) {
        for p in packs {
            if let Err(errs) = resolved(p, packs) {
                for e in errs {
                    ps.push(problem("pack_invalid", &p.label(), e));
                }
            }
        }
    }
    ps.sort_by(|a, b| (&a.pack, &a.code, &a.message).cmp(&(&b.pack, &b.code, &b.message)));
    ps
}

/// The flat HCP document of a pack: itself, merged with the packs it inherits from, checked.
pub fn resolved(pack: &Pack, all: &[Pack]) -> std::result::Result<Value, Vec<String>> {
    let ups: Vec<Value> = pack.dependencies().iter().filter_map(|(id, v, _)| all.iter().find(|p| &p.id == id && &p.version == v).map(|p| p.hcp.clone())).collect();
    let flat = fr_constraint::resolve::resolve(&pack.hcp, &ups).map_err(|ds| ds.into_iter().map(|d| format!("{} {}: {}", d.code, d.path, d.message)).collect::<Vec<_>>())?;
    let errs: Vec<String> = hcp::check(&flat).into_iter().filter(|d| d.severity == fr_constraint::diag::Severity::Error).map(|d| format!("{} {}: {}", d.code, d.path, d.message)).collect();
    if errs.is_empty() {
        Ok(flat)
    } else {
        Err(errs)
    }
}

// ---- sealing (the author's tool) -------------------------------------------------------------------------------------------

fn file_ref(path: &str, bytes: &[u8]) -> Value {
    json!({"path": path, "sha256": sha256_hex(bytes)})
}

/// Writes the computed part of a manifest: the digests of the files, the canonical digest of the HCP document, the locked sources and
/// the pinned dependencies, from what the author wrote (identity, kind, version, compatibility, dependencies as id, kind and version,
/// and the licence). Returns the manifest. The HCP document must already say what the manifest says (id, version, kind, upstream).
pub fn seal(dir: &Path) -> Result<Value> {
    let files = read_dir_files(dir)?;
    let manifest_text = files.get("pack.yaml").ok_or_else(|| Error::input(format!("{}: no pack.yaml (write the authored part of the manifest first)", dir.display())))?;
    let mut m = yaml_to_value(std::str::from_utf8(manifest_text).map_err(|e| Error::input(e.to_string()))?).map_err(|e| Error::input(format!("{}/pack.yaml: {e}", dir.display())))?;
    let hcp_bytes = files.get("hcp.yaml").ok_or_else(|| Error::input(format!("{}: no hcp.yaml", dir.display())))?;
    let src_bytes = files.get("sources.yaml").ok_or_else(|| Error::input(format!("{}: no sources.yaml", dir.display())))?;
    let hcp_doc = yaml_to_value(std::str::from_utf8(hcp_bytes).map_err(|e| Error::input(e.to_string()))?).map_err(|e| Error::input(format!("hcp.yaml: {e}")))?;
    let inv = Inventory::parse(std::str::from_utf8(src_bytes).map_err(|e| Error::input(e.to_string()))?, "sources.yaml")?;

    let mut reports = Vec::new();
    let mut documents = Vec::new();
    let mut stray = Vec::new();
    let doc_ids: BTreeMap<String, String> = inv.documents.iter().map(|d| (d.document_id.clone(), d.document_id.clone())).collect();
    for (path, bytes) in &files {
        match path.as_str() {
            "pack.yaml" | "hcp.yaml" | "sources.yaml" => {}
            p if p.starts_with("reports/") => reports.push(file_ref(p, bytes)),
            p if p.starts_with("documents/") => {
                // documents/<DOC-ID>/<file> or documents/<DOC-ID>.<ext>
                let rest = &p["documents/".len()..];
                let id = rest.split(['/', '.']).next().unwrap_or("");
                if doc_ids.contains_key(id) {
                    documents.push(json!({"document_id": id, "path": p, "sha256": sha256_hex(bytes)}));
                } else {
                    stray.push(format!("{p} (a document file is named documents/<DOCUMENT-ID>.<ext> or documents/<DOCUMENT-ID>/<file>, with an id of sources.yaml)"));
                }
            }
            p => stray.push(format!("{p} (readable reports go under reports/, original documents under documents/)")),
        }
    }
    if !stray.is_empty() {
        return Err(Error::input(format!("{}: files that belong nowhere in a pack:\n  {}", dir.display(), stray.join("\n  "))));
    }
    let lic: BTreeMap<String, String> = arr(&m["license"], "documents").iter().filter_map(|d| Some((s(d, "document_id")?.to_string(), s(d, "redistribution")?.to_string()))).collect();
    let mut source_digests = Vec::new();
    for d in &inv.documents {
        let red = lic.get(&d.document_id).cloned().ok_or_else(|| Error::input(format!("license.documents has no entry for {} (allowed or reference_only: it has to be said)", d.document_id)))?;
        let mut v = json!({"document_id": d.document_id, "version": d.version, "sha256": d.sha256.clone().ok_or_else(|| Error::input(format!("{} has no digest in sources.yaml: sync it before sealing", d.document_id)))?});
        if let Some(u) = &d.url {
            v["url"] = json!(u);
        }
        v["redistribution"] = json!(red);
        source_digests.push(v);
    }
    let mut deps = Vec::new();
    for d in arr(&m, "dependencies") {
        let (id, version) = (s(d, "id").unwrap_or(""), s(d, "version").unwrap_or(""));
        let pinned = arr(&hcp_doc["pack"], "upstream").iter().find(|u| s(u, "id") == Some(id) && s(u, "version") == Some(version)).and_then(|u| s(u, "canonical_sha256")).ok_or_else(|| Error::input(format!("dependency {id}@{version} is not in pack.upstream of hcp.yaml (the HCP document pins what it inherits)")))?;
        deps.push(json!({"id": id, "kind": d["kind"], "version": version, "canonical_sha256": pinned}));
    }
    let mut files_obj = Map::new();
    files_obj.insert("hcp".into(), file_ref("hcp.yaml", hcp_bytes));
    files_obj.insert("sources".into(), file_ref("sources.yaml", src_bytes));
    if !reports.is_empty() {
        files_obj.insert("reports".into(), Value::Array(reports));
    }
    if !documents.is_empty() {
        files_obj.insert("documents".into(), Value::Array(documents));
    }
    let obj = m.as_object_mut().ok_or_else(|| Error::input("pack.yaml is not a mapping"))?;
    obj.insert("dependencies".into(), Value::Array(deps));
    obj.insert("files".into(), Value::Object(files_obj));
    obj.insert("hcp_canonical_sha256".into(), json!(canonical_sha256(&hcp_doc)));
    obj.insert("source_digests".into(), Value::Array(source_digests));
    // a stable order of the keys makes the manifest read the same everywhere
    let order = ["manifest_version", "id", "kind", "identity", "version", "content", "review", "compatibility", "dependencies", "files", "hcp_canonical_sha256", "source_digests", "license"];
    let mut sorted = Map::new();
    for k in order {
        if let Some(v) = obj.remove(k) {
            sorted.insert(k.into(), v);
        }
    }
    for (k, v) in std::mem::take(obj) {
        sorted.insert(k, v);
    }
    let sealed = Value::Object(sorted);
    let text = format!("# The manifest of this pack. The authored part is above `files`; `files`, `hcp_canonical_sha256`, `source_digests` and the digests\n# of the dependencies are written by `hcp-manager release seal` and checked by `hcp-manager release index` / `packs builtin verify`.\n{}", hcp::to_yaml(&sealed).map_err(Error::failed)?);
    write_atomic(&dir.join("pack.yaml"), text.as_bytes())?;
    Ok(sealed)
}

// ---- the tree of packs, the index and the release --------------------------------------------------------------------------

/// The pack directories under `root` (those with a manifest): `(directory relative to root, files)`.
pub fn scan(root: &Path) -> Result<Vec<(String, Files)>> {
    fn go(dir: &Path, root: &Path, out: &mut Vec<(String, Files)>) -> Result<()> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).map_err(|e| Error::input(format!("{}: {e}", dir.display())))?.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        entries.sort();
        let manifest = dir.join("pack.yaml");
        if manifest.is_file() && std::fs::read_to_string(&manifest).map(|t| t.lines().any(|l| l.starts_with("manifest_version:"))).unwrap_or(false) {
            let rel = dir.strip_prefix(root).expect("under root").components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
            out.push((rel, read_dir_files(dir)?));
            return Ok(());
        }
        for p in entries {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if p.is_dir() && !name.starts_with('.') && name != "schemas" && name != "tools" {
                go(&p, root, out)?;
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    go(root, root, &mut out)?;
    Ok(out)
}

/// Loads and verifies every pack under `root`, and the set as a whole.
pub fn load_tree(root: &Path) -> Result<(Vec<Pack>, Vec<Problem>)> {
    let mut packs = Vec::new();
    let mut ps = Vec::new();
    for (rel, files) in scan(root)? {
        match load_pack(files, Some(&rel)) {
            Ok(p) => packs.push(p),
            Err(e) => ps.extend(e),
        }
    }
    packs.sort_by(|a, b| (&a.id, version_key(&a.version)).cmp(&(&b.id, version_key(&b.version))));
    if ps.is_empty() {
        ps.extend(verify_set(&packs));
    }
    Ok((packs, ps))
}

fn version_key(v: &str) -> Vec<(u8, u64, String)> {
    v.split(['.', '-', '+']).map(|p| p.parse::<u64>().map(|n| (0, n, String::new())).unwrap_or((1, 0, p.to_string()))).collect()
}

fn entry(p: &Pack) -> Value {
    json!({
        "id": p.id, "kind": p.kind, "version": p.version, "path": format!("{}/{}", p.id, p.version),
        "content": p.manifest["content"], "part_number": p.manifest["identity"]["part_number"],
        "manifest_sha256": p.manifest_sha256, "hcp_sha256": p.hcp_sha256, "hcp_canonical_sha256": p.canonical_sha256,
        "requires": p.dependencies().iter().map(|(id, v, k)| json!({"id": id, "version": v, "kind": k})).collect::<Vec<_>>(),
        "compatibility": p.manifest["compatibility"],
    })
}

/// `hardware-packs/index.yaml`: the generated, committed catalogue of the packs in the tree, with the digests that lock them. A
/// software release ships exactly these packs at exactly these digests.
pub fn index_text(packs: &[Pack]) -> Result<String> {
    let v = json!({"index_format": RELEASE_FORMAT, "packs": packs.iter().map(entry).collect::<Vec<_>>()});
    Ok(format!("# Generated by `hcp-manager release index`: do not edit. The packs of this tree and the digests that lock them.\n{}", hcp::to_yaml(&v).map_err(Error::failed)?))
}

// ---- the deterministic archive ---------------------------------------------------------------------------------------------

/// A `.tar.gz` of the files of one pack: sorted paths, fixed times, owners and modes, no gzip name or time. The same files give the
/// same bytes (for one version of the compression library).
pub fn archive(files: &Files) -> Result<Vec<u8>> {
    let gz = flate2::GzBuilder::new().mtime(0).operating_system(3).write(Vec::new(), flate2::Compression::new(6));
    let mut tar = tar::Builder::new(gz);
    tar.mode(tar::HeaderMode::Deterministic);
    for (path, bytes) in files {
        let mut h = tar::Header::new_gnu();
        h.set_size(bytes.len() as u64);
        h.set_mode(0o644);
        h.set_mtime(0);
        h.set_uid(0);
        h.set_gid(0);
        h.set_entry_type(tar::EntryType::Regular);
        tar.append_data(&mut h, path, bytes.as_slice()).map_err(|e| Error::output(format!("archive: {e}")))?;
    }
    let gz = tar.into_inner().map_err(|e| Error::output(format!("archive: {e}")))?;
    gz.finish().map_err(|e| Error::output(format!("archive: {e}")))
}

const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;

/// The files of an archive. Only plain files with safe relative paths are accepted.
pub fn unarchive(bytes: &[u8]) -> Result<Files> {
    let gz = flate2::read::GzDecoder::new(bytes);
    let mut tar = tar::Archive::new(gz.take(MAX_ARCHIVE_BYTES));
    let mut out = Files::new();
    for e in tar.entries().map_err(|e| Error::input(format!("archive: {e}")))? {
        let mut e = e.map_err(|e| Error::input(format!("archive: {e}")))?;
        if e.header().entry_type() != tar::EntryType::Regular {
            return Err(Error::input("archive: an entry that is not a plain file (links and directories are not allowed in a pack archive)"));
        }
        let path = e.path().map_err(|e| Error::input(format!("archive: {e}")))?.to_string_lossy().into_owned();
        let safe = !path.is_empty() && !path.starts_with('/') && path.split('/').all(|c| !c.is_empty() && c != "." && c != ".." && !c.contains('\\'));
        if !safe {
            return Err(Error::input(format!("archive: the path '{path}' is not a safe relative path")));
        }
        let mut data = Vec::new();
        e.read_to_end(&mut data).map_err(|e| Error::input(format!("archive: {e}")))?;
        if out.insert(path.clone(), data).is_some() {
            return Err(Error::input(format!("archive: '{path}' twice")));
        }
    }
    Ok(out)
}

// ---- the release directory -------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct BuildReport {
    pub format: &'static str,
    pub format_version: &'static str,
    pub out: String,
    pub packs: usize,
    pub archives: Vec<String>,
}

fn archive_rel(id: &str, version: &str) -> String {
    format!("archives/{id}/v{version}.tar.gz")
}

fn sums_line(sha: &str, path: &str) -> String {
    format!("{sha}  {path}\n")
}

/// Builds `out` (`release.yaml`, `SHA256SUMS`, `archives/`) from the packs under `root`. The tree must verify, and when
/// `<root>/index.yaml` exists it must be the current catalogue of the tree (the lock of the release).
pub fn build(root: &Path, out: &Path, force: bool) -> Result<BuildReport> {
    let (packs, problems) = load_tree(root)?;
    if !problems.is_empty() {
        return Err(Error::input(format!("the packs under {} do not verify, nothing is built:\n  {}", root.display(), problems.iter().map(|p| format!("{} [{}] {}", p.pack, p.code, p.message)).collect::<Vec<_>>().join("\n  "))));
    }
    if packs.is_empty() {
        return Err(Error::input(format!("{}: no packs (a pack directory has a pack.yaml with manifest_version)", root.display())));
    }
    let want_index = index_text(&packs)?;
    let committed = root.join("index.yaml");
    if committed.is_file() {
        let have = std::fs::read_to_string(&committed).map_err(|e| Error::input(format!("{}: {e}", committed.display())))?;
        if have != want_index {
            return Err(Error::input(format!("{} is not the catalogue of the packs in the tree (a pack changed, or one was added or removed): run `hcp-manager release index` and review the change", committed.display())));
        }
    }
    if out.exists() && std::fs::read_dir(out).map(|mut r| r.next().is_some()).unwrap_or(false) {
        if !force {
            return Err(Error::output(format!("{} exists and is not empty (use --force to replace the release)", out.display())));
        }
        std::fs::remove_dir_all(out).map_err(|e| Error::output(format!("{}: {e}", out.display())))?;
    }
    let mut sums = String::new();
    let mut entries = Vec::new();
    let mut names = Vec::new();
    for p in &packs {
        let bytes = archive(&p.files)?;
        let rel = archive_rel(&p.id, &p.version);
        write_atomic(&out.join(&rel), &bytes)?;
        let sha = sha256_hex(&bytes);
        sums.push_str(&sums_line(&sha, &rel));
        let mut e = entry(p);
        e["archive"] = json!(rel);
        e["archive_sha256"] = json!(sha);
        e.as_object_mut().expect("object").remove("path");
        entries.push(e);
        names.push(rel);
    }
    let release = json!({
        "release_format": RELEASE_FORMAT,
        "software": {"name": "fastroute", "version": APP_VERSION},
        "schemas": {"manifest_version": MANIFEST_VERSION, "hcp_schema_max": format!("1.{}", fr_constraint::hcp::SUPPORTED_MINOR), "ir_version": IR_VERSION},
        "index_sha256": sha256_hex(want_index.as_bytes()),
        "packs": entries,
    });
    let text = format!("# Generated by `hcp-manager release build`: the packs of this software release, locked by digest. Do not edit.\n{}", hcp::to_yaml(&release).map_err(Error::failed)?);
    write_atomic(&out.join("release.yaml"), text.as_bytes())?;
    let sums_all = format!("{}{}", sums_line(&sha256_hex(text.as_bytes()), "release.yaml"), sums);
    write_atomic(&out.join("SHA256SUMS"), sums_all.as_bytes())?;
    Ok(BuildReport { format: "hcp-manager-release-build", format_version: "1.0", out: out.display().to_string(), packs: packs.len(), archives: names })
}

/// An opened release directory (read only).
pub struct Release {
    pub dir: PathBuf,
    pub doc: Value,
}

impl Release {
    /// Where a release is looked for when none is named: `$HCP_PACKS_DIR`, `<program>/../share/hcp-packs`, `./dist/hcp-packs`.
    pub fn locate(arg: Option<&Path>) -> Result<PathBuf> {
        if let Some(a) = arg {
            return Ok(a.to_path_buf());
        }
        let mut tried = Vec::new();
        if let Some(e) = std::env::var_os("HCP_PACKS_DIR") {
            let p = PathBuf::from(e);
            if p.join("release.yaml").is_file() {
                return Ok(p);
            }
            tried.push(p);
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(p) = exe.parent().and_then(|d| d.parent()).map(|d| d.join("share/hcp-packs")) {
                if p.join("release.yaml").is_file() {
                    return Ok(p);
                }
                tried.push(p);
            }
        }
        let p = PathBuf::from("dist/hcp-packs");
        if p.join("release.yaml").is_file() {
            return Ok(p);
        }
        tried.push(p);
        Err(Error::input(format!("no release of packs found (looked in: {}); name one with --release DIR or build it with `hcp-manager release build`", tried.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "))))
    }

    pub fn open(dir: &Path) -> Result<Release> {
        let f = dir.join("release.yaml");
        let text = std::fs::read_to_string(&f).map_err(|e| Error::input(format!("{}: {e}", f.display())))?;
        let doc = yaml_to_value(&text).map_err(|e| Error::input(format!("{}: {e}", f.display())))?;
        if s(&doc, "release_format") != Some(RELEASE_FORMAT) {
            return Err(Error::input(format!("{}: release_format {} is not one this program reads ({RELEASE_FORMAT})", f.display(), s(&doc, "release_format").unwrap_or("?"))));
        }
        Ok(Release { dir: dir.to_path_buf(), doc })
    }

    pub fn entries(&self) -> &[Value] {
        arr(&self.doc, "packs")
    }

    /// An entry by `id`, `id@version` (the highest version when there is none). Never a similar id.
    pub fn find(&self, spec: &str) -> Result<&Value> {
        let (id, want) = match spec.rsplit_once('@') {
            Some((i, v)) => (i, Some(v)),
            None => (spec, None),
        };
        let mut found: Vec<&Value> = self.entries().iter().filter(|e| s(e, "id") == Some(id) && want.is_none_or(|w| s(e, "version") == Some(w))).collect();
        found.sort_by(|a, b| version_cmp(s(a, "version").unwrap_or(""), s(b, "version").unwrap_or("")));
        found.pop().ok_or_else(|| Error::input(format!("no pack '{spec}' in the release {} (the id is the whole vendor/family/device/variant; nothing similar is used instead)", self.dir.display())))
    }

    /// The archive of an entry, checked against the digest of the release, unpacked and verified.
    pub fn pack(&self, e: &Value) -> std::result::Result<Pack, Vec<Problem>> {
        let (id, version) = (s(e, "id").unwrap_or("?"), s(e, "version").unwrap_or("?"));
        let label = key(id, version);
        let rel = s(e, "archive").unwrap_or("");
        let safe = !rel.is_empty() && !rel.starts_with('/') && rel.split('/').all(|c| !c.is_empty() && c != "." && c != "..");
        if !safe {
            return Err(vec![problem("archive_path", &label, format!("the release names the archive '{rel}', which is not a relative path inside the release"))]);
        }
        let bytes = std::fs::read(self.dir.join(rel)).map_err(|err| vec![problem("archive_missing", &label, format!("{rel}: {err}"))])?;
        if Some(sha256_hex(&bytes).as_str()) != s(e, "archive_sha256") {
            return Err(vec![problem("archive_digest_mismatch", &label, format!("{rel} does not have the digest the release lists: the archive is damaged or was replaced"))]);
        }
        let files = unarchive(&bytes).map_err(|err| vec![problem("archive_unreadable", &label, err.message)])?;
        let p = load_pack(files, Some(&format!("{id}/{version}")))?;
        if Some(p.manifest_sha256.as_str()) != s(e, "manifest_sha256") || Some(p.hcp_sha256.as_str()) != s(e, "hcp_sha256") || Some(p.canonical_sha256.as_str()) != s(e, "hcp_canonical_sha256") {
            return Err(vec![problem("release_digest_mismatch", &label, "the manifest or the HCP document in the archive is not the one the release lists")]);
        }
        Ok(p)
    }

    /// Everything: the checksum file, every archive, every pack, and the set. Offline.
    pub fn verify(&self) -> Result<(Vec<Pack>, Vec<Problem>)> {
        let mut ps = Vec::new();
        // SHA256SUMS covers release.yaml and every archive
        let sums_path = self.dir.join("SHA256SUMS");
        match std::fs::read_to_string(&sums_path) {
            Err(e) => ps.push(problem("sums_missing", "release", format!("{}: {e}", sums_path.display()))),
            Ok(t) => {
                let mut covered = BTreeSet::new();
                for line in t.lines().filter(|l| !l.trim().is_empty()) {
                    let Some((sha, path)) = line.split_once("  ") else {
                        ps.push(problem("sums_unparseable", "release", format!("SHA256SUMS: '{line}'")));
                        continue;
                    };
                    covered.insert(path.to_string());
                    let safe = !path.starts_with('/') && path.split('/').all(|c| c != ".." && c != ".");
                    match if safe { std::fs::read(self.dir.join(path)).ok() } else { None } {
                        None => ps.push(problem("sums_file_missing", "release", format!("{path} is in SHA256SUMS but cannot be read"))),
                        Some(b) if sha256_hex(&b) != sha => ps.push(problem("sums_mismatch", "release", format!("{path} does not have the digest in SHA256SUMS"))),
                        _ => {}
                    }
                }
                for e in self.entries() {
                    let a = s(e, "archive").unwrap_or("");
                    if !covered.contains(a) {
                        ps.push(problem("sums_incomplete", "release", format!("{a} is not covered by SHA256SUMS")));
                    }
                }
                if !covered.contains("release.yaml") {
                    ps.push(problem("sums_incomplete", "release", "release.yaml is not covered by SHA256SUMS"));
                }
            }
        }
        let mut packs = Vec::new();
        for e in self.entries() {
            match self.pack(e) {
                Ok(p) => packs.push(p),
                Err(e) => ps.extend(e),
            }
        }
        if ps.is_empty() {
            ps.extend(verify_set(&packs));
        }
        Ok((packs, ps))
    }
}

// ---- installing into a workspace -------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallState {
    NotInstalled,
    /// the installed copy is exactly what the release has
    Installed,
    /// the installed copy was changed locally (a file edited, added or removed)
    Modified,
    /// installed from another archive of the same id and version
    OtherContent,
}

/// Eight hex digits that say which content a directory has (names the backup of a locally changed copy).
fn fingerprint(dir: &Path) -> String {
    let files = read_dir_files(dir).unwrap_or_default();
    let text: String = files.iter().map(|(k, v)| format!("{k}:{}\n", sha256_hex(v))).collect();
    sha256_hex(text.as_bytes())[..8].to_string()
}

fn receipt_path(dir: &Path) -> PathBuf {
    dir.join("INSTALL.json")
}

pub fn install_dir(ws: &Workspace, id: &str, version: &str) -> PathBuf {
    ws.packs_dir().join(id).join(version)
}

/// Whether, and in what state, a pack of the release is installed in the workspace. The receipt records the digests of the files as
/// installed; a local change shows as `Modified` and is never overwritten silently.
pub fn state_of(ws: &Workspace, e: &Value) -> InstallState {
    let dir = install_dir(ws, s(e, "id").unwrap_or(""), s(e, "version").unwrap_or(""));
    let Ok(rec) = std::fs::read_to_string(receipt_path(&dir)) else {
        return if dir.exists() { InstallState::Modified } else { InstallState::NotInstalled };
    };
    let Ok(rec): std::result::Result<Value, _> = serde_json::from_str(&rec) else { return InstallState::Modified };
    let Ok(now) = read_dir_files(&dir) else { return InstallState::Modified };
    let recorded: BTreeMap<String, String> = rec["files"].as_object().into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect();
    let actual: BTreeMap<String, String> = now.iter().filter(|(k, _)| k.as_str() != "INSTALL.json").map(|(k, b)| (k.clone(), sha256_hex(b))).collect();
    if recorded != actual {
        return InstallState::Modified;
    }
    if s(&rec, "archive_sha256") == s(e, "archive_sha256") {
        InstallState::Installed
    } else {
        InstallState::OtherContent
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallReport {
    pub format: &'static str,
    pub format_version: &'static str,
    /// every pack that was needed, dependencies first
    pub packs: Vec<InstalledPack>,
    pub inventory_added: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstalledPack {
    pub id: String,
    pub version: String,
    pub kind: String,
    pub path: String,
    /// `installed`, `already_installed`, `replaced` (with --force) or `kept_local_copy` (modified, not touched)
    pub action: &'static str,
}

/// Installs a pack of the release and the exact packs it depends on into `packs/<id>/<version>/` of the workspace, from the
/// archives alone. Everything is verified before anything is written; the release is only read. A copy in the workspace that was
/// changed locally is not overwritten (with `force` it is moved to `backup/` first); the same pack already installed is left alone.
pub fn install(ws: &Workspace, rel: &Release, spec: &str, force: bool) -> Result<InstallReport> {
    let top = rel.find(spec)?.clone();
    // the needed packs, dependencies first, each from its verified archive
    let mut order: Vec<Pack> = Vec::new();
    fn collect(rel: &Release, e: &Value, order: &mut Vec<Pack>, seen: &mut BTreeSet<String>, stack: &mut Vec<String>) -> Result<()> {
        let label = key(s(e, "id").unwrap_or(""), s(e, "version").unwrap_or(""));
        if seen.contains(&label) {
            return Ok(());
        }
        if stack.contains(&label) {
            return Err(Error::input(format!("circular dependency: {} -> {label}", stack.join(" -> "))));
        }
        stack.push(label.clone());
        let p = rel.pack(e).map_err(|ps| Error::input(format!("{label} does not verify:\n  {}", ps.iter().map(|p| format!("[{}] {}", p.code, p.message)).collect::<Vec<_>>().join("\n  "))))?;
        for (id, version, _) in p.dependencies() {
            let dep = rel.find(&key(&id, &version)).map_err(|_| Error::input(format!("{label} depends on {id}@{version}, which is not in the release")))?;
            collect(rel, dep, order, seen, stack)?;
        }
        stack.pop();
        seen.insert(label);
        order.push(p);
        Ok(())
    }
    collect(rel, &top, &mut order, &mut BTreeSet::new(), &mut Vec::new())?;
    // the set must be consistent with itself (digests of what is inherited, flat HCP valid)
    let ps = verify_set(&order);
    if !ps.is_empty() {
        return Err(Error::input(format!("the packs do not verify together, nothing is installed:\n  {}", ps.iter().map(|p| format!("{} [{}] {}", p.pack, p.code, p.message)).collect::<Vec<_>>().join("\n  "))));
    }
    // the documents of the packs go into the inventory of the workspace; a disagreement with what is there stops everything
    ws.init()?;
    let mut inv = if ws.inventory_path().exists() { Inventory::load(ws)? } else { Inventory::empty() };
    let mut added = Vec::new();
    for p in &order {
        let text = std::str::from_utf8(&p.files["sources.yaml"]).map_err(|e| Error::input(e.to_string()))?;
        for d in Inventory::parse(text, "sources.yaml")?.documents {
            match inv.get(&d.document_id) {
                None => {
                    added.push(d.document_id.clone());
                    inv.documents.push(d);
                }
                Some(mine) => {
                    let same = mine.version == d.version && mine.level == d.level && mine.applies_to == d.applies_to && (mine.sha256.is_none() || d.sha256.is_none() || mine.sha256 == d.sha256);
                    if !same {
                        return Err(Error::input(format!("{} of {}: the inventory of the workspace has another version or digest of this document (the inventory is not changed by an install; nothing is installed)", d.document_id, p.label())));
                    }
                }
            }
        }
    }
    // what would be written, and what would be refused, before the first write
    let mut plan: Vec<(&Pack, InstallState, &Value)> = Vec::new();
    for p in &order {
        let e = rel.find(&p.label())?;
        let st = state_of(ws, e);
        if matches!(st, InstallState::Modified | InstallState::OtherContent) && !force {
            let why = if st == InstallState::Modified { "was changed locally" } else { "was installed from another archive" };
            return Err(Error::output(format!("{} is installed and {why}: it is not overwritten (--force moves the local copy to backup/ and installs the release's)", install_dir(ws, &p.id, &p.version).display())));
        }
        plan.push((p, st, e));
    }
    let mut report = Vec::new();
    for (p, st, e) in plan {
        let dir = install_dir(ws, &p.id, &p.version);
        let action = match st {
            InstallState::Installed => "already_installed",
            InstallState::NotInstalled => "installed",
            _ => "replaced",
        };
        if st != InstallState::Installed {
            if dir.exists() {
                let backup = ws.root.join("backup/packs").join(&p.id).join(format!("{}-{}", p.version, fingerprint(&dir)));
                if let Some(parent) = backup.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| Error::output(format!("{}: {e}", parent.display())))?;
                }
                if backup.exists() {
                    std::fs::remove_dir_all(&backup).map_err(|e| Error::output(format!("{}: {e}", backup.display())))?;
                }
                std::fs::rename(&dir, &backup).map_err(|e| Error::output(format!("{}: {e}", backup.display())))?;
            }
            // the HCP document under its workspace name, the manifest and the rest as they are in the pack
            let mut sums = Map::new();
            for (path, bytes) in &p.files {
                write_atomic(&dir.join(path), bytes)?;
                sums.insert(path.clone(), json!(sha256_hex(bytes)));
            }
            let receipt = json!({
                "format": "hcp-manager-install", "format_version": "1.0",
                "id": p.id, "version": p.version, "kind": p.kind,
                "release_software_version": s(&rel.doc["software"], "version").unwrap_or(""),
                "archive_sha256": s(e, "archive_sha256").unwrap_or(""), "manifest_sha256": p.manifest_sha256, "hcp_sha256": p.hcp_sha256,
                "files": Value::Object(sums),
                "note": "A copy of the pack of the release. Edit it and it counts as modified: a later install does not overwrite it unless forced.",
            });
            write_atomic(&receipt_path(&dir), serde_json::to_string_pretty(&receipt).expect("serialises").as_bytes())?;
        }
        report.push(InstalledPack { id: p.id.clone(), version: p.version.clone(), kind: p.kind.clone(), path: dir.display().to_string(), action });
    }
    if !added.is_empty() {
        inv.save(ws)?;
    }
    Ok(InstallReport { format: "hcp-manager-install-report", format_version: "1.0", packs: report, inventory_added: added })
}
