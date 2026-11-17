//! Region selection: from a rectangle, a component, nets or report coordinates to the area
//! that has to be visible. All results are in world mm (y down) and have a positive size.

use crate::error::RenderError;
use crate::model::{BoardGeometry, Pt, Rect};

#[derive(Clone, Debug, PartialEq)]
pub enum RegionSpec {
    /// The board outline plus a margin (default: 2 % of the longer side, at least 0.5 mm).
    Board { margin_mm: Option<f64> },
    /// `x, y` = top-left corner, then width and height.
    Rect(Rect),
    /// The complete package (pads, outline, component keepouts) plus padding.
    Component { refdes: String, padding_mm: f64 },
    /// Every pad, trace, via and airline of the nets on the selected layers plus padding.
    /// Planes only count with `include_planes` (a pour would select the whole board).
    Nets { names: Vec<String>, padding_mm: f64, include_planes: bool },
    /// The bounding box of the points plus padding.
    Points { points: Vec<Pt>, padding_mm: f64 },
    /// The places of issues (`OPEN1`, `CLR2`, `DRC3`) plus padding.
    Issues { ids: Vec<String>, padding_mm: f64 },
}

/// Parses `x,y,w,h` (mm).
pub fn parse_region_mm(s: &str) -> Result<Rect, RenderError> {
    let bad = |why: &str| RenderError::BadRegion(format!("'{s}': {why}; expected x,y,w,h in mm (x,y = top-left corner, y points down)"));
    let parts: Vec<&str> = s.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        return Err(bad("four numbers needed"));
    }
    let mut v = [0.0f64; 4];
    for (i, p) in parts.iter().enumerate() {
        v[i] = p.parse().map_err(|_| bad(&format!("'{p}' is not a number")))?;
        if !v[i].is_finite() {
            return Err(bad("numbers must be finite"));
        }
    }
    if v[2] <= 0.0 || v[3] <= 0.0 {
        return Err(bad("width and height must be positive"));
    }
    Ok(Rect::new(v[0], v[1], v[0] + v[2], v[1] + v[3]))
}

fn hint(known: impl Iterator<Item = String>, wanted: &str) -> String {
    let w = wanted.to_ascii_lowercase();
    let mut all: Vec<String> = known.filter(|k| !k.is_empty()).collect();
    all.sort();
    all.dedup();
    let similar: Vec<&String> = all.iter().filter(|k| k.to_ascii_lowercase().contains(&w) || w.contains(&k.to_ascii_lowercase())).take(5).collect();
    if !similar.is_empty() {
        format!("similar: {}", similar.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))
    } else if all.is_empty() {
        "the board has none".to_string()
    } else {
        format!("e.g. {}", all.iter().take(5).map(|s| s.as_str()).collect::<Vec<_>>().join(", "))
    }
}

fn padded(r: Rect, padding_mm: f64, what: &str) -> Result<Rect, RenderError> {
    if !(padding_mm.is_finite() && padding_mm >= 0.0) {
        return Err(RenderError::BadRegion(format!("the padding must be 0 or more mm, not {padding_mm}")));
    }
    let r = r.expand(padding_mm);
    if r.width() <= 0.0 || r.height() <= 0.0 {
        return Err(RenderError::BadRegion(format!("{what} has no extent; add padding (--padding-mm=N)")));
    }
    Ok(r)
}

/// The area to show for `spec`, `layers` being the layers that will be drawn.
pub fn resolve(geom: &BoardGeometry, spec: &RegionSpec, layers: &[usize]) -> Result<Rect, RenderError> {
    match spec {
        RegionSpec::Board { margin_mm } => {
            let b = geom.outline_bbox().ok_or(RenderError::NoOutline)?;
            if b.width() <= 0.0 || b.height() <= 0.0 {
                return Err(RenderError::BadRegion("the board outline has no area".into()));
            }
            let m = margin_mm.unwrap_or_else(|| (b.width().max(b.height()) * 0.02).max(0.5));
            padded(b, m, "the board")
        }
        RegionSpec::Rect(r) => {
            if r.width() <= 0.0 || r.height() <= 0.0 || ![r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()) {
                return Err(RenderError::BadRegion("the rectangle needs a positive width and height".into()));
            }
            Ok(*r)
        }
        RegionSpec::Component { refdes, padding_mm } => {
            let c = geom
                .components
                .iter()
                .find(|c| c.name == *refdes)
                .or_else(|| geom.components.iter().find(|c| c.name.eq_ignore_ascii_case(refdes)))
                .ok_or_else(|| RenderError::NoMatch { kind: "component", name: refdes.clone(), hint: hint(geom.components.iter().map(|c| c.name.clone()), refdes) })?;
            padded(c.bbox, *padding_mm, &format!("component {}", c.name))
        }
        RegionSpec::Nets { names, padding_mm, include_planes } => {
            if names.is_empty() {
                return Err(RenderError::BadRegion("no net given".into()));
            }
            let mut boxes: Vec<Rect> = Vec::new();
            for name in names {
                let net = geom
                    .nets
                    .iter()
                    .position(|n| !n.is_empty() && n == name)
                    .or_else(|| geom.nets.iter().position(|n| !n.is_empty() && n.eq_ignore_ascii_case(name)))
                    .ok_or_else(|| RenderError::NoMatch { kind: "net", name: name.clone(), hint: hint(geom.nets.iter().cloned(), name) })?;
                let before = boxes.len();
                let on = |l: usize| layers.contains(&l);
                boxes.extend(geom.traces.iter().filter(|t| t.net == net && on(t.layer)).filter_map(|t| t.bbox()));
                boxes.extend(geom.pads.iter().filter(|p| p.net == net && on(p.layer)).filter_map(|p| p.shape.bbox()));
                boxes.extend(geom.vias.iter().filter(|v| v.net == net && layers.iter().any(|&l| v.spans(l))).map(|v| v.bbox()));
                boxes.extend(geom.airlines.iter().filter(|a| a.net == net).filter_map(|a| Rect::from_points([&a.from, &a.to])));
                if *include_planes {
                    boxes.extend(geom.planes.iter().filter(|a| a.net == net && on(a.layer)).filter_map(|a| Rect::from_points(&a.pts)));
                }
                if boxes.len() == before {
                    return Err(RenderError::NoMatch {
                        kind: "geometry of net",
                        name: name.clone(),
                        hint: format!(
                            "the net exists but has no pad, trace, via or airline on {}",
                            layers.iter().filter_map(|&l| geom.layers.get(l)).map(|l| l.name.as_str()).collect::<Vec<_>>().join(", ")
                        ),
                    });
                }
            }
            let all = boxes.iter().skip(1).fold(boxes[0], |a, b| a.union(b));
            padded(all, *padding_mm, "the net")
        }
        RegionSpec::Points { points, padding_mm } => {
            let b = Rect::from_points(points).ok_or_else(|| RenderError::BadRegion("no finite point given".into()))?;
            padded(b, *padding_mm, "the point")
        }
        RegionSpec::Issues { ids, padding_mm } => {
            if ids.is_empty() {
                return Err(RenderError::BadRegion("no issue given".into()));
            }
            let mut found: Vec<Rect> = Vec::new();
            for id in ids {
                let i = geom
                    .issues
                    .iter()
                    .find(|i| i.id == *id)
                    .ok_or_else(|| RenderError::NoMatch { kind: "issue", name: id.clone(), hint: hint(geom.issues.iter().map(|i| i.id.clone()), id) })?;
                found.push(i.extent);
            }
            let b = found.iter().skip(1).fold(found[0], |a, b| a.union(b));
            padded(b, *padding_mm, "the issue")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn board() -> BoardGeometry {
        BoardGeometry {
            layers: vec![Layer { name: "F.Cu".into(), signal: true }, Layer { name: "B.Cu".into(), signal: true }],
            nets: vec!["".into(), "GND".into(), "USB_DP".into(), "USB_DN".into()],
            outline: vec![vec![Pt::new(0.0, 0.0), Pt::new(40.0, 0.0), Pt::new(40.0, 30.0), Pt::new(0.0, 30.0)]],
            components: vec![Component { name: "U1".into(), center: Pt::new(10.0, 10.0), bbox: Rect::new(8.0, 8.0, 12.0, 13.0) }],
            traces: vec![
                Trace { layer: 0, net: 2, width: 0.2, pts: vec![Pt::new(20.0, 5.0), Pt::new(30.0, 5.0)], fixed: false },
                Trace { layer: 1, net: 2, width: 0.2, pts: vec![Pt::new(2.0, 25.0), Pt::new(4.0, 25.0)], fixed: false },
                Trace { layer: 0, net: 3, width: 0.2, pts: vec![Pt::new(20.0, 6.0), Pt::new(30.0, 6.0)], fixed: false },
            ],
            planes: vec![Area { layer: 0, net: 1, pts: vec![Pt::new(0.0, 0.0), Pt::new(40.0, 0.0), Pt::new(40.0, 30.0)] }],
            issues: vec![Issue { extent: Rect::new(31.0, 22.0, 35.0, 22.0), ..Issue::at_point("OPEN1", IssueKind::Unrouted, Pt::new(33.0, 22.0)) }],
            ..Default::default()
        }
    }

    #[test]
    fn region_mm_parsing() {
        let r = parse_region_mm("1, 2, 10, 5").unwrap();
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (1.0, 2.0, 11.0, 7.0));
        for bad in ["", "1,2,3", "1,2,3,4,5", "a,2,3,4", "1,2,0,4", "1,2,3,-4", "1,2,inf,4", "1,2,nan,4"] {
            assert!(matches!(parse_region_mm(bad), Err(RenderError::BadRegion(_))), "{bad}");
        }
    }

    #[test]
    fn board_region_has_margin() {
        let r = resolve(&board(), &RegionSpec::Board { margin_mm: None }, &[0, 1]).unwrap();
        assert!((r.x0 + 0.8).abs() < 1e-9 && (r.x1 - 40.8).abs() < 1e-9 && (r.y0 + 0.8).abs() < 1e-9);
        let none = BoardGeometry::default();
        assert_eq!(resolve(&none, &RegionSpec::Board { margin_mm: None }, &[0]), Err(RenderError::NoOutline));
    }

    #[test]
    fn component_region_is_the_whole_package_plus_padding() {
        let r = resolve(&board(), &RegionSpec::Component { refdes: "u1".into(), padding_mm: 2.0 }, &[0]).unwrap();
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (6.0, 6.0, 14.0, 15.0));
        let e = resolve(&board(), &RegionSpec::Component { refdes: "U9".into(), padding_mm: 1.0 }, &[0]).unwrap_err();
        assert!(matches!(e, RenderError::NoMatch { kind: "component", .. }), "{e}");
        assert!(e.to_string().contains("U9"));
    }

    #[test]
    fn net_region_follows_the_selected_layers_and_unions_nets() {
        let b = board();
        let top = resolve(&b, &RegionSpec::Nets { names: vec!["USB_DP".into()], padding_mm: 0.0, include_planes: false }, &[0]).unwrap();
        assert!((top.x0 - 19.9).abs() < 1e-9 && (top.x1 - 30.1).abs() < 1e-9, "top-layer part only: {top:?}");
        let both = resolve(&b, &RegionSpec::Nets { names: vec!["USB_DP".into()], padding_mm: 0.0, include_planes: false }, &[0, 1]).unwrap();
        assert!((both.x0 - 1.9).abs() < 1e-9 && (both.y1 - 25.1).abs() < 1e-9);
        // two nets (a differential pair) give the union
        let pair = resolve(&b, &RegionSpec::Nets { names: vec!["USB_DP".into(), "USB_DN".into()], padding_mm: 1.0, include_planes: false }, &[0]).unwrap();
        assert!((pair.y0 - (5.0 - 0.1 - 1.0)).abs() < 1e-9 && (pair.y1 - (6.0 + 0.1 + 1.0)).abs() < 1e-9);
        // a net that only has a plane has no geometry unless planes are asked for
        assert!(matches!(
            resolve(&b, &RegionSpec::Nets { names: vec!["GND".into()], padding_mm: 1.0, include_planes: false }, &[0]),
            Err(RenderError::NoMatch { kind: "geometry of net", .. })
        ));
        assert!(resolve(&b, &RegionSpec::Nets { names: vec!["GND".into()], padding_mm: 1.0, include_planes: true }, &[0]).is_ok());
        // unknown net, and the layer filter excluding all of a net's geometry
        assert!(matches!(
            resolve(&b, &RegionSpec::Nets { names: vec!["NOPE".into()], padding_mm: 1.0, include_planes: false }, &[0]),
            Err(RenderError::NoMatch { kind: "net", .. })
        ));
        assert!(matches!(
            resolve(&b, &RegionSpec::Nets { names: vec!["USB_DN".into()], padding_mm: 1.0, include_planes: false }, &[1]),
            Err(RenderError::NoMatch { kind: "geometry of net", .. })
        ));
    }

    #[test]
    fn points_and_issues() {
        let b = board();
        let r = resolve(&b, &RegionSpec::Issues { ids: vec!["OPEN1".into()], padding_mm: 3.0 }, &[0]).unwrap();
        // the extent of the issue (both ends of the connection), not just its marker
        assert_eq!((r.x0, r.x1, r.y0, r.y1), (28.0, 38.0, 19.0, 25.0));
        assert!(matches!(resolve(&b, &RegionSpec::Issues { ids: vec!["CLR7".into()], padding_mm: 3.0 }, &[0]), Err(RenderError::NoMatch { kind: "issue", .. })));
        // one point without padding has no extent
        assert!(matches!(resolve(&b, &RegionSpec::Points { points: vec![Pt::new(1.0, 1.0)], padding_mm: 0.0 }, &[0]), Err(RenderError::BadRegion(_))));
        assert!(resolve(&b, &RegionSpec::Points { points: vec![Pt::new(1.0, 1.0), Pt::new(5.0, 2.0)], padding_mm: 0.0 }, &[0]).is_ok());
        assert!(matches!(resolve(&b, &RegionSpec::Points { points: vec![], padding_mm: 1.0 }, &[0]), Err(RenderError::BadRegion(_))));
        assert!(matches!(resolve(&b, &RegionSpec::Points { points: vec![Pt::new(1.0, 1.0)], padding_mm: -1.0 }, &[0]), Err(RenderError::BadRegion(_))));
    }
}
