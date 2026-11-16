//! Hardware Constraint Packs (HCP): a versioned YAML/JSON description of what a device requires,
//! with evidence, that people, AI models and the deterministic checkers all read. The pack is
//! compiled to a Constraint IR (`ir`, `compile`), which the validator (`validate`) and the
//! fastroute adapter consume together with a design snapshot (`snapshot`).
//! Nothing here uses a network, an AI model, a vector index or the clock.

pub mod compile;
pub mod diag;
pub mod export;
pub mod geom;
pub mod hcp;
pub mod ir;
pub mod jsonschema;
pub mod kicad;
pub mod pack_report;
pub mod report;
pub mod snapshot;
pub mod validate;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
