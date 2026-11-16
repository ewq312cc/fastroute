//! A loaded board → plain geometry in millimetres ([`fr_render::BoardGeometry`]) plus the
//! typed analysis (unrouted connections, clearance violations) that `inspect` writes.
//!
//! Coordinates are KiCad's: mm, x right, y down. Specctra's y points up, so y is negated
//! (as in `--report`). The board unit → mm factor comes from the coordinate transform of the
//! loaded board and the unit of the DSN, so it is right for `um`, `mm`, `mil` and `inch` designs.
//!
//! The extraction only reads the board.

use std::collections::{BTreeMap, HashSet};

use fr_engine::board::{BasicBoard, ItemKey, ItemKind, RoutingBoard};
use fr_engine::drc::{all_clearance_violations, DesignRulesChecker};
use fr_engine::ids::ItemId;
use fr_geom::{FloatPoint, Shape};
use fr_render::{Airline, Area, BoardGeometry, Component, Issue, IssueKind, Layer, Pad, PadShape, Pt, Rect, Trace, Via};
use serde::{Deserialize, Serialize};

/// Board units per millimetre.
pub fn units_per_mm(board: &BasicBoard) -> f64 {
    board.communication.coordinate_transform.scale_factor() * 1000.0 / board.communication.unit.micrometers()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemRef {
    /// `pin`, `via`, `trace`, `keepout`, `plane`, `outline` or `board_outline`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub component: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pin: Option<String>,
    /// First and last layer the item is on.
    pub layers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Endpoint {
    pub x: f64,
    pub y: f64,
    pub item: ItemRef,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Unrouted {
    pub id: String,
    pub net: String,
    pub length_mm: f64,
    /// The net class is ignored by the autorouter, so the connection is left unrouted on purpose.
    pub ignored_class: bool,
    pub from: Endpoint,
    pub to: Endpoint,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Violation {
    pub id: String,
    pub layer: String,
    pub x: f64,
    pub y: f64,
    pub required_mm: f64,
    pub actual_mm: f64,
    /// Neither item can be moved by the router (pre-existing, e.g. between two pads).
    pub unfixable: bool,
    pub first: ItemRef,
    pub second: ItemRef,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompInfo {
    pub name: String,
    /// `front` or `back`.
    pub side: &'static str,
    pub rotation_deg: f64,
}

/// A value on each copper layer, in board order.
pub type PerLayer = Vec<(String, f64)>;

#[derive(Clone, Debug, PartialEq)]
pub struct ClassRule {
    pub name: String,
    pub nets: Vec<String>,
    pub trace_width_mm: PerLayer,
    /// Clearance between two traces of this class.
    pub clearance_mm: PerLayer,
    pub ignored_by_autorouter: bool,
    pub min_length_mm: Option<f64>,
    pub max_length_mm: Option<f64>,
}

pub struct Analysis {
    pub geometry: BoardGeometry,
    pub per_mm: f64,
    pub unrouted: Vec<Unrouted>,
    pub violations: Vec<Violation>,
    /// Details of `geometry.components`, same order.
    pub components: Vec<CompInfo>,
    /// Net class name per net number (index 0 = no net).
    pub net_class: Vec<String>,
    pub classes: Vec<ClassRule>,
}

struct Conv {
    per_mm: f64,
}

impl Conv {
    fn pt(&self, x: f64, y: f64) -> Pt {
        Pt::new(x / self.per_mm, -y / self.per_mm)
    }
    fn fp(&self, p: &FloatPoint) -> Pt {
        self.pt(p.x, p.y)
    }
    fn len(&self, v: f64) -> f64 {
        v / self.per_mm
    }
}

fn layer_name(board: &BasicBoard, l: i32) -> String {
    board.layer_structure.layers.get(l as usize).map(|x| x.name.clone()).unwrap_or_else(|| format!("L{l}"))
}

fn first_net(board: &BasicBoard, key: ItemKey) -> usize {
    board.item(key).net_numbers().first().copied().filter(|n| *n > 0).unwrap_or(0) as usize
}

fn item_ref(board: &BasicBoard, key: ItemKey) -> ItemRef {
    let item = board.item(key);
    let kind = match &item.kind {
        ItemKind::Pin(_) => "pin",
        ItemKind::Via(_) => "via",
        ItemKind::Trace(_) => "trace",
        ItemKind::ObstacleArea(_) => "keepout",
        ItemKind::ConductionArea(_) => "plane",
        ItemKind::ComponentOutline(_) => "outline",
        ItemKind::BoardOutline(_) => "board_outline",
    };
    let (mut component, mut pin) = (None, None);
    if item.component_no() > 0 {
        let comp = board.components.get(item.component_no());
        component = Some(comp.name.clone());
        if let ItemKind::Pin(p) = &item.kind {
            pin = board.library.packages.get(comp.get_package()).get_pin(p.pin_index).map(|x| x.name.clone());
        }
    }
    let (a, b) = (layer_name(board, item.first_layer(board)), layer_name(board, item.last_layer(board)));
    ItemRef { kind: kind.to_string(), component, pin, layers: if a == b { vec![a] } else { vec![a, b] } }
}

fn int_box_rect(c: &Conv, b: &fr_geom::IntBox) -> Rect {
    let (a, d) = (c.pt(b.ll.x as f64, b.ll.y as f64), c.pt(b.ur.x as f64, b.ur.y as f64));
    Rect::new(a.x, a.y, d.x, d.y)
}

/// Reads the whole board. `synthetic` are trace items to leave out (see `load`).
pub fn analyse(board: &RoutingBoard, name: &str, synthetic: &HashSet<ItemId>) -> Analysis {
    let basic: &BasicBoard = board;
    let conv = Conv { per_mm: units_per_mm(basic) };
    let mut g = BoardGeometry { name: name.to_string(), ..Default::default() };
    g.layers = basic.layer_structure.layers.iter().map(|l| Layer { name: l.name.clone(), signal: l.is_signal }).collect();
    g.nets.push(String::new());
    for n in 1..=basic.rules.nets.max_net_number() {
        g.nets.push(basic.rules.nets.get(n).map(|n| n.name.clone()).unwrap_or_default());
    }

    for key in basic.get_items() {
        let it = basic.item(key);
        match &it.kind {
            ItemKind::BoardOutline(o) => {
                for s in o.shapes() {
                    let pts: Vec<Pt> = s.corner_approx_arr().iter().map(|p| conv.fp(p)).collect();
                    if pts.len() >= 3 {
                        g.outline.push(pts);
                    }
                }
            }
            ItemKind::Pin(p) => {
                let (component, pin) = if it.component_no() > 0 {
                    let comp = basic.components.get(it.component_no());
                    let pin = basic.library.packages.get(comp.get_package()).get_pin(p.pin_index).map(|x| x.name.clone()).unwrap_or_default();
                    (comp.name.clone(), pin)
                } else {
                    (String::new(), String::new())
                };
                let net = first_net(basic, key);
                for layer in it.first_layer(basic)..=it.last_layer(basic) {
                    let Some(shape) = it.drill_shape_on_layer(basic, layer) else { continue };
                    let shape = match &shape {
                        Shape::Circle(c) => PadShape::Circle { c: conv.pt(c.center.x as f64, c.center.y as f64), r: conv.len(c.radius as f64) },
                        s => {
                            let pts: Vec<Pt> = s.corner_approx_arr().iter().map(|p| conv.fp(p)).collect();
                            if pts.len() < 3 {
                                continue;
                            }
                            PadShape::Poly(pts)
                        }
                    };
                    g.pads.push(Pad { layer: layer as usize, net, component: component.clone(), pin: pin.clone(), shape });
                }
            }
            ItemKind::Trace(tr) => {
                if synthetic.contains(&it.id()) {
                    continue;
                }
                let pts: Vec<Pt> = tr.polyline().corners().iter().map(|c| conv.fp(&c.to_float())).collect();
                g.traces.push(Trace { layer: tr.layer() as usize, net: first_net(basic, key), width: conv.len(2.0 * tr.half_width() as f64), pts, fixed: it.is_user_fixed() });
            }
            ItemKind::Via(_) => {
                let c = it.center(basic).to_float();
                let b = it.bounding_box(basic);
                let r = ((b.ur.x as i64 - b.ll.x as i64) as f64 / 2.0).max(1.0);
                g.vias.push(Via {
                    c: conv.fp(&c),
                    r: conv.len(r),
                    net: first_net(basic, key),
                    first_layer: it.first_layer(basic) as usize,
                    last_layer: it.last_layer(basic) as usize,
                    fixed: it.is_user_fixed(),
                });
            }
            ItemKind::ObstacleArea(a) => {
                let pts: Vec<Pt> = a.get_area(basic).corner_approx_arr().iter().map(|p| conv.fp(p)).collect();
                if pts.len() >= 3 {
                    g.keepouts.push(Area { layer: a.layer() as usize, net: 0, pts });
                }
            }
            ItemKind::ConductionArea(c) => {
                let pts: Vec<Pt> = c.area().get_area(basic).corner_approx_arr().iter().map(|p| conv.fp(p)).collect();
                if pts.len() >= 3 {
                    g.planes.push(Area { layer: c.area().layer() as usize, net: first_net(basic, key), pts });
                }
            }
            _ => {}
        }
    }

    let mut comp_info = Vec::new();
    for c in basic.components.get_all() {
        let mut bbox: Option<Rect> = None;
        for k in basic.get_component_items(c.id) {
            let b = basic.item(k).bounding_box(basic);
            if !b.is_empty() {
                let r = int_box_rect(&conv, &b);
                bbox = Some(bbox.map_or(r, |a| a.union(&r)));
            }
        }
        let (Some(bbox), Some(loc)) = (bbox, c.get_location()) else { continue };
        g.components.push(Component { name: c.name.clone(), center: conv.fp(&loc.to_float()), bbox });
        comp_info.push(CompInfo { name: c.name.clone(), side: if c.placed_on_front() { "front" } else { "back" }, rotation_deg: c.get_rotation_in_degree() });
    }

    // unrouted connections
    let mut checker = DesignRulesChecker::new();
    let mut unrouted = Vec::new();
    for (i, a) in checker.get_all_airlines(basic).iter().enumerate() {
        let id = format!("OPEN{}", i + 1);
        let (from, to) = (conv.fp(&a.from_corner), conv.fp(&a.to_corner));
        let net = a.net_number.max(0) as usize;
        g.airlines.push(Airline { net, from, to });
        let mid = Pt::new((from.x + to.x) / 2.0, (from.y + to.y) / 2.0);
        g.issues.push(Issue {
            extent: Rect::from_points([&from, &to]).expect("finite"),
            net: Some(net),
            ..Issue::at_point(id.clone(), IssueKind::Unrouted, mid)
        });
        let ignored = basic.rules.nets.get(a.net_number).is_some_and(|n| basic.rules.net_classes[n.get_net_class()].is_ignored_by_autorouter);
        unrouted.push(Unrouted {
            id,
            net: g.net_name(net).to_string(),
            length_mm: conv.len(((a.to_corner.x - a.from_corner.x).powi(2) + (a.to_corner.y - a.from_corner.y).powi(2)).sqrt()),
            ignored_class: ignored,
            from: Endpoint { x: from.x, y: from.y, item: item_ref(basic, a.from_item) },
            to: Endpoint { x: to.x, y: to.y, item: item_ref(basic, a.to_item) },
        });
    }

    // clearance violations (fastroute's own check)
    let mut violations = Vec::new();
    for (i, v) in all_clearance_violations(basic).iter().enumerate() {
        let id = format!("CLR{}", i + 1);
        let b = v.shape.bounding_box();
        let extent = int_box_rect(&conv, &b);
        let c = extent.center();
        let net = Some(first_net(basic, v.first_item)).filter(|n| *n > 0).or(Some(first_net(basic, v.second_item)).filter(|n| *n > 0));
        g.issues.push(Issue { extent, layer: Some(v.layer.max(0) as usize), net, ..Issue::at_point(id.clone(), IssueKind::Violation, c) });
        violations.push(Violation {
            id,
            layer: layer_name(basic, v.layer),
            x: c.x,
            y: c.y,
            required_mm: conv.len(v.expected_clearance),
            actual_mm: conv.len(v.actual_clearance),
            unfixable: v.is_unfixable(basic),
            first: item_ref(basic, v.first_item),
            second: item_ref(basic, v.second_item),
        });
    }

    // net classes and their rules
    let mut net_class = vec![String::new()];
    for n in 1..=basic.rules.nets.max_net_number() {
        net_class.push(basic.rules.nets.get(n).map(|net| basic.rules.net_classes[net.get_net_class()].get_name().to_string()).unwrap_or_default());
    }
    let rules = &basic.rules;
    let mut classes = Vec::new();
    for id in rules.net_classes.iter() {
        let c = &rules.net_classes[id];
        let cl = c.get_trace_clearance_class();
        let per_layer = |f: &dyn Fn(i32) -> f64| -> PerLayer { (0..g.layers.len()).map(|l| (g.layers[l].name.clone(), f(l as i32))).collect() };
        let positive = |v: f64| Some(conv.len(v)).filter(|x| *x > 0.0);
        classes.push(ClassRule {
            name: c.get_name().to_string(),
            nets: (1..g.nets.len()).filter(|&n| net_class[n] == c.get_name()).map(|n| g.nets[n].clone()).collect(),
            trace_width_mm: per_layer(&|l| conv.len(2.0 * c.get_trace_half_width(l) as f64)),
            clearance_mm: per_layer(&|l| conv.len(rules.clearance_matrix.get_value(cl, cl, l, false) as f64)),
            ignored_by_autorouter: c.is_ignored_by_autorouter,
            min_length_mm: positive(c.get_minimum_trace_length()),
            max_length_mm: positive(c.get_maximum_trace_length()),
        });
    }
    Analysis { geometry: g, per_mm: conv.per_mm, unrouted, violations, components: comp_info, net_class, classes }
}

/// Total trace length (mm) per net number, from the extracted geometry.
pub fn trace_length_per_net(g: &BoardGeometry) -> BTreeMap<usize, f64> {
    let mut m = BTreeMap::new();
    for t in &g.traces {
        let l: f64 = t.pts.windows(2).map(|w| ((w[0].x - w[1].x).powi(2) + (w[0].y - w[1].y).powi(2)).sqrt()).sum();
        *m.entry(t.net).or_insert(0.0) += l;
    }
    m
}
