//! The plain geometry the renderer draws: everything in millimetres, x to the right, **y
//! downwards** (KiCad's convention, the same coordinates `--report` prints). It knows nothing
//! about the routing engine; `fastroute` fills it from a loaded board, tests build it by hand.

use crate::error::RenderError;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

impl Pt {
    pub fn new(x: f64, y: f64) -> Pt {
        Pt { x, y }
    }
    pub fn is_finite(&self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

/// An axis-parallel rectangle `[x0, x1] x [y0, y1]` (mm, y down), `x0 <= x1`, `y0 <= y1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Rect {
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
        Rect { x0: x0.min(x1), y0: y0.min(y1), x1: x0.max(x1), y1: y0.max(y1) }
    }
    pub fn from_points<'a>(pts: impl IntoIterator<Item = &'a Pt>) -> Option<Rect> {
        let mut it = pts.into_iter().filter(|p| p.is_finite());
        let first = it.next()?;
        let mut r = Rect { x0: first.x, y0: first.y, x1: first.x, y1: first.y };
        for p in it {
            r.x0 = r.x0.min(p.x);
            r.y0 = r.y0.min(p.y);
            r.x1 = r.x1.max(p.x);
            r.y1 = r.y1.max(p.y);
        }
        Some(r)
    }
    pub fn width(&self) -> f64 {
        self.x1 - self.x0
    }
    pub fn height(&self) -> f64 {
        self.y1 - self.y0
    }
    pub fn center(&self) -> Pt {
        Pt::new((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0)
    }
    pub fn union(&self, o: &Rect) -> Rect {
        Rect { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }
    pub fn expand(&self, d: f64) -> Rect {
        Rect { x0: self.x0 - d, y0: self.y0 - d, x1: self.x1 + d, y1: self.y1 + d }
    }
    /// Closed-interval overlap (touching rectangles intersect).
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x0 <= o.x1 && o.x0 <= self.x1 && self.y0 <= o.y1 && o.y0 <= self.y1
    }
    pub fn contains(&self, p: Pt) -> bool {
        p.x >= self.x0 && p.x <= self.x1 && p.y >= self.y0 && p.y <= self.y1
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub name: String,
    /// A routing layer (as opposed to e.g. a power plane layer).
    pub signal: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PadShape {
    Circle { c: Pt, r: f64 },
    Poly(Vec<Pt>),
}

impl PadShape {
    pub fn bbox(&self) -> Option<Rect> {
        match self {
            PadShape::Circle { c, r } => Some(Rect::new(c.x - r, c.y - r, c.x + r, c.y + r)),
            PadShape::Poly(p) => Rect::from_points(p),
        }
    }
    pub fn center(&self) -> Option<Pt> {
        match self {
            PadShape::Circle { c, .. } => Some(*c),
            PadShape::Poly(_) => self.bbox().map(|b| b.center()),
        }
    }
}

/// One pad on one layer (a through-hole pad appears once per layer it spans).
#[derive(Clone, Debug, PartialEq)]
pub struct Pad {
    pub layer: usize,
    /// Net number (index into [`BoardGeometry::nets`]); 0 = no net.
    pub net: usize,
    pub component: String,
    pub pin: String,
    /// The pad's centre as the router uses it: where connections and unrouted connections end.
    /// For a pad whose copper is not centred on its anchor (a KiCad custom-shaped pad) this is
    /// the centre of the copper, not KiCad's pad position.
    pub center: Pt,
    pub shape: PadShape,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Trace {
    pub layer: usize,
    pub net: usize,
    pub width: f64,
    pub pts: Vec<Pt>,
    pub fixed: bool,
}

impl Trace {
    pub fn bbox(&self) -> Option<Rect> {
        Rect::from_points(&self.pts).map(|r| r.expand(self.width / 2.0))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Via {
    pub c: Pt,
    /// Outer radius.
    pub r: f64,
    pub net: usize,
    pub first_layer: usize,
    pub last_layer: usize,
    pub fixed: bool,
}

impl Via {
    pub fn bbox(&self) -> Rect {
        Rect::new(self.c.x - self.r, self.c.y - self.r, self.c.x + self.r, self.c.y + self.r)
    }
    pub fn spans(&self, layer: usize) -> bool {
        layer >= self.first_layer && layer <= self.last_layer
    }
}

/// A filled polygon on one layer: a plane / zone or a keepout.
#[derive(Clone, Debug, PartialEq)]
pub struct Area {
    pub layer: usize,
    pub net: usize,
    pub pts: Vec<Pt>,
}

/// An unrouted connection (airline) between two points.
#[derive(Clone, Debug, PartialEq)]
pub struct Airline {
    pub net: usize,
    pub from: Pt,
    pub to: Pt,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Component {
    pub name: String,
    pub center: Pt,
    /// Everything that belongs to the package: pads, outline, component keepouts.
    pub bbox: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueKind {
    /// An unrouted connection (`OPEN..`).
    Unrouted,
    /// A clearance violation found by fastroute's own check (`CLR..`).
    Violation,
    /// A violation or unconnected item reported by an external checker, e.g. KiCad DRC (`DRC..`).
    Drc,
}

/// A problem at a place on the board; drawn as a numbered marker when asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    /// Stable id referenced from `inspect.json` / `manifest.json`: `OPEN1`, `CLR3`, `DRC2`. (Not
    /// `U1`/`D1`/`V1`: those are component references on real boards.)
    pub id: String,
    pub kind: IssueKind,
    /// Where the marker is drawn (for a connection: the middle of its airline).
    pub at: Pt,
    /// Everything the issue covers (both ends of a connection, the area of a violation);
    /// a point issue has a zero-size extent at `at`.
    pub extent: Rect,
    pub layer: Option<usize>,
    pub net: Option<usize>,
}

impl Issue {
    /// A point issue (extent = `at`).
    pub fn at_point(id: impl Into<String>, kind: IssueKind, at: Pt) -> Issue {
        Issue { id: id.into(), kind, at, extent: Rect { x0: at.x, y0: at.y, x1: at.x, y1: at.y }, layer: None, net: None }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoardGeometry {
    pub name: String,
    pub layers: Vec<Layer>,
    /// Net names; index 0 is the empty "no net".
    pub nets: Vec<String>,
    /// Closed outline polygons (a hole in the board is a second polygon).
    pub outline: Vec<Vec<Pt>>,
    pub pads: Vec<Pad>,
    pub traces: Vec<Trace>,
    pub vias: Vec<Via>,
    pub planes: Vec<Area>,
    pub keepouts: Vec<Area>,
    pub airlines: Vec<Airline>,
    pub components: Vec<Component>,
    pub issues: Vec<Issue>,
}

impl BoardGeometry {
    pub fn net_name(&self, net: usize) -> &str {
        self.nets.get(net).map(String::as_str).unwrap_or("")
    }

    /// Bounding box of the board outline, `None` if there is none.
    pub fn outline_bbox(&self) -> Option<Rect> {
        Rect::from_points(self.outline.iter().flatten())
    }

    /// Layer indices for a `--layers` argument: comma separated layer names (any case) or
    /// indices, or `all` / `signal` / `top` / `bottom`. Order of the result: board order.
    pub fn parse_layers(&self, spec: &str) -> Result<Vec<usize>, RenderError> {
        let mut picked = vec![false; self.layers.len()];
        let mut any = false;
        for word in spec.split(',').map(str::trim).filter(|w| !w.is_empty()) {
            any = true;
            match word.to_ascii_lowercase().as_str() {
                "all" => picked.iter_mut().for_each(|p| *p = true),
                "signal" => {
                    for (i, l) in self.layers.iter().enumerate() {
                        picked[i] |= l.signal;
                    }
                }
                "top" => {
                    if let Some(p) = picked.first_mut() {
                        *p = true;
                    }
                }
                "bottom" => {
                    if let Some(p) = picked.last_mut() {
                        *p = true;
                    }
                }
                _ => {
                    let by_name = self.layers.iter().position(|l| l.name.eq_ignore_ascii_case(word));
                    match by_name {
                        Some(i) => picked[i] = true,
                        None => {
                            return Err(RenderError::UnknownLayer {
                                name: word.to_string(),
                                known: self.layers.iter().map(|l| l.name.clone()).collect(),
                            })
                        }
                    }
                }
            }
        }
        if !any {
            return Err(RenderError::NoLayers);
        }
        let layers: Vec<usize> = picked.iter().enumerate().filter(|(_, p)| **p).map(|(i, _)| i).collect();
        if layers.is_empty() {
            return Err(RenderError::NoLayers);
        }
        Ok(layers)
    }

    pub fn all_layers(&self) -> Vec<usize> {
        (0..self.layers.len()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> BoardGeometry {
        BoardGeometry {
            layers: vec![
                Layer { name: "F.Cu".into(), signal: true },
                Layer { name: "In1.Cu".into(), signal: false },
                Layer { name: "B.Cu".into(), signal: true },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn layer_specs() {
        let b = board();
        assert_eq!(b.parse_layers("all").unwrap(), vec![0, 1, 2]);
        assert_eq!(b.parse_layers("signal").unwrap(), vec![0, 2]);
        assert_eq!(b.parse_layers("b.cu, f.cu").unwrap(), vec![0, 2]);
        assert_eq!(b.parse_layers("top").unwrap(), vec![0]);
        assert_eq!(b.parse_layers("bottom").unwrap(), vec![2]);
        assert!(matches!(b.parse_layers("In9.Cu"), Err(RenderError::UnknownLayer { .. })));
        assert!(matches!(b.parse_layers(" , "), Err(RenderError::NoLayers)));
        assert!(matches!(b.parse_layers(""), Err(RenderError::NoLayers)));
    }

    #[test]
    fn rect_helpers() {
        let r = Rect::new(5.0, 4.0, 1.0, 2.0);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (1.0, 2.0, 5.0, 4.0));
        assert!(r.intersects(&Rect::new(5.0, 4.0, 9.0, 9.0)), "touching rectangles intersect");
        assert!(!r.intersects(&Rect::new(5.1, 4.0, 9.0, 9.0)));
        assert_eq!(Rect::from_points(&[Pt::new(f64::NAN, 0.0)]), None);
    }
}
