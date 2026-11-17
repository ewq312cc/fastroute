//! The Constraint IR: what an HCP compiles to. A versioned JSON document that the validator and the
//! fastroute adapter read instead of the pack. It keeps the evidence of every rule, says per rule
//! what can and cannot be executed (nothing is silently dropped or approximated), and carries an
//! integrity hash so that a tool can tell it was compiled by a compiler and not edited by hand.

use crate::diag::Diagnostic;
use crate::hcp::{canonical_json, sha256_hex};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const IR_VERSION: &str = "1.0";

pub const IR_SCHEMA_TEXT: &str = include_str!("../../../skills/hardware-constraint-design/schemas/constraint-ir-v1.schema.json");

pub fn ir_schema() -> Value {
    serde_json::from_str(IR_SCHEMA_TEXT).expect("the embedded IR schema is valid JSON")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ir {
    pub ir_version: String,
    pub compiler: Compiler,
    pub pack: PackRef,
    pub sources: Vec<IrSource>,
    pub devices: Vec<IrDevice>,
    pub blocks: Vec<IrBlock>,
    pub regions: Vec<IrRegion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_design: Option<Value>,
    pub rules: Vec<IrRule>,
    /// Warnings of the compilation (errors stop it).
    pub diagnostics: Vec<Diagnostic>,
    pub integrity: Integrity,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Compiler {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackRef {
    pub id: String,
    pub version: String,
    /// SHA-256 of the pack's canonical JSON: the same facts give the same hash.
    pub sha256: String,
    pub review: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Integrity {
    pub algorithm: String,
    /// SHA-256 of the canonical JSON of everything above.
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrSource {
    pub source_id: String,
    pub document_id: String,
    pub title: String,
    pub version: String,
    pub level: String,
    pub sha256: String,
    pub applies_to: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrPin {
    pub number: String,
    pub name: String,
    pub class: String,
    pub functions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrDevice {
    pub id: String,
    pub vendor: String,
    pub part_number: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    pub package: String,
    pub pins: Vec<IrPin>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrEvidence {
    pub source_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_id: Option<String>,
    pub locator: Value,
    pub quote: String,
    pub basis: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrBlock {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub modifiability: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    pub components: Vec<Value>,
    pub connects: Vec<Value>,
    pub evidence: Vec<IrEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrRegion {
    pub id: String,
    pub kind: String,
    pub frame: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polygon: Option<Vec<[f64; 2]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub circle: Option<Value>,
    pub layers: Vec<String>,
    pub modifiability: String,
    pub evidence: Vec<IrEvidence>,
}

/// What can be done with a rule: by the deterministic validator and by the fastroute adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Execution {
    /// `supported` or `unsupported`
    pub validator: String,
    /// `supported`, `unsupported` or `not_applicable` (not a routing rule)
    pub fastroute: String,
    /// Why something is unsupported, in words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Parameters of a supported rule that no check looks at (reported as unchecked).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unchecked_params: Vec<String>,
    /// Parameters that the fastroute adapter cannot hand to the router (reported, never dropped).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unmapped_params: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrRule {
    pub rule_id: String,
    pub kind: String,
    pub modifiability: String,
    pub severity: String,
    /// `executable`, `unsupported`, `unknown` or `conflict`: `unknown` and `conflict` never run.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub subject: Value,
    pub params: Value,
    pub applies_when: Vec<String>,
    pub evidence: Vec<IrEvidence>,
    pub derived_from: Vec<String>,
    pub execution: Execution,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<Value>,
    /// Where the rule comes from: `hcp` (a rule of the pack) or `pin_table` (made from the device's
    /// pin functions).
    pub origin: String,
}

impl Ir {
    /// The hash that `integrity.sha256` must have.
    pub fn compute_integrity(&self) -> String {
        let mut v = serde_json::to_value(self).expect("the IR serialises");
        v.as_object_mut().expect("an object").remove("integrity");
        sha256_hex(canonical_json(&v).as_bytes())
    }

    /// Whether the document is what a compiler wrote (not edited afterwards).
    pub fn verify_integrity(&self) -> bool {
        self.integrity.algorithm == "sha256-canonical-json" && self.integrity.sha256 == self.compute_integrity()
    }

    pub fn to_json_text(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("the IR serialises");
        s.push('\n');
        s
    }

    pub fn rule(&self, id: &str) -> Option<&IrRule> {
        self.rules.iter().find(|r| r.rule_id == id)
    }

    pub fn block(&self, id: &str) -> Option<&IrBlock> {
        self.blocks.iter().find(|b| b.id == id)
    }

    pub fn region(&self, id: &str) -> Option<&IrRegion> {
        self.regions.iter().find(|r| r.id == id)
    }

    pub fn device(&self, id: &str) -> Option<&IrDevice> {
        self.devices.iter().find(|d| d.id == id)
    }
}
