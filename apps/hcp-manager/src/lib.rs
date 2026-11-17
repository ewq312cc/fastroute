//! hcp-manager: the document, source and knowledge-pack manager behind the hardware-constraint
//! Skill. It owns the inventory of documents, their cache and digests, the knowledge packs of a
//! workspace, the embedded index (text blocks and vectors) with keyword, vector and hybrid search,
//! and checks that the evidence of a pack is really in the cited documents.
//!
//! The inventory and the packs are the facts; the cache and the index are derived and can be
//! deleted and rebuilt. Nothing in `fr-constraint` or `fastroute` reads this application: compiling
//! and validating work with the pack alone, with no network, no model and no index.
//!
//! The library is the same code as the command line (`hcp-manager`): `inventory`, `sources`,
//! `index`, `search`, `evidence`, `packs`, `embed` and `store` are the public modules. The contract
//! that other programs negotiate against is [`INTERFACE_VERSION`] and `hcp-manager info`.

pub mod embed;
pub mod error;
pub mod evidence;
pub mod fetch;
pub mod index;
pub mod inventory;
pub mod packs;
pub mod release;
pub mod search;
pub mod store;
pub mod sources;
pub mod text;
pub mod workspace;

/// The version of the command line / JSON contract that the Skill negotiates against.
pub const INTERFACE_VERSION: &str = "1";
