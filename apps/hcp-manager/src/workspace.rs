//! The workspace: one directory that holds the inventory (the source of truth) and everything
//! derived from it. Deleting `cache/` or `derived/` loses nothing that cannot be fetched or rebuilt.
//!
//! ```text
//! <workspace>/sources.yaml              the inventory of documents
//! <workspace>/cache/<DOC>/<sha256>.<ext>  a cached document, named by its digest
//! <workspace>/cache/<DOC>/meta.json     which file is current, its digest, where it came from
//! <workspace>/derived/chunks/<DOC>.jsonl  text blocks with stable ids (rebuildable)
//! <workspace>/derived/index.json        what the blocks were built from
//! <workspace>/packs/...                 knowledge packs (HCP YAML)
//! ```

use crate::error::{Error, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
}

impl Workspace {
    /// `--workspace`, else `$HCP_MANAGER_HOME`, else `./.hcp`.
    pub fn resolve(arg: Option<PathBuf>) -> Workspace {
        let root = arg.or_else(|| std::env::var_os("HCP_MANAGER_HOME").map(PathBuf::from)).unwrap_or_else(|| PathBuf::from(".hcp"));
        Workspace { root }
    }

    pub fn inventory_path(&self) -> PathBuf {
        self.root.join("sources.yaml")
    }
    pub fn cache_dir(&self, doc: &str) -> PathBuf {
        self.root.join("cache").join(doc)
    }
    pub fn meta_path(&self, doc: &str) -> PathBuf {
        self.cache_dir(doc).join("meta.json")
    }
    pub fn derived_dir(&self) -> PathBuf {
        self.root.join("derived")
    }
    pub fn chunks_path(&self, doc: &str) -> PathBuf {
        self.derived_dir().join("chunks").join(format!("{doc}.jsonl"))
    }
    pub fn index_path(&self) -> PathBuf {
        self.derived_dir().join("index.json")
    }
    pub fn packs_dir(&self) -> PathBuf {
        self.root.join("packs")
    }

    pub fn init(&self) -> Result<()> {
        for d in [self.root.clone(), self.root.join("cache"), self.derived_dir(), self.packs_dir()] {
            std::fs::create_dir_all(&d).map_err(|e| Error::output(format!("{}: {e}", d.display())))?;
        }
        Ok(())
    }
}

/// Writes through a temporary file and a rename, so that a failure never leaves half a file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::output(format!("{}: {e}", dir.display())))?;
    }
    let tmp = path.with_file_name(format!(".{}.tmp", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
    std::fs::write(&tmp, bytes).map_err(|e| Error::output(format!("{}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        Error::output(format!("{}: {e}", path.display()))
    })
}

/// Today (UTC) as `YYYY-MM-DD`; `$HCP_TODAY` overrides it (tests, reproducible runs).
pub fn today() -> String {
    if let Ok(d) = std::env::var("HCP_TODAY") {
        return d;
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let days = secs.div_euclid(86_400);
    // civil from days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}
