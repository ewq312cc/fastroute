//! Rendering behaviour on a small hand-made board (no routing engine involved).

use fr_render::*;

const TOP: [u8; 3] = [255, 84, 84];
const BOTTOM: [u8; 3] = [84, 142, 255];

/// 40 x 30 mm, two layers; a top trace across the middle, a bottom trace, a via, two pads of U1,
/// an unrouted connection and a keepout.
fn board() -> BoardGeometry {
    let sq = |cx: f64, cy: f64, h: f64| PadShape::Poly(vec![Pt::new(cx - h, cy - h), Pt::new(cx + h, cy - h), Pt::new(cx + h, cy + h), Pt::new(cx - h, cy + h)]);
    BoardGeometry {
        name: "demo".into(),
        layers: vec![Layer { name: "F.Cu".into(), signal: true }, Layer { name: "B.Cu".into(), signal: true }],
        nets: vec!["".into(), "GND".into(), "SIG".into()],
        outline: vec![vec![Pt::new(0.0, 0.0), Pt::new(40.0, 0.0), Pt::new(40.0, 30.0), Pt::new(0.0, 30.0)]],
        pads: vec![
            Pad { layer: 0, net: 1, component: "U1".into(), pin: "1".into(), shape: sq(10.0, 10.0, 1.0) },
            Pad { layer: 0, net: 2, component: "U1".into(), pin: "2".into(), shape: sq(10.0, 14.0, 1.0) },
            Pad { layer: 1, net: 1, component: "J1".into(), pin: "1".into(), shape: PadShape::Circle { c: Pt::new(30.0, 20.0), r: 1.0 } },
        ],
        traces: vec![
            Trace { layer: 0, net: 2, width: 0.5, pts: vec![Pt::new(5.0, 20.0), Pt::new(35.0, 20.0)], fixed: false },
            Trace { layer: 1, net: 1, width: 0.5, pts: vec![Pt::new(20.0, 2.0), Pt::new(20.0, 28.0)], fixed: false },
        ],
        vias: vec![Via { c: Pt::new(20.0, 20.0), r: 0.4, net: 2, first_layer: 0, last_layer: 1, fixed: false }],
        keepouts: vec![Area { layer: 0, net: 0, pts: vec![Pt::new(30.0, 2.0), Pt::new(38.0, 2.0), Pt::new(38.0, 8.0), Pt::new(30.0, 8.0)] }],
        airlines: vec![Airline { net: 2, from: Pt::new(10.0, 14.0), to: Pt::new(10.0, 25.0) }],
        components: vec![
            Component { name: "U1".into(), center: Pt::new(10.0, 12.0), bbox: Rect::new(8.0, 8.0, 12.0, 16.0) },
            Component { name: "J1".into(), center: Pt::new(30.0, 20.0), bbox: Rect::new(28.0, 18.0, 32.0, 22.0) },
        ],
        issues: vec![Issue { net: Some(2), ..Issue::at_point("OPEN1", IssueKind::Unrouted, Pt::new(10.0, 25.0)) }],
        planes: vec![],
    }
}

fn spec(g: &BoardGeometry, size: SizeSpec, layers: &str) -> RenderSpec {
    let region = resolve(g, &RegionSpec::Board { margin_mm: None }, &g.all_layers()).unwrap();
    let mut s = RenderSpec::new(region, size, g.parse_layers(layers).unwrap());
    s.annotations = Annotations::none();
    s
}

fn near(a: [u8; 4], rgb: [u8; 3], tol: i32) -> bool {
    (0..3).all(|i| (a[i] as i32 - rgb[i] as i32).abs() <= tol)
}

/// Number of pixels whose colour is within `tol` of `rgb`.
fn count(r: &Rendered, rgb: [u8; 3], tol: i32) -> usize {
    let mut n = 0;
    for y in 0..r.meta.height_px {
        for x in 0..r.meta.width_px {
            if near(r.pixel(x, y).unwrap(), rgb, tol) {
                n += 1;
            }
        }
    }
    n
}

fn at_mm(r: &Rendered, x: f64, y: f64) -> [u8; 4] {
    let (u, v) = r.meta.transform.to_pixel(x, y);
    r.pixel(u.floor() as u32, v.floor() as u32).unwrap_or([0; 4])
}

#[test]
fn whole_board_is_inside_the_image_with_a_margin() {
    let g = board();
    let r = render(&g, &spec(&g, SizeSpec::LongSide(1000), "all")).unwrap();
    assert_eq!(r.meta.width_px.max(r.meta.height_px), 1000);
    let reg = r.meta.region_mm;
    assert!(reg.x < 0.0 && reg.y < 0.0 && reg.x + reg.width > 40.0 && reg.y + reg.height > 30.0, "{reg:?}");
    // all four outline corners are inside the image, not on its border
    for (x, y) in [(0.0, 0.0), (40.0, 0.0), (40.0, 30.0), (0.0, 30.0)] {
        let (u, v) = r.meta.transform.to_pixel(x, y);
        assert!(u > 2.0 && v > 2.0 && u < r.meta.width_px as f64 - 2.0 && v < r.meta.height_px as f64 - 2.0, "({x},{y}) -> ({u},{v})");
    }
    // the outline is drawn: yellow pixels on the left edge, the middle of the board stays dark
    assert!(near(at_mm(&r, 0.0, 15.0), [246, 230, 99], 12), "{:?}", at_mm(&r, 0.0, 15.0));
    assert_eq!(at_mm(&r, 25.0, 5.0)[3], 255);
}

#[test]
fn layer_filter_gives_single_layer_and_overlay_images() {
    let g = board();
    let both = render(&g, &spec(&g, SizeSpec::LongSide(800), "all")).unwrap();
    let top = render(&g, &spec(&g, SizeSpec::LongSide(800), "F.Cu")).unwrap();
    let bot = render(&g, &spec(&g, SizeSpec::LongSide(800), "B.Cu")).unwrap();
    assert!(count(&both, TOP, 6) > 0 && count(&both, BOTTOM, 6) > 0);
    assert!(count(&top, TOP, 6) > 0, "top trace missing in the top image");
    assert_eq!(count(&top, BOTTOM, 6), 0, "bottom copper leaked into the top image");
    assert!(count(&bot, BOTTOM, 6) > 0);
    assert_eq!(count(&bot, TOP, 6), 0, "top copper leaked into the bottom image");
    assert_eq!(top.meta.layers, vec!["F.Cu"]);
    assert_eq!(both.meta.layers, vec!["F.Cu", "B.Cu"]);
    // the via spans both layers: visible in either single-layer image
    for img in [&top, &bot] {
        assert!(near(at_mm(img, 20.0, 20.0 - 0.38), [215, 215, 215], 6) || near(at_mm(img, 20.0, 20.0 - 0.3), [215, 215, 215], 6), "via ring");
    }
    // a pad that is only on the bottom is not in the top image, and the other way round
    assert!(!top.meta.contains.components.contains(&"J1".to_string()));
    assert!(bot.meta.contains.components.contains(&"J1".to_string()));
    assert!(!bot.meta.contains.components.contains(&"U1".to_string()));
    assert_eq!(both.meta.contains.components, vec!["J1", "U1"]);
    assert_eq!(both.meta.contains.nets, vec!["GND", "SIG"]);
}

#[test]
fn thin_lines_do_not_disappear() {
    let mut g = board();
    g.traces[0].width = 0.005; // 5 um at about 20 px/mm is a tenth of a pixel
    let r = render(&g, &spec(&g, SizeSpec::LongSide(800), "F.Cu")).unwrap();
    let (u, v) = r.meta.transform.to_pixel(12.0, 20.0);
    let ink = (-2..=2).filter_map(|d| r.pixel(u as u32, (v as i32 + d) as u32)).filter(|p| p[0] > 150 && p[1] < 120).count();
    assert!(ink >= 1, "a 5 um trace left no pixel at all");
    // still much narrower than a normal trace
    let wide = render(&board(), &spec(&board(), SizeSpec::LongSide(800), "F.Cu")).unwrap();
    assert!(count(&wide, TOP, 40) > count(&r, TOP, 40));
}

#[test]
fn same_input_gives_identical_pixels_and_png_bytes() {
    let g = board();
    let mut s = spec(&g, SizeSpec::LongSide(900), "all");
    s.annotations = Annotations::all();
    let a = render(&g, &s).unwrap();
    let b = render(&g, &s).unwrap();
    assert_eq!(a.pixmap.data(), b.pixmap.data());
    assert_eq!(a.png_bytes().unwrap(), b.png_bytes().unwrap());
    assert_eq!(a.meta, b.meta);
    // and the bytes are a PNG of the stated size
    let png = a.png_bytes().unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), a.meta.width_px);
    assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), a.meta.height_px);
}

#[test]
fn geometry_crossing_the_region_border_is_not_dropped() {
    let g = board();
    // the region 15..25 x 17..23 cuts the horizontal top trace (5..35 at y=20) on both sides
    let mut s = RenderSpec::new(Rect::new(26.0, 17.0, 34.0, 23.0), SizeSpec::LongSide(800), vec![0]);
    s.annotations = Annotations::none();
    let r = render(&g, &s).unwrap();
    let reg = r.meta.region_mm;
    // trace pixels at both the left and the right border of the image
    let (_, v) = r.meta.transform.to_pixel(0.0, 20.0);
    let row = v as u32;
    assert!(near(r.pixel(0, row).unwrap(), TOP, 30), "trace missing at the left border: {:?}", r.pixel(0, row));
    assert!(near(r.pixel(r.meta.width_px - 1, row).unwrap(), TOP, 30), "trace missing at the right border");
    assert!(r.meta.contains.nets.contains(&"SIG".to_string()));
    // the bottom trace (x=20) is outside the region: GND is only listed through the J1 pad on B.Cu
    let _ = reg;
    assert!(!r.meta.contains.nets.contains(&"GND".to_string()), "GND has nothing on F.Cu inside the region");
    let mut s2 = s.clone();
    s2.layers = vec![1];
    assert!(render(&g, &s2).unwrap().meta.contains.nets.contains(&"GND".to_string()));
}

#[test]
fn same_region_at_different_scales_keeps_the_geometry_in_place() {
    let g = board();
    let region = Rect::new(5.0, 5.0, 25.0, 25.0);
    let mut hi = RenderSpec::new(region, SizeSpec::PxPerMm(40.0), vec![0]);
    hi.annotations = Annotations::none();
    let mut lo = hi.clone();
    lo.size = SizeSpec::PxPerMm(10.0);
    let (a, b) = (render(&g, &hi).unwrap(), render(&g, &lo).unwrap());
    assert_eq!((a.meta.width_px, b.meta.width_px), (800, 200));
    assert!((a.meta.px_per_mm / b.meta.px_per_mm - 4.0).abs() < 1e-9);
    // the two pads of U1 and the centre of the trace are at the same world position in both
    // (the airline of U1-2 runs down from (10, 14), so sample beside it)
    for (x, y, rgb) in [(10.0, 10.0, [255, 135, 135]), (10.6, 14.5, [255, 135, 135]), (15.0, 20.0, TOP)] {
        let pa = at_mm(&a, x, y);
        let pb = at_mm(&b, x, y);
        assert!(near(pa, rgb, 8) && near(pb, rgb, 8), "({x},{y}): {pa:?} vs {pb:?}");
    }
    // the transform of the metadata maps world <-> pixels both ways
    for m in [&a.meta, &b.meta] {
        for (x, y) in [(5.0, 5.0), (12.3, 17.7), (25.0, 25.0)] {
            let (u, v) = m.transform.to_pixel(x, y);
            let (x2, y2) = m.transform.to_world(u, v);
            assert!((x - x2).abs() < 1e-9 && (y - y2).abs() < 1e-9);
        }
        assert!((m.mm_per_px * m.px_per_mm - 1.0).abs() < 1e-12);
    }
}

#[test]
fn metadata_survives_json() {
    let g = board();
    let r = render(&g, &spec(&g, SizeSpec::LongSide(500), "all")).unwrap();
    let json = serde_json::to_string(&r.meta).unwrap();
    let back: ImageMeta = serde_json::from_str(&json).unwrap();
    assert_eq!(back, r.meta);
}

#[test]
fn bad_arguments_are_errors_not_blank_images() {
    let g = board();
    let ok = spec(&g, SizeSpec::LongSide(300), "all");
    let mut s = ok.clone();
    s.layers = vec![];
    assert_eq!(render(&g, &s).err(), Some(RenderError::NoLayers));
    let mut s = ok.clone();
    s.layers = vec![7];
    assert!(matches!(render(&g, &s), Err(RenderError::UnknownLayer { .. })));
    let mut s = ok.clone();
    s.region = Rect::new(0.0, 0.0, 0.0, 10.0);
    assert!(matches!(render(&g, &s), Err(RenderError::BadRegion(_))));
    let mut s = ok.clone();
    s.size = SizeSpec::LongSide(100_000);
    assert!(matches!(render(&g, &s), Err(RenderError::TooLarge { .. })));
    // an empty board: no outline, no layers
    let empty = BoardGeometry::default();
    assert_eq!(resolve(&empty, &RegionSpec::Board { margin_mm: None }, &[]), Err(RenderError::NoOutline));
    assert_eq!(empty.parse_layers("all"), Err(RenderError::NoLayers));
}

#[test]
fn an_empty_region_of_a_real_board_renders_the_background() {
    let g = board();
    let mut s = RenderSpec::new(Rect::new(100.0, 100.0, 110.0, 110.0), SizeSpec::LongSide(100), vec![0, 1]);
    s.annotations = Annotations::none();
    let r = render(&g, &s).unwrap();
    assert!(r.meta.contains.components.is_empty() && r.meta.contains.nets.is_empty() && r.meta.contains.issues.is_empty());
    assert!(near(r.pixel(50, 50).unwrap(), [14, 17, 22], 0));
}

#[test]
fn annotations_draw_and_report_what_they_drew() {
    let g = board();
    let mut s = spec(&g, SizeSpec::LongSide(1200), "all");
    s.annotations = Annotations::all();
    let r = render(&g, &s).unwrap();
    assert_eq!(r.meta.contains.issues, vec!["OPEN1"]);
    assert!(r.meta.annotations.contains(&"grid".to_string()) && r.meta.annotations.contains(&"scale".to_string()));
    assert!(r.meta.labels.drawn > 5, "{:?}", r.meta.labels);
    // the marker ring of the issue at (10, 25) is orange
    let (u, v) = r.meta.transform.to_pixel(10.0, 25.0);
    let ring = (0..40).any(|d| {
        let p = r.pixel(u as u32 + d, v as u32).unwrap();
        near(p, [255, 165, 0], 20)
    });
    assert!(ring, "marker ring not found");
    // without annotations there is none, and nothing is reported
    let plain = render(&g, &spec(&g, SizeSpec::LongSide(1200), "all")).unwrap();
    assert_eq!(count(&plain, [255, 165, 0], 20), 0);
    assert_eq!(plain.meta.labels, LabelStats::default());
    assert!(plain.meta.annotations.is_empty());
}

#[test]
fn colliding_labels_are_skipped_not_overdrawn() {
    let mut g = board();
    // the same net label on ten traces at the same place
    for _ in 0..10 {
        g.traces.push(Trace { layer: 0, net: 2, width: 0.5, pts: vec![Pt::new(5.0, 20.0), Pt::new(35.0, 20.0)], fixed: false });
    }
    let mut s = spec(&g, SizeSpec::LongSide(1200), "F.Cu");
    s.annotations = Annotations::parse("nets").unwrap();
    let r = render(&g, &s).unwrap();
    assert!(r.meta.labels.drawn >= 1 && r.meta.labels.drawn <= 3, "{:?}", r.meta.labels);
}

#[test]
fn explicit_grid_must_be_visible() {
    let g = board();
    let mut s = spec(&g, SizeSpec::LongSide(300), "all");
    s.annotations = Annotations::parse("grid=0.1").unwrap();
    assert!(matches!(check_grid(&s), Err(RenderError::GridTooDense { .. })));
    s.annotations = Annotations::parse("grid=5").unwrap();
    assert!(check_grid(&s).is_ok());
    let r = render(&g, &s).unwrap();
    // a grid line at x = 20 mm (and the others) is a faint line over the background
    let (u, _) = r.meta.transform.to_pixel(20.0, 0.0);
    let p = r.pixel(u.round() as u32, 3).unwrap();
    assert!(p[0] > 14 && p[0] < 120, "grid line pixel {p:?}");
}

#[test]
fn big_images_are_fine() {
    let g = board();
    let r = render(&g, &spec(&g, SizeSpec::LongSide(8000), "all")).unwrap();
    assert_eq!((r.meta.width_px, r.meta.height_px), (8000, (8000.0 * r.meta.region_mm.height / r.meta.region_mm.width).ceil() as u32));
}
