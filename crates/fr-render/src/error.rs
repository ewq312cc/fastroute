//! Errors of the renderer. Every variant has a message that says what to change.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum RenderError {
    /// The board has no outline, so there is no "whole board" to draw.
    NoOutline,
    /// The region is empty, not finite or has no extent.
    BadRegion(String),
    /// The size / scale arguments cannot be used.
    BadSize(String),
    /// The image would be larger than [`crate::MAX_SIDE_PX`] / [`crate::MAX_PIXELS`].
    TooLarge { width: u64, height: u64 },
    /// `--layers` names a layer the board does not have.
    UnknownLayer { name: String, known: Vec<String> },
    /// No layer is selected.
    NoLayers,
    /// A component / net / issue that the selection asks for does not exist.
    NoMatch { kind: &'static str, name: String, hint: String },
    /// The grid step would put the lines closer than a few pixels.
    GridTooDense { step_mm: f64, px: f64 },
    /// PNG encoding failed.
    Encode(String),
    /// Writing the file failed.
    Io(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::NoOutline => write!(f, "the board has no outline (Edge.Cuts / boundary), so there is no board area to render; give a region instead (--region-mm=x,y,w,h)"),
            RenderError::BadRegion(m) => write!(f, "invalid region: {m}"),
            RenderError::BadSize(m) => write!(f, "invalid size: {m}"),
            RenderError::TooLarge { width, height } => write!(
                f,
                "the image would be {width} x {height} pixels; the limit is {} pixels per side and {} megapixels in total (use a smaller --size or --px-per-mm, or a smaller region)",
                crate::MAX_SIDE_PX,
                crate::MAX_PIXELS / 1_000_000
            ),
            RenderError::UnknownLayer { name, known } => write!(f, "unknown layer '{name}' (the board has: {})", known.join(", ")),
            RenderError::NoLayers => write!(f, "no layer selected"),
            RenderError::NoMatch { kind, name, hint } => {
                write!(f, "no {kind} '{name}' on the board")?;
                if !hint.is_empty() {
                    write!(f, " ({hint})")?;
                }
                Ok(())
            }
            RenderError::GridTooDense { step_mm, px } => {
                write!(f, "grid step {step_mm} mm is only {px:.1} pixels at this scale; use a larger --grid step or a larger image")
            }
            RenderError::Encode(m) => write!(f, "PNG encoding failed: {m}"),
            RenderError::Io(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for RenderError {}
