//! Hardware Constraint Packs (HCP): a versioned YAML/JSON description of what a device requires,
//! with evidence, that people, AI models and the deterministic checkers all read. The pack is
//! compiled to a Constraint IR (see `ir`), which the validator and the fastroute adapter consume.
//! Nothing here uses a network, an AI model or a vector index.

pub mod diag;
pub mod hcp;
pub mod jsonschema;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
