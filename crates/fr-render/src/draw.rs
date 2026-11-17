//! Rasterisation of a [`BoardGeometry`] into a PNG-ready pixmap.
//!
//! Always drawn from the vector geometry at the requested scale (never cropped or enlarged
//! from another image). Nothing is clipped before drawing, so a trace or via that crosses the
//! region border is drawn up to the border; the image edge does the clipping.
//!
//! Draw order (bottom to top): background, grid, planes, keepouts, per layer from the bottom
//! one up: traces then pads, vias, board outline, airlines, issue markers, labels, scale bar
//! and layer badge. Thin things never vanish: lines are at least [`MIN_LINE_PX`] wide.

use tiny_skia::{FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Stroke, StrokeDash, Transform};

use crate::error::RenderError;
use crate::font;
use crate::meta::{Contains, ImageMeta, LabelStats, RegionMm, Transform as WorldTransform};
use crate::model::{Area, BoardGeometry, IssueKind, PadShape, Pt, Rect};
use crate::view::{SizeSpec, View};

/// Thinnest line that is drawn, in pixels.
pub const MIN_LINE_PX: f32 = 1.0;

const BACKGROUND: [u8; 4] = [14, 17, 22, 255];
const OUTLINE: [u8; 4] = [246, 230, 99, 255];
const AIRLINE: [u8; 4] = [255, 255, 255, 235];
const KEEPOUT: [u8; 3] = [255, 40, 210];
const VIA_RING: [u8; 4] = [215, 215, 215, 255];
const VIA_HOLE: [u8; 4] = [14, 17, 22, 255];
const GRID: [u8; 4] = [255, 255, 255, 38];
const GRID_MAJOR: [u8; 4] = [255, 255, 255, 70];
const TEXT: [u8; 4] = [255, 255, 255, 255];
const HALO: [u8; 4] = [0, 0, 0, 230];

/// Colour of copper layer `i` of `n`: top red, bottom blue, inner layers in a fixed cycle.
pub fn layer_color(i: usize, n: usize) -> [u8; 3] {
    const INNER: [[u8; 3]; 6] = [[90, 225, 120], [255, 214, 90], [196, 130, 255], [80, 225, 225], [255, 156, 70], [236, 236, 236]];
    if i == 0 {
        [255, 84, 84]
    } else if i + 1 == n {
        [84, 142, 255]
    } else {
        INNER[(i - 1) % INNER.len()]
    }
}

fn marker_color(k: IssueKind) -> [u8; 4] {
    match k {
        IssueKind::Unrouted => [255, 165, 0, 255],
        IssueKind::Violation => [255, 0, 170, 255],
        IssueKind::Drc => [0, 240, 255, 255],
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GridSpec {
    Off,
    /// A step chosen from the scale (1, 2 or 5 times a power of ten mm, about 60 px apart).
    Auto,
    Step(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Annotations {
    pub grid: GridSpec,
    pub scale_bar: bool,
    pub layer_names: bool,
    pub net_names: bool,
    pub pad_names: bool,
    pub ref_des: bool,
    pub markers: bool,
}

impl Default for Annotations {
    fn default() -> Self {
        Annotations { grid: GridSpec::Off, scale_bar: true, layer_names: true, net_names: false, pad_names: false, ref_des: false, markers: true }
    }
}

impl Annotations {
    pub fn none() -> Annotations {
        Annotations { grid: GridSpec::Off, scale_bar: false, layer_names: false, net_names: false, pad_names: false, ref_des: false, markers: false }
    }
    pub fn all() -> Annotations {
        Annotations { grid: GridSpec::Auto, scale_bar: true, layer_names: true, net_names: true, pad_names: true, ref_des: true, markers: true }
    }

    /// `scale,layer,grid,grid=2.5,nets,pads,refs,markers` | `all` | `none`, comma separated.
    /// The list replaces the default (`scale,layer,markers`).
    pub fn parse(list: &str) -> Result<Annotations, String> {
        let mut a = Annotations::none();
        for w in list.split(',').map(str::trim).filter(|w| !w.is_empty()) {
            match w.to_ascii_lowercase().as_str() {
                "none" => a = Annotations::none(),
                "all" => a = Annotations::all(),
                "grid" => a.grid = GridSpec::Auto,
                "scale" | "scale-bar" => a.scale_bar = true,
                "layer" | "layers" => a.layer_names = true,
                "nets" | "net" => a.net_names = true,
                "pads" | "pad" => a.pad_names = true,
                "refs" | "ref" => a.ref_des = true,
                "markers" | "issues" => a.markers = true,
                g if g.starts_with("grid=") => {
                    let step: f64 = g["grid=".len()..].trim_end_matches("mm").parse().map_err(|_| format!("bad grid step in '{w}' (mm)"))?;
                    if !(step.is_finite() && step > 0.0) {
                        return Err(format!("the grid step must be positive: '{w}'"));
                    }
                    a.grid = GridSpec::Step(step);
                }
                _ => return Err(format!("unknown annotation '{w}' (grid, grid=MM, scale, layer, nets, pads, refs, markers, all, none)")),
            }
        }
        Ok(a)
    }

    pub fn names(&self) -> Vec<String> {
        let mut v = Vec::new();
        if self.grid != GridSpec::Off {
            v.push("grid".to_string());
        }
        for (on, n) in [
            (self.scale_bar, "scale"),
            (self.layer_names, "layer"),
            (self.net_names, "nets"),
            (self.pad_names, "pads"),
            (self.ref_des, "refs"),
            (self.markers, "markers"),
        ] {
            if on {
                v.push(n.to_string());
            }
        }
        v
    }
}

#[derive(Clone, Debug)]
pub struct RenderSpec {
    /// The area that has to be visible (mm, y down).
    pub region: Rect,
    pub size: SizeSpec,
    /// Layer indices to draw; sorted and deduplicated by [`render`].
    pub layers: Vec<usize>,
    pub annotations: Annotations,
    /// Draw the unrouted connections.
    pub airlines: bool,
    /// Integer scale of the 5x7 label font; `None` = from the image size.
    pub text_scale: Option<u32>,
}

impl RenderSpec {
    pub fn new(region: Rect, size: SizeSpec, layers: Vec<usize>) -> RenderSpec {
        RenderSpec { region, size, layers, annotations: Annotations::default(), airlines: true, text_scale: None }
    }
}

pub struct Rendered {
    pub pixmap: Pixmap,
    pub meta: ImageMeta,
}

impl Rendered {
    pub fn png_bytes(&self) -> Result<Vec<u8>, RenderError> {
        self.pixmap.encode_png().map_err(|e| RenderError::Encode(e.to_string()))
    }

    /// Writes the PNG to `path` through a temporary file, so a failure never leaves half an image.
    pub fn save_png(&self, path: &std::path::Path) -> Result<(), RenderError> {
        let bytes = self.png_bytes()?;
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = std::path::PathBuf::from(tmp);
        std::fs::write(&tmp, bytes).map_err(|e| RenderError::Io(format!("{}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            RenderError::Io(format!("{}: {e}", path.display()))
        })
    }

    /// RGBA of a pixel (not premultiplied); `None` outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        self.pixmap.pixel(x, y).map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
    }
}

/// Renders `geom` as described by `spec`.
pub fn render(geom: &BoardGeometry, spec: &RenderSpec) -> Result<Rendered, RenderError> {
    let mut layers = spec.layers.clone();
    layers.sort_unstable();
    layers.dedup();
    if layers.is_empty() {
        return Err(RenderError::NoLayers);
    }
    if let Some(&bad) = layers.iter().find(|&&l| l >= geom.layers.len()) {
        return Err(RenderError::UnknownLayer { name: format!("#{bad}"), known: geom.layers.iter().map(|l| l.name.clone()).collect() });
    }
    let view = View::fit(spec.region, spec.size)?;
    let mut pixmap = Pixmap::new(view.width_px, view.height_px).ok_or(RenderError::TooLarge { width: view.width_px as u64, height: view.height_px as u64 })?;
    pixmap.fill(color(BACKGROUND));
    let long = view.width_px.max(view.height_px);
    let text_scale = spec.text_scale.unwrap_or_else(|| ((long as f32 / 700.0).round() as u32).clamp(1, 4)).max(1);
    let mut cx = Cx {
        geom,
        view,
        pm: pixmap,
        layers,
        n_layers: geom.layers.len(),
        spec,
        text_scale,
        placed: Vec::new(),
        stats: LabelStats::default(),
        contains: ContainsSets::default(),
    };
    cx.draw();
    let meta = cx.finish();
    Ok(Rendered { pixmap: cx.pm, meta })
}

fn color(c: [u8; 4]) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c[0], c[1], c[2], c[3])
}

fn paint(c: [u8; 4], aa: bool) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = aa;
    p
}

fn lighten(c: [u8; 3], f: f32) -> [u8; 3] {
    [0, 1, 2].map(|i| (c[i] as f32 + (255.0 - c[i] as f32) * f).round() as u8)
}

#[derive(Default)]
struct ContainsSets {
    components: std::collections::BTreeSet<String>,
    nets: std::collections::BTreeSet<String>,
    issues: Vec<String>,
}

/// Rectangles (px) of the labels already placed; a new label must not overlap one.
type PlacedRect = (f32, f32, f32, f32);

struct Cx<'a> {
    geom: &'a BoardGeometry,
    view: View,
    pm: Pixmap,
    layers: Vec<usize>,
    n_layers: usize,
    spec: &'a RenderSpec,
    text_scale: u32,
    placed: Vec<PlacedRect>,
    stats: LabelStats,
    contains: ContainsSets,
}

const MAX_LABELS: usize = 4000;

impl Cx<'_> {
    fn px(&self, p: Pt) -> (f32, f32) {
        let (u, v) = self.view.to_px(p);
        (u as f32, v as f32)
    }

    fn visible(&self, bbox: Option<Rect>) -> bool {
        bbox.is_some_and(|b| b.intersects(&self.view.region()))
    }

    fn layer_rgb(&self, l: usize) -> [u8; 3] {
        layer_color(l, self.n_layers)
    }

    fn selected(&self, l: usize) -> bool {
        self.layers.binary_search(&l).is_ok()
    }

    fn poly_path(&self, pts: &[Pt], close: bool) -> Option<Path> {
        if pts.len() < 2 || !pts.iter().all(Pt::is_finite) {
            return None;
        }
        let mut pb = PathBuilder::new();
        let (x, y) = self.px(pts[0]);
        pb.move_to(x, y);
        for p in &pts[1..] {
            let (x, y) = self.px(*p);
            pb.line_to(x, y);
        }
        if close {
            pb.close();
        }
        pb.finish()
    }

    fn fill_area(&mut self, a: &Area, c: [u8; 4]) {
        if a.pts.len() < 3 || !self.visible(Rect::from_points(&a.pts)) {
            return;
        }
        if let Some(p) = self.poly_path(&a.pts, true) {
            self.pm.fill_path(&p, &paint(c, true), FillRule::Winding, Transform::identity(), None);
        }
    }

    fn stroke(&mut self, path: &Path, c: [u8; 4], width: f32, round: bool, dash: Option<&[f32]>) {
        let stroke = Stroke {
            width: width.max(MIN_LINE_PX),
            line_cap: if round { LineCap::Round } else { LineCap::Butt },
            line_join: if round { LineJoin::Round } else { LineJoin::Miter },
            dash: dash.and_then(|d| StrokeDash::new(d.to_vec(), 0.0)),
            ..Stroke::default()
        };
        self.pm.stroke_path(path, &paint(c, true), &stroke, Transform::identity(), None);
    }

    fn circle(&mut self, c: Pt, r_mm: f64, col: [u8; 4]) {
        let (x, y) = self.px(c);
        let r = (r_mm * self.view.px_per_mm) as f32;
        if let Some(p) = PathBuilder::from_circle(x, y, r.max(0.5)) {
            self.pm.fill_path(&p, &paint(col, true), FillRule::Winding, Transform::identity(), None);
        }
    }

    fn draw(&mut self) {
        let region = self.view.region();
        if self.spec.annotations.grid != GridSpec::Off {
            self.draw_grid_lines();
        }
        // planes and keepouts of the selected layers
        for l in self.layers.clone().into_iter().rev() {
            let rgb = self.layer_rgb(l);
            for a in self.geom.planes.iter().filter(|a| a.layer == l) {
                if self.visible(Rect::from_points(&a.pts)) {
                    self.note_net(a.net);
                }
                self.fill_area(a, [rgb[0], rgb[1], rgb[2], 64]);
            }
        }
        let selected_layers = self.layers.clone();
        for a in self.geom.keepouts.iter().filter(|a| selected_layers.binary_search(&a.layer).is_ok()) {
            self.fill_area(a, [KEEPOUT[0], KEEPOUT[1], KEEPOUT[2], 40]);
            if let Some(p) = self.poly_path(&a.pts, true) {
                if a.pts.len() >= 3 && self.visible(Rect::from_points(&a.pts)) {
                    self.stroke(&p, [KEEPOUT[0], KEEPOUT[1], KEEPOUT[2], 210], 1.0, false, Some(&[5.0, 3.0]));
                }
            }
        }
        // copper, bottom layer first so the top layer is on top
        for l in self.layers.clone().into_iter().rev() {
            let rgb = self.layer_rgb(l);
            for t in self.geom.traces.iter().filter(|t| t.layer == l) {
                if !self.visible(t.bbox()) {
                    continue;
                }
                self.note_net(t.net);
                if let Some(p) = self.poly_path(&t.pts, false) {
                    let w = (t.width * self.view.px_per_mm) as f32;
                    self.stroke(&p, [rgb[0], rgb[1], rgb[2], 255], w, true, None);
                } else if let Some(&only) = t.pts.first() {
                    // a degenerate trace (one point): a dot of its width
                    self.circle(only, t.width / 2.0, [rgb[0], rgb[1], rgb[2], 255]);
                }
            }
            let pad_rgb = lighten(rgb, 0.3);
            for p in self.geom.pads.iter().filter(|p| p.layer == l) {
                if !self.visible(p.shape.bbox()) {
                    continue;
                }
                self.note_net(p.net);
                if !p.component.is_empty() {
                    self.contains.components.insert(p.component.clone());
                }
                let col = [pad_rgb[0], pad_rgb[1], pad_rgb[2], 255];
                match &p.shape {
                    PadShape::Circle { c, r } => self.circle(*c, *r, col),
                    PadShape::Poly(pts) => {
                        if let Some(path) = self.poly_path(pts, true) {
                            self.pm.fill_path(&path, &paint(col, true), FillRule::Winding, Transform::identity(), None);
                        }
                    }
                }
            }
        }
        for v in &self.geom.vias {
            if !(self.layers.iter().any(|&l| v.spans(l)) && self.visible(Some(v.bbox()))) {
                continue;
            }
            self.note_net(v.net);
            self.circle(v.c, v.r, VIA_RING);
            if v.r * self.view.px_per_mm >= 2.5 {
                self.circle(v.c, v.r * 0.45, VIA_HOLE);
            }
        }
        // board outline
        let ow = (0.15 * self.view.px_per_mm) as f32;
        for poly in &self.geom.outline {
            if let Some(p) = self.poly_path(poly, true) {
                self.stroke(&p, OUTLINE, ow.max(1.5), true, None);
            }
        }
        // unrouted connections
        if self.spec.airlines {
            for a in &self.geom.airlines {
                if !self.visible(Rect::from_points([&a.from, &a.to])) {
                    continue;
                }
                self.note_net(a.net);
                if let Some(p) = self.poly_path(&[a.from, a.to], false) {
                    self.stroke(&p, AIRLINE, 1.4, false, Some(&[6.0, 4.0]));
                }
            }
        }
        // issue markers (and the list of issues in the image)
        for i in self.geom.issues.iter() {
            let on_layer = i.layer.is_none_or(|l| self.selected(l));
            if on_layer && region.intersects(&i.extent) {
                self.contains.issues.push(i.id.clone());
            }
        }
        let a = self.spec.annotations.clone();
        // labels, most important first (a later label that would overlap is skipped)
        if a.layer_names {
            self.layer_badge();
        }
        if a.scale_bar {
            self.scale_bar();
        }
        if a.grid != GridSpec::Off {
            self.grid_labels();
        }
        if a.markers {
            self.markers();
        }
        if a.ref_des {
            self.ref_des();
        }
        if a.pad_names {
            self.pad_names();
        }
        if a.net_names {
            self.net_names();
        }
    }

    fn note_net(&mut self, net: usize) {
        if net > 0 {
            let n = self.geom.net_name(net);
            if !n.is_empty() {
                self.contains.nets.insert(n.to_string());
            }
        }
    }

    // ---- grid -------------------------------------------------------------------------

    fn grid_step(&self) -> Option<f64> {
        match self.spec.annotations.grid {
            GridSpec::Off => None,
            GridSpec::Step(s) => Some(s),
            GridSpec::Auto => Some(nice_step(60.0 / self.view.px_per_mm)),
        }
    }

    fn draw_grid_lines(&mut self) {
        let Some(step) = self.grid_step() else { return };
        let region = self.view.region();
        let step_px = step * self.view.px_per_mm;
        if step_px < 4.0 {
            return; // checked as an error by `check_grid` before rendering; nothing sensible to draw
        }
        let (w, h) = (self.view.width_px as f32, self.view.height_px as f32);
        let first = (region.x0 / step).ceil() as i64;
        let last = (region.x1 / step).floor() as i64;
        for k in first..=last {
            let (u, _) = self.px(Pt::new(k as f64 * step, 0.0));
            let u = u.round() + 0.5;
            let major = k % 5 == 0;
            let mut pb = PathBuilder::new();
            pb.move_to(u, 0.0);
            pb.line_to(u, h);
            if let Some(p) = pb.finish() {
                self.stroke(&p, if major { GRID_MAJOR } else { GRID }, 1.0, false, None);
            }
        }
        let first = (region.y0 / step).ceil() as i64;
        let last = (region.y1 / step).floor() as i64;
        for k in first..=last {
            let (_, v) = self.px(Pt::new(0.0, k as f64 * step));
            let v = v.round() + 0.5;
            let major = k % 5 == 0;
            let mut pb = PathBuilder::new();
            pb.move_to(0.0, v);
            pb.line_to(w, v);
            if let Some(p) = pb.finish() {
                self.stroke(&p, if major { GRID_MAJOR } else { GRID }, 1.0, false, None);
            }
        }
    }

    fn grid_labels(&mut self) {
        let Some(step) = self.grid_step() else { return };
        let region = self.view.region();
        let s = self.text_scale;
        let step_px = step * self.view.px_per_mm;
        if step_px < 4.0 {
            return;
        }
        let first = (region.x0 / step).ceil() as i64;
        let last = (region.x1 / step).floor() as i64;
        for k in first..=last {
            let x = k as f64 * step;
            let (u, _) = self.px(Pt::new(x, 0.0));
            self.label(&fmt_coord(x, step), u + 2.0, 2.0, s, TEXT, false);
        }
        let first = (region.y0 / step).ceil() as i64;
        let last = (region.y1 / step).floor() as i64;
        for k in first..=last {
            let y = k as f64 * step;
            let (_, v) = self.px(Pt::new(0.0, y));
            self.label(&fmt_coord(y, step), 2.0, v + 2.0, s, TEXT, false);
        }
    }

    // ---- other annotations -------------------------------------------------------------

    fn layer_badge(&mut self) {
        let names: Vec<&str> = {
            let mut l = self.layers.clone();
            l.sort_unstable();
            l.iter().map(|&i| self.geom.layers[i].name.as_str()).collect()
        };
        let text = names.join("+");
        let s = self.text_scale;
        let w = font::text_width(&text) as f32 * s as f32;
        let x = (self.view.width_px as f32 - w - 6.0 * s as f32).max(2.0);
        self.label(&text, x, 4.0 * s as f32, s, TEXT, true);
    }

    fn scale_bar(&mut self) {
        let (wpx, hpx) = (self.view.width_px as f32, self.view.height_px as f32);
        if wpx < 120.0 || hpx < 60.0 {
            return;
        }
        let s = self.text_scale;
        // longest "nice" length that takes at most a third of the width
        let max_mm = wpx as f64 / 3.0 / self.view.px_per_mm;
        let len = nice_step_below(max_mm);
        let len_px = (len * self.view.px_per_mm) as f32;
        if len_px < 8.0 {
            return;
        }
        let x = 8.0 * s as f32;
        let y = hpx - 10.0 * s as f32;
        let bar_h = 3.0 * s as f32;
        let text = format!("{} MM", fmt_len(len));
        let text_w = font::text_width(&text) as f32 * s as f32;
        // reserve the place for the bar and its text
        let w_total = len_px.max(text_w);
        self.placed.push((x - 3.0, y - bar_h - 9.0 * s as f32 - 3.0, x + w_total + 3.0, y + bar_h + 3.0));
        let dark = paint(HALO, false);
        let white = paint(TEXT, false);
        if let Some(r) = tiny_skia::Rect::from_xywh(x - 1.0, y - bar_h - 1.0, len_px + 2.0, bar_h + 2.0) {
            self.pm.fill_rect(r, &dark, Transform::identity(), None);
        }
        if let Some(r) = tiny_skia::Rect::from_xywh(x, y - bar_h, len_px, bar_h) {
            self.pm.fill_rect(r, &white, Transform::identity(), None);
        }
        self.text_at(&text, x, y - bar_h - 8.0 * s as f32 - 2.0, s, TEXT, true);
    }

    fn markers(&mut self) {
        let region = self.view.region();
        let s = self.text_scale;
        let issues: Vec<_> = self.geom.issues.iter().filter(|i| region.contains(i.at) && i.layer.is_none_or(|l| self.selected(l))).cloned().collect();
        for i in issues {
            let (x, y) = self.px(i.at);
            let col = marker_color(i.kind);
            let r = 6.0 * s as f32;
            if let Some(p) = PathBuilder::from_circle(x, y, r) {
                self.stroke(&p, col, 2.0 * s as f32 * 0.75, false, None);
            }
            // cross hair
            let mut pb = PathBuilder::new();
            pb.move_to(x - r * 1.5, y);
            pb.line_to(x - r * 0.6, y);
            pb.move_to(x + r * 0.6, y);
            pb.line_to(x + r * 1.5, y);
            pb.move_to(x, y - r * 1.5);
            pb.line_to(x, y - r * 0.6);
            pb.move_to(x, y + r * 0.6);
            pb.line_to(x, y + r * 1.5);
            if let Some(p) = pb.finish() {
                self.stroke(&p, col, 1.0, false, None);
            }
            self.placed.push((x - r, y - r, x + r, y + r));
            self.label(&i.id, x + r + 2.0, y - r - 2.0 - 7.0 * s as f32, s, col, false);
        }
    }

    fn ref_des(&mut self) {
        let s = self.text_scale;
        for c in &self.geom.components {
            let visible_pads = self.geom.pads.iter().any(|p| p.component == c.name && self.selected(p.layer));
            if !visible_pads || !c.bbox.intersects(&self.view.region()) {
                continue;
            }
            let (x, y) = self.px(c.center);
            let w = font::text_width(&c.name) as f32 * s as f32;
            let bw = (c.bbox.width() * self.view.px_per_mm) as f32;
            if bw < w * 0.8 {
                self.stats.skipped_small += 1;
                continue;
            }
            self.label(&c.name, x - w / 2.0, y - 3.5 * s as f32, s, [255, 255, 160, 255], false);
        }
    }

    fn pad_names(&mut self) {
        let s = self.text_scale;
        let pads: Vec<_> = self.geom.pads.iter().filter(|p| self.selected(p.layer) && !p.pin.is_empty()).collect();
        let mut done: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
        for p in pads {
            let Some(b) = p.shape.bbox() else { continue };
            if !b.intersects(&self.view.region()) || !done.insert((p.component.clone(), p.pin.clone())) {
                continue;
            }
            let w = font::text_width(&p.pin) as f32 * s as f32;
            let h = font::GLYPH_H as f32 * s as f32;
            let (bw, bh) = ((b.width() * self.view.px_per_mm) as f32, (b.height() * self.view.px_per_mm) as f32);
            if bw < w + 2.0 || bh < h + 2.0 {
                self.stats.skipped_small += 1;
                continue;
            }
            let (x, y) = self.px(b.center());
            self.label(&p.pin, x - w / 2.0, y - h / 2.0, s, [10, 10, 10, 255], false);
        }
    }

    fn net_names(&mut self) {
        let s = self.text_scale;
        let mut per_net: std::collections::HashMap<usize, u32> = std::collections::HashMap::new();
        let selected_layers = self.layers.clone();
        for t in self.geom.traces.iter().filter(|t| selected_layers.binary_search(&t.layer).is_ok() && t.net > 0) {
            if !self.visible(t.bbox()) {
                continue;
            }
            let name = self.geom.net_name(t.net).to_string();
            if name.is_empty() || per_net.get(&t.net).copied().unwrap_or(0) >= 3 {
                continue;
            }
            // the longest segment, its part inside the image
            let region = self.view.region();
            let mut best: Option<(f64, Pt)> = None;
            for w in t.pts.windows(2) {
                let (a, b) = (w[0], w[1]);
                let mid = Pt::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
                let len = ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();
                if region.contains(mid) && best.is_none_or(|(l, _)| len > l) {
                    best = Some((len, mid));
                }
            }
            let Some((len, mid)) = best else { continue };
            let text_w = font::text_width(&name) as f32 * s as f32;
            if (len * self.view.px_per_mm) as f32 + 2.0 < text_w {
                self.stats.skipped_small += 1;
                continue;
            }
            let (x, y) = self.px(mid);
            let before = self.stats.drawn;
            self.label(&name, x - text_w / 2.0, y - 3.5 * s as f32, s, [255, 255, 255, 255], false);
            if self.stats.drawn > before {
                *per_net.entry(t.net).or_insert(0) += 1;
            }
        }
    }

    // ---- text --------------------------------------------------------------------------

    /// Draws `text` at the top-left `(x, y)` unless it would overlap an earlier label or leave
    /// the image. Returns whether it was drawn.
    fn label(&mut self, text: &str, x: f32, y: f32, scale: u32, col: [u8; 4], backing: bool) -> bool {
        let w = font::text_width(text) as f32 * scale as f32;
        let h = font::GLYPH_H as f32 * scale as f32;
        let rect = (x - 2.0, y - 2.0, x + w + 2.0, y + h + 2.0);
        let (iw, ih) = (self.view.width_px as f32, self.view.height_px as f32);
        if rect.2 < 0.0 || rect.3 < 0.0 || rect.0 > iw || rect.1 > ih {
            return false; // outside the image: not a skipped label, just not there
        }
        if self.placed.len() >= MAX_LABELS || self.placed.iter().any(|p| rect.0 < p.2 && p.0 < rect.2 && rect.1 < p.3 && p.1 < rect.3) {
            self.stats.skipped_overlap += 1;
            return false;
        }
        self.placed.push(rect);
        self.text_at(text, x, y, scale, col, backing);
        self.stats.drawn += 1;
        true
    }

    fn text_at(&mut self, text: &str, x: f32, y: f32, scale: u32, col: [u8; 4], backing: bool) {
        if backing {
            let w = font::text_width(text) as f32 * scale as f32;
            let h = font::GLYPH_H as f32 * scale as f32;
            if let Some(r) = tiny_skia::Rect::from_xywh(x - 2.0, y - 2.0, w + 4.0, h + 4.0) {
                self.pm.fill_rect(r, &paint([0, 0, 0, 170], false), Transform::identity(), None);
            }
        }
        let Some(path) = font::text_path(text, x, y, scale) else { return };
        let d = scale.max(1) as f32;
        let halo = paint(HALO, false);
        for (dx, dy) in [(-d, 0.0), (d, 0.0), (0.0, -d), (0.0, d)] {
            self.pm.fill_path(&path, &halo, FillRule::Winding, Transform::from_translate(dx, dy), None);
        }
        self.pm.fill_path(&path, &paint(col, false), FillRule::Winding, Transform::identity(), None);
    }

    fn finish(&mut self) -> ImageMeta {
        let v = self.view;
        let mut layers: Vec<String> = self.layers.iter().map(|&l| self.geom.layers[l].name.clone()).collect();
        layers.dedup();
        let mut issues = std::mem::take(&mut self.contains.issues);
        issues.sort();
        issues.dedup();
        ImageMeta {
            width_px: v.width_px,
            height_px: v.height_px,
            region_mm: RegionMm::from(v.region()),
            px_per_mm: v.px_per_mm,
            mm_per_px: v.mm_per_px(),
            transform: WorldTransform::from_view(&v),
            layers,
            annotations: self.spec.annotations.names(),
            airlines: self.spec.airlines,
            contains: Contains {
                components: self.contains.components.iter().cloned().collect(),
                nets: self.contains.nets.iter().cloned().collect(),
                issues,
            },
            labels: self.stats,
        }
    }
}

/// Smallest 1, 2 or 5 times a power of ten that is at least `min`.
pub fn nice_step(min: f64) -> f64 {
    if !(min.is_finite() && min > 0.0) {
        return 1.0;
    }
    let e = min.log10().floor();
    let base = 10f64.powf(e);
    for m in [1.0, 2.0, 5.0, 10.0] {
        if base * m >= min * (1.0 - 1e-9) {
            return base * m;
        }
    }
    base * 10.0
}

/// Largest 1, 2 or 5 times a power of ten that is at most `max`.
pub fn nice_step_below(max: f64) -> f64 {
    if !(max.is_finite() && max > 0.0) {
        return 1.0;
    }
    let e = max.log10().floor();
    let base = 10f64.powf(e);
    for m in [5.0, 2.0, 1.0] {
        if base * m <= max * (1.0 + 1e-9) {
            return base * m;
        }
    }
    base
}

fn fmt_len(mm: f64) -> String {
    if mm >= 1.0 {
        format!("{mm:.0}")
    } else {
        let s = format!("{mm:.3}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn fmt_coord(v: f64, step: f64) -> String {
    // as many decimals as the step needs: 5 -> 0, 0.5 -> 1, 0.25 -> 2
    let decimals = (0..=6usize).find(|d| {
        let x = step * 10f64.powi(*d as i32);
        (x - x.round()).abs() < 1e-6
    });
    let s = format!("{v:.*}", decimals.unwrap_or(3));
    // no "-0" or "-0.00"
    if s.starts_with('-') && s.chars().all(|c| matches!(c, '-' | '0' | '.')) {
        s[1..].to_string()
    } else {
        s
    }
}

/// Fails if an explicit grid step would be drawn with lines closer than 4 pixels.
pub fn check_grid(spec: &RenderSpec) -> Result<(), RenderError> {
    if let GridSpec::Step(step) = spec.annotations.grid {
        let view = View::fit(spec.region, spec.size)?;
        let px = step * view.px_per_mm;
        if px < 4.0 {
            return Err(RenderError::GridTooDense { step_mm: step, px });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_steps() {
        assert_eq!(nice_step(0.7), 1.0);
        assert_eq!(nice_step(1.5), 2.0);
        assert_eq!(nice_step(3.0), 5.0);
        assert_eq!(nice_step(5.0), 5.0);
        assert_eq!(nice_step(7.0), 10.0);
        assert!((nice_step(0.03) - 0.05).abs() < 1e-12);
        assert_eq!(nice_step_below(7.0), 5.0);
        assert_eq!(nice_step_below(1.9), 1.0);
        assert_eq!(nice_step_below(20.0), 20.0);
        assert!((nice_step_below(0.3) - 0.2).abs() < 1e-12);
        assert_eq!(nice_step(f64::NAN), 1.0);
    }

    #[test]
    fn coordinate_labels() {
        assert_eq!(fmt_coord(12.0, 1.0), "12");
        assert_eq!(fmt_coord(12.5, 0.5), "12.5");
        assert_eq!(fmt_coord(0.25, 0.25), "0.25");
        assert_eq!(fmt_coord(-0.0, 1.0), "0");
        assert_eq!(fmt_coord(-3.0, 1.0), "-3");
        assert_eq!(fmt_len(5.0), "5");
        assert_eq!(fmt_len(0.5), "0.5");
        assert_eq!(fmt_len(0.02), "0.02");
    }

    #[test]
    fn annotation_lists() {
        let a = Annotations::parse("grid,nets").unwrap();
        assert_eq!(a.grid, GridSpec::Auto);
        assert!(a.net_names && !a.scale_bar && !a.markers);
        assert_eq!(Annotations::parse("grid=2.5mm").unwrap().grid, GridSpec::Step(2.5));
        assert_eq!(Annotations::parse("none").unwrap(), Annotations::none());
        assert_eq!(Annotations::parse("all").unwrap(), Annotations::all());
        assert!(Annotations::parse("grid=0").is_err());
        assert!(Annotations::parse("grid=x").is_err());
        assert!(Annotations::parse("colors").is_err());
        assert_eq!(Annotations::default().names(), vec!["scale", "layer", "markers"]);
    }

    #[test]
    fn layer_colors_are_distinct_for_neighbours() {
        let n = 8;
        for i in 0..n - 1 {
            assert_ne!(layer_color(i, n), layer_color(i + 1, n), "layers {i} and {}", i + 1);
        }
    }
}
