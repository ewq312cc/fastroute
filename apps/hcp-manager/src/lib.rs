//! hcp-manager: the document, source and knowledge-pack manager behind the hardware-constraint
//! Skill. It owns the inventory of documents, their cache and digests, the text blocks and the
//! keyword search, and checks that the evidence of a pack is really in the cited documents.
//!
//! The inventory and the packs are the facts; the cache, the blocks and any index are derived and
//! can be deleted and rebuilt. Nothing in `fr-constraint` or `fastroute` reads this application:
//! compiling and validating work with the pack alone.

pub mod error;
pub mod evidence;
pub mod fetch;
pub mod index;
pub mod inventory;
pub mod packs;
pub mod search;
pub mod sources;
pub mod text;
pub mod workspace;

/// The version of the command line / JSON contract that the Skill negotiates against.
pub const INTERFACE_VERSION: &str = "1";
