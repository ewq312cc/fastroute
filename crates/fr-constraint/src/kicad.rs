//! A read-only reader of KiCad boards (`.kicad_pcb`, format of KiCad 6 to 10) that produces a design
//! snapshot: components, pads with their nets and absolute positions, tracks, vias, zones, the
//! outline and mounting holes, all in millimetres, x to the right, y down (KiCad's own frame).
//!
//! It parses the file's text deterministically; it does not need KiCad and never writes anything.
//! Approximations are named: an arc becomes two chords, a board outline that cannot be chained into
//! one closed polygon falls back to the bounding box of the Edge.Cuts items. The result was compared
//! with KiCad's own Python API on KiCad's demo boards (see `crates/fastroute/tests/real_kicad.rs`).

use crate::geom::{polygon_area, to_board};
use crate::hcp::sha256_hex;
use crate::snapshot::*;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Sx {
    List(Vec<Sx>),
    Atom(String),
    Str(String),
}

impl Sx {
    fn head(&self) -> Option<&str> {
        match self {
            Sx::List(v) => match v.first() {
                Some(Sx::Atom(a)) => Some(a),
                _ => None,
            },
            _ => None,
        }
    }

    fn items(&self) -> &[Sx] {
        match self {
            Sx::List(v) => v,
            _ => &[],
        }
    }

    /// The text of an atom or string.
    fn text(&self) -> Option<&str> {
        match self {
            Sx::Atom(a) | Sx::Str(a) => Some(a),
            _ => None,
        }
    }

    fn num(&self) -> Option<f64> {
        self.text()?.parse().ok()
    }

    /// The child lists with this head.
    fn children<'a>(&'a self, head: &'a str) -> impl Iterator<Item = &'a Sx> + 'a {
        self.items().iter().filter(move |c| c.head() == Some(head))
    }

    fn child(&self, head: &str) -> Option<&Sx> {
        self.items().iter().find(|c| c.head() == Some(head))
    }

    /// The n-th element after the head, as text.
    fn arg(&self, n: usize) -> Option<&str> {
        self.items().get(n + 1).and_then(Sx::text)
    }

    fn argf(&self, n: usize) -> Option<f64> {
        self.items().get(n + 1).and_then(Sx::num)
    }
}

/// Parses the text of a KiCad file into an S-expression tree.
pub fn parse_sexpr(text: &str) -> Result<Sx, String> {
    let b: Vec<char> = text.trim_start_matches('\u{feff}').chars().collect();
    let mut stack: Vec<Vec<Sx>> = vec![vec![]];
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            '(' => {
                stack.push(vec![]);
                i += 1;
            }
            ')' => {
                let done = stack.pop().ok_or("unbalanced parentheses")?;
                stack.last_mut().ok_or("unbalanced parentheses")?.push(Sx::List(done));
                i += 1;
            }
            '"' => {
                let mut s = String::new();
                i += 1;
                while i < b.len() && b[i] != '"' {
                    if b[i] == '\\' && i + 1 < b.len() {
                        i += 1;
                        s.push(match b[i] {
                            'n' => '\n',
                            't' => '\t',
                            c => c,
                        });
                    } else {
                        s.push(b[i]);
                    }
                    i += 1;
                }
                if i >= b.len() {
                    return Err("an unterminated string".into());
                }
                i += 1;
                stack.last_mut().ok_or("unbalanced parentheses")?.push(Sx::Str(s));
            }
            c if c.is_whitespace() => i += 1,
            _ => {
                let start = i;
                while i < b.len() && !b[i].is_whitespace() && b[i] != '(' && b[i] != ')' && b[i] != '"' {
                    i += 1;
                }
                stack.last_mut().ok_or("unbalanced parentheses")?.push(Sx::Atom(b[start..i].iter().collect()));
            }
        }
    }
    if stack.len() != 1 {
        return Err("unbalanced parentheses (a missing closing parenthesis)".into());
    }
    let mut top = stack.pop().unwrap_or_default();
    top.pop().ok_or_else(|| "the file is empty".to_string())
}

fn xy(s: &Sx) -> Option<[f64; 2]> {
    Some([s.argf(0)?, s.argf(1)?])
}

fn points(pts: &Sx) -> Vec<[f64; 2]> {
    pts.children("xy").filter_map(xy).collect()
}

/// The net of an element: `(net 5 "GND")` (KiCad 6 to 9) or `(net "GND")` (KiCad 10).
fn net_of(s: &Sx, names: &BTreeMap<i64, String>) -> Option<String> {
    let n = s.child("net")?;
    let items = n.items();
    match items.len() {
        3 => items[2].text().map(str::to_string).filter(|x| !x.is_empty()),
        2 => match &items[1] {
            Sx::Str(name) => Some(name.clone()).filter(|x| !x.is_empty()),
            Sx::Atom(a) => a.parse::<i64>().ok().and_then(|id| names.get(&id).cloned()).filter(|x| !x.is_empty()),
            _ => None,
        },
        _ => None,
    }
    .or_else(|| s.child("net_name").and_then(|n| n.arg(0)).map(str::to_string).filter(|x| !x.is_empty()))
}

fn layers_of(s: &Sx) -> Vec<String> {
    if let Some(l) = s.child("layer").and_then(|l| l.arg(0)) {
        return vec![l.to_string()];
    }
    let mut out = Vec::new();
    if let Some(ls) = s.child("layers") {
        for l in ls.items().iter().skip(1).filter_map(Sx::text) {
            match l {
                "F&B.Cu" => {
                    out.push("F.Cu".to_string());
                    out.push("B.Cu".to_string());
                }
                "*.Cu" => out.push("all".to_string()),
                other => out.push(other.to_string()),
            }
        }
    }
    out
}

/// Chains line segments into the largest closed polygon; falls back to the bounding box.
fn chain_outline(segs: &[([f64; 2], [f64; 2])]) -> Vec<[f64; 2]> {
    let close = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3;
    let mut left: Vec<([f64; 2], [f64; 2])> = segs.to_vec();
    let mut best: Vec<[f64; 2]> = vec![];
    while let Some((a, b)) = left.pop() {
        let mut poly = vec![a, b];
        loop {
            let tail = *poly.last().unwrap();
            if poly.len() > 2 && close(tail, poly[0]) {
                poly.pop();
                break;
            }
            match left.iter().position(|(p, q)| close(*p, tail) || close(*q, tail)) {
                Some(i) => {
                    let (p, q) = left.remove(i);
                    poly.push(if close(p, tail) { q } else { p });
                }
                None => break,
            }
        }
        if poly.len() >= 3 && polygon_area(&poly).abs() > polygon_area(&best).abs() {
            best = poly;
        }
    }
    if best.len() >= 3 {
        return best;
    }
    // no closed chain: the bounding box of everything on Edge.Cuts
    let pts: Vec<[f64; 2]> = segs.iter().flat_map(|(a, b)| [*a, *b]).collect();
    if pts.is_empty() {
        return vec![];
    }
    let (minx, maxx) = pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p[0]), a.1.max(p[0])));
    let (miny, maxy) = pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p[1]), a.1.max(p[1])));
    vec![[minx, miny], [maxx, miny], [maxx, maxy], [minx, maxy]]
}

fn edge_items(root: &Sx, segs: &mut Vec<([f64; 2], [f64; 2])>, offset: Option<([f64; 2], f64)>) {
    let place = |p: [f64; 2]| match offset {
        Some((o, r)) => to_board(p, o, r),
        None => p,
    };
    let prefix = if offset.is_some() { "fp_" } else { "gr_" };
    for it in root.items() {
        let Some(head) = it.head() else { continue };
        if it.child("layer").and_then(|l| l.arg(0)) != Some("Edge.Cuts") || !head.starts_with(prefix) {
            continue;
        }
        let (s, e) = (it.child("start").and_then(xy), it.child("end").and_then(xy));
        match &head[3..] {
            "line" => {
                if let (Some(s), Some(e)) = (s, e) {
                    segs.push((place(s), place(e)));
                }
            }
            "rect" => {
                if let (Some(s), Some(e)) = (s, e) {
                    let c = [s, [e[0], s[1]], e, [s[0], e[1]]];
                    for i in 0..4 {
                        segs.push((place(c[i]), place(c[(i + 1) % 4])));
                    }
                }
            }
            "arc" => {
                // (start) (mid) (end): two chords; the older format has (start centre) (end) (angle)
                if let (Some(s), Some(m), Some(e)) = (s, it.child("mid").and_then(xy), e) {
                    segs.push((place(s), place(m)));
                    segs.push((place(m), place(e)));
                } else if let (Some(s), Some(e)) = (s, e) {
                    segs.push((place(s), place(e)));
                }
            }
            "poly" => {
                if let Some(p) = it.child("pts") {
                    let pts = points(p);
                    for i in 0..pts.len() {
                        segs.push((place(pts[i]), place(pts[(i + 1) % pts.len()])));
                    }
                }
            }
            "circle" => {
                if let (Some(c), Some(e)) = (it.child("center").and_then(xy), e) {
                    let r = ((e[0] - c[0]).powi(2) + (e[1] - c[1]).powi(2)).sqrt();
                    let n = 24;
                    let pts: Vec<[f64; 2]> = (0..n).map(|k| [c[0] + r * (k as f64 / n as f64 * std::f64::consts::TAU).cos(), c[1] + r * (k as f64 / n as f64 * std::f64::consts::TAU).sin()]).collect();
                    for i in 0..n {
                        segs.push((place(pts[i]), place(pts[(i + 1) % n])));
                    }
                }
            }
            _ => {}
        }
    }
}

/// Reads a board. `name` is the project name to record; `bytes` the file (for its digest).
pub fn read_pcb(text: &str, name: &str) -> Result<Snapshot, String> {
    let root = parse_sexpr(text)?;
    if root.head() != Some("kicad_pcb") {
        return Err("not a KiCad board (the file does not start with (kicad_pcb ...))".into());
    }
    // the net table: (net 3 "GND")
    let mut names: BTreeMap<i64, String> = BTreeMap::new();
    for n in root.children("net") {
        if let (Some(id), Some(name)) = (n.arg(0).and_then(|a| a.parse::<i64>().ok()), n.arg(1)) {
            names.insert(id, name.to_string());
        }
    }
    let mut snap = Snapshot { snapshot_version: SNAPSHOT_VERSION.into(), project: Project { name: name.to_string(), source_kind: Some("kicad_pcb".into()), source_sha256: Some(sha256_hex(text.as_bytes())) }, ..Default::default() };

    let mut edge: Vec<([f64; 2], [f64; 2])> = Vec::new();
    edge_items(&root, &mut edge, None);

    for fp in root.items().iter().filter(|c| matches!(c.head(), Some("footprint") | Some("module"))) {
        let at = fp.child("at");
        let (x, y, rot) = at.map(|a| (a.argf(0).unwrap_or(0.0), a.argf(1).unwrap_or(0.0), a.argf(2).unwrap_or(0.0))).unwrap_or((0.0, 0.0, 0.0));
        let mut reference = String::new();
        let mut value = None;
        let mut device = None;
        for p in fp.children("property") {
            match (p.arg(0), p.arg(1)) {
                (Some("Reference"), Some(v)) => reference = v.to_string(),
                (Some("Value"), Some(v)) => value = Some(v.to_string()),
                (Some("HCP_DEVICE"), Some(v)) => device = Some(v.to_string()),
                _ => {}
            }
        }
        for t in fp.children("fp_text") {
            match (t.arg(0), t.arg(1)) {
                (Some("reference"), Some(v)) if reference.is_empty() => reference = v.to_string(),
                (Some("value"), Some(v)) if value.is_none() => value = Some(v.to_string()),
                _ => {}
            }
        }
        let footprint = fp.arg(0).map(str::to_string);
        let side = if fp.child("layer").and_then(|l| l.arg(0)) == Some("B.Cu") { "bottom" } else { "top" };
        let uuid = fp.child("uuid").or_else(|| fp.child("tstamp")).and_then(|u| u.arg(0)).map(str::to_string);
        let locked = fp.items().iter().any(|c| c.text() == Some("locked")) || fp.child("locked").is_some_and(|l| l.arg(0) != Some("no"));
        if reference.is_empty() {
            reference = format!("?{}", snap.components.len() + 1);
        }
        edge_items(fp, &mut edge, Some(([x, y], rot)));
        let mut comp = Component { reference: reference.clone(), device, value, footprint, kind: None, x, y, rotation_deg: rot, side: side.into(), uuid, locked };
        comp.kind = Some(Snapshot::kind_of(&comp));
        // pads
        for pad in fp.children("pad") {
            let number = pad.arg(0).unwrap_or("").to_string();
            let ptype = pad.arg(1).unwrap_or("");
            let local = pad.child("at").and_then(xy).unwrap_or([0.0, 0.0]);
            let abs = to_board(local, [x, y], rot);
            if ptype == "np_thru_hole" {
                // a mounting hole: its drill is its size
                let drill = pad.child("drill").and_then(|d| d.items().iter().skip(1).find_map(Sx::num)).unwrap_or(0.0);
                snap.mount_holes.push(MountHole { reference: Some(reference.clone()), x: abs[0], y: abs[1], radius_mm: drill / 2.0 });
                continue;
            }
            if number.is_empty() {
                continue;
            }
            snap.pins.push(PinUse { reference: reference.clone(), pin: number, name: pad.child("pinfunction").and_then(|f| f.arg(0)).map(str::to_string), net: net_of(pad, &names), x: abs[0], y: abs[1] });
        }
        snap.components.push(comp);
    }
    for seg in root.children("segment") {
        let (Some(s), Some(e)) = (seg.child("start").and_then(xy), seg.child("end").and_then(xy)) else { continue };
        snap.tracks.push(Track {
            net: net_of(seg, &names),
            layer: seg.child("layer").and_then(|l| l.arg(0)).unwrap_or("").to_string(),
            width_mm: seg.child("width").and_then(|w| w.argf(0)).unwrap_or(0.0),
            x1: s[0],
            y1: s[1],
            x2: e[0],
            y2: e[1],
            uuid: seg.child("uuid").or_else(|| seg.child("tstamp")).and_then(|u| u.arg(0)).map(str::to_string),
        });
    }
    for arc in root.children("arc") {
        // (arc (start) (mid) (end) ...): two chords
        let (Some(s), Some(m), Some(e)) = (arc.child("start").and_then(xy), arc.child("mid").and_then(xy), arc.child("end").and_then(xy)) else { continue };
        let (net, layer, width) = (net_of(arc, &names), arc.child("layer").and_then(|l| l.arg(0)).unwrap_or("").to_string(), arc.child("width").and_then(|w| w.argf(0)).unwrap_or(0.0));
        let uuid = arc.child("uuid").or_else(|| arc.child("tstamp")).and_then(|u| u.arg(0)).map(str::to_string);
        for (a, b) in [(s, m), (m, e)] {
            snap.tracks.push(Track { net: net.clone(), layer: layer.clone(), width_mm: width, x1: a[0], y1: a[1], x2: b[0], y2: b[1], uuid: uuid.clone() });
        }
    }
    for v in root.children("via") {
        let Some(at) = v.child("at").and_then(xy) else { continue };
        snap.vias.push(Via { net: net_of(v, &names), x: at[0], y: at[1], diameter_mm: v.child("size").and_then(|s| s.argf(0)).unwrap_or(0.0), uuid: v.child("uuid").or_else(|| v.child("tstamp")).and_then(|u| u.arg(0)).map(str::to_string) });
    }
    for z in root.children("zone") {
        let Some(poly) = z.child("polygon").and_then(|p| p.child("pts")).map(points) else { continue };
        if poly.len() < 3 {
            continue;
        }
        let keepout = z.child("keepout").is_some();
        let net = net_of(z, &names);
        for layer in layers_of(z) {
            snap.zones.push(Zone { net: net.clone(), layer, polygon: poly.clone(), keepout, uuid: z.child("uuid").or_else(|| z.child("tstamp")).and_then(|u| u.arg(0)).map(str::to_string) });
        }
    }
    snap.outline = chain_outline(&edge);
    // deterministic order: the file's order of components is kept; pins and copper by identity
    snap.pins.sort_by(|a, b| a.reference.cmp(&b.reference).then(crate::compile::pin_order(&a.pin, &b.pin)));
    Ok(snap)
}
