//! `manifest.json`: which images exist, what each one shows, and how to convert between the
//! board's millimetres and the image's pixels. Format description: `docs/AI_DATA_FORMAT.md`
//! and `docs/schema/manifest-1.0.schema.json`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fr_render::ImageMeta;
use serde::{Deserialize, Serialize};

use crate::load::FileInfo;

pub const FORMAT: &str = "fastroute-manifest";
pub const FORMAT_VERSION: &str = "1.0";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoordinateSystem {
    pub unit: String,
    pub x_axis: String,
    pub y_axis: String,
    pub origin: String,
    pub pixel_origin: String,
    pub pixel_axes: String,
}

impl CoordinateSystem {
    pub fn standard() -> CoordinateSystem {
        CoordinateSystem {
            unit: "mm".into(),
            x_axis: "right".into(),
            y_axis: "down".into(),
            origin: "origin of the Specctra design (for a KiCad export: the KiCad board origin); y is the DSN y negated, so coordinates equal KiCad's".into(),
            pixel_origin: "top-left corner of the image; pixel centres are at +0.5".into(),
            pixel_axes: "u to the right, v downwards; world and pixel axes are not flipped against each other".into(),
        }
    }
}

/// How the region of an image was chosen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    /// `board`, `region`, `component`, `net` or `issues`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Component references, net names or issue ids the region was built around.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padding_mm: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageEntry {
    /// Path of the PNG, relative to the manifest (absolute if there is no common base).
    pub file: String,
    /// `overview`, `region`, `region-combined` or `region-layer`.
    pub purpose: String,
    pub selection: Selection,
    pub bytes: u64,
    pub sha256: String,
    #[serde(flatten)]
    pub meta: ImageMeta,
}

/// One companion file (`inspect.json`, `route-report.json`, ...).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReportEntry {
    /// Path relative to the manifest, `None` when the file does not exist.
    pub file: Option<String>,
    /// `present`, `not_run`, `unavailable`, `failed`, `timeout`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Size and SHA-256 of the file (when there is one), so that a changed report is noticed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

impl ReportEntry {
    /// An entry for a file that exists in `dir`, with its size and hash.
    pub fn of_file(dir: &Path, name: &str, status: &str, detail: Option<String>) -> ReportEntry {
        let bytes = std::fs::read(dir.join(name)).ok();
        ReportEntry {
            file: Some(name.to_string()),
            status: status.to_string(),
            detail,
            bytes: bytes.as_ref().map(|b| b.len() as u64),
            sha256: bytes.as_ref().map(|b| crate::load::sha256_hex(b)),
        }
    }

    /// An entry for a report that does not exist, with the reason.
    pub fn missing(status: &str, detail: Option<String>) -> ReportEntry {
        ReportEntry { file: None, status: status.to_string(), detail, bytes: None, sha256: None }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErrorEntry {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub format_version: String,
    pub fastroute_version: String,
    pub generated_by: String,
    /// `ok`, `partial` (something could not be produced, see `warnings`) or `error`.
    pub status: String,
    /// The DRC status of the pack (`not_run`, `unavailable`, `error`, `timeout`, `failed`,
    /// `passed`), the same as `inspect.json` `checks.drc.status`; `passed` only from a real report.
    pub drc_status: String,
    pub input: Option<FileInfo>,
    pub session: Option<FileInfo>,
    pub coordinate_system: CoordinateSystem,
    pub images: Vec<ImageEntry>,
    /// `inspect`, `route_report`, `drc_report`, `rules`.
    pub reports: BTreeMap<String, ReportEntry>,
    pub warnings: Vec<String>,
    pub errors: Vec<ErrorEntry>,
}

impl Manifest {
    pub fn new(generated_by: &str, input: Option<FileInfo>, session: Option<FileInfo>) -> Manifest {
        Manifest {
            format: FORMAT.into(),
            format_version: FORMAT_VERSION.into(),
            fastroute_version: env!("CARGO_PKG_VERSION").into(),
            generated_by: generated_by.into(),
            status: "ok".into(),
            drc_status: "not_run".into(),
            input,
            session,
            coordinate_system: CoordinateSystem::standard(),
            images: Vec::new(),
            reports: BTreeMap::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
        }
    }

    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("a manifest serialises");
        s.push('\n');
        s
    }
}

/// `target` as seen from the directory `base`: `a/b.png` for `base/a/b.png`; the absolute path
/// when `target` is not below `base`.
pub fn relative_to(base: &Path, target: &Path) -> String {
    let abs = |p: &Path| {
        let p = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(p) };
        std::fs::canonicalize(&p).unwrap_or_else(|_| match (p.parent().and_then(|d| std::fs::canonicalize(d).ok()), p.file_name()) {
            (Some(d), Some(f)) => d.join(f),
            _ => p,
        })
    };
    let (b, t): (PathBuf, PathBuf) = (abs(base), abs(target));
    match t.strip_prefix(&b) {
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => t.to_string_lossy().into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths() {
        let dir = std::env::temp_dir().join(format!("fastroute-manifest-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        assert_eq!(relative_to(&dir, &dir.join("a.png")), "a.png");
        assert_eq!(relative_to(&dir, &dir.join("sub/b.png")), "sub/b.png");
        let outside = std::env::temp_dir().join("elsewhere.png");
        assert!(relative_to(&dir.join("sub"), &outside).ends_with("elsewhere.png"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
