//! PNG, SVG and Word (.docx) export.

use crate::geom::route_all;
use crate::model::*;
use crate::scene::*;
use ab_glyph::{Font, ScaleFont};
use std::io::Write;
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, StrokeDash, Transform};

pub fn scene_for_export(d: &Diagram) -> Vec<Prim> {
    let geoms = route_all(d);
    build(d, &geoms, &SceneOpts { draw_background: false })
}

/// Bounding box of all primitives.
pub fn prim_bounds(prims: &[Prim]) -> (P, P) {
    let mut lo = P::new(f32::MAX, f32::MAX);
    let mut hi = P::new(f32::MIN, f32::MIN);
    let mut add = |p: P| {
        lo.x = lo.x.min(p.x);
        lo.y = lo.y.min(p.y);
        hi.x = hi.x.max(p.x);
        hi.y = hi.y.max(p.y);
    };
    for p in prims {
        match p {
            Prim::Poly { pts, stroke, .. } => {
                let w = stroke.map_or(0.0, |s| s.width);
                for q in pts {
                    add(q.sub(P::new(w, w)));
                    add(q.add(P::new(w, w)));
                }
            }
            Prim::Text { cx, top, text, size, bold, .. } => {
                let w = text_width(text, *size, *bold) / 2.0;
                add(P::new(cx - w, *top));
                add(P::new(cx + w, top + size * LINE_H));
            }
        }
    }
    if lo.x > hi.x {
        return (P::new(0.0, 0.0), P::new(400.0, 300.0));
    }
    (lo, hi)
}

fn dash_array(d: Dash, w: f32) -> Option<Vec<f32>> {
    match d {
        Dash::Solid => None,
        Dash::Dashed => Some(vec![w * 4.0 + 4.0, w * 2.5 + 3.0]),
        Dash::Dotted => Some(vec![w.max(1.0), w * 2.0 + 2.0]),
    }
}

/// Baseline offset of a text line from the top of its line box.
pub fn baseline_offset(size: f32, bold: bool) -> f32 {
    let f = if bold { &fonts().bold } else { &fonts().regular };
    let sf = f.as_scaled(px_scale(f, size));
    let asc = sf.ascent();
    let desc = sf.descent();
    (size * LINE_H - (asc - desc)) / 2.0 + asc
}

// ------------------------------------------------------------------- PNG

pub fn render_png(d: &Diagram, scale: f32, transparent: bool) -> Result<(Vec<u8>, u32, u32), String> {
    let prims = scene_for_export(d);
    let (lo, hi) = prim_bounds(&prims);
    let margin = 24.0;
    let w = ((hi.x - lo.x + 2.0 * margin) * scale).ceil().max(1.0) as u32;
    let h = ((hi.y - lo.y + 2.0 * margin) * scale).ceil().max(1.0) as u32;
    if (w as u64) * (h as u64) > 400_000_000 {
        return Err("Diagram is too large to render at this resolution".into());
    }
    let mut pm = Pixmap::new(w, h).ok_or("could not allocate image")?;
    if !transparent {
        let b = d.background;
        pm.fill(tiny_skia::Color::from_rgba8(b.0, b.1, b.2, 255));
    }
    let tf = Transform::from_row(scale, 0.0, 0.0, scale, (margin - lo.x) * scale, (margin - lo.y) * scale);
    for p in &prims {
        match p {
            Prim::Poly { pts, closed, fill, stroke } => {
                if pts.len() < 2 {
                    continue;
                }
                let mut pb = PathBuilder::new();
                pb.move_to(pts[0].x, pts[0].y);
                for q in &pts[1..] {
                    pb.line_to(q.x, q.y);
                }
                if *closed {
                    pb.close();
                }
                let Some(path) = pb.finish() else { continue };
                if let Some(c) = fill {
                    let mut paint = Paint::default();
                    paint.set_color_rgba8(c.0, c.1, c.2, c.3);
                    paint.anti_alias = true;
                    pm.fill_path(&path, &paint, FillRule::Winding, tf, None);
                }
                if let Some(s) = stroke {
                    let mut paint = Paint::default();
                    paint.set_color_rgba8(s.color.0, s.color.1, s.color.2, s.color.3);
                    paint.anti_alias = true;
                    let mut st = Stroke { width: s.width, line_join: LineJoin::Round, line_cap: LineCap::Round, ..Default::default() };
                    if let Some(da) = dash_array(s.dash, s.width) {
                        st.dash = StrokeDash::new(da, 0.0);
                        st.line_cap = LineCap::Butt;
                    }
                    pm.stroke_path(&path, &paint, &st, tf, None);
                }
            }
            Prim::Text { cx, top, text, size, color, bold } => {
                draw_text(&mut pm, *cx, *top, text, *size, *color, *bold, scale, lo, margin);
            }
        }
    }
    let png = pm.encode_png().map_err(|e| e.to_string())?;
    Ok((png, w, h))
}

#[allow(clippy::too_many_arguments)]
fn draw_text(pm: &mut Pixmap, cx: f32, top: f32, text: &str, size: f32, color: Rgba, bold: bool, scale: f32, lo: P, margin: f32) {
    let f = if bold { &fonts().bold } else { &fonts().regular };
    let sf = f.as_scaled(px_scale(f, size * scale));
    let width = text_width(text, size, bold) * scale;
    let mut x = (cx - lo.x + margin) * scale - width / 2.0;
    let y = (top + baseline_offset(size, bold) - lo.y + margin) * scale;
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let data = pm.data_mut();
    let mut prev = None;
    for ch in text.chars() {
        let id = sf.glyph_id(ch);
        if let Some(p) = prev {
            x += sf.kern(p, id);
        }
        let glyph = id.with_scale_and_position(sf.scale(), ab_glyph::point(x, y));
        if let Some(og) = f.outline_glyph(glyph) {
            let bb = og.px_bounds();
            og.draw(|gx, gy, cov| {
                let px = bb.min.x as i32 + gx as i32;
                let py = bb.min.y as i32 + gy as i32;
                if px < 0 || py < 0 || px >= pw || py >= ph {
                    return;
                }
                let a = cov.clamp(0.0, 1.0) * color.3 as f32 / 255.0;
                let i = ((py * pw + px) * 4) as usize;
                // premultiplied source-over
                for (k, c) in [color.0, color.1, color.2].iter().enumerate() {
                    let dst = data[i + k] as f32;
                    data[i + k] = (*c as f32 * a + dst * (1.0 - a)).round() as u8;
                }
                let da = data[i + 3] as f32;
                data[i + 3] = (255.0 * a + da * (1.0 - a)).round() as u8;
            });
        }
        x += sf.h_advance(id);
        prev = Some(id);
    }
}

// ------------------------------------------------------------------- SVG

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn render_svg(d: &Diagram) -> (String, f32, f32) {
    let prims = scene_for_export(d);
    let (lo, hi) = prim_bounds(&prims);
    let m = 24.0;
    let (w, h) = (hi.x - lo.x + 2.0 * m, hi.y - lo.y + 2.0 * m);
    let mut s = String::new();
    s += &format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.0}\" height=\"{h:.0}\" viewBox=\"{:.2} {:.2} {w:.2} {h:.2}\">\n",
        lo.x - m,
        lo.y - m
    );
    let b = d.background;
    s += &format!("<rect x=\"{:.2}\" y=\"{:.2}\" width=\"{w:.2}\" height=\"{h:.2}\" fill=\"#{}\"/>\n", lo.x - m, lo.y - m, b.hex());
    for p in &prims {
        match p {
            Prim::Poly { pts, closed, fill, stroke } => {
                if pts.len() < 2 {
                    continue;
                }
                let mut dstr = format!("M{:.2} {:.2}", pts[0].x, pts[0].y);
                for q in &pts[1..] {
                    dstr += &format!(" L{:.2} {:.2}", q.x, q.y);
                }
                if *closed {
                    dstr += " Z";
                }
                let fill_attr = match fill {
                    Some(c) => format!("fill=\"#{}\" fill-opacity=\"{:.3}\"", c.hex(), c.3 as f32 / 255.0),
                    None => "fill=\"none\"".into(),
                };
                let stroke_attr = match stroke {
                    Some(st) => {
                        let mut a = format!(
                            " stroke=\"#{}\" stroke-opacity=\"{:.3}\" stroke-width=\"{:.2}\" stroke-linejoin=\"round\" stroke-linecap=\"{}\"",
                            st.color.hex(),
                            st.color.3 as f32 / 255.0,
                            st.width,
                            if st.dash == Dash::Solid { "round" } else { "butt" }
                        );
                        if let Some(da) = dash_array(st.dash, st.width) {
                            a += &format!(" stroke-dasharray=\"{:.1} {:.1}\"", da[0], da[1]);
                        }
                        a
                    }
                    None => String::new(),
                };
                s += &format!("<path d=\"{dstr}\" {fill_attr}{stroke_attr}/>\n");
            }
            Prim::Text { cx, top, text, size, color, bold } => {
                s += &format!(
                    "<text x=\"{cx:.2}\" y=\"{:.2}\" font-family=\"DejaVu Sans, Verdana, Arial, sans-serif\" font-size=\"{size:.1}\"{} fill=\"#{}\" text-anchor=\"middle\" xml:space=\"preserve\">{}</text>\n",
                    top + baseline_offset(*size, *bold),
                    if *bold { " font-weight=\"bold\"" } else { "" },
                    color.hex(),
                    esc(text)
                );
            }
        }
    }
    s += "</svg>\n";
    (s, w, h)
}

// ------------------------------------------------------------------ DOCX

pub struct DocxOptions {
    pub landscape: bool,
    pub include_title: bool,
    pub caption: String,
}

pub fn write_docx(d: &Diagram, opts: &DocxOptions) -> Result<Vec<u8>, String> {
    let (png, pw, ph) = render_png(d, 3.0, false)?;
    let (svg, _, _) = render_svg(d);
    // page geometry (A4), 1 inch margins
    let (page_w, page_h) = if opts.landscape { (16838, 11906) } else { (11906, 16838) };
    let usable_w_in = (page_w as f32 - 2.0 * 1440.0) / 1440.0;
    let usable_h_in = (page_h as f32 - 2.0 * 1440.0) / 1440.0 - 1.0;
    let aspect = ph as f32 / pw as f32;
    let mut w_in = usable_w_in;
    let mut h_in = w_in * aspect;
    if h_in > usable_h_in {
        h_in = usable_h_in;
        w_in = h_in / aspect;
    }
    let emu = 914400.0;
    let (cx, cy) = ((w_in * emu) as i64, (h_in * emu) as i64);
    let title = esc(&d.title);

    let mut body = String::new();
    if opts.include_title {
        body += &format!(
            "<w:p><w:pPr><w:jc w:val=\"center\"/><w:spacing w:after=\"200\"/></w:pPr><w:r><w:rPr><w:b/><w:sz w:val=\"32\"/></w:rPr><w:t xml:space=\"preserve\">{title}</w:t></w:r></w:p>"
        );
    }
    body += &format!(
        r#"<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="1" name="Framework" descr="{title}"/><wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect="1"/></wp:cNvGraphicFramePr><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="1" name="framework.png"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdPng"><a:extLst><a:ext uri="{{96DAC541-7B7A-43D3-8B79-37D633B846F1}}"><asvg:svgBlip r:embed="rIdSvg"/></a:ext></a:extLst></a:blip><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#
    );
    if !opts.caption.trim().is_empty() {
        body += &format!(
            "<w:p><w:pPr><w:jc w:val=\"center\"/><w:spacing w:before=\"120\"/></w:pPr><w:r><w:rPr><w:i/><w:sz w:val=\"20\"/></w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
            esc(&opts.caption)
        );
    }
    let orient = if opts.landscape { " w:orient=\"landscape\"" } else { "" };
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:asvg="http://schemas.microsoft.com/office/drawing/2016/SVG/main"><w:body>{body}<w:sectPr><w:pgSz w:w="{page_w}" w:h="{page_h}"{orient}/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr></w:body></w:document>"#
    );
    let content_types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="svg" ContentType="image/svg+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/></Types>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>"#;
    let doc_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdPng" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/framework.png"/><Relationship Id="rIdSvg" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/framework.svg"/></Relationships>"#;
    let core = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>{title}</dc:title><dc:creator>FrameMap Studio</dc:creator></cp:coreProperties>"#
    );

    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let o: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let files: [(&str, &[u8]); 7] = [
            ("[Content_Types].xml", content_types.as_bytes()),
            ("_rels/.rels", rels.as_bytes()),
            ("word/document.xml", document.as_bytes()),
            ("word/_rels/document.xml.rels", doc_rels.as_bytes()),
            ("docProps/core.xml", core.as_bytes()),
            ("word/media/framework.png", &png),
            ("word/media/framework.svg", svg.as_bytes()),
        ];
        for (name, data) in files {
            z.start_file(name, o).map_err(|e| e.to_string())?;
            z.write_all(data).map_err(|e| e.to_string())?;
        }
        z.finish().map_err(|e| e.to_string())?;
    }
    Ok(buf.into_inner())
}
