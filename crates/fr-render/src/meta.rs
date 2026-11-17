//! What the renderer reports about each image. These structs are the `images[]` entries of
//! `manifest.json` (see `docs/AI_DATA_FORMAT.md`).

use serde::{Deserialize, Serialize};

use crate::model::Rect;
use crate::view::View;

/// `value_out = scale * value_in + offset`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AxisMap {
    pub scale: f64,
    pub offset: f64,
}

impl AxisMap {
    pub fn apply(&self, v: f64) -> f64 {
        self.scale * v + self.offset
    }
}

/// The region of the image in world coordinates: `x`, `y` is the top-left corner (smallest x,
/// smallest y; y points down), sizes in mm.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegionMm {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl RegionMm {
    pub fn to_rect(&self) -> Rect {
        Rect::new(self.x, self.y, self.x + self.width, self.y + self.height)
    }
}

impl From<Rect> for RegionMm {
    fn from(r: Rect) -> Self {
        RegionMm { x: r.x0, y: r.y0, width: r.width(), height: r.height() }
    }
}

/// World (mm) → pixel and back. `u` is the pixel column, `v` the pixel row, both measured from
/// the top-left *corner* of the image (pixel centres are at +0.5); there is no axis flip.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    /// `u = world_to_pixel.u.scale * x + world_to_pixel.u.offset`, likewise `v` from `y`.
    pub world_to_pixel_u: AxisMap,
    pub world_to_pixel_v: AxisMap,
    /// `x = pixel_to_world.x.scale * u + pixel_to_world.x.offset`, likewise `y` from `v`.
    pub pixel_to_world_x: AxisMap,
    pub pixel_to_world_y: AxisMap,
}

impl Transform {
    pub fn from_view(v: &View) -> Transform {
        let s = v.px_per_mm;
        Transform {
            world_to_pixel_u: AxisMap { scale: s, offset: -v.x0 * s },
            world_to_pixel_v: AxisMap { scale: s, offset: -v.y0 * s },
            pixel_to_world_x: AxisMap { scale: 1.0 / s, offset: v.x0 },
            pixel_to_world_y: AxisMap { scale: 1.0 / s, offset: v.y0 },
        }
    }
    pub fn to_pixel(&self, x: f64, y: f64) -> (f64, f64) {
        (self.world_to_pixel_u.apply(x), self.world_to_pixel_v.apply(y))
    }
    pub fn to_world(&self, u: f64, v: f64) -> (f64, f64) {
        (self.pixel_to_world_x.apply(u), self.pixel_to_world_y.apply(v))
    }
}

/// The objects that touch the image (their geometry reaches into the region, also when it
/// continues outside).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Contains {
    pub components: Vec<String>,
    pub nets: Vec<String>,
    /// Issue ids (`OPEN1`, `CLR2`, `DRC3`) of `inspect.json` whose location is inside the region.
    pub issues: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LabelStats {
    pub drawn: u32,
    /// Not drawn because the label would have covered another one.
    pub skipped_overlap: u32,
    /// Not drawn because the object is too small at this scale.
    pub skipped_small: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageMeta {
    pub width_px: u32,
    pub height_px: u32,
    /// The area the image really shows (contains the requested region).
    pub region_mm: RegionMm,
    pub px_per_mm: f64,
    pub mm_per_px: f64,
    pub transform: Transform,
    /// Copper layers drawn, top to bottom.
    pub layers: Vec<String>,
    /// Annotations switched on: `grid`, `scale`, `layer`, `nets`, `pads`, `refs`, `markers`.
    pub annotations: Vec<String>,
    pub airlines: bool,
    pub contains: Contains,
    pub labels: LabelStats,
}
