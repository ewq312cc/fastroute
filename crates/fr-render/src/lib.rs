//! `fr-render`: offline PNG rendering of PCB geometry.
//!
//! The crate draws a plain, engine-independent description of a board ([`BoardGeometry`], all
//! coordinates in millimetres, y pointing down as in KiCad) with a pure-Rust rasteriser. It needs
//! no window system, browser, network or fonts, and the same input, options and version give
//! the same pixels. `fastroute` fills the geometry from a loaded board.
//!
//! * [`select`] chooses the area (rectangle, component, nets, report coordinates),
//! * [`view`] turns area + size into the exact world↔pixel mapping,
//! * [`draw`] rasterises and reports what is in the image ([`meta::ImageMeta`]).

pub mod draw;
pub mod error;
pub mod font;
pub mod meta;
pub mod model;
pub mod select;
pub mod view;

pub use draw::{check_grid, layer_color, render, Annotations, GridSpec, RenderSpec, Rendered, MIN_LINE_PX};
pub use error::RenderError;
pub use meta::{AxisMap, Contains, ImageMeta, LabelStats, RegionMm, Transform};
pub use model::*;
pub use select::{parse_region_mm, resolve, RegionSpec};
pub use view::{SizeSpec, View, MAX_PIXELS, MAX_SIDE_PX};
