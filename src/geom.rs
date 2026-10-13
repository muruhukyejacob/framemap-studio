//! Geometry: shape outlines, connector routing, crossing detection, line hops and
//! the automatic crossing minimiser.

use crate::model::*;
use std::f32::consts::PI;

// ---------------------------------------------------------------- shapes

fn ellipse_pts(cx: f32, cy: f32, rx: f32, ry: f32, a0: f32, a1: f32, n: usize) -> Vec<P> {
    (0..=n)
        .map(|i| {
            let t = a0 + (a1 - a0) * i as f32 / n as f32;
            P::new(cx + rx * t.cos(), cy + ry * t.sin())
        })
        .collect()
}

/// Closed outline polygon of a node (flattened).
pub fn outline(n: &Node) -> Vec<P> {
    let (x, y, w, h) = (n.pos.x, n.pos.y, n.w, n.h);
    let c = n.center();
    match n.shape {
        Shape::Rectangle | Shape::Text => vec![P::new(x, y), P::new(x + w, y), P::new(x + w, y + h), P::new(x, y + h)],
        Shape::RoundedRect => {
            let r = 14.0_f32.min(w / 4.0).min(h / 4.0);
            let mut v = vec![];
            v.extend(ellipse_pts(x + w - r, y + r, r, r, -PI / 2.0, 0.0, 6));
            v.extend(ellipse_pts(x + w - r, y + h - r, r, r, 0.0, PI / 2.0, 6));
            v.extend(ellipse_pts(x + r, y + h - r, r, r, PI / 2.0, PI, 6));
            v.extend(ellipse_pts(x + r, y + r, r, r, PI, 1.5 * PI, 6));
            v
        }
        Shape::Ellipse => {
            let mut v = ellipse_pts(c.x, c.y, w / 2.0, h / 2.0, 0.0, 2.0 * PI, 72);
            v.pop();
            v
        }
        Shape::Circle => {
            let r = w.min(h) / 2.0;
            let mut v = ellipse_pts(c.x, c.y, r, r, 0.0, 2.0 * PI, 72);
            v.pop();
            v
        }
        Shape::Diamond => vec![P::new(c.x, y), P::new(x + w, c.y), P::new(c.x, y + h), P::new(x, c.y)],
        Shape::Triangle => vec![P::new(c.x, y), P::new(x + w, y + h), P::new(x, y + h)],
        Shape::Parallelogram => {
            let k = (w * 0.18).min(h * 0.6);
            vec![P::new(x + k, y), P::new(x + w, y), P::new(x + w - k, y + h), P::new(x, y + h)]
        }
        Shape::Hexagon => {
            let k = (w * 0.2).min(h * 0.5);
            vec![P::new(x + k, y), P::new(x + w - k, y), P::new(x + w, c.y), P::new(x + w - k, y + h), P::new(x + k, y + h), P::new(x, c.y)]
        }
        Shape::Octagon => {
            let k = w.min(h) * 0.29;
            vec![
                P::new(x + k, y),
                P::new(x + w - k, y),
                P::new(x + w, y + k),
                P::new(x + w, y + h - k),
                P::new(x + w - k, y + h),
                P::new(x + k, y + h),
                P::new(x, y + h - k),
                P::new(x, y + k),
            ]
        }
        Shape::Cylinder => {
            let ry = (h * 0.14).min(18.0);
            let mut v = ellipse_pts(c.x, y + ry, w / 2.0, ry, PI, 2.0 * PI, 24);
            v.extend(ellipse_pts(c.x, y + h - ry, w / 2.0, ry, 0.0, PI, 24));
            v
        }
        Shape::Document => {
            let a = (h * 0.09).min(12.0);
            let mut v = vec![P::new(x, y), P::new(x + w, y)];
            let n = 32;
            for i in 0..=n {
                let t = i as f32 / n as f32;
                let xx = x + w - w * t;
                v.push(P::new(xx, y + h - a - a * (2.0 * PI * t).sin()));
            }
            v
        }
        Shape::Cloud => {
            let mut v = vec![];
            let n = 160;
            for i in 0..n {
                let t = 2.0 * PI * i as f32 / n as f32;
                let bump = 0.86 + 0.14 * (5.0 * t).sin().abs();
                v.push(P::new(c.x + w / 2.0 * bump * t.cos(), c.y + h / 2.0 * bump * t.sin()));
            }
            v
        }
        Shape::Note => {
            let f = 18.0_f32.min(w / 4.0).min(h / 4.0);
            vec![P::new(x, y), P::new(x + w - f, y), P::new(x + w, y + f), P::new(x + w, y + h), P::new(x, y + h)]
        }
    }
}

/// Extra open strokes that decorate a shape (cylinder rim, note fold).
pub fn decorations(n: &Node) -> Vec<Vec<P>> {
    let (x, y, w, h) = (n.pos.x, n.pos.y, n.w, n.h);
    match n.shape {
        Shape::Cylinder => {
            let ry = (h * 0.14).min(18.0);
            vec![ellipse_pts(x + w / 2.0, y + ry, w / 2.0, ry, 0.0, PI, 24)]
        }
        Shape::Note => {
            let f = 18.0_f32.min(w / 4.0).min(h / 4.0);
            vec![vec![P::new(x + w - f, y), P::new(x + w - f, y + f), P::new(x + w, y + f)]]
        }
        _ => vec![],
    }
}

/// Box in which a node's text is laid out.
pub fn text_box(n: &Node) -> (P, f32, f32) {
    let (x, y, w, h) = (n.pos.x, n.pos.y, n.w, n.h);
    match n.shape {
        Shape::Diamond | Shape::Ellipse | Shape::Circle | Shape::Cloud | Shape::Octagon => {
            (P::new(x + w * 0.16, y + h * 0.12), w * 0.68, h * 0.76)
        }
        Shape::Triangle => (P::new(x + w * 0.25, y + h * 0.42), w * 0.5, h * 0.55),
        Shape::Cylinder => {
            let ry = (h * 0.14).min(18.0);
            (P::new(x + 6.0, y + 2.0 * ry), w - 12.0, h - 3.0 * ry)
        }
        Shape::Parallelogram | Shape::Hexagon => (P::new(x + w * 0.18, y + 4.0), w * 0.64, h - 8.0),
        Shape::Document => (P::new(x + 8.0, y + 4.0), w - 16.0, h - 16.0),
        _ => (P::new(x + 8.0, y + 4.0), w - 16.0, h - 8.0),
    }
}

// --------------------------------------------------------------- helpers

pub fn seg_intersect(a: P, b: P, c: P, d: P) -> Option<(f32, f32)> {
    let r = b.sub(a);
    let s = d.sub(c);
    let den = r.x * s.y - r.y * s.x;
    if den.abs() < 1e-9 {
        return None;
    }
    let ca = c.sub(a);
    let t = (ca.x * s.y - ca.y * s.x) / den;
    let u = (ca.x * r.y - ca.y * r.x) / den;
    if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
        Some((t, u))
    } else {
        None
    }
}

pub fn point_in_poly(p: P, poly: &[P]) -> bool {
    let mut inside = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

pub fn dist_to_seg(p: P, a: P, b: P) -> f32 {
    let ab = b.sub(a);
    let l2 = ab.dot(ab);
    if l2 < 1e-9 {
        return p.dist(a);
    }
    let t = (p.sub(a).dot(ab) / l2).clamp(0.0, 1.0);
    p.dist(a.add(ab.mul(t)))
}

pub fn dist_to_polyline(p: P, pts: &[P]) -> f32 {
    pts.windows(2).map(|w| dist_to_seg(p, w[0], w[1])).fold(f32::MAX, f32::min)
}

pub fn polyline_len(pts: &[P]) -> f32 {
    pts.windows(2).map(|w| w[0].dist(w[1])).sum()
}

/// Point and direction at arc length `s`.
pub fn point_at(pts: &[P], s: f32) -> (P, P) {
    let mut acc = 0.0;
    for w in pts.windows(2) {
        let l = w[0].dist(w[1]);
        if acc + l >= s && l > 1e-6 {
            let t = (s - acc) / l;
            return (w[0].lerp(w[1], t), w[1].sub(w[0]).norm());
        }
        acc += l;
    }
    let n = pts.len();
    (pts[n - 1], pts[n - 1].sub(pts[n.saturating_sub(2)]).norm())
}

/// Cut a polyline down to the arc-length interval [s0, s1].
pub fn sub_polyline(pts: &[P], s0: f32, s1: f32) -> Vec<P> {
    let mut out = vec![];
    if s1 <= s0 {
        return out;
    }
    out.push(point_at(pts, s0).0);
    let mut acc = 0.0;
    for w in pts.windows(2) {
        acc += w[0].dist(w[1]);
        if acc > s0 && acc < s1 {
            out.push(w[1]);
        }
    }
    out.push(point_at(pts, s1).0);
    out
}

/// Where a ray from the node centre towards `toward` leaves the node's outline.
pub fn anchor(n: &Node, toward: P) -> P {
    let c = n.center();
    let dir = toward.sub(c);
    if dir.len() < 1e-3 {
        return c;
    }
    let far = c.add(dir.norm().mul(10000.0));
    let poly = outline(n);
    let mut best: Option<f32> = None;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if let Some((t, _)) = seg_intersect(c, far, a, b) {
            best = Some(best.map_or(t, |bt: f32| bt.max(t)));
        }
    }
    match best {
        Some(t) => c.lerp(far, t),
        None => c,
    }
}

fn catmull_rom(pts: &[P]) -> Vec<P> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut out = vec![pts[0]];
    for i in 0..pts.len() - 1 {
        let p0 = if i == 0 { pts[0] } else { pts[i - 1] };
        let p1 = pts[i];
        let p2 = pts[i + 1];
        let p3 = if i + 2 < pts.len() { pts[i + 2] } else { pts[i + 1] };
        let steps = ((p1.dist(p2) / 6.0) as usize).clamp(8, 40);
        for k in 1..=steps {
            let t = k as f32 / steps as f32;
            let t2 = t * t;
            let t3 = t2 * t;
            let f = |a: f32, b: f32, c: f32, d: f32| {
                0.5 * (2.0 * b + (-a + c) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (-a + 3.0 * b - 3.0 * c + d) * t3)
            };
            out.push(P::new(f(p0.x, p1.x, p2.x, p3.x), f(p0.y, p1.y, p2.y, p3.y)));
        }
    }
    out
}

// --------------------------------------------------------------- routing

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HandleKind {
    /// Existing bend point (index into `bends`).
    Bend(usize),
    /// Virtual handle between control points; dragging inserts a bend at `index`.
    Insert(usize),
    /// Elbow middle segment.
    ElbowMid,
}

#[derive(Clone, Debug)]
pub struct EdgeGeom {
    pub id: u64,
    /// Flattened path from source boundary to target boundary (no hops, untrimmed).
    pub pts: Vec<P>,
    pub handles: Vec<(HandleKind, P)>,
    pub elbow_vertical: bool,
}

pub fn elbow_is_vertical(a: &Node, b: &Node, e: &Edge) -> bool {
    if let Some(v) = e.elbow_vertical {
        return v;
    }
    let (ca, cb) = (a.center(), b.center());
    let gap_x = (cb.x - ca.x).abs() - (a.w + b.w) / 2.0;
    let gap_y = (cb.y - ca.y).abs() - (a.h + b.h) / 2.0;
    gap_y > gap_x
}

pub fn route_edge(d: &Diagram, e: &Edge) -> Option<EdgeGeom> {
    let a = d.node(e.from)?;
    let b = d.node(e.to)?;
    match e.route {
        Route::Straight | Route::Curved => {
            let first = e.bends.first().copied().unwrap_or(b.center());
            let last = e.bends.last().copied().unwrap_or(a.center());
            let (s, t) = if e.from == e.to && e.bends.is_empty() {
                // self loop: synthesise two bends above the node
                let c = a.center();
                let s = anchor(a, P::new(c.x - a.w * 0.2, a.pos.y - 10.0));
                let t = anchor(a, P::new(c.x + a.w * 0.2, a.pos.y - 10.0));
                let loop_pts = vec![s, P::new(c.x - a.w * 0.25, a.pos.y - 50.0), P::new(c.x + a.w * 0.25, a.pos.y - 50.0), t];
                return Some(EdgeGeom { id: e.id, pts: catmull_rom(&loop_pts), handles: vec![], elbow_vertical: false });
            } else {
                (anchor(a, first), anchor(b, last))
            };
            let mut ctrl = vec![s];
            ctrl.extend(e.bends.iter().copied());
            ctrl.push(t);
            let pts = if e.route == Route::Curved { catmull_rom(&ctrl) } else { ctrl.clone() };
            let mut handles: Vec<(HandleKind, P)> = e.bends.iter().enumerate().map(|(i, p)| (HandleKind::Bend(i), *p)).collect();
            for i in 0..ctrl.len() - 1 {
                let mid = if e.route == Route::Curved {
                    // put insert handle on the actual curve near the span middle
                    let target = ctrl[i].lerp(ctrl[i + 1], 0.5);
                    *pts.iter().min_by(|p, q| p.dist(target).partial_cmp(&q.dist(target)).unwrap()).unwrap()
                } else {
                    ctrl[i].lerp(ctrl[i + 1], 0.5)
                };
                handles.push((HandleKind::Insert(i), mid));
            }
            Some(EdgeGeom { id: e.id, pts, handles, elbow_vertical: false })
        }
        Route::Elbow => {
            let vertical = elbow_is_vertical(a, b, e);
            let (ca, cb) = (a.center(), b.center());
            let pts = if !vertical {
                let dir = if cb.x >= ca.x { 1.0 } else { -1.0 };
                let s = anchor(a, ca.add(P::new(dir, 0.0)));
                let t = anchor(b, cb.add(P::new(-dir, 0.0)));
                let mx = e.elbow_mid.unwrap_or((s.x + t.x) / 2.0);
                vec![s, P::new(mx, s.y), P::new(mx, t.y), t]
            } else {
                let dir = if cb.y >= ca.y { 1.0 } else { -1.0 };
                let s = anchor(a, ca.add(P::new(0.0, dir)));
                let t = anchor(b, cb.add(P::new(0.0, -dir)));
                let my = e.elbow_mid.unwrap_or((s.y + t.y) / 2.0);
                vec![s, P::new(s.x, my), P::new(t.x, my), t]
            };
            let mid = pts[1].lerp(pts[2], 0.5);
            // drop zero-length segments
            let mut clean: Vec<P> = vec![];
            for p in pts {
                if clean.last().map_or(true, |q: &P| q.dist(p) > 0.5) {
                    clean.push(p);
                }
            }
            if clean.len() < 2 {
                clean.push(clean[0]);
            }
            Some(EdgeGeom { id: e.id, pts: clean, handles: vec![(HandleKind::ElbowMid, mid)], elbow_vertical: vertical })
        }
    }
}

pub fn route_all(d: &Diagram) -> Vec<EdgeGeom> {
    d.edges.iter().filter_map(|e| route_edge(d, e)).collect()
}

// ------------------------------------------------------- crossings & hops

/// Arc-length positions along `a` where it crosses `b` (ignoring points
/// where the lines merely meet near an end, e.g. at a shared node).
pub fn crossings(a: &[P], b: &[P], end_margin: f32) -> Vec<(f32, P)> {
    let mut out = vec![];
    if a.len() < 2 || b.len() < 2 {
        return out;
    }
    let (b0, b1) = (b[0], b[b.len() - 1]);
    let la = polyline_len(a);
    let mut acc = 0.0;
    for wa in a.windows(2) {
        let seg_l = wa[0].dist(wa[1]);
        for wb in b.windows(2) {
            if let Some((t, _)) = seg_intersect(wa[0], wa[1], wb[0], wb[1]) {
                let p = wa[0].lerp(wa[1], t);
                let s = acc + seg_l * t;
                if s < end_margin || s > la - end_margin || p.dist(b0) < end_margin || p.dist(b1) < end_margin {
                    continue;
                }
                if out.iter().any(|(s2, _): &(f32, P)| (s2 - s).abs() < 1.0) {
                    continue;
                }
                out.push((s, p));
            }
        }
        acc += seg_l;
    }
    out.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
    out
}

/// Total number of crossings between connectors in the diagram.
pub fn count_crossings(geoms: &[EdgeGeom], r: f32) -> usize {
    let mut n = 0;
    for i in 0..geoms.len() {
        for j in 0..i {
            n += crossings(&geoms[i].pts, &geoms[j].pts, r * 1.2).len();
        }
    }
    n
}

/// Apply hops to a (trimmed) polyline. `cross_s` are arc-length positions of
/// crossings on this path. Returns one or more polylines (several for Gap style).
pub fn apply_hops(pts: &[P], cross_s: &[f32], style: HopStyle, r: f32) -> Vec<Vec<P>> {
    if cross_s.is_empty() || style == HopStyle::None {
        return vec![pts.to_vec()];
    }
    let total = polyline_len(pts);
    // merge crossings that are closer than a hop diameter into one interval
    let mut ivs: Vec<(f32, f32)> = vec![];
    for &s in cross_s {
        let (a, b) = ((s - r).max(0.0), (s + r).min(total));
        if let Some(last) = ivs.last_mut() {
            if a <= last.1 + 2.0 {
                last.1 = b;
                continue;
            }
        }
        ivs.push((a, b));
    }
    let mut pieces: Vec<Vec<P>> = vec![];
    let mut cur: Vec<P> = vec![];
    let mut pos = 0.0;
    for (a, b) in ivs {
        let seg = sub_polyline(pts, pos, a);
        cur.extend(seg);
        match style {
            HopStyle::Arc => {
                let (pa, _) = point_at(pts, a);
                let (pb, _) = point_at(pts, b);
                let chord = pb.sub(pa);
                let mid = pa.lerp(pb, 0.5);
                let rad = chord.len() / 2.0;
                let dir = chord.norm();
                let mut n = dir.perp();
                // bulge upwards on screen (or to the left for vertical lines) for a consistent look
                if n.y > 0.0 || (n.y.abs() < 1e-3 && n.x > 0.0) {
                    n = n.mul(-1.0);
                }
                let h = rad.min(r * 1.1);
                for k in 0..=16 {
                    let th = PI * k as f32 / 16.0;
                    cur.push(mid.sub(dir.mul(rad * th.cos())).add(n.mul(h * th.sin())));
                }
            }
            HopStyle::Gap => {
                pieces.push(std::mem::take(&mut cur));
            }
            HopStyle::None => {}
        }
        pos = b;
    }
    cur.extend(sub_polyline(pts, pos, total));
    pieces.push(cur);
    pieces.retain(|p| p.len() >= 2);
    pieces
}

// ------------------------------------------------------ crossing minimiser

/// (penalty, tidiness): penalty counts crossings, overlaps and lines through shapes;
/// tidiness (length, bends) only breaks ties between equally good routes.
fn edge_cost(d: &Diagram, geoms: &[EdgeGeom], idx: usize, r: f32) -> (f32, f32) {
    let g = &geoms[idx];
    let e = &d.edges[idx];
    let mut cost = 0.0;
    for (j, o) in geoms.iter().enumerate() {
        if j != idx {
            cost += 10.0 * crossings(&g.pts, &o.pts, r * 1.2).len() as f32;
        }
    }
    // passing through (or brushing against) nodes that are not its endpoints is worse than a crossing
    let total = polyline_len(&g.pts);
    let samples = ((total / 4.0) as usize).max(2);
    for n in &d.nodes {
        if n.id == e.from || n.id == e.to {
            continue;
        }
        let m = 10.0;
        let hit = (0..=samples).any(|k| {
            let p = point_at(&g.pts, total * k as f32 / samples as f32).0;
            p.x > n.pos.x - m && p.x < n.pos.x + n.w + m && p.y > n.pos.y - m && p.y < n.pos.y + n.h + m
        });
        if hit {
            cost += 25.0;
        }
    }
    // running along (on top of) another connector hides it: heavy penalty
    for (j, o) in geoms.iter().enumerate() {
        if j == idx || o.pts.len() < 2 {
            continue;
        }
        let on_top = (0..=samples)
            .filter(|k| {
                let p = point_at(&g.pts, total * *k as f32 / samples as f32).0;
                dist_to_polyline(p, &o.pts) < 4.0
            })
            .count();
        if on_top as f32 > samples as f32 * 0.15 {
            cost += 40.0;
        }
    }
    // bends count a little, so simple routes win ties
    (cost, e.bends.len() as f32 * 0.4 + polyline_len(&g.pts) * 0.004)
}

/// Greedy search that re-routes connectors to remove crossings.
/// Returns (crossings before, crossings after).
pub fn minimize_crossings(d: &mut Diagram) -> (usize, usize) {
    let r = d.hop_size;
    let before = count_crossings(&route_all(d), r);
    let original = d.clone();
    if d.edges.len() < 2 {
        return (before, before);
    }
    for _pass in 0..4 {
        let mut improved = false;
        for idx in 0..d.edges.len() {
            let mut geoms = route_all(d);
            if geoms.len() != d.edges.len() {
                return (before, before);
            }
            let cur_cost = edge_cost(d, &geoms, idx, r);
            if cur_cost.0 < 1.0 {
                continue;
            }
            let orig = d.edges[idx].clone();
            let (Some(a), Some(b)) = (d.node(orig.from).cloned(), d.node(orig.to).cloned()) else { continue };
            let (ca, cb) = (a.center(), b.center());
            let mid = ca.lerp(cb, 0.5);
            let n = cb.sub(ca).norm().perp();
            let mut cands: Vec<Edge> = vec![];
            let mut base = orig.clone();
            base.bends.clear();
            base.elbow_mid = None;
            base.elbow_vertical = None;
            let mut s = base.clone();
            s.route = Route::Straight;
            cands.push(s);
            for vert in [false, true] {
                for off in [0.0, -50.0, 50.0, -110.0, 110.0, -180.0, 180.0] {
                    let mut c = base.clone();
                    c.route = Route::Elbow;
                    c.elbow_vertical = Some(vert);
                    let m = if vert { mid.y } else { mid.x };
                    c.elbow_mid = if off == 0.0 { None } else { Some(m + off) };
                    cands.push(c);
                }
            }
            for off in [-60.0, 60.0, -120.0, 120.0, -200.0, 200.0, -300.0, 300.0] {
                for route in [Route::Curved, Route::Straight] {
                    let mut c = base.clone();
                    c.route = route;
                    c.bends = vec![mid.add(n.mul(off))];
                    cands.push(c);
                }
            }
            // routes that travel around the outside of the diagram
            if let Some((lo, hi)) = d.bounds() {
                for k in [0.0, 1.0, 2.0] {
                    let m = 40.0 + 30.0 * k + 6.0 * idx as f32 % 24.0;
                    let (top, bot, left, right) = (lo.y - m, hi.y + m, lo.x - m, hi.x + m);
                    for bends in [
                        vec![P::new(ca.x, top), P::new(cb.x, top)],
                        vec![P::new(ca.x, bot), P::new(cb.x, bot)],
                        vec![P::new(left, ca.y), P::new(left, cb.y)],
                        vec![P::new(right, ca.y), P::new(right, cb.y)],
                    ] {
                        let mut c = base.clone();
                        c.route = Route::Straight;
                        c.bends = bends;
                        cands.push(c);
                    }
                    for (vert, mm) in [(true, top), (true, bot), (false, left), (false, right)] {
                        let mut c = base.clone();
                        c.route = Route::Elbow;
                        c.elbow_vertical = Some(vert);
                        c.elbow_mid = Some(mm);
                        cands.push(c);
                    }
                }
            }
            // only accept a route that is strictly better on penalty; tidiness breaks ties
            let mut best: (f32, f32, Option<Edge>) = (cur_cost.0 - 0.5, f32::MAX, None);
            for c in cands {
                if let Some(g) = route_edge(d, &c) {
                    geoms[idx] = g;
                    let mut tmp = d.edges[idx].clone();
                    std::mem::swap(&mut tmp, &mut d.edges[idx]);
                    d.edges[idx] = c.clone();
                    let (pen, tidy) = edge_cost(d, &geoms, idx, r);
                    let tidy = tidy + if c.route != orig.route { 0.3 } else { 0.0 };
                    d.edges[idx] = tmp;
                    if pen < best.0 - 0.01 || (pen <= best.0 + 0.01 && best.2.is_some() && tidy < best.1) {
                        best = (pen, tidy, Some(c));
                    }
                }
            }
            if let (_, _, Some(c)) = best {
                d.edges[idx] = c;
                improved = true;
            }
        }
        if !improved {
            break;
        }
    }
    let after = count_crossings(&route_all(d), r);
    if after >= before {
        *d = original;
        return (before, before);
    }
    (before, after)
}
