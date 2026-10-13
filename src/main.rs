#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! FrameMap Studio — conceptual framework & systems map builder.

mod app;
mod export;
mod geom;
mod model;
mod scene;
mod templates;

use std::path::PathBuf;

static ICON_PNG: &[u8] = include_bytes!("../assets/icon_256.png");

fn load_icon() -> Option<eframe::egui::IconData> {
    let dec = png::Decoder::new(ICON_PNG);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba {
        return None;
    }
    buf.truncate(info.buffer_size());
    Some(eframe::egui::IconData { rgba: buf, width: info.width, height: info.height })
}

/// Command-line mode, useful for batch export and testing:
///   framemap --export <in.fmap|template:NAME> <out.docx|out.png|out.svg|out.fmap> [--minimize] [--landscape]
fn cli(args: &[String]) -> Option<i32> {
    let pos = args.iter().position(|a| a == "--export")?;
    let (Some(input), Some(output)) = (args.get(pos + 1), args.get(pos + 2)) else {
        eprintln!("usage: framemap --export <in.fmap|template:NAME> <out.docx|png|svg|fmap> [--minimize] [--landscape]");
        return Some(2);
    };
    let mut d: model::Diagram = if let Some(t) = input.strip_prefix("template:") {
        templates::make(t)
    } else {
        match std::fs::read_to_string(input).map_err(|e| e.to_string()).and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string())) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("cannot read {input}: {e}");
                return Some(1);
            }
        }
    };
    if args.iter().any(|a| a == "--minimize") {
        let (b, a) = geom::minimize_crossings(&mut d);
        eprintln!("crossings: {b} -> {a}");
    }
    let geoms = geom::route_all(&d);
    eprintln!("crossings in output: {}", geom::count_crossings(&geoms, d.hop_size));
    let res: Result<Vec<u8>, String> = if output.ends_with(".docx") {
        export::write_docx(&d, &export::DocxOptions { landscape: args.iter().any(|a| a == "--landscape"), include_title: true, caption: format!("Figure 1: {}", d.title) })
    } else if output.ends_with(".png") {
        export::render_png(&d, 2.0, false).map(|r| r.0)
    } else if output.ends_with(".svg") {
        Ok(export::render_svg(&d).0.into_bytes())
    } else {
        serde_json::to_string_pretty(&d).map(|s| s.into_bytes()).map_err(|e| e.to_string())
    };
    match res.and_then(|b| std::fs::write(output, b).map_err(|e| e.to_string())) {
        Ok(()) => {
            eprintln!("wrote {output}");
            Some(0)
        }
        Err(e) => {
            eprintln!("export failed: {e}");
            Some(1)
        }
    }
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = cli(&args) {
        std::process::exit(code);
    }
    // a .fmap passed by the OS (double-click / "Open with")
    let open = args.iter().skip(1).find(|a| a.ends_with(".fmap")).map(PathBuf::from);

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("FrameMap Studio")
        .with_inner_size([1360.0, 840.0])
        .with_min_inner_size([900.0, 560.0])
        .with_app_id("framemap-studio");
    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("FrameMap Studio", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, open)))))
}
