//! The source inventory (`sources.yaml`): which documents there are, for which exact device model
//! and version, and how trustworthy their kind is. Source priority and "which document may be used
//! for this device" are decided here and nowhere else.

use crate::error::{Error, Result};
use crate::workspace::{write_atomic, Workspace};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SOURCES_SCHEMA_TEXT: &str = include_str!("../../../skills/hardware-constraint-design/schemas/sources.schema.json");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Applies {
    pub part_number: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Doc {
    pub document_id: String,
    pub title: String,
    pub level: String,
    pub version: String,
    pub applies_to: Vec<Applies>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieved: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// May the text of this document be sent to a remote embedding service? Default: no. Only an explicit `true` allows it, and
    /// only together with the other authorisations (see `embed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embed_remote: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inventory {
    pub sources_version: String,
    pub documents: Vec<Doc>,
}

/// 1 is the best source. Official manual and errata first, then the design guide and reference
/// design, then the official dev board, then an open board with a version, then the community.
pub fn priority(level: &str) -> u8 {
    match level {
        "official_manual" | "official_errata" => 1,
        "official_guide" | "official_reference_design" => 2,
        "official_devkit" => 3,
        "open_board" => 4,
        _ => 5,
    }
}

pub fn is_official(level: &str) -> bool {
    level.starts_with("official_")
}

pub const LEVELS: &[&str] = &["official_manual", "official_errata", "official_guide", "official_reference_design", "official_devkit", "open_board", "community"];

impl Inventory {
    pub fn empty() -> Inventory {
        Inventory { sources_version: "1.0".into(), documents: vec![] }
    }

    pub fn parse(text: &str, origin: &str) -> Result<Inventory> {
        let y: serde_yaml::Value = serde_yaml::from_str(text).map_err(|e| Error::input(format!("{origin}: not valid YAML: {e}")))?;
        let v: Value = serde_json::to_value(&y).map_err(|e| Error::input(format!("{origin}: {e}")))?;
        let schema: Value = serde_json::from_str(SOURCES_SCHEMA_TEXT).expect("the embedded schema is valid JSON");
        let errors = fr_constraint::jsonschema::validate(&schema, &v);
        if let Some(e) = errors.first() {
            return Err(Error::input(format!("{origin}: not a valid source inventory: {}: {} ({} problem(s))", e.path, e.message, errors.len())));
        }
        let inv: Inventory = serde_json::from_value(v).map_err(|e| Error::input(format!("{origin}: {e}")))?;
        let mut seen = std::collections::BTreeSet::new();
        for d in &inv.documents {
            if !seen.insert(d.document_id.as_str()) {
                return Err(Error::input(format!("{origin}: the document id {} is used twice", d.document_id)));
            }
        }
        Ok(inv)
    }

    pub fn load(ws: &Workspace) -> Result<Inventory> {
        let p = ws.inventory_path();
        if !p.exists() {
            return Err(Error::input(format!("{}: no inventory here (run `hcp-manager init` and `hcp-manager sources add`)", p.display())));
        }
        let text = std::fs::read_to_string(&p).map_err(|e| Error::input(format!("{}: {e}", p.display())))?;
        Inventory::parse(&text, &p.display().to_string())
    }

    pub fn save(&self, ws: &Workspace) -> Result<()> {
        let mut doc = self.clone();
        doc.documents.sort_by(|a, b| a.document_id.cmp(&b.document_id));
        let text = serde_yaml::to_string(&doc).map_err(|e| Error::failed(format!("cannot write the inventory: {e}")))?;
        write_atomic(&ws.inventory_path(), text.as_bytes())
    }

    pub fn get(&self, id: &str) -> Option<&Doc> {
        self.documents.iter().find(|d| d.document_id == id)
    }
}

impl Doc {
    /// What the index records about this entry of the inventory: if it changes, the blocks built before are stale (their version,
    /// level and device applicability are part of each block).
    pub fn index_signature(&self) -> String {
        let mut applies: Vec<String> = self.applies_to.iter().map(|a| format!("{}:{}", a.part_number.to_ascii_lowercase(), a.revision.clone().unwrap_or_default().to_ascii_lowercase())).collect();
        applies.sort();
        let text = format!("{}\n{}\n{}", self.version, self.level, applies.join(","));
        fr_constraint::hcp::sha256_hex(text.as_bytes())[..16].to_string()
    }

    /// Whether this document is for exactly this device (part number, and the revision if both name one).
    pub fn applies(&self, part: &str, revision: Option<&str>) -> bool {
        self.applies_to.iter().any(|a| a.part_number.eq_ignore_ascii_case(part) && (revision.is_none() || a.revision.is_none() || a.revision.as_deref().map(|r| r.eq_ignore_ascii_case(revision.unwrap_or(""))).unwrap_or(true)))
    }
}

/// One line of the resolved list: a document for the device, in priority order.
#[derive(Debug, Clone, Serialize)]
pub struct Resolved {
    pub document_id: String,
    pub title: String,
    pub level: String,
    pub priority: u8,
    pub version: String,
    pub official: bool,
    /// `usable`, or why it is listed but must not be used as evidence
    pub usable: bool,
    pub notes: Vec<String>,
    pub sha256: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Resolution {
    pub format: &'static str,
    pub format_version: &'static str,
    pub device: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub documents: Vec<Resolved>,
    /// The kinds of source that this exact device has no usable document for.
    pub missing: Vec<String>,
    /// Documents of other (similar) models: shown so that nobody wonders, never used.
    pub similar_models_excluded: Vec<String>,
    pub warnings: Vec<String>,
}

fn similar(a: &str, b: &str) -> bool {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    a != b && (a.starts_with(&b) || b.starts_with(&a) || a.split(['-', '_', ' ']).next() == b.split(['-', '_', ' ']).next())
}

/// The documents for one device, best source first. A document of a version that was asked for
/// differently, a dead link or a superseded document is listed but not usable; a similar model's
/// document is never used in its place.
pub fn resolve(inv: &Inventory, part: &str, revision: Option<&str>, version: Option<&str>) -> Resolution {
    let mut documents: Vec<Resolved> = Vec::new();
    let mut similar_models: Vec<String> = Vec::new();
    for d in &inv.documents {
        if d.applies(part, revision) {
            let mut notes = Vec::new();
            let mut usable = true;
            if let Some(v) = version {
                if d.version != v {
                    usable = false;
                    notes.push(format!("version {} was asked for, this is {}", v, d.version));
                }
            }
            match d.status.as_deref() {
                Some("superseded") => {
                    usable = false;
                    notes.push("superseded by a newer document".into());
                }
                Some("dead") => notes.push("the link is dead; only the cached copy remains".into()),
                Some("changed") => {
                    usable = false;
                    notes.push("the document at the URL changed since it was cached: review before use".into());
                }
                _ => {}
            }
            if d.sha256.is_none() {
                notes.push("not cached yet (no digest): run `hcp-manager sources sync`".into());
            }
            if !is_official(&d.level) {
                notes.push(if d.level == "open_board" { "an open board shows what one design does; it never overrides the manufacturer's documents".into() } else { "a community hint, not evidence".into() });
            }
            documents.push(Resolved {
                document_id: d.document_id.clone(),
                title: d.title.clone(),
                level: d.level.clone(),
                priority: priority(&d.level),
                version: d.version.clone(),
                official: is_official(&d.level),
                usable,
                notes,
                sha256: d.sha256.clone(),
                status: d.status.clone(),
            });
        } else if d.applies_to.iter().any(|a| similar(&a.part_number, part)) {
            similar_models.push(format!("{} ({}, for {})", d.document_id, d.title, d.applies_to.iter().map(|a| a.part_number.clone()).collect::<Vec<_>>().join(", ")));
        }
    }
    documents.sort_by(|a, b| (a.priority, !a.usable, &a.document_id).cmp(&(b.priority, !b.usable, &b.document_id)));
    let has = |levels: &[&str]| documents.iter().any(|d| d.usable && levels.contains(&d.level.as_str()));
    let mut missing = Vec::new();
    if !has(&["official_manual"]) {
        missing.push("official_manual".to_string());
    }
    if !has(&["official_guide", "official_reference_design"]) {
        missing.push("official_guide".to_string());
    }
    let mut warnings = Vec::new();
    if !has(&["official_manual"]) {
        warnings.push(format!(
            "no usable official manual for exactly {part}{}{}: do not generate fixed or required rules from other documents; list what is missing{}",
            revision.map(|r| format!(" revision {r}")).unwrap_or_default(),
            version.map(|v| format!(" version {v}")).unwrap_or_default(),
            if similar_models.is_empty() { String::new() } else { " (documents of similar models exist, but they are for other models)".to_string() }
        ));
    }
    Resolution {
        format: "hcp-manager-resolution",
        format_version: "1.0",
        device: part.to_string(),
        revision: revision.map(str::to_string),
        version: version.map(str::to_string),
        documents,
        missing,
        similar_models_excluded: similar_models,
        warnings,
    }
}
