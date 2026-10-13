//! Converts a diagram into backend-neutral drawing primitives. The editor canvas,
//! PNG, SVG and Word exports all draw from the same primitive list, so what you
//! see on screen is what ends up in the document.

use crate::geom::*;
use crate::model::*;
use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use std::sync::OnceLock;

pub static FONT_REGULAR: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");
pub static FONT_BOLD: &[u8] = include_bytes!("../assets/DejaVuSans-Bold.ttf");

pub struct Fonts {
    pub regular: FontRef<'static>,
    pub bold: FontRef<'static>,
}

pub fn fonts() -> &'static Fonts {
    static F: OnceLock<Fonts> = OnceLock::new();
    F.get_or_init(|| Fonts {
        regular: FontRef::try_from_slice(FONT_REGULAR).expect("font"),
        bold: FontRef::try_from_slice(FONT_BOLD).expect("font"),
    })
}

/// Pixel scale so that one em equals `size` px.
pub fn px_scale(font: &FontRef<'static>, size: f32) -> PxScale {
    let upem = font.units_per_em().unwrap_or(2048.0);
    PxScale::from(size * font.height_unscaled() / upem)
}

pub fn text_width(text: &str, size: f32, bold: bool) -> f32 {
    let f = if bold { &fonts().bold } else { &fonts().regular };
    let sf = f.as_scaled(px_scale(f, size));
    let mut w = 0.0;
    let mut prev = None;
    for ch in text.chars() {
        let id = sf.glyph_id(ch);
        if let Some(p) = prev {
            w += sf.kern(p, id);
        }
        w += sf.h_advance(id);
        prev = Some(id);
    }
    w
}

/// Greedy word wrap, honouring explicit newlines.
pub fn wrap(text: &str, size: f32, bold: bool, max_w: f32) -> Vec<String> {
    let mut lines = vec![];
    for para in text.split('\n') {
        let mut cur = String::new();
        for word in para.split(' ') {
            let cand = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
            if text_width(&cand, size, bold) <= max_w || cur.is_empty() {
                cur = cand;
            } else {
                lines.push(std::mem::take(&mut cur));
                cur = word.to_string();
            }
        }
        lines.push(cur);
    }
    lines
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeSpec {
    pub color: Rgba,
    pub width: f32,
    pub dash: Dash,
}

#[derive(Clone, Debug)]
pub enum Prim {
    Poly { pts: Vec<P>, closed: bool, fill: Option<Rgba>, stroke: Option<StrokeSpec> },
    /// One line of text, horizontally centred on `cx`, top at `top`.
    Text { cx: f32, top: f32, text: String, size: f32, color: Rgba, bold: bool },
}

pub const LINE_H: f32 = 1.22;

fn head_trim(h: Head, s: f32) -> f32 {
    match h {
        Head::Arrow => s * 0.85,
        Head::Triangle => s,
        Head::Diamond => s * 1.3,
        Head::Circle => s * 0.8,
        Head::OpenArrow | Head::Bar | Head::None => 0.0,
    }
}

fn head_prims(h: Head, tip: P, dir: P, s: f32, color: Rgba, width: f32, bg: Rgba, out: &mut Vec<Prim>) {
    let n = dir.perp();
    let back = |k: f32| tip.sub(dir.mul(s * k));
    let st = Some(StrokeSpec { color, width: width.max(1.0), dash: Dash::Solid });
    match h {
        Head::None => {}
        Head::Arrow => out.push(Prim::Poly {
            pts: vec![tip, back(1.0).add(n.mul(s * 0.45)), back(0.78), back(1.0).sub(n.mul(s * 0.45))],
            closed: true,
            fill: Some(color),
            stroke: st,
        }),
        Head::OpenArrow => out.push(Prim::Poly {
            pts: vec![back(1.0).add(n.mul(s * 0.5)), tip, back(1.0).sub(n.mul(s * 0.5))],
            closed: false,
            fill: None,
            stroke: st,
        }),
        Head::Triangle => out.push(Prim::Poly {
            pts: vec![tip, back(1.0).add(n.mul(s * 0.5)), back(1.0).sub(n.mul(s * 0.5))],
            closed: true,
            fill: Some(bg),
            stroke: st,
        }),
        Head::Diamond => out.push(Prim::Poly {
            pts: vec![tip, back(0.65).add(n.mul(s * 0.4)), back(1.3), back(0.65).sub(n.mul(s * 0.4))],
            closed: true,
            fill: Some(color),
            stroke: st,
        }),
        Head::Circle => {
            let c = back(0.4);
            let r = s * 0.4;
            let pts = (0..24)
                .map(|i| {
                    let t = std::f32::consts::TAU * i as f32 / 24.0;
                    P::new(c.x + r * t.cos(), c.y + r * t.sin())
                })
                .collect();
            out.push(Prim::Poly { pts, closed: true, fill: Some(bg), stroke: st });
        }
        Head::Bar => out.push(Prim::Poly {
            pts: vec![back(0.15).add(n.mul(s * 0.55)), back(0.15).sub(n.mul(s * 0.55))],
            closed: false,
            fill: None,
            stroke: st,
        }),
    }
}

/// Trim a polyline by `a` at the start and `b` at the end.
fn trim(pts: &[P], a: f32, b: f32) -> Vec<P> {
    let l = polyline_len(pts);
    if a + b >= l - 1.0 {
        return pts.to_vec();
    }
    sub_polyline(pts, a, l - b)
}

pub struct SceneOpts {
    /// Ids of edges currently being dragged etc. are not special-cased; kept for future use.
    pub draw_background: bool,
}

/// Build primitives for the whole diagram. Order: shadows, edges, nodes, labels.
pub fn build(d: &Diagram, geoms: &[EdgeGeom], opts: &SceneOpts) -> Vec<Prim> {
    let mut out = vec![];
    if opts.draw_background {
        if let Some((lo, hi)) = d.bounds() {
            let m = 30.0;
            out.push(Prim::Poly {
                pts: vec![P::new(lo.x - m, lo.y - m), P::new(hi.x + m, lo.y - m), P::new(hi.x + m, hi.y + m), P::new(lo.x - m, hi.y + m)],
                closed: true,
                fill: Some(d.background),
                stroke: None,
            });
        }
    }
    let r = d.hop_size;
    // ---- edges (later edges hop over earlier ones)
    let mut labels = vec![];
    for (i, g) in geoms.iter().enumerate() {
        let Some(e) = d.edges.iter().find(|e| e.id == g.id) else { continue };
        if g.pts.len() < 2 {
            continue;
        }
        let ts = head_trim(e.start_head, e.head_size);
        let te = head_trim(e.end_head, e.head_size);
        let body = trim(&g.pts, ts, te);
        let mut cross_s: Vec<f32> = vec![];
        for o in &geoms[..i] {
            for (s, _) in crossings(&body, &o.pts, r * 1.2) {
                cross_s.push(s);
            }
        }
        cross_s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let stroke = StrokeSpec { color: e.color, width: e.width, dash: e.dash };
        for piece in apply_hops(&body, &cross_s, d.hop_style, r) {
            out.push(Prim::Poly { pts: piece, closed: false, fill: None, stroke: Some(stroke) });
        }
        let n = g.pts.len();
        // directions at the ends (look back a few px for curves)
        let end_dir = g.pts[n - 1].sub(point_at(&g.pts, (polyline_len(&g.pts) - e.head_size).max(0.0)).0).norm();
        let start_dir = g.pts[0].sub(point_at(&g.pts, e.head_size.min(polyline_len(&g.pts))).0).norm();
        head_prims(e.end_head, g.pts[n - 1], end_dir, e.head_size, e.color, e.width, d.background, &mut out);
        head_prims(e.start_head, g.pts[0], start_dir, e.head_size, e.color, e.width, d.background, &mut out);
        if !e.label.trim().is_empty() {
            let (mid, _) = point_at(&g.pts, polyline_len(&g.pts) / 2.0);
            labels.push((mid, e));
        }
    }
    // ---- nodes
    for nd in &d.nodes {
        let poly = outline(nd);
        if nd.shadow && nd.shape != Shape::Text {
            out.push(Prim::Poly {
                pts: poly.iter().map(|p| p.add(P::new(5.0, 5.0))).collect(),
                closed: true,
                fill: Some(Rgba(0, 0, 0, 45)),
                stroke: None,
            });
        }
        let stroke = if nd.stroke.3 > 0 && nd.stroke_width > 0.0 {
            Some(StrokeSpec { color: nd.stroke, width: nd.stroke_width, dash: nd.dash })
        } else {
            None
        };
        let fill = if nd.fill.3 > 0 { Some(nd.fill) } else { None };
        out.push(Prim::Poly { pts: poly, closed: true, fill, stroke });
        if let Some(st) = stroke {
            for dec in decorations(nd) {
                out.push(Prim::Poly { pts: dec, closed: false, fill: None, stroke: Some(st) });
            }
        }
        if !nd.text.is_empty() {
            let (tp, tw, th) = text_box(nd);
            let lines = wrap(&nd.text, nd.font_size, nd.bold, tw.max(10.0));
            let lh = nd.font_size * LINE_H;
            let total = lh * lines.len() as f32;
            let mut top = tp.y + (th - total) / 2.0;
            for l in lines {
                out.push(Prim::Text { cx: tp.x + tw / 2.0, top, text: l, size: nd.font_size, color: nd.text_color, bold: nd.bold });
                top += lh;
            }
        }
    }
    // ---- edge labels on top, with a background plate
    for (mid, e) in labels {
        let size = if e.label_size > 0.0 { e.label_size } else { 13.0 };
        let lines: Vec<String> = e.label.split('\n').map(|s| s.to_string()).collect();
        let w = lines.iter().map(|l| text_width(l, size, false)).fold(0.0, f32::max) + 10.0;
        let h = size * LINE_H * lines.len() as f32 + 4.0;
        let (x0, y0) = (mid.x - w / 2.0, mid.y - h / 2.0);
        out.push(Prim::Poly {
            pts: vec![P::new(x0, y0), P::new(x0 + w, y0), P::new(x0 + w, y0 + h), P::new(x0, y0 + h)],
            closed: true,
            fill: Some(d.background),
            stroke: None,
        });
        let mut top = y0 + 2.0;
        for l in lines {
            out.push(Prim::Text { cx: mid.x, top, text: l, size, color: e.color, bold: false });
            top += size * LINE_H;
        }
    }
    out
}
