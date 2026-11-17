//! What is known about the design a draft is for: nets and their classes, layers, and the limits
//! the design already has. Read from `inspect.json` (alone, or the one of an ai-pack).

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use crate::cmd::CmdError;
use crate::load::sha256_hex;

#[derive(Clone, Debug, Default)]
pub struct ClassFacts {
    pub trace_width_mm: Vec<(String, f64)>,
    pub clearance_mm: Vec<(String, f64)>,
    pub max_length_mm: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct DesignFacts {
    /// net -> class
    pub nets: BTreeMap<String, String>,
    pub classes: BTreeMap<String, ClassFacts>,
    pub layers: Vec<String>,
    pub components: Vec<String>,
    /// SHA-256 of the DSN as recorded in inspect.json, and of inspect.json itself.
    pub input_sha256: Option<String>,
    pub inspect_sha256: String,
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64().filter(|x| x.is_finite())
}

fn per_layer(v: &Value) -> Vec<(String, f64)> {
    v.as_array().into_iter().flatten().filter_map(|x| Some((x["layer"].as_str()?.to_string(), num(&x["value_mm"])?))).collect()
}

impl DesignFacts {
    pub fn from_inspect_text(text: &str) -> Result<DesignFacts, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("not valid JSON: {e}"))?;
        if v["format"] != "fastroute-inspect" {
            return Err("not an inspect.json (\"format\" is not \"fastroute-inspect\")".into());
        }
        if v["format_version"].as_str().and_then(|s| s.split('.').next()) != Some("1") {
            return Err(format!("inspect.json format_version {} is not supported (1.x)", v["format_version"]));
        }
        if v["status"] != "ok" {
            return Err("this inspect.json records a failed run, it has no design data".into());
        }
        let mut f = DesignFacts { inspect_sha256: sha256_hex(text.as_bytes()), input_sha256: v["input"]["sha256"].as_str().map(str::to_string), ..Default::default() };
        for n in v["nets"].as_array().ok_or("no \"nets\" list")? {
            if let Some(name) = n["name"].as_str() {
                f.nets.insert(name.to_string(), n["class"].as_str().unwrap_or("").to_string());
            }
        }
        for c in v["rules"]["net_classes"].as_array().into_iter().flatten() {
            if let Some(name) = c["name"].as_str() {
                f.classes.insert(name.to_string(), ClassFacts { trace_width_mm: per_layer(&c["trace_width_mm"]), clearance_mm: per_layer(&c["clearance_mm"]), max_length_mm: num(&c["max_length_mm"]) });
            }
        }
        f.layers = v["board"]["layers"].as_array().into_iter().flatten().filter_map(|l| l["name"].as_str().map(str::to_string)).collect();
        f.components = v["components"].as_array().into_iter().flatten().filter_map(|c| c["ref"].as_str().map(str::to_string)).collect();
        Ok(f)
    }

    pub fn from_inspect_file(path: &Path) -> Result<DesignFacts, CmdError> {
        let text = std::fs::read_to_string(path).map_err(|e| CmdError::input(format!("{}: {e}", path.display())))?;
        DesignFacts::from_inspect_text(&text).map_err(|e| CmdError::input(format!("{}: {e}", path.display())))
    }

    /// The `inspect.json` of an ai-pack directory, after the pack has been verified.
    pub fn from_ai_pack(dir: &Path) -> Result<DesignFacts, CmdError> {
        let problems = crate::aipack::verify_pack(dir);
        if !problems.is_empty() {
            return Err(CmdError::input(format!("{}: the pack is not consistent: {}", dir.display(), problems.join("; "))));
        }
        DesignFacts::from_inspect_file(&dir.join(crate::aipack::INSPECT))
    }

    pub fn has_layer(&self, name: &str) -> bool {
        self.layers.iter().any(|l| l == name)
    }

    /// Names that are nearly `wanted` (for error messages).
    pub fn similar_nets(&self, wanted: &str) -> Vec<&str> {
        let w = wanted.to_ascii_lowercase();
        self.nets.keys().map(String::as_str).filter(|n| {
            let l = n.to_ascii_lowercase();
            l.contains(&w) || w.contains(&l) || l.trim_start_matches('/') == w.trim_start_matches('/')
        }).take(5).collect()
    }

    /// The classes of `nets` (or of one class, or of all nets), for baseline lookups.
    pub fn classes_of<'a>(&'a self, scope: &Scope) -> Vec<&'a ClassFacts> {
        let names: Vec<&String> = match scope {
            Scope::Nets(nets) => nets.iter().filter_map(|n| self.nets.get(n)).collect(),
            Scope::Class(c) => self.classes.keys().filter(|k| *k == c).collect(),
            Scope::All => self.classes.keys().collect(),
        };
        let mut seen = Vec::new();
        names.into_iter().filter(|n| { let new = !seen.contains(n); seen.push(*n); new }).filter_map(|n| self.classes.get(n)).collect()
    }
}

/// What a rule applies to.
#[derive(Clone, Debug, PartialEq)]
pub enum Scope {
    Nets(Vec<String>),
    Class(String),
    All,
}
