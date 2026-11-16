//! The little geometry the checks need (millimetres, no tolerances hidden: callers pass them).

pub type Pt = [f64; 2];

/// Even-odd point in polygon (points on the border count as inside).
pub fn point_in_polygon(p: Pt, poly: &[Pt]) -> bool {
    if poly.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if point_segment_distance(p, a, b) < 1e-9 {
            return true;
        }
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

pub fn point_segment_distance(p: Pt, a: Pt, b: Pt) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0) };
    let (cx, cy) = (a[0] + t * dx, a[1] + t * dy);
    ((p[0] - cx).powi(2) + (p[1] - cy).powi(2)).sqrt()
}

fn orient(a: Pt, b: Pt, c: Pt) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

pub fn segments_intersect(a: Pt, b: Pt, c: Pt, d: Pt) -> bool {
    let (o1, o2, o3, o4) = (orient(a, b, c), orient(a, b, d), orient(c, d, a), orient(c, d, b));
    if ((o1 > 0.0) != (o2 > 0.0)) && ((o3 > 0.0) != (o4 > 0.0)) && o1 != 0.0 && o2 != 0.0 && o3 != 0.0 && o4 != 0.0 {
        return true;
    }
    point_segment_distance(c, a, b) < 1e-9 || point_segment_distance(d, a, b) < 1e-9 || point_segment_distance(a, c, d) < 1e-9 || point_segment_distance(b, c, d) < 1e-9
}

/// The distance between two segments (0 if they cross).
pub fn segment_segment_distance(a: Pt, b: Pt, c: Pt, d: Pt) -> f64 {
    if segments_intersect(a, b, c, d) {
        return 0.0;
    }
    point_segment_distance(a, c, d).min(point_segment_distance(b, c, d)).min(point_segment_distance(c, a, b)).min(point_segment_distance(d, a, b))
}

/// The distance from a segment to a polygon (0 if they touch or the segment is inside).
pub fn segment_polygon_distance(a: Pt, b: Pt, poly: &[Pt]) -> f64 {
    if point_in_polygon(a, poly) || point_in_polygon(b, poly) {
        return 0.0;
    }
    let mut best = f64::INFINITY;
    for i in 0..poly.len() {
        best = best.min(segment_segment_distance(a, b, poly[i], poly[(i + 1) % poly.len()]));
    }
    best
}

/// The distance from a point to a polygon (0 inside).
pub fn point_polygon_distance(p: Pt, poly: &[Pt]) -> f64 {
    if point_in_polygon(p, poly) {
        return 0.0;
    }
    (0..poly.len()).map(|i| point_segment_distance(p, poly[i], poly[(i + 1) % poly.len()])).fold(f64::INFINITY, f64::min)
}

/// Whether two polygons overlap (an edge crosses, or one contains a vertex of the other).
pub fn polygons_overlap(a: &[Pt], b: &[Pt]) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a.iter().any(|p| point_in_polygon(*p, b)) || b.iter().any(|p| point_in_polygon(*p, a)) {
        return true;
    }
    (0..a.len()).any(|i| (0..b.len()).any(|j| segments_intersect(a[i], a[(i + 1) % a.len()], b[j], b[(j + 1) % b.len()])))
}

/// KiCad's convention: x right, y down, a positive rotation turns counter-clockwise on the screen.
pub fn rotate_kicad(p: Pt, deg: f64) -> Pt {
    let r = deg.to_radians();
    let (c, s) = (r.cos(), r.sin());
    [p[0] * c + p[1] * s, -p[0] * s + p[1] * c]
}

/// A point of a device's own frame in the board frame.
pub fn to_board(local: Pt, origin: Pt, rotation_deg: f64) -> Pt {
    let r = rotate_kicad(local, rotation_deg);
    [origin[0] + r[0], origin[1] + r[1]]
}

pub fn polygon_area(poly: &[Pt]) -> f64 {
    let mut a = 0.0;
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        a += p[0] * q[1] - q[0] * p[1];
    }
    a / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_basics() {
        let sq = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        assert!(point_in_polygon([5.0, 5.0], &sq));
        assert!(point_in_polygon([10.0, 5.0], &sq), "the border counts");
        assert!(!point_in_polygon([11.0, 5.0], &sq));
        assert_eq!(segment_polygon_distance([12.0, 0.0], [12.0, 10.0], &sq), 2.0);
        assert_eq!(segment_polygon_distance([-5.0, 5.0], [15.0, 5.0], &sq), 0.0, "crossing through");
        assert_eq!(segment_polygon_distance([4.0, 4.0], [6.0, 6.0], &sq), 0.0, "inside");
        assert!(polygons_overlap(&sq, &[[9.0, 9.0], [20.0, 9.0], [20.0, 20.0]]));
        assert!(!polygons_overlap(&sq, &[[11.0, 11.0], [20.0, 11.0], [20.0, 20.0]]));
        assert_eq!(polygon_area(&sq), 100.0);
        // KiCad: rotating (1,0) by 90 degrees counter-clockwise on the screen (y down) gives (0,-1)
        let r = rotate_kicad([1.0, 0.0], 90.0);
        assert!((r[0]).abs() < 1e-12 && (r[1] + 1.0).abs() < 1e-12, "{r:?}");
    }
}
