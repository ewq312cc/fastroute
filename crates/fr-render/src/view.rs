//! Region + size → the exact pixel mapping of an image.
//!
//! World coordinates are millimetres, x right, y **down**. Pixel coordinates `(u, v)` have the
//! origin at the top-left *corner* of the top-left pixel (pixel centres are at `+0.5`), u right,
//! v down, so there is no flip between the two:
//!
//! ```text
//! u = (x - x0) * px_per_mm        v = (y - y0) * px_per_mm
//! ```
//!
//! Scale (`px_per_mm`) and pixel size are independent inputs: the same region can be rendered
//! at any scale, and the geometry stays at the same *world* position. The region that is
//! actually drawn (`View::region`) can be a little larger than the requested one because the
//! image has a whole number of pixels (and, with an exact `WxH` size, because of its aspect
//! ratio); it is always centred on the requested region and always contains it.

use crate::error::RenderError;
use crate::model::{Pt, Rect};

/// Largest image side, in pixels.
pub const MAX_SIDE_PX: u32 = 16384;
/// Largest image, in pixels (RGBA, 4 bytes each).
pub const MAX_PIXELS: u64 = 100_000_000;

/// How the output size is chosen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SizeSpec {
    /// The longer side of the image has this many pixels; the other follows the aspect ratio.
    LongSide(u32),
    /// Exactly this many pixels; the region grows on one axis to fill the aspect ratio.
    Exact(u32, u32),
    /// A fixed scale in pixels per millimetre; the image size follows from the region.
    PxPerMm(f64),
}

impl SizeSpec {
    /// Parses `N` (long side), `WxH` (exact) or `Npx/mm` is *not* accepted here: the scale has
    /// its own option.
    pub fn parse(s: &str) -> Result<SizeSpec, RenderError> {
        let bad = || RenderError::BadSize(format!("'{s}' is neither N nor WxH (pixels)"));
        let s = s.trim();
        let num = |t: &str| t.trim().parse::<u32>().map_err(|_| bad());
        let spec = match s.split_once(['x', 'X']) {
            Some((w, h)) => SizeSpec::Exact(num(w)?, num(h)?),
            None => SizeSpec::LongSide(num(s)?),
        };
        match spec {
            SizeSpec::LongSide(0) | SizeSpec::Exact(0, _) | SizeSpec::Exact(_, 0) => Err(RenderError::BadSize(format!("'{s}': the size must be at least 1 pixel"))),
            _ => Ok(spec),
        }
    }
}

/// The mapping between world (mm) and pixels of one image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// World x of the left edge of the image.
    pub x0: f64,
    /// World y of the top edge of the image.
    pub y0: f64,
    pub px_per_mm: f64,
    pub width_px: u32,
    pub height_px: u32,
}

impl View {
    /// Chooses the image for `region` (the area that has to be visible).
    pub fn fit(region: Rect, size: SizeSpec) -> Result<View, RenderError> {
        let (w, h) = (region.width(), region.height());
        if ![region.x0, region.y0, region.x1, region.y1].iter().all(|v| v.is_finite()) {
            return Err(RenderError::BadRegion("the region is not finite".into()));
        }
        if w <= 0.0 || h <= 0.0 {
            return Err(RenderError::BadRegion(format!("the region is {w} x {h} mm; width and height must be positive")));
        }
        let (px_per_mm, width_px, height_px) = match size {
            SizeSpec::LongSide(n) => {
                if n == 0 {
                    return Err(RenderError::BadSize("the size must be at least 1 pixel".into()));
                }
                let s = n as f64 / w.max(h);
                (s, ceil_px(w * s), ceil_px(h * s))
            }
            SizeSpec::Exact(pw, ph) => {
                if pw == 0 || ph == 0 {
                    return Err(RenderError::BadSize("the size must be at least 1 pixel".into()));
                }
                ((pw as f64 / w).min(ph as f64 / h), pw as u64, ph as u64)
            }
            SizeSpec::PxPerMm(s) => {
                if !(s.is_finite() && s > 0.0) {
                    return Err(RenderError::BadSize(format!("the scale must be a positive number of pixels per mm, not {s}")));
                }
                (s, ceil_px(w * s), ceil_px(h * s))
            }
        };
        if width_px == 0 || height_px == 0 {
            return Err(RenderError::BadSize("the image would be empty".into()));
        }
        if width_px > MAX_SIDE_PX as u64 || height_px > MAX_SIDE_PX as u64 || width_px * height_px > MAX_PIXELS {
            return Err(RenderError::TooLarge { width: width_px, height: height_px });
        }
        let c = region.center();
        Ok(View {
            x0: c.x - width_px as f64 / px_per_mm / 2.0,
            y0: c.y - height_px as f64 / px_per_mm / 2.0,
            px_per_mm,
            width_px: width_px as u32,
            height_px: height_px as u32,
        })
    }

    /// World → pixel (`u`, `v`).
    pub fn to_px(&self, p: Pt) -> (f64, f64) {
        ((p.x - self.x0) * self.px_per_mm, (p.y - self.y0) * self.px_per_mm)
    }

    /// Pixel (`u`, `v`) → world.
    pub fn to_mm(&self, u: f64, v: f64) -> Pt {
        Pt::new(u / self.px_per_mm + self.x0, v / self.px_per_mm + self.y0)
    }

    pub fn mm_per_px(&self) -> f64 {
        1.0 / self.px_per_mm
    }

    /// The world area the image shows.
    pub fn region(&self) -> Rect {
        Rect::new(self.x0, self.y0, self.x0 + self.width_px as f64 / self.px_per_mm, self.y0 + self.height_px as f64 / self.px_per_mm)
    }
}

fn ceil_px(v: f64) -> u64 {
    // (a value like 2048.0000000001 must not become 2049 pixels)
    let r = (v - 1e-6).ceil();
    if r.is_finite() && r >= 1.0 {
        r.min(u64::MAX as f64 / 2.0) as u64
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_specs() {
        assert_eq!(SizeSpec::parse("2048").unwrap(), SizeSpec::LongSide(2048));
        assert_eq!(SizeSpec::parse("800x600").unwrap(), SizeSpec::Exact(800, 600));
        assert_eq!(SizeSpec::parse("800X600").unwrap(), SizeSpec::Exact(800, 600));
        for bad in ["", "0", "x", "10x", "-4", "1.5", "axb", "10x0"] {
            assert!(SizeSpec::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn long_side_keeps_aspect_and_contains_the_region() {
        let r = Rect::new(10.0, 20.0, 60.0, 50.0); // 50 x 30
        let v = View::fit(r, SizeSpec::LongSide(1000)).unwrap();
        assert_eq!((v.width_px, v.height_px), (1000, 600));
        assert!((v.px_per_mm - 20.0).abs() < 1e-9);
        let a = v.region();
        assert!(a.x0 <= r.x0 + 1e-9 && a.y0 <= r.y0 + 1e-9 && a.x1 >= r.x1 - 1e-9 && a.y1 >= r.y1 - 1e-9);
        assert!((v.to_px(Pt::new(10.0, 20.0)).0).abs() < 1e-9);
    }

    #[test]
    fn exact_size_grows_one_axis_around_the_centre() {
        let r = Rect::new(0.0, 0.0, 40.0, 10.0);
        let v = View::fit(r, SizeSpec::Exact(400, 400)).unwrap();
        assert_eq!((v.width_px, v.height_px), (400, 400));
        assert!((v.px_per_mm - 10.0).abs() < 1e-9);
        let a = v.region();
        assert!((a.x0 - 0.0).abs() < 1e-9 && (a.x1 - 40.0).abs() < 1e-9);
        assert!((a.y0 - (-15.0)).abs() < 1e-9 && (a.y1 - 25.0).abs() < 1e-9);
        assert!((a.center().y - 5.0).abs() < 1e-9);
    }

    #[test]
    fn scale_and_size_are_independent_and_round_trip() {
        let r = Rect::new(-3.0, 7.0, 9.5, 12.25);
        for s in [0.5, 1.0, 7.0, 40.0] {
            let v = View::fit(r, SizeSpec::PxPerMm(s)).unwrap();
            for p in [Pt::new(-3.0, 7.0), Pt::new(0.123, 9.876), Pt::new(9.5, 12.25)] {
                let (u, w) = v.to_px(p);
                let q = v.to_mm(u, w);
                assert!((q.x - p.x).abs() < 1e-9 && (q.y - p.y).abs() < 1e-9);
            }
        }
        // the same world point sits at proportional pixels in two scales of one region
        let a = View::fit(r, SizeSpec::PxPerMm(10.0)).unwrap();
        let b = View::fit(r, SizeSpec::PxPerMm(20.0)).unwrap();
        let p = Pt::new(2.0, 9.0);
        let (ua, va) = a.to_px(p);
        let (ub, vb) = b.to_px(p);
        // (the images are centred on the same region, so the offset of the two differs by at
        // most the rounding of one pixel of each)
        assert!((ub / 2.0 - ua).abs() <= 1.0 && (vb / 2.0 - va).abs() <= 1.0);
    }

    #[test]
    fn rejects_bad_input() {
        let ok = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(matches!(View::fit(Rect::new(0.0, 0.0, 0.0, 5.0), SizeSpec::LongSide(100)), Err(RenderError::BadRegion(_))));
        assert!(matches!(View::fit(Rect::new(0.0, 0.0, f64::NAN, 5.0), SizeSpec::LongSide(100)), Err(RenderError::BadRegion(_))));
        assert!(matches!(View::fit(ok, SizeSpec::LongSide(0)), Err(RenderError::BadSize(_))));
        assert!(matches!(View::fit(ok, SizeSpec::PxPerMm(-1.0)), Err(RenderError::BadSize(_))));
        assert!(matches!(View::fit(ok, SizeSpec::PxPerMm(f64::NAN)), Err(RenderError::BadSize(_))));
        assert!(matches!(View::fit(ok, SizeSpec::LongSide(20000)), Err(RenderError::TooLarge { .. })));
        assert!(matches!(View::fit(ok, SizeSpec::Exact(16000, 16000)), Err(RenderError::TooLarge { .. })));
    }

    #[test]
    fn exact_pixel_counts_do_not_gain_a_pixel_from_rounding() {
        let r = Rect::new(0.0, 0.0, 3.0, 1.0);
        assert_eq!(View::fit(r, SizeSpec::LongSide(2048)).unwrap().width_px, 2048);
        assert_eq!(View::fit(Rect::new(0.0, 0.0, 0.1, 0.1), SizeSpec::LongSide(3)).unwrap().width_px, 3);
    }
}
