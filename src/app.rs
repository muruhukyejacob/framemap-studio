//! The editor: canvas interaction, panels, menus, undo/redo history.

use crate::export::{self, DocxOptions};
use crate::geom::{self, EdgeGeom, HandleKind};
use crate::model::*;
use crate::scene::{self, Prim, SceneOpts, LINE_H};
use crate::templates;
use crate::update;
use eframe::egui::{
    self, color_picker, Align2, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Key, Modifiers, Pos2, Rect, Sense,
    Shape as EShape, Stroke, StrokeKind, Vec2,
};
use std::collections::BTreeSet;
use std::path::PathBuf;

const MAX_HISTORY: usize = 300;
const CLIP_PREFIX: &str = "FRAMEMAP-CLIP:";

fn c32(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c.0, c.1, c.2, c.3)
}
fn from_c32(c: Color32) -> Rgba {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Rgba(r, g, b, a)
}

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Select,
    Connect,
    /// Draw a free-standing arrow anywhere on the canvas.
    Arrow,
    Place(Shape),
}

#[derive(Clone, Copy, PartialEq)]
enum CtxTarget {
    Canvas(P),
    Node(u64),
    Edge(u64),
    Bend(u64, usize),
}

/// Snap `p` so the segment from `origin` is horizontal, vertical or at 45°.
fn snap_angle(origin: P, p: P) -> P {
    let d = p.sub(origin);
    let k = std::f32::consts::FRAC_1_SQRT_2;
    let dirs = [
        P::new(1.0, 0.0),
        P::new(-1.0, 0.0),
        P::new(0.0, 1.0),
        P::new(0.0, -1.0),
        P::new(k, k),
        P::new(-k, k),
        P::new(k, -k),
        P::new(-k, -k),
    ];
    let best = dirs.iter().copied().max_by(|a, b| d.dot(*a).partial_cmp(&d.dot(*b)).unwrap()).unwrap();
    let r = origin.add(best.mul(d.dot(best)));
    // make axis-aligned results exact
    if best.x == 0.0 {
        P::new(origin.x, r.y)
    } else if best.y == 0.0 {
        P::new(r.x, origin.y)
    } else {
        r
    }
}

#[derive(Clone)]
enum Drag {
    None,
    Move { start: P, orig: Vec<(u64, P)>, edge_orig: Vec<(Edge, bool)>, moved: bool, prepared: bool },
    Draw { start: P, from: Option<u64>, cur: P },
    Resize { id: u64, handle: usize, orig: Node, start: P },
    Connect { from: u64, cur: P },
    Marquee { start: P, cur: P, additive: bool },
    EdgeHandle { edge: u64, kind: HandleKind },
    EdgeEnd { edge: u64, is_start: bool, cur: P },
    Pan { last: Pos2 },
}

#[derive(Clone, Copy, PartialEq)]
enum Editing {
    Node(u64),
    Edge(u64),
}

struct EdgeDefaults {
    route: Route,
    start: Head,
    end: Head,
    color: Rgba,
    width: f32,
    dash: Dash,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Clip {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
}

pub struct App {
    doc: Diagram,
    committed: Diagram,
    saved: Diagram,
    undo: Vec<(String, Diagram)>,
    redo: Vec<(String, Diagram)>,
    pending_label: Option<String>,
    file: Option<PathBuf>,
    sel_n: BTreeSet<u64>,
    sel_e: BTreeSet<u64>,
    offset: Vec2,
    zoom: f32,
    tool: Tool,
    drag: Drag,
    editing: Option<Editing>,
    edit_focus_pending: bool,
    snap: bool,
    grid: bool,
    defaults: EdgeDefaults,
    status: String,
    show_history: bool,
    show_help: bool,
    show_about: bool,
    docx_dialog: Option<(bool, bool, String)>,
    confirm_close: bool,
    allow_close: bool,
    pending_after_save: Option<Box<dyn FnOnce(&mut App)>>,
    ask_save: Option<&'static str>,
    pending_template: Option<&'static str>,
    pending_open: bool,
    fit_request: bool,
    canvas_rect: Rect,
    geoms: Vec<EdgeGeom>,
    crossing_count: usize,
    sel_bend: Option<(u64, usize)>,
    ctx_target: CtxTarget,
    move_step: f32,
    update: std::sync::Arc<std::sync::Mutex<update::State>>,
    update_dialog: bool,
    update_banner_closed: bool,
    show_update_help: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Self {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert("dv".into(), std::sync::Arc::new(egui::FontData::from_static(scene::FONT_REGULAR)));
        fonts.font_data.insert("dvb".into(), std::sync::Arc::new(egui::FontData::from_static(scene::FONT_BOLD)));
        fonts.families.insert(FontFamily::Name("dv".into()), vec!["dv".into()]);
        fonts.families.insert(FontFamily::Name("dvb".into()), vec!["dvb".into()]);
        // DejaVu as a fallback gives the UI arrows and other symbols
        for fam in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts.families.entry(fam).or_default().push("dv".into());
        }
        cc.egui_ctx.set_fonts(fonts);
        cc.egui_ctx.set_visuals(egui::Visuals::light());

        let doc = templates::make("conceptual");
        let mut app = App {
            committed: doc.clone(),
            saved: doc.clone(),
            doc,
            undo: vec![],
            redo: vec![],
            pending_label: None,
            file: None,
            sel_n: BTreeSet::new(),
            sel_e: BTreeSet::new(),
            offset: Vec2::new(40.0, 40.0),
            zoom: 1.0,
            tool: Tool::Select,
            drag: Drag::None,
            editing: None,
            edit_focus_pending: false,
            snap: true,
            grid: true,
            defaults: EdgeDefaults {
                route: Route::Straight,
                start: Head::None,
                end: Head::Arrow,
                color: Rgba(31, 41, 55, 255),
                width: 2.0,
                dash: Dash::Solid,
            },
            status: "Welcome! Drag from the blue dots on a shape to connect it. Double-click to edit text.".into(),
            show_history: false,
            show_help: false,
            show_about: false,
            docx_dialog: None,
            confirm_close: false,
            allow_close: false,
            pending_after_save: None,
            ask_save: None,
            pending_template: None,
            pending_open: false,
            fit_request: true,
            canvas_rect: Rect::NOTHING,
            geoms: vec![],
            crossing_count: 0,
            sel_bend: None,
            ctx_target: CtxTarget::Canvas(P::new(0.0, 0.0)),
            move_step: 20.0,
            update: update::check_in_background(cc.egui_ctx.clone()),
            update_dialog: false,
            update_banner_closed: false,
            show_update_help: false,
        };
        if let Some(p) = open {
            app.open_path(p);
        }
        app
    }

    // ------------------------------------------------------------ history

    fn label(&mut self, s: &str) {
        self.pending_label = Some(s.to_string());
    }

    /// Record the current state as an undo step if it changed.
    fn commit(&mut self) {
        if self.doc != self.committed {
            let label = self.pending_label.take().unwrap_or_else(|| "Edit".into());
            self.undo.push((label, std::mem::replace(&mut self.committed, self.doc.clone())));
            if self.undo.len() > MAX_HISTORY {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        self.pending_label = None;
    }

    fn undo(&mut self) {
        self.commit();
        if let Some((label, prev)) = self.undo.pop() {
            self.redo.push((label.clone(), std::mem::replace(&mut self.doc, prev)));
            self.committed = self.doc.clone();
            self.status = format!("Undid: {label}");
            self.prune_selection();
        }
    }

    fn redo(&mut self) {
        self.commit();
        if let Some((label, next)) = self.redo.pop() {
            self.undo.push((label.clone(), std::mem::replace(&mut self.doc, next)));
            self.committed = self.doc.clone();
            self.status = format!("Redid: {label}");
            self.prune_selection();
        }
    }

    fn prune_selection(&mut self) {
        let doc = &self.doc;
        self.sel_n.retain(|id| doc.node(*id).is_some());
        self.sel_e.retain(|id| doc.edges.iter().any(|e| e.id == *id));
        if let Some(Editing::Node(id)) = self.editing {
            if doc.node(id).is_none() {
                self.editing = None;
            }
        }
        if let Some(Editing::Edge(id)) = self.editing {
            if !doc.edges.iter().any(|e| e.id == id) {
                self.editing = None;
            }
        }
    }

    fn dirty(&self) -> bool {
        self.doc != self.saved
    }

    // ------------------------------------------------------------ coords

    fn to_screen(&self, p: P) -> Pos2 {
        Pos2::new(self.canvas_rect.min.x + self.offset.x + p.x * self.zoom, self.canvas_rect.min.y + self.offset.y + p.y * self.zoom)
    }
    fn to_world(&self, s: Pos2) -> P {
        P::new((s.x - self.canvas_rect.min.x - self.offset.x) / self.zoom, (s.y - self.canvas_rect.min.y - self.offset.y) / self.zoom)
    }
    fn snap_v(&self, v: f32) -> f32 {
        if self.snap {
            (v / 10.0).round() * 10.0
        } else {
            v
        }
    }

    fn fit(&mut self) {
        let prims = scene::build(&self.doc, &geom::route_all(&self.doc), &SceneOpts { draw_background: false });
        if prims.is_empty() || self.canvas_rect.width() < 10.0 {
            return;
        }
        let (lo, hi) = export::prim_bounds(&prims);
        let (w, h) = (hi.x - lo.x + 80.0, hi.y - lo.y + 80.0);
        self.zoom = (self.canvas_rect.width() / w).min(self.canvas_rect.height() / h).clamp(0.2, 2.0);
        let c = lo.lerp(hi, 0.5);
        self.offset = Vec2::new(self.canvas_rect.width() / 2.0 - c.x * self.zoom, self.canvas_rect.height() / 2.0 - c.y * self.zoom);
    }

    // ------------------------------------------------------------ actions

    fn view_center(&self) -> P {
        self.to_world(self.canvas_rect.center())
    }

    fn add_shape(&mut self, shape: Shape, at: Option<P>) {
        self.label(&format!("Add {}", shape.name().to_lowercase()));
        let c = at.unwrap_or_else(|| {
            let c = self.view_center();
            let k = (self.doc.nodes.len() % 6) as f32 * 20.0;
            P::new(c.x + k, c.y + k)
        });
        let c = P::new(self.snap_v(c.x), self.snap_v(c.y));
        let text = if shape == Shape::Text { "Text" } else { "" };
        let id = self.doc.add_node(shape, c, text);
        self.sel_n = [id].into();
        self.sel_e.clear();
        self.start_edit(Editing::Node(id));
    }

    fn start_edit(&mut self, e: Editing) {
        self.editing = Some(e);
        self.edit_focus_pending = true;
    }

    fn connect(&mut self, from: u64, to: u64) -> u64 {
        self.label("Connect");
        let id = self.doc.add_edge(from, to);
        self.apply_defaults(id);
        let route = self.defaults.route;
        if route == Route::Curved {
            // give new curved connectors a gentle bend that the user can drag
            if let (Some(a), Some(b)) = (self.doc.node(from), self.doc.node(to)) {
                let (ca, cb) = (a.center(), b.center());
                let mid = ca.lerp(cb, 0.5).add(cb.sub(ca).perp().mul(0.18));
                self.doc.edge_mut(id).unwrap().bends = vec![mid];
            }
        }
        id
    }

    fn apply_defaults(&mut self, id: u64) {
        let d = &self.defaults;
        let (route, start, end, color, width, dash) = (d.route, d.start, d.end, d.color, d.width, d.dash);
        if let Some(e) = self.doc.edge_mut(id) {
            e.route = route;
            e.start_head = start;
            e.end_head = end;
            e.color = color;
            e.width = width;
            e.dash = dash;
        }
    }

    fn delete_selection(&mut self) {
        if self.sel_n.is_empty() && self.sel_e.is_empty() {
            return;
        }
        self.label("Delete");
        let n: Vec<u64> = self.sel_n.iter().copied().collect();
        let e: Vec<u64> = self.sel_e.iter().copied().collect();
        self.doc.delete(&n, &e);
        self.sel_n.clear();
        self.sel_e.clear();
        self.editing = None;
    }

    fn group_selection(&mut self) {
        if self.sel_n.len() < 2 {
            self.status = "Select two or more shapes to group them".into();
            return;
        }
        self.label("Group");
        let g = self.doc.new_id();
        // absorb any existing groups touched by the selection
        let old: BTreeSet<u64> = self.sel_n.iter().filter_map(|id| self.doc.node(*id).and_then(|n| n.group)).collect();
        for n in &mut self.doc.nodes {
            if self.sel_n.contains(&n.id) || n.group.is_some_and(|og| old.contains(&og)) {
                n.group = Some(g);
                self.sel_n.insert(n.id);
            }
        }
        self.status = format!("Grouped {} shapes — they now move together", self.sel_n.len());
    }

    fn ungroup_selection(&mut self) {
        let mut any = false;
        let sel = self.sel_n.clone();
        for n in &mut self.doc.nodes {
            if sel.contains(&n.id) && n.group.is_some() {
                n.group = None;
                any = true;
            }
        }
        if any {
            self.label("Ungroup");
            self.status = "Ungrouped".into();
        }
    }

    fn select_all(&mut self) {
        self.sel_n = self.doc.nodes.iter().map(|n| n.id).collect();
        self.sel_e = self.doc.edges.iter().map(|e| e.id).collect();
    }

    fn copy(&mut self, ctx: &egui::Context) {
        let nodes: Vec<Node> = self.doc.nodes.iter().filter(|n| self.sel_n.contains(&n.id)).cloned().collect();
        let edges: Vec<Edge> = self
            .doc
            .edges
            .iter()
            .filter(|e| self.sel_n.contains(&e.from) && self.sel_n.contains(&e.to))
            .cloned()
            .collect();
        if nodes.is_empty() {
            return;
        }
        if let Ok(s) = serde_json::to_string(&Clip { nodes, edges }) {
            ctx.copy_text(format!("{CLIP_PREFIX}{s}"));
            self.status = "Copied".into();
        }
    }

    fn paste_clip(&mut self, clip: Clip, offset: f32) {
        self.label("Paste");
        let mut map = std::collections::HashMap::new();
        let mut gmap = std::collections::HashMap::new();
        self.sel_n.clear();
        self.sel_e.clear();
        for mut n in clip.nodes {
            let nid = self.doc.new_id();
            map.insert(n.id, nid);
            n.id = nid;
            n.pos = n.pos.add(P::new(offset, offset));
            if let Some(g) = n.group {
                let ng = *gmap.entry(g).or_insert_with(|| self.doc.new_id());
                n.group = Some(ng);
            }
            self.sel_n.insert(nid);
            self.doc.nodes.push(n);
        }
        for mut e in clip.edges {
            let (Some(f), Some(t)) = (map.get(&e.from), map.get(&e.to)) else { continue };
            e.id = self.doc.new_id();
            e.from = *f;
            e.to = *t;
            for b in &mut e.bends {
                *b = b.add(P::new(offset, offset));
            }
            e.elbow_mid = e.elbow_mid.map(|m| m + offset);
            self.sel_e.insert(e.id);
            self.doc.edges.push(e);
        }
    }

    fn duplicate(&mut self) {
        let nodes: Vec<Node> = self.doc.nodes.iter().filter(|n| self.sel_n.contains(&n.id)).cloned().collect();
        let edges: Vec<Edge> = self
            .doc
            .edges
            .iter()
            .filter(|e| self.sel_n.contains(&e.from) && self.sel_n.contains(&e.to))
            .cloned()
            .collect();
        if !nodes.is_empty() {
            self.paste_clip(Clip { nodes, edges }, 30.0);
            self.label("Duplicate");
        }
    }

    fn reorder(&mut self, front: bool) {
        if self.sel_n.is_empty() {
            return;
        }
        self.label(if front { "Bring to front" } else { "Send to back" });
        let (mut sel, mut rest): (Vec<Node>, Vec<Node>) = self.doc.nodes.drain(..).partition(|n| self.sel_n.contains(&n.id));
        if front {
            rest.append(&mut sel);
            self.doc.nodes = rest;
        } else {
            sel.append(&mut rest);
            self.doc.nodes = sel;
        }
    }

    fn align(&mut self, how: &str) {
        let ids: Vec<u64> = self.sel_n.iter().copied().collect();
        if ids.len() < 2 {
            self.status = "Select two or more shapes to align".into();
            return;
        }
        self.label(&format!("Align {how}"));
        let ns: Vec<Node> = ids.iter().filter_map(|i| self.doc.node(*i).cloned()).collect();
        let minx = ns.iter().map(|n| n.pos.x).fold(f32::MAX, f32::min);
        let maxx = ns.iter().map(|n| n.pos.x + n.w).fold(f32::MIN, f32::max);
        let miny = ns.iter().map(|n| n.pos.y).fold(f32::MAX, f32::min);
        let maxy = ns.iter().map(|n| n.pos.y + n.h).fold(f32::MIN, f32::max);
        match how {
            "distribute horizontally" | "distribute vertically" => {
                let horiz = how.ends_with("horizontally");
                let mut sorted = ns.clone();
                sorted.sort_by(|a, b| {
                    let (ka, kb) = if horiz { (a.center().x, b.center().x) } else { (a.center().y, b.center().y) };
                    ka.partial_cmp(&kb).unwrap()
                });
                let total: f32 = sorted.iter().map(|n| if horiz { n.w } else { n.h }).sum();
                let span = if horiz { maxx - minx } else { maxy - miny };
                let gap = (span - total) / (sorted.len() as f32 - 1.0);
                let mut cur = if horiz { minx } else { miny };
                for n in sorted {
                    let node = self.doc.node_mut(n.id).unwrap();
                    if horiz {
                        node.pos.x = cur;
                        cur += n.w + gap;
                    } else {
                        node.pos.y = cur;
                        cur += n.h + gap;
                    }
                }
            }
            _ => {
                let first = ns[0].clone();
                for n in &ns {
                    let node = self.doc.node_mut(n.id).unwrap();
                    match how {
                        "left" => node.pos.x = minx,
                        "right" => node.pos.x = maxx - n.w,
                        "center" => node.pos.x = (minx + maxx) / 2.0 - n.w / 2.0,
                        "top" => node.pos.y = miny,
                        "bottom" => node.pos.y = maxy - n.h,
                        "middle" => node.pos.y = (miny + maxy) / 2.0 - n.h / 2.0,
                        "same size" => {
                            node.w = first.w;
                            node.h = first.h;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn minimize(&mut self) {
        self.label("Minimise crossings");
        let (b, a) = geom::minimize_crossings(&mut self.doc);
        self.status = if b == 0 {
            "No crossings to remove".into()
        } else if a == b {
            format!("These {b} crossing(s) can't be avoided by re-routing alone, so they stay drawn as hops. Moving a shape usually frees them.")
        } else if a == 0 {
            format!("Removed all {b} crossing(s). Undo (Ctrl+Z) if you prefer the old layout.")
        } else {
            format!("Crossings reduced from {b} to {a}; the remaining ones are drawn with hops. Move shapes apart to remove more.")
        };
    }

    // ------------------------------------------------------- connector tools

    fn edge(&self, id: u64) -> Option<&Edge> {
        self.doc.edges.iter().find(|e| e.id == id)
    }

    fn elbow_flag(&self, id: u64) -> bool {
        self.geoms.iter().find(|g| g.id == id).map(|g| g.elbow_vertical).unwrap_or(false)
    }

    /// Connectors whose both ends sit on shapes that are being moved.
    fn is_internal(&self, e: &Edge) -> bool {
        e.from_pt.is_none() && e.to_pt.is_none() && self.sel_n.contains(&e.from) && self.sel_n.contains(&e.to)
    }

    /// Give a connector something it can move: a line attached to shapes gets two
    /// bend points (so it can be pulled anywhere), an elbow gets a fixed middle.
    fn prepare_edge_for_move(&mut self, id: u64) {
        let Some(e) = self.edge(id).cloned() else { return };
        let Some(g) = geom::route_edge(&self.doc, &e) else { return };
        let attached = e.from_pt.is_none() || e.to_pt.is_none();
        if !attached {
            return;
        }
        let t = self.doc.edge_mut(id).unwrap();
        match e.route {
            Route::Elbow => {
                if e.elbow_mid.is_none() {
                    if let Some((_, hp)) = g.handles.iter().find(|(k, _)| *k == HandleKind::ElbowMid) {
                        t.elbow_vertical = Some(g.elbow_vertical);
                        t.elbow_mid = Some(if g.elbow_vertical { hp.y } else { hp.x });
                    }
                }
            }
            _ => {
                if e.bends.is_empty() {
                    let l = geom::polyline_len(&g.pts);
                    t.bends = vec![geom::point_at(&g.pts, l / 3.0).0, geom::point_at(&g.pts, 2.0 * l / 3.0).0];
                    t.lock = None;
                }
            }
        }
    }

    /// Snapshot of every connector that moves with the current selection.
    fn prepare_move_edges(&mut self) -> Vec<(Edge, bool)> {
        let ids: Vec<u64> = self.doc.edges.iter().filter(|e| self.sel_e.contains(&e.id) || self.is_internal(e)).map(|e| e.id).collect();
        for id in &ids {
            let internal = self.edge(*id).map(|e| self.is_internal(e)).unwrap_or(true);
            if !internal {
                self.prepare_edge_for_move(*id);
            }
        }
        // refresh routing so elbow flags match the prepared edges
        self.geoms = geom::route_all(&self.doc);
        ids.iter().filter_map(|id| self.edge(*id).cloned()).map(|e| {
            let v = self.elbow_flag(e.id);
            (e, v)
        }).collect()
    }

    fn start_move(&mut self, wp: P) {
        let orig = self.sel_n.iter().filter_map(|id| self.doc.node(*id).map(|nd| (*id, nd.pos))).collect();
        self.drag = Drag::Move { start: wp, orig, edge_orig: vec![], moved: false, prepared: false };
        self.label(if self.sel_n.is_empty() { "Move connector" } else { "Move" });
    }

    /// Move the selected shapes and connectors by `d` (arrow keys / panel buttons).
    fn nudge_selection(&mut self, d: P) {
        if self.sel_n.is_empty() && self.sel_e.is_empty() {
            return;
        }
        self.pending_label.get_or_insert_with(|| "Move".into());
        for (e, v) in self.prepare_move_edges() {
            let mut e = e;
            e.translate(d, v);
            if let Some(t) = self.doc.edge_mut(e.id) {
                *t = e;
            }
        }
        for id in self.sel_n.clone() {
            if let Some(n) = self.doc.node_mut(id) {
                n.pos = n.pos.add(d);
            }
        }
    }

    fn remove_bend(&mut self, eid: u64, i: usize) {
        if let Some(e) = self.doc.edge_mut(eid) {
            if i < e.bends.len() {
                e.bends.remove(i);
                self.label("Remove bend");
                self.status = "Bend removed".into();
            }
        }
        self.sel_bend = None;
    }

    fn remove_all_bends(&mut self) {
        let ids: Vec<u64> = self.sel_e.iter().copied().collect();
        if ids.is_empty() {
            self.status = "Select a connector first".into();
            return;
        }
        self.label("Remove all bends");
        for id in ids {
            if let Some(e) = self.doc.edge_mut(id) {
                e.bends.clear();
                e.elbow_mid = None;
                e.elbow_vertical = None;
                if e.route == Route::Elbow {
                    e.route = Route::Straight;
                }
            }
        }
        self.sel_bend = None;
        self.status = "All bends removed — the line now runs straight".into();
    }

    /// Move shapes (and connectors that run between them) by `d`.
    fn move_nodes_by(&mut self, ids: &[u64], d: P) {
        for id in ids {
            if let Some(n) = self.doc.node_mut(*id) {
                n.pos = n.pos.add(d);
            }
        }
        let geoms = geom::route_all(&self.doc);
        for e in &mut self.doc.edges {
            if e.from_pt.is_none() && e.to_pt.is_none() && ids.contains(&e.from) && ids.contains(&e.to) {
                let v = geoms.iter().find(|g| g.id == e.id).map(|g| g.elbow_vertical).unwrap_or(false);
                e.translate(d, v);
            }
        }
    }

    /// Would moving `ids` by `d` put any of them on top of another shape?
    fn would_overlap(&self, ids: &[u64], d: P) -> bool {
        let m = 12.0;
        self.doc.nodes.iter().filter(|n| ids.contains(&n.id)).any(|n| {
            let (x0, y0, x1, y1) = (n.pos.x + d.x - m, n.pos.y + d.y - m, n.pos.x + d.x + n.w + m, n.pos.y + d.y + n.h + m);
            self.doc
                .nodes
                .iter()
                .filter(|o| !ids.contains(&o.id))
                .any(|o| x0 < o.pos.x + o.w && x1 > o.pos.x && y0 < o.pos.y + o.h && y1 > o.pos.y)
        })
    }

    /// Does the straight segment p–q avoid every shape not in `skip`?
    fn path_clear(&self, p: P, q: P, skip: &[u64]) -> bool {
        let n = ((p.dist(q) / 4.0) as usize).max(2);
        !self.doc.nodes.iter().filter(|o| !skip.contains(&o.id)).any(|o| {
            (0..=n).any(|k| {
                let r = p.lerp(q, k as f32 / n as f32);
                r.x > o.pos.x - 4.0 && r.x < o.pos.x + o.w + 4.0 && r.y > o.pos.y - 4.0 && r.y < o.pos.y + o.h + 4.0
            })
        })
    }

    /// Make one connector exactly horizontal (or vertical). Returns a note for the status bar.
    fn axis_edge(&mut self, id: u64, vertical: bool) -> Option<String> {
        let e = self.edge(id)?.clone();
        let axis = if vertical { Axis::Vertical } else { Axis::Horizontal };
        {
            let t = self.doc.edge_mut(id)?;
            t.bends.clear();
            t.elbow_mid = None;
            t.elbow_vertical = None;
            t.route = Route::Straight;
            t.lock = Some(axis);
        }
        let across = |p: P| if vertical { p.x } else { p.y };
        let set_across = |p: P, v: f32| if vertical { P::new(v, p.y) } else { P::new(p.x, v) };
        match (e.from_pt, e.to_pt) {
            (Some(p), Some(q)) => {
                // keep the arrow's length; turn it to the chosen direction
                let d = q.sub(p);
                let len = d.len().max(20.0);
                let along = if vertical { d.y } else { d.x };
                let sign = if along < 0.0 { -1.0 } else { 1.0 };
                let nq = if vertical { P::new(p.x, p.y + sign * len) } else { P::new(p.x + sign * len, p.y) };
                self.doc.edge_mut(id)?.to_pt = Some(nq);
            }
            (Some(p), None) => {
                let c = self.doc.node(e.to)?.center();
                self.doc.edge_mut(id)?.from_pt = Some(set_across(p, across(c)));
            }
            (None, Some(q)) => {
                let c = self.doc.node(e.from)?.center();
                self.doc.edge_mut(id)?.to_pt = Some(set_across(q, across(c)));
            }
            (None, None) => {
                let (a, b) = (self.doc.node(e.from)?.clone(), self.doc.node(e.to)?.clone());
                if geom::locked_ends(geom::End::Node(&a), geom::End::Node(&b), vertical).is_none() {
                    // The shapes don't line up: move one of them (with its group) into line,
                    // choosing a spot where it doesn't land on top of another shape.
                    let mem_b = self.doc.group_members(b.id);
                    let mem_a = self.doc.group_members(a.id);
                    if mem_b.contains(&a.id) {
                        return Some("The two shapes are in the same group, so they can't be lined up. Ungroup them first.".into());
                    }
                    let delta = across(a.center()) - across(b.center());
                    let off = |v: f32| if vertical { P::new(v, 0.0) } else { P::new(0.0, v) };
                    let slide = |v: f32| if vertical { P::new(0.0, v) } else { P::new(v, 0.0) };
                    // direction pointing from a towards b along the line
                    let away = {
                        let along = if vertical { b.center().y - a.center().y } else { b.center().x - a.center().x };
                        if along < 0.0 { -1.0 } else { 1.0 }
                    };
                    let mut plan: Option<(Vec<u64>, P, &Node)> = None;
                    'search: for k in 0..30 {
                        let extra = slide(away * 20.0 * k as f32);
                        for (members, d, who) in [(&mem_b, off(delta).add(extra), &b), (&mem_a, off(-delta).sub(extra), &a)] {
                            let (pa, pb) = if who.id == b.id { (a.center(), b.center().add(d)) } else { (a.center().add(d), b.center()) };
                            let skip: Vec<u64> = mem_a.iter().chain(mem_b.iter()).copied().collect();
                            if !self.would_overlap(members, d) && self.path_clear(pa, pb, &skip) {
                                plan = Some((members.clone(), d, who));
                                break 'search;
                            }
                        }
                    }
                    let Some((members, d, who)) = plan else {
                        // nowhere clear to put either shape: use right-angled segments instead
                        if let Some(t) = self.doc.edge_mut(id) {
                            t.lock = None;
                            t.route = Route::Elbow;
                            t.elbow_vertical = Some(vertical);
                        }
                        return Some(format!(
                            "No clear spot to line the shapes up, so the line now uses {} right-angled segments.",
                            if vertical { "vertical" } else { "horizontal" }
                        ));
                    };
                    self.move_nodes_by(&members, d);
                    let name = who.text.lines().next().unwrap_or("").trim().to_string();
                    let msg = format!(
                        "Moved {} so the line can be {}.",
                        if name.is_empty() { "a shape".to_string() } else { format!("\"{name}\"") },
                        if vertical { "vertical" } else { "horizontal" }
                    );
                    return Some(msg);
                }
            }
        }
        None
    }

    fn make_axis(&mut self, vertical: bool) {
        let ids: Vec<u64> = self.sel_e.iter().copied().collect();
        if ids.is_empty() {
            self.status = "Select a connector first (click the line)".into();
            return;
        }
        self.label(if vertical { "Make vertical" } else { "Make horizontal" });
        let mut notes = vec![];
        for id in ids {
            if let Some(n) = self.axis_edge(id, vertical) {
                notes.push(n);
            }
        }
        self.sel_bend = None;
        self.status = if notes.is_empty() {
            format!("Line is now {} and stays that way when you move the shapes (as long as they overlap)", if vertical { "vertical" } else { "horizontal" })
        } else {
            notes.join(" ")
        };
    }

    /// Turn every slanted segment into a horizontal or vertical one.
    fn snap_right_angles(&mut self) {
        let ids: Vec<u64> = self.sel_e.iter().copied().collect();
        if ids.is_empty() {
            self.status = "Select a connector first (click the line)".into();
            return;
        }
        self.label("Snap to right angles");
        for id in ids {
            let Some(e) = self.edge(id).cloned() else { continue };
            let Some((a, b)) = geom::edge_ends(&self.doc, &e) else { continue };
            let (start, end) = (a.center(), b.center());
            if e.route == Route::Elbow {
                continue; // already right-angled
            }
            if e.bends.is_empty() {
                let d = end.sub(start);
                let vertical = d.y.abs() > d.x.abs();
                self.axis_edge(id, vertical);
                continue;
            }
            let mut bends = e.bends.clone();
            let n = bends.len();
            let mut prev = start;
            for i in 0..n {
                let b = bends[i];
                bends[i] = if i == n - 1 {
                    let c1 = P::new(end.x, prev.y);
                    let c2 = P::new(prev.x, end.y);
                    if c1.dist(b) <= c2.dist(b) { c1 } else { c2 }
                } else if (b.x - prev.x).abs() >= (b.y - prev.y).abs() {
                    P::new(b.x, prev.y)
                } else {
                    P::new(prev.x, b.y)
                };
                prev = bends[i];
            }
            if let Some(t) = self.doc.edge_mut(id) {
                t.bends = bends;
                t.route = Route::Straight;
                t.lock = None;
            }
        }
        self.status = "Every segment is now horizontal or vertical".into();
    }

    fn reverse_edges(&mut self) {
        self.label("Reverse direction");
        for id in self.sel_e.clone() {
            if let Some(t) = self.doc.edge_mut(id) {
                std::mem::swap(&mut t.from, &mut t.to);
                std::mem::swap(&mut t.from_pt, &mut t.to_pt);
                t.bends.reverse();
            }
        }
    }

    fn edge_menu_items(&mut self, ui: &mut egui::Ui) {
        let has_bends = self.sel_e.iter().filter_map(|id| self.edge(*id)).any(|e| !e.bends.is_empty() || e.route == Route::Elbow);
        if ui.add_enabled(has_bends, egui::Button::new("Remove all bends (straight line)    Shift+B")).clicked() {
            self.remove_all_bends();
            ui.close_menu();
        }
        if ui.button("─  Make horizontal    Shift+H").clicked() {
            self.make_axis(false);
            ui.close_menu();
        }
        if ui.button("│  Make vertical    Shift+V").clicked() {
            self.make_axis(true);
            ui.close_menu();
        }
        if ui.button("└  Snap all segments to right angles").clicked() {
            self.snap_right_angles();
            ui.close_menu();
        }
        ui.separator();
        if ui.button("⇄  Reverse direction").clicked() {
            self.reverse_edges();
            ui.close_menu();
        }
        if ui.button("🗑  Delete connector").clicked() {
            self.delete_selection();
            ui.close_menu();
        }
    }

    fn context_menu_ui(&mut self, ui: &mut egui::Ui) {
        match self.ctx_target {
            CtxTarget::Bend(eid, i) => {
                if ui.button("×  Remove this bend    Del").clicked() {
                    self.remove_bend(eid, i);
                    ui.close_menu();
                }
                self.edge_menu_items(ui);
            }
            CtxTarget::Edge(_) => self.edge_menu_items(ui),
            CtxTarget::Node(n) => {
                if ui.button("Edit text    F2").clicked() {
                    self.start_edit(Editing::Node(n));
                    ui.close_menu();
                }
                if ui.button("Duplicate    Ctrl+D").clicked() {
                    self.duplicate();
                    ui.close_menu();
                }
                if ui.button("Group selected    Ctrl+G").clicked() {
                    self.group_selection();
                    ui.close_menu();
                }
                if ui.button("Ungroup    Ctrl+Shift+G").clicked() {
                    self.ungroup_selection();
                    ui.close_menu();
                }
                if ui.button("Bring to front").clicked() {
                    self.reorder(true);
                    ui.close_menu();
                }
                if ui.button("Send to back").clicked() {
                    self.reorder(false);
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("🗑  Delete    Del").clicked() {
                    self.delete_selection();
                    ui.close_menu();
                }
            }
            CtxTarget::Canvas(p) => {
                if ui.button("Add a box here").clicked() {
                    self.add_shape(Shape::RoundedRect, Some(p));
                    ui.close_menu();
                }
                if ui.button("Draw a free arrow    A").clicked() {
                    self.tool = Tool::Arrow;
                    ui.close_menu();
                }
                if ui.button("Select all    Ctrl+A").clicked() {
                    self.select_all();
                    ui.close_menu();
                }
                if ui.button("Fit to window    Ctrl+0").clicked() {
                    self.fit();
                    ui.close_menu();
                }
            }
        }
    }

    // ------------------------------------------------------------ files

    fn new_from(&mut self, kind: &str) {
        self.label("New document");
        self.doc = templates::make(kind);
        self.committed = self.doc.clone();
        self.saved = self.doc.clone();
        self.undo.clear();
        self.redo.clear();
        self.file = None;
        self.sel_n.clear();
        self.sel_e.clear();
        self.editing = None;
        self.fit_request = true;
    }

    fn open_path(&mut self, p: PathBuf) {
        match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|s| serde_json::from_str::<Diagram>(&s).map_err(|e| e.to_string())) {
            Ok(d) => {
                self.doc = d;
                self.committed = self.doc.clone();
                self.saved = self.doc.clone();
                self.undo.clear();
                self.redo.clear();
                self.sel_n.clear();
                self.sel_e.clear();
                self.editing = None;
                self.status = format!("Opened {}", p.display());
                self.file = Some(p);
                self.fit_request = true;
            }
            Err(e) => self.status = format!("Could not open file: {e}"),
        }
    }

    fn open_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new().add_filter("FrameMap diagram", &["fmap"]).pick_file() {
            self.open_path(p);
        }
    }

    fn save(&mut self, save_as: bool) -> bool {
        let path = if save_as || self.file.is_none() {
            let name = format!("{}.fmap", sanitize(&self.doc.title));
            match rfd::FileDialog::new().add_filter("FrameMap diagram", &["fmap"]).set_file_name(name).save_file() {
                Some(p) => p,
                None => return false,
            }
        } else {
            self.file.clone().unwrap()
        };
        match serde_json::to_string_pretty(&self.doc).map_err(|e| e.to_string()).and_then(|s| std::fs::write(&path, s).map_err(|e| e.to_string())) {
            Ok(()) => {
                self.saved = self.doc.clone();
                self.status = format!("Saved {}", path.display());
                self.file = Some(path);
                true
            }
            Err(e) => {
                self.status = format!("Save failed: {e}");
                false
            }
        }
    }

    fn export_docx(&mut self, landscape: bool, include_title: bool, caption: String) {
        let name = format!("{}.docx", sanitize(&self.doc.title));
        let Some(p) = rfd::FileDialog::new().add_filter("Word document", &["docx"]).set_file_name(name).save_file() else { return };
        match export::write_docx(&self.doc, &DocxOptions { landscape, include_title, caption }) {
            Ok(bytes) => match std::fs::write(&p, bytes) {
                Ok(()) => self.status = format!("Exported to Word: {}", p.display()),
                Err(e) => self.status = format!("Export failed: {e}"),
            },
            Err(e) => self.status = format!("Export failed: {e}"),
        }
    }

    fn export_png(&mut self) {
        let name = format!("{}.png", sanitize(&self.doc.title));
        let Some(p) = rfd::FileDialog::new().add_filter("PNG image", &["png"]).set_file_name(name).save_file() else { return };
        match export::render_png(&self.doc, 3.0, false).and_then(|(b, _, _)| std::fs::write(&p, b).map_err(|e| e.to_string())) {
            Ok(()) => self.status = format!("Exported image: {}", p.display()),
            Err(e) => self.status = format!("Export failed: {e}"),
        }
    }

    fn export_svg(&mut self) {
        let name = format!("{}.svg", sanitize(&self.doc.title));
        let Some(p) = rfd::FileDialog::new().add_filter("SVG vector image", &["svg"]).set_file_name(name).save_file() else { return };
        match std::fs::write(&p, export::render_svg(&self.doc).0) {
            Ok(()) => self.status = format!("Exported SVG: {}", p.display()),
            Err(e) => self.status = format!("Export failed: {e}"),
        }
    }

    /// Run `f` now if there are no unsaved changes, otherwise ask first.
    fn guard(&mut self, what: &'static str, f: Box<dyn FnOnce(&mut App)>) {
        if self.dirty() {
            self.ask_save = Some(what);
            self.pending_after_save = Some(f);
        } else {
            f(self);
        }
    }
}

fn sanitize(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' }).collect();
    let t = t.trim().to_string();
    if t.is_empty() {
        "framework".into()
    } else {
        t
    }
}

// ---------------------------------------------------------------- triangulation

/// Ear-clipping triangulation for filling non-convex shapes.
fn triangulate(pts: &[Pos2]) -> Vec<[usize; 3]> {
    let n = pts.len();
    if n < 3 {
        return vec![];
    }
    let area: f32 = (0..n).map(|i| {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        a.x * b.y - b.x * a.y
    }).sum();
    let mut idx: Vec<usize> = if area > 0.0 { (0..n).collect() } else { (0..n).rev().collect() };
    let cross = |o: Pos2, a: Pos2, b: Pos2| (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
    let mut tris = vec![];
    let mut guard = 0;
    while idx.len() > 3 && guard < 10 * n {
        guard += 1;
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let (ia, ib, ic) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let (a, b, c) = (pts[ia], pts[ib], pts[ic]);
            if cross(a, b, c) <= 1e-6 {
                continue;
            }
            let inside = idx.iter().any(|&j| {
                if j == ia || j == ib || j == ic {
                    return false;
                }
                let p = pts[j];
                cross(a, b, p) > 0.0 && cross(b, c, p) > 0.0 && cross(c, a, p) > 0.0
            });
            if !inside {
                tris.push([ia, ib, ic]);
                idx.remove(i);
                clipped = true;
                break;
            }
        }
        if !clipped {
            break;
        }
    }
    if idx.len() == 3 {
        tris.push([idx[0], idx[1], idx[2]]);
    } else if idx.len() > 3 {
        for i in 1..idx.len() - 1 {
            tris.push([idx[0], idx[i], idx[i + 1]]);
        }
    }
    tris
}

// ---------------------------------------------------------------- drawing

impl App {
    fn paint_prims(&self, painter: &egui::Painter, prims: &[Prim]) {
        let z = self.zoom;
        for p in prims {
            match p {
                Prim::Poly { pts, closed, fill, stroke } => {
                    if pts.len() < 2 {
                        continue;
                    }
                    let sp: Vec<Pos2> = pts.iter().map(|q| self.to_screen(*q)).collect();
                    if let (Some(f), true) = (fill, *closed) {
                        let mut mesh = egui::Mesh::default();
                        for q in &sp {
                            mesh.colored_vertex(*q, c32(*f));
                        }
                        for t in triangulate(&sp) {
                            mesh.add_triangle(t[0] as u32, t[1] as u32, t[2] as u32);
                        }
                        painter.add(EShape::mesh(mesh));
                    }
                    if let Some(s) = stroke {
                        let st = Stroke::new((s.width * z).max(0.6), c32(s.color));
                        let mut line = sp.clone();
                        if *closed {
                            line.push(sp[0]);
                        }
                        match s.dash {
                            Dash::Solid => {
                                painter.add(EShape::line(line, st));
                            }
                            Dash::Dashed => {
                                painter.extend(EShape::dashed_line(&line, st, (s.width * 4.0 + 4.0) * z, (s.width * 2.5 + 3.0) * z));
                            }
                            Dash::Dotted => {
                                painter.extend(EShape::dashed_line(&line, st, s.width.max(1.0) * z, (s.width * 2.0 + 2.0) * z));
                            }
                        }
                    }
                }
                Prim::Text { cx, top, text, size, color, bold } => {
                    if text.is_empty() {
                        continue;
                    }
                    let fam = FontFamily::Name(if *bold { "dvb" } else { "dv" }.into());
                    let px = *size * z;
                    if px < 2.0 {
                        continue;
                    }
                    let galley = painter.layout_no_wrap(text.clone(), FontId::new(px, fam), c32(*color));
                    let line_h = size * LINE_H * z;
                    let pos = self.to_screen(P::new(*cx, *top)) + Vec2::new(-galley.size().x / 2.0, (line_h - galley.size().y) / 2.0);
                    painter.galley(pos, galley, c32(*color));
                }
            }
        }
    }

    fn resize_handles(&self, n: &Node) -> [Pos2; 8] {
        let a = self.to_screen(n.pos);
        let b = self.to_screen(n.pos.add(P::new(n.w, n.h)));
        let m = a.lerp(b, 0.5);
        [
            a,
            Pos2::new(m.x, a.y),
            Pos2::new(b.x, a.y),
            Pos2::new(b.x, m.y),
            b,
            Pos2::new(m.x, b.y),
            Pos2::new(a.x, b.y),
            Pos2::new(a.x, m.y),
        ]
    }

    /// Connection dots sit just outside the shape (so they never clash with resize handles).
    fn conn_dots(&self, n: &Node) -> [P; 4] {
        let c = n.center();
        let off = 16.0 / self.zoom;
        let dirs = [P::new(0.0, -1.0), P::new(1.0, 0.0), P::new(0.0, 1.0), P::new(-1.0, 0.0)];
        dirs.map(|d| geom::anchor(n, c.add(d)).add(d.mul(off)))
    }

    fn node_at(&self, p: P) -> Option<u64> {
        self.doc.nodes.iter().rev().find(|n| {
            let poly = geom::outline(n);
            geom::point_in_poly(p, &poly) || (n.shape == Shape::Text && n.contains(p))
        }).map(|n| n.id)
    }

    /// Topmost node whose box (grown by a margin) contains `p` — keeps dots visible just outside a shape.
    fn node_near(&self, p: P) -> Option<u64> {
        let m = 26.0 / self.zoom;
        self.node_at(p).or_else(|| {
            self.doc
                .nodes
                .iter()
                .rev()
                .find(|n| p.x > n.pos.x - m && p.x < n.pos.x + n.w + m && p.y > n.pos.y - m && p.y < n.pos.y + n.h + m)
                .map(|n| n.id)
        })
    }

    fn edge_at(&self, p: P) -> Option<u64> {
        let tol = 7.0 / self.zoom;
        self.geoms
            .iter()
            .map(|g| (g.id, geom::dist_to_polyline(p, &g.pts)))
            .filter(|(_, d)| *d < tol)
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .map(|(id, _)| id)
    }

    fn select_node_click(&mut self, id: u64, additive: bool, alt: bool) {
        let members = if alt { vec![id] } else { self.doc.group_members(id) };
        if additive {
            if self.sel_n.contains(&id) {
                for m in members {
                    self.sel_n.remove(&m);
                }
            } else {
                self.sel_n.extend(members);
            }
        } else if !self.sel_n.contains(&id) {
            self.sel_n = members.into_iter().collect();
            self.sel_e.clear();
        }
    }

    // ------------------------------------------------------------ canvas

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        self.canvas_rect = resp.rect;
        if self.fit_request {
            self.fit();
            self.fit_request = false;
        }
        let ctx = ui.ctx().clone();
        painter.rect_filled(resp.rect, CornerRadius::ZERO, Color32::from_rgb(241, 245, 249));

        // ---- zoom & pan with wheel
        if resp.hovered() {
            let (zd, scroll, hover) = ctx.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.pointer.hover_pos()));
            if zd != 1.0 {
                if let Some(h) = hover {
                    let w = self.to_world(h);
                    self.zoom = (self.zoom * zd).clamp(0.15, 5.0);
                    let s = self.to_screen(w);
                    self.offset += h - s;
                }
            } else if scroll != Vec2::ZERO {
                self.offset += scroll;
            }
        }

        // ---- paper / grid
        let paper_bg = c32(self.doc.background);
        painter.rect_filled(resp.rect.shrink(0.0), CornerRadius::ZERO, paper_bg);
        if self.grid {
            let step = 20.0 * self.zoom;
            if step > 6.0 {
                let ox = (self.canvas_rect.min.x + self.offset.x).rem_euclid(step);
                let oy = (self.canvas_rect.min.y + self.offset.y).rem_euclid(step);
                let col = Color32::from_rgba_unmultiplied(148, 163, 184, 70);
                let mut x = resp.rect.min.x - (resp.rect.min.x - ox).rem_euclid(step) + step;
                while x < resp.rect.max.x {
                    let mut y = resp.rect.min.y - (resp.rect.min.y - oy).rem_euclid(step) + step;
                    while y < resp.rect.max.y {
                        painter.circle_filled(Pos2::new(x, y), 1.0, col);
                        y += step;
                    }
                    x += step;
                }
            }
        }

        // ---- interaction
        let pointer = ctx.input(|i| i.pointer.hover_pos());
        let wp = pointer.map(|p| self.to_world(p)).unwrap_or_default();
        let (pressed, released, down, mods, dbl, middle_pressed, middle_down, space) = ctx.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.pointer.primary_down(),
                i.modifiers,
                i.pointer.button_double_clicked(egui::PointerButton::Primary),
                i.pointer.button_pressed(egui::PointerButton::Middle),
                i.pointer.button_down(egui::PointerButton::Middle),
                i.key_down(Key::Space),
            )
        });
        let over_canvas = resp.hovered();
        let hovered_node = if over_canvas { self.node_at(wp) } else { None };

        // middle-button or space-drag pans
        if over_canvas && (middle_pressed || (pressed && space && self.editing.is_none())) {
            self.drag = Drag::Pan { last: pointer.unwrap_or_default() };
        }

        let single_sel_node = if self.sel_n.len() == 1 && self.sel_e.is_empty() { self.sel_n.iter().next().copied() } else { None };
        let single_sel_edge = if self.sel_e.len() == 1 && self.sel_n.is_empty() { self.sel_e.iter().next().copied() } else { None };

        if dbl && over_canvas && matches!(self.drag, Drag::None) {
            // double click: remove bend / edit text / edit label / new shape
            let mut handled = false;
            if let Some(eid) = single_sel_edge {
                if let Some(g) = self.geoms.iter().find(|g| g.id == eid) {
                    for (k, hp) in g.handles.clone() {
                        if let HandleKind::Bend(i) = k {
                            if self.to_screen(hp).distance(pointer.unwrap()) < 8.0 {
                                self.label("Remove bend");
                                self.doc.edge_mut(eid).unwrap().bends.remove(i);
                                handled = true;
                                break;
                            }
                        }
                    }
                }
            }
            if !handled {
                if let Some(n) = hovered_node {
                    self.sel_n = [n].into();
                    self.sel_e.clear();
                    self.start_edit(Editing::Node(n));
                } else if let Some(e) = self.edge_at(wp) {
                    self.sel_e = [e].into();
                    self.sel_n.clear();
                    self.start_edit(Editing::Edge(e));
                } else {
                    self.add_shape(Shape::RoundedRect, Some(wp));
                }
            }
        } else if pressed && over_canvas && matches!(self.drag, Drag::None) {
            let sp = ctx.input(|i| i.pointer.press_origin()).or(pointer).unwrap();
            let wp = self.to_world(sp);
            let hovered_node = self.node_at(wp);
            self.sel_bend = None;
            // finish any text editing when clicking on the canvas
            if self.editing.is_some() {
                self.editing = None;
            }
            let mut started = false;
            // 1) resize handles
            if let Some(id) = single_sel_node {
                if let Some(n) = self.doc.node(id).cloned() {
                    for (i, h) in self.resize_handles(&n).iter().enumerate() {
                        if h.distance(sp) < 7.0 {
                            self.drag = Drag::Resize { id, handle: i, orig: n.clone(), start: wp };
                            self.label("Resize");
                            started = true;
                            break;
                        }
                    }
                }
            }
            // 2) edge handles
            if !started {
                if let Some(eid) = single_sel_edge {
                    if let Some(g) = self.geoms.iter().find(|g| g.id == eid).cloned() {
                        let ends = [(true, g.pts[0]), (false, *g.pts.last().unwrap())];
                        for (is_start, p) in ends {
                            if self.to_screen(p).distance(sp) < 8.0 {
                                self.drag = Drag::EdgeEnd { edge: eid, is_start, cur: wp };
                                started = true;
                                break;
                            }
                        }
                        if !started {
                            for (k, hp) in g.handles {
                                if self.to_screen(hp).distance(sp) < 8.0 {
                                    let kind = match k {
                                        HandleKind::Insert(i) => {
                                            let e = self.doc.edge_mut(eid).unwrap();
                                            e.bends.insert(i, hp);
                                            e.lock = None;
                                            self.label("Bend connector");
                                            self.sel_bend = Some((eid, i));
                                            HandleKind::Bend(i)
                                        }
                                        HandleKind::Bend(i) => {
                                            self.label("Move bend");
                                            self.sel_bend = Some((eid, i));
                                            HandleKind::Bend(i)
                                        }
                                        other => {
                                            self.label("Move bend");
                                            other
                                        }
                                    };
                                    self.drag = Drag::EdgeHandle { edge: eid, kind };
                                    started = true;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            // 3) connection dots on hovered/selected node
            if !started {
                let candidates: Vec<u64> = self.node_near(wp).into_iter().chain(self.sel_n.iter().copied()).collect();
                'outer: for id in candidates {
                    if let Some(n) = self.doc.node(id) {
                        for d in self.conn_dots(n) {
                            if self.to_screen(d).distance(sp) < 9.0 {
                                self.drag = Drag::Connect { from: id, cur: wp };
                                started = true;
                                break 'outer;
                            }
                        }
                    }
                }
            }
            if !started {
                match self.tool {
                    Tool::Place(shape) => {
                        self.add_shape(shape, Some(wp));
                        if !mods.shift {
                            self.tool = Tool::Select;
                        }
                    }
                    Tool::Arrow => {
                        let start = P::new(self.snap_v(wp.x), self.snap_v(wp.y));
                        self.drag = Drag::Draw { start, from: hovered_node, cur: start };
                    }
                    Tool::Connect => {
                        if let Some(n) = hovered_node {
                            self.drag = Drag::Connect { from: n, cur: wp };
                        } else {
                            self.drag = Drag::Marquee { start: wp, cur: wp, additive: mods.shift || mods.command };
                        }
                    }
                    Tool::Select => {
                        if let Some(n) = hovered_node {
                            self.select_node_click(n, mods.shift || mods.command, mods.alt);
                            if self.sel_n.contains(&n) {
                                self.start_move(wp);
                            }
                        } else if let Some(e) = self.edge_at(wp) {
                            if mods.shift || mods.command {
                                if !self.sel_e.remove(&e) {
                                    self.sel_e.insert(e);
                                }
                            } else if !self.sel_e.contains(&e) {
                                self.sel_e = [e].into();
                                self.sel_n.clear();
                            }
                            // dragging the line itself moves it anywhere
                            if self.sel_e.contains(&e) {
                                self.start_move(wp);
                            }
                        } else {
                            if !(mods.shift || mods.command) {
                                self.sel_n.clear();
                                self.sel_e.clear();
                            }
                            self.drag = Drag::Marquee { start: wp, cur: wp, additive: mods.shift || mods.command };
                        }
                    }
                }
            }
        }

        // ---- dragging
        let drag = self.drag.clone();
        match drag {
            Drag::Pan { last } => {
                if let Some(p) = pointer {
                    self.offset += p - last;
                    self.drag = Drag::Pan { last: p };
                }
                ctx.set_cursor_icon(CursorIcon::Grabbing);
                if !down && !middle_down {
                    self.drag = Drag::None;
                }
            }
            Drag::Move { start, orig, mut edge_orig, moved, mut prepared } => {
                let mut dlt = wp.sub(start);
                if let Some((_, p0)) = orig.first() {
                    let target = p0.add(dlt);
                    dlt = P::new(self.snap_v(target.x) - p0.x, self.snap_v(target.y) - p0.y);
                } else {
                    dlt = P::new(self.snap_v(dlt.x), self.snap_v(dlt.y));
                }
                if mods.shift {
                    // constrain to a straight horizontal / vertical / diagonal move
                    dlt = snap_angle(P::new(0.0, 0.0), dlt);
                }
                let moved = moved || dlt.len() > 0.5;
                if moved {
                    if !prepared {
                        edge_orig = self.prepare_move_edges();
                        prepared = true;
                    }
                    for (id, p) in &orig {
                        if let Some(n) = self.doc.node_mut(*id) {
                            n.pos = p.add(dlt);
                        }
                    }
                    for (e0, v) in &edge_orig {
                        let mut e = e0.clone();
                        e.translate(dlt, *v);
                        if let Some(t) = self.doc.edge_mut(e.id) {
                            *t = e;
                        }
                    }
                    ctx.set_cursor_icon(CursorIcon::Grabbing);
                }
                self.drag = Drag::Move { start, orig, edge_orig, moved, prepared };
                if released || !down {
                    self.drag = Drag::None;
                }
            }
            Drag::Draw { start, from, .. } => {
                let mut cur = P::new(self.snap_v(wp.x), self.snap_v(wp.y));
                if mods.shift {
                    cur = snap_angle(start, cur);
                }
                self.drag = Drag::Draw { start, from, cur };
                ctx.set_cursor_icon(CursorIcon::Crosshair);
                if released || !down {
                    self.drag = Drag::None;
                    let to = hovered_node.filter(|n| Some(*n) != from);
                    if start.dist(cur) > 12.0 || to.is_some() {
                        self.label("Draw arrow");
                        let id = self.doc.add_edge(from.unwrap_or(0), to.unwrap_or(0));
                        self.apply_defaults(id);
                        if let Some(e) = self.doc.edge_mut(id) {
                            if e.route == Route::Elbow && (from.is_none() || to.is_none()) {
                                e.route = Route::Straight;
                            }
                            if from.is_none() {
                                e.from_pt = Some(start);
                            }
                            if to.is_none() {
                                e.to_pt = Some(cur);
                            }
                        }
                        self.sel_e = [id].into();
                        self.sel_n.clear();
                        self.status = "Arrow drawn. Drag the line to move it, drag a green end to resize or attach it to a shape.".into();
                    }
                    if !mods.shift {
                        self.tool = Tool::Select;
                    }
                }
            }
            Drag::Resize { id, handle, orig, start } => {
                let d = wp.sub(start);
                let (mut x0, mut y0, mut x1, mut y1) = (orig.pos.x, orig.pos.y, orig.pos.x + orig.w, orig.pos.y + orig.h);
                match handle {
                    0 => {
                        x0 += d.x;
                        y0 += d.y
                    }
                    1 => y0 += d.y,
                    2 => {
                        x1 += d.x;
                        y0 += d.y
                    }
                    3 => x1 += d.x,
                    4 => {
                        x1 += d.x;
                        y1 += d.y
                    }
                    5 => y1 += d.y,
                    6 => {
                        x0 += d.x;
                        y1 += d.y
                    }
                    _ => x0 += d.x,
                }
                let (x0, y0, x1, y1) = (self.snap_v(x0), self.snap_v(y0), self.snap_v(x1), self.snap_v(y1));
                if let Some(n) = self.doc.node_mut(id) {
                    n.pos = P::new(x0.min(x1 - 20.0), y0.min(y1 - 20.0));
                    n.w = (x1 - x0).abs().max(20.0);
                    n.h = (y1 - y0).abs().max(20.0);
                    if mods.shift || n.shape == Shape::Circle {
                        let s = n.w.max(n.h);
                        n.w = s;
                        n.h = s;
                    }
                }
                if released || !down {
                    self.drag = Drag::None;
                }
            }
            Drag::Connect { from, .. } => {
                self.drag = Drag::Connect { from, cur: wp };
                ctx.set_cursor_icon(CursorIcon::Crosshair);
                if released || !down {
                    self.drag = Drag::None;
                    match hovered_node {
                        Some(to) => {
                            let id = self.connect(from, to);
                            self.sel_e = [id].into();
                            self.sel_n.clear();
                        }
                        None if over_canvas => {
                            // drop on empty canvas: create a new shape there and connect it
                            let src = self.doc.node(from).cloned();
                            if let Some(src) = src {
                                if src.center().dist(wp) > 40.0 {
                                    self.label("Add and connect");
                                    let nid = self.doc.add_node(src.shape, P::new(self.snap_v(wp.x), self.snap_v(wp.y)), "");
                                    if let Some(n) = self.doc.node_mut(nid) {
                                        n.w = src.w;
                                        n.h = src.h;
                                        n.pos = P::new(wp.x - src.w / 2.0, wp.y - src.h / 2.0);
                                        n.fill = src.fill;
                                        n.stroke = src.stroke;
                                        n.text_color = src.text_color;
                                        n.font_size = src.font_size;
                                        n.stroke_width = src.stroke_width;
                                    }
                                    self.connect(from, nid);
                                    self.sel_n = [nid].into();
                                    self.sel_e.clear();
                                    self.start_edit(Editing::Node(nid));
                                }
                            }
                        }
                        None => {}
                    }
                }
            }
            Drag::Marquee { start, additive, .. } => {
                self.drag = Drag::Marquee { start, cur: wp, additive };
                if released || !down {
                    let (lo, hi) = (P::new(start.x.min(wp.x), start.y.min(wp.y)), P::new(start.x.max(wp.x), start.y.max(wp.y)));
                    if hi.sub(lo).len() > 3.0 {
                        let inside = |p: P| p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y;
                        let mut ids: Vec<u64> = vec![];
                        for n in &self.doc.nodes {
                            if inside(n.pos) && inside(n.pos.add(P::new(n.w, n.h))) {
                                ids.push(n.id);
                            }
                        }
                        for id in ids {
                            for m in self.doc.group_members(id) {
                                self.sel_n.insert(m);
                            }
                        }
                        for g in &self.geoms {
                            if g.pts.iter().all(|p| inside(*p)) {
                                self.sel_e.insert(g.id);
                            }
                        }
                        let _ = additive;
                    }
                    self.drag = Drag::None;
                }
            }
            Drag::EdgeHandle { edge, kind } => {
                let mut p = P::new(self.snap_v(wp.x), self.snap_v(wp.y));
                let vertical = self.geoms.iter().find(|g| g.id == edge).map(|g| g.elbow_vertical).unwrap_or(false);
                if mods.shift {
                    if let HandleKind::Bend(i) = kind {
                        let prev = if i == 0 {
                            self.geoms.iter().find(|g| g.id == edge).map(|g| g.pts[0])
                        } else {
                            self.doc.edges.iter().find(|e| e.id == edge).map(|e| e.bends[i - 1])
                        };
                        if let Some(prev) = prev {
                            p = snap_angle(prev, wp);
                        }
                    }
                }
                if let Some(e) = self.doc.edge_mut(edge) {
                    match kind {
                        HandleKind::Bend(i) => {
                            if i < e.bends.len() {
                                e.bends[i] = p;
                            }
                        }
                        HandleKind::ElbowMid => {
                            e.elbow_vertical = Some(vertical);
                            e.elbow_mid = Some(if vertical { p.y } else { p.x });
                        }
                        HandleKind::Insert(_) => {}
                    }
                }
                ctx.set_cursor_icon(CursorIcon::Grabbing);
                if released || !down {
                    self.drag = Drag::None;
                }
            }
            Drag::EdgeEnd { edge, is_start, .. } => {
                let mut p = P::new(self.snap_v(wp.x), self.snap_v(wp.y));
                if mods.shift {
                    if let Some(g) = self.geoms.iter().find(|g| g.id == edge) {
                        let other = if is_start { *g.pts.last().unwrap() } else { g.pts[0] };
                        p = snap_angle(other, wp);
                    }
                }
                self.drag = Drag::EdgeEnd { edge, is_start, cur: p };
                ctx.set_cursor_icon(CursorIcon::Crosshair);
                let target = hovered_node;
                if let Some(e) = self.doc.edge_mut(edge) {
                    // follow the pointer live
                    if is_start {
                        e.from_pt = Some(p);
                    } else {
                        e.to_pt = Some(p);
                    }
                }
                if released || !down {
                    if let Some(e) = self.doc.edge_mut(edge) {
                        match target {
                            Some(n) => {
                                if is_start {
                                    e.from = n;
                                    e.from_pt = None;
                                } else {
                                    e.to = n;
                                    e.to_pt = None;
                                }
                            }
                            None => {
                                if is_start {
                                    e.from = 0;
                                } else {
                                    e.to = 0;
                                }
                                if e.route == Route::Elbow {
                                    e.route = Route::Straight;
                                }
                            }
                        }
                    }
                    self.label(if target.is_some() { "Reconnect" } else { "Move line end" });
                    self.drag = Drag::None;
                }
            }
            Drag::None => {}
        }

        // ---- right-click: remember what was clicked for the context menu
        if over_canvas && ctx.input(|i| i.pointer.secondary_pressed()) {
            let sp = pointer.unwrap_or_default();
            let mut target = None;
            for g in &self.geoms {
                for (k, hp) in &g.handles {
                    if let HandleKind::Bend(i) = k {
                        if self.to_screen(*hp).distance(sp) < 9.0 {
                            target = Some(CtxTarget::Bend(g.id, *i));
                        }
                    }
                }
            }
            let target = target.unwrap_or_else(|| {
                if let Some(n) = self.node_at(wp) {
                    CtxTarget::Node(n)
                } else if let Some(e) = self.edge_at(wp) {
                    CtxTarget::Edge(e)
                } else {
                    CtxTarget::Canvas(wp)
                }
            });
            match target {
                CtxTarget::Node(n) => {
                    if !self.sel_n.contains(&n) {
                        self.select_node_click(n, false, false);
                        self.sel_e.clear();
                    }
                }
                CtxTarget::Edge(e) | CtxTarget::Bend(e, _) => {
                    if !self.sel_e.contains(&e) {
                        self.sel_e = [e].into();
                        self.sel_n.clear();
                    }
                    if let CtxTarget::Bend(e, i) = target {
                        self.sel_bend = Some((e, i));
                    }
                }
                CtxTarget::Canvas(_) => {}
            }
            self.ctx_target = target;
        }
        resp.context_menu(|ui| self.context_menu_ui(ui));

        // ---- keyboard
        if !ctx.wants_keyboard_input() {
            self.canvas_keys(&ctx);
        }

        // ---- draw diagram
        self.geoms = geom::route_all(&self.doc);
        self.crossing_count = geom::count_crossings(&self.geoms, self.doc.hop_size);
        let prims = scene::build(&self.doc, &self.geoms, &SceneOpts { draw_background: false });
        let painter = painter.with_clip_rect(resp.rect);
        self.paint_prims(&painter, &prims);

        // ---- overlays
        let accent = Color32::from_rgb(37, 99, 235);
        // groups
        let mut groups_drawn = BTreeSet::new();
        for id in &self.sel_n {
            if let Some(g) = self.doc.node(*id).and_then(|n| n.group) {
                if groups_drawn.insert(g) {
                    let ms: Vec<&Node> = self.doc.nodes.iter().filter(|n| n.group == Some(g)).collect();
                    let lo = ms.iter().fold(P::new(f32::MAX, f32::MAX), |a, n| P::new(a.x.min(n.pos.x), a.y.min(n.pos.y)));
                    let hi = ms.iter().fold(P::new(f32::MIN, f32::MIN), |a, n| P::new(a.x.max(n.pos.x + n.w), a.y.max(n.pos.y + n.h)));
                    let r = Rect::from_min_max(self.to_screen(lo), self.to_screen(hi)).expand(8.0);
                    let pts = vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
                    painter.extend(EShape::dashed_line(&pts, Stroke::new(1.5_f32, Color32::from_rgb(124, 58, 237)), 6.0, 4.0));
                    painter.text(r.left_top() + Vec2::new(4.0, -14.0), Align2::LEFT_TOP, "group", FontId::proportional(11.0), Color32::from_rgb(124, 58, 237));
                }
            }
        }
        for id in &self.sel_n {
            if let Some(n) = self.doc.node(*id) {
                let r = Rect::from_min_max(self.to_screen(n.pos), self.to_screen(n.pos.add(P::new(n.w, n.h)))).expand(3.0);
                painter.rect_stroke(r, CornerRadius::same(2), Stroke::new(1.5_f32, accent), StrokeKind::Middle);
            }
        }
        if let Some(id) = single_sel_node {
            if let Some(n) = self.doc.node(id) {
                for h in self.resize_handles(n) {
                    let r = Rect::from_center_size(h, Vec2::splat(8.0));
                    painter.rect_filled(r, CornerRadius::same(1), Color32::WHITE);
                    painter.rect_stroke(r, CornerRadius::same(1), Stroke::new(1.5_f32, accent), StrokeKind::Middle);
                }
            }
        }
        for g in &self.geoms {
            if self.sel_e.contains(&g.id) {
                let pts: Vec<Pos2> = g.pts.iter().map(|p| self.to_screen(*p)).collect();
                painter.add(EShape::line(pts, Stroke::new(8.0_f32, Color32::from_rgba_unmultiplied(37, 99, 235, 60))));
            }
        }
        if let Some(eid) = single_sel_edge {
            if let Some(g) = self.geoms.iter().find(|g| g.id == eid) {
                for (k, hp) in &g.handles {
                    let s = self.to_screen(*hp);
                    match k {
                        HandleKind::Bend(i) if self.sel_bend == Some((eid, *i)) => {
                            painter.circle(s, 7.5, Color32::from_rgb(234, 88, 12), Stroke::new(2.0_f32, Color32::WHITE));
                        }
                        HandleKind::Insert(_) => {
                            painter.circle(s, 4.0, Color32::from_rgba_unmultiplied(255, 255, 255, 220), Stroke::new(1.2_f32, accent));
                        }
                        _ => {
                            painter.circle(s, 5.5, accent, Stroke::new(1.5_f32, Color32::WHITE));
                        }
                    }
                }
                for p in [g.pts[0], *g.pts.last().unwrap()] {
                    let r = Rect::from_center_size(self.to_screen(p), Vec2::splat(9.0));
                    painter.rect_filled(r, CornerRadius::same(1), Color32::from_rgb(16, 185, 129));
                }
            }
        }
        // connection dots
        let show_dots: Vec<u64> = if matches!(self.drag, Drag::None | Drag::Connect { .. }) {
            let near = if over_canvas { self.node_near(wp) } else { None };
            let mut v: Vec<u64> = near.into_iter().collect();
            if self.tool == Tool::Connect {
                v = self.doc.nodes.iter().map(|n| n.id).collect();
            }
            v
        } else {
            vec![]
        };
        for id in show_dots {
            if let Some(n) = self.doc.node(id) {
                for d in self.conn_dots(n) {
                    let s = self.to_screen(d);
                    let hot = pointer.is_some_and(|p| p.distance(s) < 9.0);
                    painter.circle(s, if hot { 6.5 } else { 4.5 }, Color32::from_rgb(59, 130, 246), Stroke::new(1.5_f32, Color32::WHITE));
                    if hot && matches!(self.drag, Drag::None) {
                        ctx.set_cursor_icon(CursorIcon::Crosshair);
                    }
                }
            }
        }
        match &self.drag {
            Drag::Connect { from, cur } => {
                if let Some(a) = self.doc.node(*from) {
                    let s = geom::anchor(a, *cur);
                    painter.line_segment([self.to_screen(s), self.to_screen(*cur)], Stroke::new(2.0_f32, accent));
                    painter.circle_filled(self.to_screen(*cur), 4.0, accent);
                }
                if let Some(t) = hovered_node {
                    if let Some(n) = self.doc.node(t) {
                        let r = Rect::from_min_max(self.to_screen(n.pos), self.to_screen(n.pos.add(P::new(n.w, n.h)))).expand(4.0);
                        painter.rect_stroke(r, CornerRadius::same(4), Stroke::new(2.5_f32, Color32::from_rgb(16, 185, 129)), StrokeKind::Middle);
                    }
                }
            }
            Drag::EdgeEnd { cur, .. } => {
                painter.circle_filled(self.to_screen(*cur), 5.0, Color32::from_rgb(16, 185, 129));
                if let Some(t) = hovered_node {
                    if let Some(n) = self.doc.node(t) {
                        let r = Rect::from_min_max(self.to_screen(n.pos), self.to_screen(n.pos.add(P::new(n.w, n.h)))).expand(4.0);
                        painter.rect_stroke(r, CornerRadius::same(4), Stroke::new(2.5_f32, Color32::from_rgb(16, 185, 129)), StrokeKind::Middle);
                    }
                }
            }
            Drag::Draw { start, from, cur } => {
                let s = match from.and_then(|f| self.doc.node(f)) {
                    Some(n) => geom::anchor(n, *cur),
                    None => *start,
                };
                let (a, b) = (self.to_screen(s), self.to_screen(*cur));
                painter.line_segment([a, b], Stroke::new(2.0_f32, accent));
                let dir = (b - a).normalized();
                let n = Vec2::new(-dir.y, dir.x);
                painter.add(EShape::convex_polygon(vec![b, b - dir * 12.0 + n * 5.0, b - dir * 12.0 - n * 5.0], accent, Stroke::NONE));
                if let Some(t) = hovered_node {
                    if let Some(n) = self.doc.node(t) {
                        let r = Rect::from_min_max(self.to_screen(n.pos), self.to_screen(n.pos.add(P::new(n.w, n.h)))).expand(4.0);
                        painter.rect_stroke(r, CornerRadius::same(4), Stroke::new(2.5_f32, Color32::from_rgb(16, 185, 129)), StrokeKind::Middle);
                    }
                }
            }
            Drag::Marquee { start, cur, .. } => {
                let r = Rect::from_two_pos(self.to_screen(*start), self.to_screen(*cur));
                painter.rect_filled(r, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(37, 99, 235, 25));
                painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.0_f32, accent), StrokeKind::Middle);
            }
            _ => {}
        }
        if let Tool::Place(s) = self.tool {
            if let Some(p) = pointer {
                if over_canvas {
                    painter.text(p + Vec2::new(14.0, 14.0), Align2::LEFT_TOP, format!("Click to place: {}", s.name()), FontId::proportional(12.0), accent);
                }
            }
        }

        // ---- inline text editor
        self.text_editor(&ctx);
    }

    fn text_editor(&mut self, ctx: &egui::Context) {
        let Some(ed) = self.editing else { return };
        let (rect, size) = match ed {
            Editing::Node(id) => {
                let Some(n) = self.doc.node(id) else {
                    self.editing = None;
                    return;
                };
                let (tp, tw, th) = geom::text_box(n);
                let r = Rect::from_min_size(self.to_screen(tp), Vec2::new(tw * self.zoom, th * self.zoom));
                (r, n.font_size)
            }
            Editing::Edge(id) => {
                let Some(g) = self.geoms.iter().find(|g| g.id == id) else {
                    self.editing = None;
                    return;
                };
                let (mid, _) = geom::point_at(&g.pts, geom::polyline_len(&g.pts) / 2.0);
                let c = self.to_screen(mid);
                (Rect::from_center_size(c, Vec2::new(160.0, 40.0)), 13.0)
            }
        };
        let w = rect.width().max(120.0);
        let mut close = false;
        egui::Area::new(egui::Id::new("inline-text-editor"))
            .fixed_pos(Pos2::new(rect.center().x - w / 2.0, rect.min.y))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).inner_margin(4.0).show(ui, |ui| {
                    let text = match ed {
                        Editing::Node(id) => &mut self.doc.node_mut(id).unwrap().text,
                        Editing::Edge(id) => &mut self.doc.edge_mut(id).unwrap().label,
                    };
                    let resp = ui.add(
                        egui::TextEdit::multiline(text)
                            .desired_width(w)
                            .desired_rows(2)
                            .font(FontId::new((size * self.zoom).clamp(11.0, 26.0), FontFamily::Name("dv".into())))
                            .hint_text("Type text… (Enter = new line, Esc or click outside = done)"),
                    );
                    if self.edit_focus_pending {
                        resp.request_focus();
                        if !ctx.input(|i| i.pointer.any_down()) && resp.has_focus() {
                            self.edit_focus_pending = false;
                        }
                    } else if resp.lost_focus() || ctx.input(|i| i.key_pressed(Key::Escape)) {
                        close = true;
                    }
                    ui.label(egui::RichText::new("Esc or click outside when done").size(10.0).weak());
                });
            });
        if close {
            self.pending_label.get_or_insert_with(|| "Edit text".into());
            self.editing = None;
        }
    }

    fn canvas_keys(&mut self, ctx: &egui::Context) {
        let mut copy = false;
        let mut cut = false;
        let mut paste: Option<String> = None;
        ctx.input(|i| {
            for ev in &i.events {
                match ev {
                    egui::Event::Copy => copy = true,
                    egui::Event::Cut => cut = true,
                    egui::Event::Paste(s) => paste = Some(s.clone()),
                    _ => {}
                }
            }
        });
        if copy || cut {
            self.copy(ctx);
        }
        if cut {
            self.delete_selection();
            self.label("Cut");
        }
        if let Some(s) = paste {
            if let Some(json) = s.strip_prefix(CLIP_PREFIX) {
                if let Ok(c) = serde_json::from_str::<Clip>(json) {
                    self.paste_clip(c, 30.0);
                }
            } else if !s.trim().is_empty() {
                // pasting plain text makes a text box
                let c = self.view_center();
                self.label("Paste text");
                let id = self.doc.add_node(Shape::RoundedRect, c, s.trim());
                self.sel_n = [id].into();
                self.sel_e.clear();
            }
        }
        let k = |m: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(m, key));
        if k(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || k(Modifiers::COMMAND, Key::Y) {
            self.redo();
        } else if k(Modifiers::COMMAND, Key::Z) {
            self.undo();
        }
        if k(Modifiers::COMMAND | Modifiers::SHIFT, Key::G) {
            self.ungroup_selection();
        } else if k(Modifiers::COMMAND, Key::G) {
            self.group_selection();
        }
        if k(Modifiers::COMMAND, Key::D) {
            self.duplicate();
        }
        if k(Modifiers::COMMAND, Key::A) {
            self.select_all();
        }
        if k(Modifiers::NONE, Key::Delete) || k(Modifiers::NONE, Key::Backspace) {
            match self.sel_bend {
                Some((eid, i)) if self.sel_e.contains(&eid) => self.remove_bend(eid, i),
                _ => self.delete_selection(),
            }
        }
        if k(Modifiers::NONE, Key::A) {
            self.tool = Tool::Arrow;
        }
        if k(Modifiers::SHIFT, Key::H) {
            self.make_axis(false);
        }
        if k(Modifiers::SHIFT, Key::V) {
            self.make_axis(true);
        }
        if k(Modifiers::SHIFT, Key::B) {
            self.remove_all_bends();
        }
        if k(Modifiers::NONE, Key::Escape) {
            self.tool = Tool::Select;
            self.sel_n.clear();
            self.sel_e.clear();
        }
        if k(Modifiers::NONE, Key::V) {
            self.tool = Tool::Select;
        }
        if k(Modifiers::NONE, Key::C) {
            self.tool = Tool::Connect;
        }
        if k(Modifiers::NONE, Key::F2) {
            if let Some(id) = self.sel_n.iter().next().copied() {
                self.start_edit(Editing::Node(id));
            }
        }
        if k(Modifiers::COMMAND, Key::Num0) {
            self.fit();
        }
        if k(Modifiers::COMMAND, Key::Plus) || k(Modifiers::COMMAND, Key::Equals) {
            self.zoom_by(1.2);
        }
        if k(Modifiers::COMMAND, Key::Minus) {
            self.zoom_by(1.0 / 1.2);
        }
        // use the modifiers recorded with each key press (not the current state), so a
        // quick Ctrl/Shift+arrow tap always moves by the right amount
        let mut nudge = P::new(0.0, 0.0);
        ctx.input(|i| {
            for ev in &i.events {
                if let egui::Event::Key { key, pressed: true, modifiers, .. } = ev {
                    let step = if modifiers.command { 50.0 } else if modifiers.shift { 10.0 } else { 1.0 };
                    match key {
                        Key::ArrowLeft => nudge.x -= step,
                        Key::ArrowRight => nudge.x += step,
                        Key::ArrowUp => nudge.y -= step,
                        Key::ArrowDown => nudge.y += step,
                        _ => {}
                    }
                }
            }
        });
        if nudge.len() > 0.0 {
            self.nudge_selection(nudge);
        }
    }

    fn zoom_by(&mut self, f: f32) {
        let c = self.canvas_rect.center();
        let w = self.to_world(c);
        self.zoom = (self.zoom * f).clamp(0.15, 5.0);
        let s = self.to_screen(w);
        self.offset += c - s;
    }
}

// ---------------------------------------------------------------- panels

fn shape_icon(ui: &mut egui::Ui, shape: Shape, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(46.0, 38.0), Sense::click());
    let painter = ui.painter_at(rect);
    let bg = if selected {
        Color32::from_rgb(219, 234, 254)
    } else if resp.hovered() {
        Color32::from_rgb(241, 245, 249)
    } else {
        Color32::TRANSPARENT
    };
    painter.rect_filled(rect, CornerRadius::same(4), bg);
    let r = rect.shrink2(Vec2::new(8.0, 9.0));
    let (w, h) = match shape {
        Shape::Circle => (r.height(), r.height()),
        _ => (r.width(), r.height()),
    };
    let n = Node {
        id: 0,
        shape,
        pos: P::new(r.center().x - w / 2.0, r.center().y - h / 2.0),
        w,
        h,
        text: String::new(),
        fill: Rgba::NONE,
        stroke: Rgba::BLACK,
        stroke_width: 1.0,
        dash: Dash::Solid,
        text_color: Rgba::BLACK,
        font_size: 10.0,
        bold: false,
        group: None,
        shadow: false,
    };
    let mut pts: Vec<Pos2> = geom::outline(&n).iter().map(|p| Pos2::new(p.x, p.y)).collect();
    if shape == Shape::Text {
        painter.text(r.center(), Align2::CENTER_CENTER, "Aa", FontId::proportional(15.0), Color32::from_rgb(30, 41, 59));
    } else {
        pts.push(pts[0]);
        let mut mesh = egui::Mesh::default();
        for p in &pts[..pts.len() - 1] {
            mesh.colored_vertex(*p, Color32::from_rgb(219, 234, 254));
        }
        for t in triangulate(&pts[..pts.len() - 1]) {
            mesh.add_triangle(t[0] as u32, t[1] as u32, t[2] as u32);
        }
        painter.add(EShape::mesh(mesh));
        painter.add(EShape::line(pts, Stroke::new(1.4_f32, Color32::from_rgb(30, 64, 175))));
        for d in geom::decorations(&n) {
            painter.add(EShape::line(d.iter().map(|p| Pos2::new(p.x, p.y)).collect(), Stroke::new(1.4_f32, Color32::from_rgb(30, 64, 175))));
        }
    }
    resp.on_hover_text(shape.name())
}

fn color_row(ui: &mut egui::Ui, label: &str, c: &mut Rgba, allow_none: bool) {
    ui.horizontal(|ui| {
        ui.label(label);
        let mut cc = c32(*c);
        if color_picker::color_edit_button_srgba(ui, &mut cc, color_picker::Alpha::OnlyBlend).changed() {
            *c = from_c32(cc);
        }
        if allow_none && ui.small_button("none").on_hover_text("Transparent").clicked() {
            *c = Rgba::NONE;
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(3.0);
        for p in PALETTE {
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::click());
            ui.painter().rect_filled(r, CornerRadius::same(3), c32(p));
            ui.painter().rect_stroke(r, CornerRadius::same(3), Stroke::new(1.0_f32, Color32::from_gray(160)), StrokeKind::Inside);
            if resp.clicked() {
                *c = p;
            }
        }
    });
}

fn dash_combo(ui: &mut egui::Ui, id: &str, d: &mut Dash) {
    egui::ComboBox::from_id_salt(id).selected_text(format!("{d:?}")).show_ui(ui, |ui| {
        for v in [Dash::Solid, Dash::Dashed, Dash::Dotted] {
            ui.selectable_value(d, v, format!("{v:?}"));
        }
    });
}

fn head_combo(ui: &mut egui::Ui, id: &str, h: &mut Head) {
    egui::ComboBox::from_id_salt(id).width(130.0).selected_text(h.name()).show_ui(ui, |ui| {
        for v in Head::ALL {
            ui.selectable_value(h, v, v.name());
        }
    });
}

impl App {
    fn properties(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            if !self.sel_n.is_empty() {
                self.node_props(ui);
            }
            if !self.sel_e.is_empty() {
                if !self.sel_n.is_empty() {
                    ui.separator();
                }
                self.edge_props(ui);
            }
            if self.sel_n.is_empty() && self.sel_e.is_empty() {
                self.doc_props(ui);
            }
        });
    }

    fn doc_props(&mut self, ui: &mut egui::Ui) {
        ui.heading("Diagram");
        ui.label("Title (used for Word export)");
        ui.text_edit_singleline(&mut self.doc.title);
        color_row(ui, "Background", &mut self.doc.background, false);
        ui.separator();
        ui.strong("Line crossings");
        ui.label("Where connectors must cross, they are drawn so it's clear they don't join:");
        ui.radio_value(&mut self.doc.hop_style, HopStyle::Arc, "Hop / bridge (recommended)");
        ui.radio_value(&mut self.doc.hop_style, HopStyle::Gap, "Gap in line");
        ui.radio_value(&mut self.doc.hop_style, HopStyle::None, "Plain crossing");
        ui.add(egui::Slider::new(&mut self.doc.hop_size, 3.0..=16.0).text("hop size"));
        if ui.button("✨ Minimise crossings").on_hover_text("Automatically re-route connectors to avoid crossings").clicked() {
            self.minimize();
        }
        ui.separator();
        ui.strong("Style for new connectors");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.defaults.route, Route::Straight, "Straight");
            ui.selectable_value(&mut self.defaults.route, Route::Curved, "Curved");
            ui.selectable_value(&mut self.defaults.route, Route::Elbow, "Elbow");
        });
        ui.horizontal(|ui| {
            for (lbl, s, e) in [("→", Head::None, Head::Arrow), ("↔", Head::Arrow, Head::Arrow), ("—", Head::None, Head::None)] {
                if ui.selectable_label(self.defaults.start == s && self.defaults.end == e, lbl).clicked() {
                    self.defaults.start = s;
                    self.defaults.end = e;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Start");
            head_combo(ui, "def_start", &mut self.defaults.start);
        });
        ui.horizontal(|ui| {
            ui.label("End  ");
            head_combo(ui, "def_end", &mut self.defaults.end);
        });
        ui.add(egui::Slider::new(&mut self.defaults.width, 0.5..=8.0).text("width"));
        ui.horizontal(|ui| {
            ui.label("Line");
            dash_combo(ui, "def_dash", &mut self.defaults.dash);
        });
        color_row(ui, "Colour", &mut self.defaults.color, false);
        ui.separator();
        ui.label(egui::RichText::new("Tip: select a shape or connector to edit it here.").weak());
    }

    fn node_props(&mut self, ui: &mut egui::Ui) {
        let ids: Vec<u64> = self.sel_n.iter().copied().collect();
        let Some(first) = self.doc.node(ids[0]).cloned() else { return };
        ui.heading(if ids.len() == 1 { "Shape".to_string() } else { format!("{} shapes", ids.len()) });
        let mut n = first.clone();
        if ids.len() == 1 {
            ui.label("Text");
            ui.add(egui::TextEdit::multiline(&mut n.text).desired_rows(3).desired_width(f32::INFINITY));
        }
        ui.horizontal(|ui| {
            ui.label("Shape");
            egui::ComboBox::from_id_salt("shape").width(160.0).selected_text(n.shape.name()).show_ui(ui, |ui| {
                for s in Shape::ALL {
                    ui.selectable_value(&mut n.shape, s, s.name());
                }
            });
        });
        color_row(ui, "Fill", &mut n.fill, true);
        color_row(ui, "Border", &mut n.stroke, true);
        ui.add(egui::Slider::new(&mut n.stroke_width, 0.0..=8.0).text("border width"));
        ui.horizontal(|ui| {
            ui.label("Border line");
            dash_combo(ui, "ndash", &mut n.dash);
        });
        color_row(ui, "Text colour", &mut n.text_color, false);
        ui.add(egui::Slider::new(&mut n.font_size, 8.0..=48.0).text("font size"));
        ui.horizontal(|ui| {
            ui.checkbox(&mut n.bold, "Bold");
            ui.checkbox(&mut n.shadow, "Shadow");
        });
        if ids.len() == 1 {
            ui.horizontal(|ui| {
                ui.label("W");
                ui.add(egui::DragValue::new(&mut n.w).range(20.0..=2000.0));
                ui.label("H");
                ui.add(egui::DragValue::new(&mut n.h).range(20.0..=2000.0));
            });
            ui.horizontal(|ui| {
                ui.label("X");
                ui.add(egui::DragValue::new(&mut n.pos.x));
                ui.label("Y");
                ui.add(egui::DragValue::new(&mut n.pos.y));
            });
        }
        // apply changed fields to all selected nodes
        if n != first {
            for id in &ids {
                if let Some(t) = self.doc.node_mut(*id) {
                    if ids.len() == 1 {
                        let g = t.group;
                        *t = n.clone();
                        t.group = g;
                        continue;
                    }
                    if n.shape != first.shape {
                        t.shape = n.shape;
                    }
                    if n.fill != first.fill {
                        t.fill = n.fill;
                    }
                    if n.stroke != first.stroke {
                        t.stroke = n.stroke;
                    }
                    if n.stroke_width != first.stroke_width {
                        t.stroke_width = n.stroke_width;
                    }
                    if n.dash != first.dash {
                        t.dash = n.dash;
                    }
                    if n.text_color != first.text_color {
                        t.text_color = n.text_color;
                    }
                    if n.font_size != first.font_size {
                        t.font_size = n.font_size;
                    }
                    if n.bold != first.bold {
                        t.bold = n.bold;
                    }
                    if n.shadow != first.shadow {
                        t.shadow = n.shadow;
                    }
                }
            }
            self.pending_label.get_or_insert_with(|| "Format shape".into());
        }
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui.button("Group").clicked() {
                self.group_selection();
            }
            if ui.button("Ungroup").clicked() {
                self.ungroup_selection();
            }
            if ui.button("Front").clicked() {
                self.reorder(true);
            }
            if ui.button("Back").clicked() {
                self.reorder(false);
            }
            if ui.button("Duplicate").clicked() {
                self.duplicate();
            }
            if ui.button("🗑 Delete").clicked() {
                self.delete_selection();
            }
        });
        if ids.len() > 1 {
            ui.label("Align");
            ui.horizontal_wrapped(|ui| {
                for a in ["left", "center", "right", "top", "middle", "bottom"] {
                    if ui.small_button(a).clicked() {
                        self.align(a);
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("distribute ↔").clicked() {
                    self.align("distribute horizontally");
                }
                if ui.small_button("distribute ↕").clicked() {
                    self.align("distribute vertically");
                }
                if ui.small_button("same size").clicked() {
                    self.align("same size");
                }
            });
        }
    }

    fn edge_props(&mut self, ui: &mut egui::Ui) {
        let ids: Vec<u64> = self.sel_e.iter().copied().collect();
        let Some(first) = self.doc.edges.iter().find(|e| e.id == ids[0]).cloned() else { return };
        ui.heading(if ids.len() == 1 { "Connector".to_string() } else { format!("{} connectors", ids.len()) });
        let mut e = first.clone();
        ui.label("Path");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut e.route, Route::Straight, "Straight");
            ui.selectable_value(&mut e.route, Route::Curved, "Curved");
            ui.selectable_value(&mut e.route, Route::Elbow, "Elbow");
        });
        ui.label("Direction");
        ui.horizontal(|ui| {
            for (lbl, tip, s, en) in [
                ("→", "Single arrow", Head::None, Head::Arrow),
                ("←", "Reverse arrow", Head::Arrow, Head::None),
                ("↔", "Double-sided arrow", Head::Arrow, Head::Arrow),
                ("—", "No arrows", Head::None, Head::None),
            ] {
                if ui.selectable_label(e.start_head == s && e.end_head == en, lbl).on_hover_text(tip).clicked() {
                    e.start_head = s;
                    e.end_head = en;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Start");
            head_combo(ui, "e_start", &mut e.start_head);
        });
        ui.horizontal(|ui| {
            ui.label("End  ");
            head_combo(ui, "e_end", &mut e.end_head);
        });
        ui.add(egui::Slider::new(&mut e.head_size, 6.0..=30.0).text("arrow size"));
        ui.add(egui::Slider::new(&mut e.width, 0.5..=10.0).text("line width"));
        ui.horizontal(|ui| {
            ui.label("Line");
            dash_combo(ui, "e_dash", &mut e.dash);
        });
        color_row(ui, "Colour", &mut e.color, false);
        if ids.len() == 1 {
            ui.label("Label (e.g. + / − for causal loops)");
            ui.text_edit_singleline(&mut e.label);
            ui.add(egui::Slider::new(&mut e.label_size, 8.0..=32.0).text("label size"));
        }
        if e.route == Route::Curved && first.route != Route::Curved && e.bends.is_empty() {
            if let (Some(a), Some(b)) = (self.doc.node(e.from), self.doc.node(e.to)) {
                let (ca, cb) = (a.center(), b.center());
                e.bends = vec![ca.lerp(cb, 0.5).add(cb.sub(ca).perp().mul(0.18))];
            }
        }
        if e != first {
            for id in &ids {
                if let Some(t) = self.doc.edge_mut(*id) {
                    if ids.len() == 1 {
                        *t = e.clone();
                        continue;
                    }
                    if e.route != first.route {
                        t.route = e.route;
                    }
                    if e.start_head != first.start_head {
                        t.start_head = e.start_head;
                    }
                    if e.end_head != first.end_head {
                        t.end_head = e.end_head;
                    }
                    if e.head_size != first.head_size {
                        t.head_size = e.head_size;
                    }
                    if e.width != first.width {
                        t.width = e.width;
                    }
                    if e.dash != first.dash {
                        t.dash = e.dash;
                    }
                    if e.color != first.color {
                        t.color = e.color;
                    }
                }
            }
            self.pending_label.get_or_insert_with(|| "Format connector".into());
        }
        ui.separator();
        ui.strong("Bends and angle");
        let nb: usize = ids.iter().filter_map(|id| self.edge(*id)).map(|e| e.bends.len()).sum();
        let any_elbow = ids.iter().filter_map(|id| self.edge(*id)).any(|e| e.route == Route::Elbow);
        ui.label(if any_elbow { "Elbow line (right-angled corners)".to_string() } else { format!("Bend points: {nb}") });
        ui.horizontal_wrapped(|ui| {
            if let Some((eid, i)) = self.sel_bend {
                if ids.contains(&eid) && ui.button("× Remove selected bend").on_hover_text("Delete key also works").clicked() {
                    self.remove_bend(eid, i);
                }
            }
            if ui
                .add_enabled(nb > 0 || any_elbow, egui::Button::new("Remove all bends"))
                .on_hover_text("Straighten the line (Shift+B)")
                .clicked()
            {
                self.remove_all_bends();
            }
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("─ Horizontal").on_hover_text("Make the line exactly horizontal (Shift+H)").clicked() {
                self.make_axis(false);
            }
            if ui.button("│ Vertical").on_hover_text("Make the line exactly vertical (Shift+V)").clicked() {
                self.make_axis(true);
            }
            if ui.button("└ Right angles").on_hover_text("Turn every slanted segment into a horizontal or vertical one").clicked() {
                self.snap_right_angles();
            }
        });
        if ids.len() == 1 {
            if let Some(lock) = self.edge(ids[0]).and_then(|e| if e.bends.is_empty() { e.lock } else { None }) {
                ui.horizontal(|ui| {
                    ui.label(format!("✔ Kept {}", if lock == Axis::Vertical { "vertical" } else { "horizontal" }));
                    if ui.small_button("free angle").clicked() {
                        self.label("Free angle");
                        if let Some(t) = self.doc.edge_mut(ids[0]) {
                            t.lock = None;
                        }
                    }
                });
            }
        }
        ui.separator();
        ui.strong("Move the line");
        ui.horizontal(|ui| {
            let st = self.move_step;
            if ui.button("←").on_hover_text("Move left").clicked() {
                self.label("Move connector");
                self.nudge_selection(P::new(-st, 0.0));
            }
            if ui.button("↑").on_hover_text("Move up").clicked() {
                self.label("Move connector");
                self.nudge_selection(P::new(0.0, -st));
            }
            if ui.button("↓").on_hover_text("Move down").clicked() {
                self.label("Move connector");
                self.nudge_selection(P::new(0.0, st));
            }
            if ui.button("→").on_hover_text("Move right").clicked() {
                self.label("Move connector");
                self.nudge_selection(P::new(st, 0.0));
            }
            ui.add(egui::DragValue::new(&mut self.move_step).range(1.0..=1000.0).suffix(" px"));
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("⇄ Reverse").clicked() {
                self.reverse_edges();
            }
            if ui.button("🗑 Delete").clicked() {
                self.delete_selection();
            }
        });
        ui.label(
            egui::RichText::new(
                "• Drag the line itself to move it anywhere (Shift keeps the move straight).\n\
                 • Drag a hollow dot to add a bend; click a solid dot and press Delete to remove it, or right-click it.\n\
                 • Drag a green end onto a shape to attach it, or onto empty space to leave it free.\n\
                 • Hold Shift while dragging ends or bends to snap to horizontal, vertical or 45°.\n\
                 • Arrow keys move the selection (Shift ×10, Ctrl ×50).",
            )
            .weak()
            .size(11.0),
        );
    }
}

// ---------------------------------------------------------------- updates

impl App {
    fn update_banner(&mut self, ctx: &egui::Context) {
        if self.update_banner_closed {
            return;
        }
        let state = self.update.lock().map(|s| s.clone()).unwrap_or(update::State::Checking);
        if let update::State::Available { version, .. } = state {
            egui::TopBottomPanel::bottom("update-banner").frame(egui::Frame::default().fill(Color32::from_rgb(219, 234, 254)).inner_margin(6.0)).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("⬆ FrameMap Studio {version} is available (you have {}).", env!("CARGO_PKG_VERSION"))).strong());
                    if ui.button("Download update").clicked() {
                        let _ = webbrowser::open(&update::download_url());
                        self.show_update_help = true;
                    }
                    if ui.button("What's new").clicked() {
                        self.update_dialog = true;
                    }
                    if ui.button("Later").clicked() {
                        self.update_banner_closed = true;
                    }
                });
            });
        }
    }

    fn update_windows(&mut self, ctx: &egui::Context) {
        if self.update_dialog {
            let state = self.update.lock().map(|s| s.clone()).unwrap_or(update::State::Checking);
            let mut open = true;
            egui::Window::new("Software update").open(&mut open).collapsible(false).resizable(false).default_width(380.0).show(ctx, |ui| {
                match &state {
                    update::State::Checking => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Checking for a newer version…");
                        });
                    }
                    update::State::UpToDate { latest } => {
                        ui.label(format!("✔ You have the latest version ({}).", env!("CARGO_PKG_VERSION")));
                        if !latest.is_empty() {
                            ui.label(egui::RichText::new(format!("Latest published: {latest}")).weak());
                        }
                    }
                    update::State::Available { version, notes } => {
                        ui.label(egui::RichText::new(format!("Version {version} is available. You have {}.", env!("CARGO_PKG_VERSION"))).strong());
                        if !notes.trim().is_empty() {
                            egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                                ui.label(notes);
                            });
                        }
                        if ui.button("Download update").clicked() {
                            let _ = webbrowser::open(&update::download_url());
                            self.show_update_help = true;
                        }
                    }
                    update::State::Failed(e) => {
                        ui.label("Couldn't check for updates (are you online?).");
                        ui.label(egui::RichText::new(e).weak().size(11.0));
                        if ui.button("Open the download page").clicked() {
                            let _ = webbrowser::open(&update::releases_page());
                        }
                    }
                }
            });
            self.update_dialog = open;
        }
        if self.show_update_help {
            let mut open = true;
            egui::Window::new("Installing the update").open(&mut open).collapsible(false).resizable(false).default_width(400.0).show(ctx, |ui| {
                ui.label("Your browser is downloading the new version.");
                ui.add_space(4.0);
                if cfg!(target_os = "macos") {
                    ui.label("1. Save your work and quit FrameMap Studio.");
                    ui.label("2. Open the downloaded .dmg and drag FrameMap Studio into Applications.");
                    ui.label("3. Choose “Replace” when asked.");
                } else {
                    ui.label("1. Save your work and close FrameMap Studio.");
                    ui.label("2. Run the downloaded FrameMap-Studio-Setup.exe.");
                    ui.label("3. Click through the installer — it replaces the old version.");
                }
                ui.add_space(4.0);
                ui.label(egui::RichText::new("Your saved diagrams (.fmap files) are not touched by the update.").weak());
                if ui.button("Save now").clicked() {
                    self.save(false);
                }
            });
            self.show_update_help = open;
        }
    }
}

// ---------------------------------------------------------------- eframe

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // window title
        let title = format!(
            "{}{} — FrameMap Studio",
            if self.dirty() { "● " } else { "" },
            self.file.as_ref().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| self.doc.title.clone())
        );
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        // global shortcuts (work even while not focused on the canvas)
        if !ctx.wants_keyboard_input() {
            let k = |m: Modifiers, key: Key| ctx.input_mut(|i| i.consume_key(m, key));
            if k(Modifiers::COMMAND | Modifiers::SHIFT, Key::S) {
                self.save(true);
            } else if k(Modifiers::COMMAND, Key::S) {
                self.save(false);
            }
            if k(Modifiers::COMMAND, Key::O) {
                self.pending_open = true;
            }
            if k(Modifiers::COMMAND, Key::N) {
                self.pending_template = Some("blank");
            }
            if k(Modifiers::COMMAND, Key::E) {
                self.docx_dialog = Some((false, true, format!("Figure 1: {}", self.doc.title)));
            }
        }

        // close handling
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close && self.dirty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close = true;
        }

        self.menu_bar(ctx);
        self.toolbar(ctx);

        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "FrameMap Studio v{} · © 2026 Muruhukye Jacob, Biostatistician/Programmer and Researcher · Author and owner · All rights reserved",
                        env!("CARGO_PKG_VERSION")
                    ))
                    .size(11.0)
                    .color(Color32::from_rgb(71, 85, 105)),
                );
            });
        });

        self.update_banner(ctx);

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("{:.0}%", self.zoom * 100.0));
                    ui.separator();
                    let cc = self.crossing_count;
                    let txt = if cc == 0 {
                        egui::RichText::new("✔ no crossings").color(Color32::from_rgb(5, 150, 105))
                    } else {
                        egui::RichText::new(format!("{cc} crossing(s) shown as hops")).color(Color32::from_rgb(180, 83, 9))
                    };
                    ui.label(txt).on_hover_text("Click 'Minimise crossings' to re-route connectors automatically");
                    ui.separator();
                    ui.label(format!("{} shapes · {} connectors", self.doc.nodes.len(), self.doc.edges.len()));
                });
            });
        });

        egui::SidePanel::left("palette").resizable(false).exact_width(112.0).show(ctx, |ui| {
            ui.add_space(4.0);
            ui.strong("Tools");
            ui.horizontal(|ui| {
                if ui.selectable_label(self.tool == Tool::Select, "⬉ Select").on_hover_text("Select & move (V)").clicked() {
                    self.tool = Tool::Select;
                }
            });
            if ui.selectable_label(self.tool == Tool::Connect, "↗ Connect").on_hover_text("Drag from one shape to another (C)").clicked() {
                self.tool = Tool::Connect;
            }
            if ui.selectable_label(self.tool == Tool::Arrow, "➚ Free arrow").on_hover_text("Draw an arrow anywhere, not attached to shapes (A)").clicked() {
                self.tool = Tool::Arrow;
            }
            ui.separator();
            ui.strong("Shapes");
            ui.label(egui::RichText::new("click, then click canvas").size(10.0).weak());
            egui::Grid::new("shapes").spacing(Vec2::splat(2.0)).show(ui, |ui| {
                for (i, s) in Shape::ALL.iter().enumerate() {
                    if shape_icon(ui, *s, self.tool == Tool::Place(*s)).clicked() {
                        self.tool = Tool::Place(*s);
                    }
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
        });

        egui::SidePanel::right("props").resizable(true).default_width(250.0).show(ctx, |ui| {
            ui.add_space(4.0);
            self.properties(ui);
        });

        egui::CentralPanel::default().frame(egui::Frame::default()).show(ctx, |ui| {
            self.canvas(ui);
        });

        self.dialogs(ctx);

        // deferred actions (need file dialogs, so run outside UI closures)
        if let Some(t) = self.pending_template.take() {
            self.guard("starting a new diagram", Box::new(move |a: &mut App| a.new_from(t)));
        }
        if self.pending_open {
            self.pending_open = false;
            self.guard("opening another file", Box::new(|a: &mut App| a.open_dialog()));
        }

        // commit an undo step once a gesture/edit has finished
        let busy = ctx.input(|i| i.pointer.any_down()) || ctx.memory(|m| m.focused().is_some()) || self.editing.is_some();
        if !busy {
            self.commit();
        }
    }
}

impl App {
    fn menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    ui.menu_button("New from template", |ui| {
                        for (name, kind) in templates::TEMPLATES {
                            if ui.button(name).clicked() {
                                self.pending_template = Some(kind);
                                ui.close_menu();
                            }
                        }
                    });
                    if ui.button("Open…            Ctrl+O").clicked() {
                        self.pending_open = true;
                        ui.close_menu();
                    }
                    if ui.button("Save               Ctrl+S").clicked() {
                        ui.close_menu();
                        self.save(false);
                    }
                    if ui.button("Save as…    Ctrl+Shift+S").clicked() {
                        ui.close_menu();
                        self.save(true);
                    }
                    ui.separator();
                    if ui.button("📄 Export to Word (.docx)…  Ctrl+E").clicked() {
                        self.docx_dialog = Some((false, true, format!("Figure 1: {}", self.doc.title)));
                        ui.close_menu();
                    }
                    if ui.button("Export image (.png)…").clicked() {
                        ui.close_menu();
                        self.export_png();
                    }
                    if ui.button("Export vector (.svg)…").clicked() {
                        ui.close_menu();
                        self.export_svg();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ui.close_menu();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Edit", |ui| {
                    if ui.add_enabled(!self.undo.is_empty() || self.doc != self.committed, egui::Button::new("Undo   Ctrl+Z")).clicked() {
                        self.undo();
                        ui.close_menu();
                    }
                    if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Redo   Ctrl+Y")).clicked() {
                        self.redo();
                        ui.close_menu();
                    }
                    if ui.button("History…").clicked() {
                        self.show_history = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Copy        Ctrl+C").clicked() {
                        self.copy(ctx);
                        ui.close_menu();
                    }
                    if ui.button("Duplicate  Ctrl+D").clicked() {
                        self.duplicate();
                        ui.close_menu();
                    }
                    if ui.button("Delete      Del").clicked() {
                        self.delete_selection();
                        ui.close_menu();
                    }
                    if ui.button("Select all  Ctrl+A").clicked() {
                        self.select_all();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Group           Ctrl+G").clicked() {
                        self.group_selection();
                        ui.close_menu();
                    }
                    if ui.button("Ungroup  Ctrl+Shift+G").clicked() {
                        self.ungroup_selection();
                        ui.close_menu();
                    }
                });
                ui.menu_button("Arrange", |ui| {
                    if ui.button("Bring to front").clicked() {
                        self.reorder(true);
                        ui.close_menu();
                    }
                    if ui.button("Send to back").clicked() {
                        self.reorder(false);
                        ui.close_menu();
                    }
                    ui.separator();
                    for a in ["left", "center", "right", "top", "middle", "bottom", "distribute horizontally", "distribute vertically", "same size"] {
                        if ui.button(format!("Align {a}").replace("Align distribute", "Distribute").replace("Align same", "Make same")).clicked() {
                            self.align(a);
                            ui.close_menu();
                        }
                    }
                });
                ui.menu_button("Connectors", |ui| {
                    if ui.button("✨ Minimise crossings").clicked() {
                        self.minimize();
                        ui.close_menu();
                    }
                    if ui.button("➚ Draw a free arrow    A").clicked() {
                        self.tool = Tool::Arrow;
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.label(egui::RichText::new("For the selected connector(s):").weak());
                    self.edge_menu_items(ui);
                    ui.separator();
                    ui.label("Show unavoidable crossings as:");
                    ui.radio_value(&mut self.doc.hop_style, HopStyle::Arc, "Hop / bridge");
                    ui.radio_value(&mut self.doc.hop_style, HopStyle::Gap, "Gap");
                    ui.radio_value(&mut self.doc.hop_style, HopStyle::None, "Plain");
                });
                ui.menu_button("View", |ui| {
                    if ui.button("Fit to window   Ctrl+0").clicked() {
                        self.fit();
                        ui.close_menu();
                    }
                    if ui.button("Zoom in   Ctrl++").clicked() {
                        self.zoom_by(1.2);
                        ui.close_menu();
                    }
                    if ui.button("Zoom out  Ctrl+-").clicked() {
                        self.zoom_by(1.0 / 1.2);
                        ui.close_menu();
                    }
                    ui.checkbox(&mut self.grid, "Show grid");
                    ui.checkbox(&mut self.snap, "Snap to grid");
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("How to use & shortcuts").clicked() {
                        self.show_help = true;
                        ui.close_menu();
                    }
                    if ui.button("Check for updates…").clicked() {
                        self.update = update::check_in_background(ctx.clone());
                        self.update_dialog = true;
                        ui.close_menu();
                    }
                    if ui.button("Download page").clicked() {
                        let _ = webbrowser::open(&update::releases_page());
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("About").clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                });
            });
        });
    }

    fn toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().button_padding = Vec2::new(8.0, 4.0);
                if ui.add_enabled(!self.undo.is_empty() || self.doc != self.committed, egui::Button::new("↶ Undo")).clicked() {
                    self.undo();
                }
                if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("↷ Redo")).clicked() {
                    self.redo();
                }
                ui.separator();
                if ui.selectable_label(self.tool == Tool::Select, "⬉ Select").clicked() {
                    self.tool = Tool::Select;
                }
                if ui.selectable_label(self.tool == Tool::Connect, "↗ Connect").clicked() {
                    self.tool = Tool::Connect;
                }
                if ui.selectable_label(self.tool == Tool::Arrow, "➚ Free arrow").on_hover_text("Draw an arrow anywhere (A)").clicked() {
                    self.tool = Tool::Arrow;
                }
                ui.separator();
                if ui.button("⧉ Group").on_hover_text("Join selected shapes so they move as one (Ctrl+G)").clicked() {
                    self.group_selection();
                }
                if ui.button("Ungroup").clicked() {
                    self.ungroup_selection();
                }
                ui.separator();
                if ui.button("✨ Minimise crossings").clicked() {
                    self.minimize();
                }
                ui.separator();
                if ui.button("📄 Export to Word").clicked() {
                    self.docx_dialog = Some((false, true, format!("Figure 1: {}", self.doc.title)));
                }
                if ui.button("💾 Save").clicked() {
                    self.save(false);
                }
                ui.separator();
                if ui.button("−").clicked() {
                    self.zoom_by(1.0 / 1.2);
                }
                if ui.button("Fit").clicked() {
                    self.fit();
                }
                if ui.button("+").clicked() {
                    self.zoom_by(1.2);
                }
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        // Word export
        if let Some((mut landscape, mut include_title, mut caption)) = self.docx_dialog.clone() {
            let mut open = true;
            let mut go = false;
            egui::Window::new("Export to Word").collapsible(false).resizable(false).open(&mut open).anchor(Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
                ui.label("Title");
                ui.text_edit_singleline(&mut self.doc.title);
                ui.checkbox(&mut include_title, "Put the title above the figure");
                ui.label("Caption below the figure (leave empty for none)");
                ui.text_edit_singleline(&mut caption);
                ui.horizontal(|ui| {
                    ui.radio_value(&mut landscape, false, "Portrait (A4)");
                    ui.radio_value(&mut landscape, true, "Landscape (A4)");
                });
                ui.label(egui::RichText::new("The figure is embedded as a sharp vector image (with a high-resolution fallback for older Word versions).").weak().size(11.0));
                ui.add_space(6.0);
                if ui.button("Choose location and export…").clicked() {
                    go = true;
                }
            });
            self.docx_dialog = if open && !go { Some((landscape, include_title, caption.clone())) } else { None };
            if go {
                self.export_docx(landscape, include_title, caption);
            }
        }

        // save prompt
        if let Some(what) = self.ask_save {
            let mut choice = None;
            egui::Window::new("Unsaved changes").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
                ui.label(format!("Save your changes before {what}?"));
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        choice = Some(0);
                    }
                    if ui.button("Don't save").clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(2);
                    }
                });
            });
            match choice {
                Some(0) => {
                    self.ask_save = None;
                    if self.save(false) {
                        if let Some(f) = self.pending_after_save.take() {
                            f(self);
                        }
                    }
                }
                Some(1) => {
                    self.ask_save = None;
                    if let Some(f) = self.pending_after_save.take() {
                        f(self);
                    }
                }
                Some(_) => {
                    self.ask_save = None;
                    self.pending_after_save = None;
                }
                None => {}
            }
        }

        if self.confirm_close {
            let mut choice = None;
            egui::Window::new("Quit FrameMap Studio?").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
                ui.label("You have unsaved changes.");
                ui.horizontal(|ui| {
                    if ui.button("Save and quit").clicked() {
                        choice = Some(0);
                    }
                    if ui.button("Quit without saving").clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(2);
                    }
                });
            });
            match choice {
                Some(0) => {
                    self.confirm_close = false;
                    if self.save(false) {
                        self.allow_close = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                Some(1) => {
                    self.confirm_close = false;
                    self.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Some(_) => self.confirm_close = false,
                None => {}
            }
        }

        if self.show_history {
            let mut open = true;
            let mut jump: Option<i64> = None;
            egui::Window::new("History").open(&mut open).default_width(240.0).show(ctx, |ui| {
                ui.label(egui::RichText::new("Click a step to go back (or forward) to it. Every action can be reversed.").weak().size(11.0));
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    ui.weak("Start");
                    let n = self.undo.len() as i64;
                    for (i, (l, _)) in self.undo.iter().enumerate() {
                        if ui.selectable_label(false, format!("{}. {l}", i + 1)).clicked() {
                            jump = Some(i as i64 + 1 - n);
                        }
                    }
                    ui.colored_label(Color32::from_rgb(37, 99, 235), "▶ current state");
                    for (i, (l, _)) in self.redo.iter().rev().enumerate() {
                        if ui.selectable_label(false, egui::RichText::new(format!("↷ {l}")).weak()).clicked() {
                            jump = Some(i as i64 + 1);
                        }
                    }
                });
            });
            self.show_history = open;
            if let Some(j) = jump {
                if j < 0 {
                    for _ in 0..(-j) {
                        self.undo();
                    }
                } else {
                    for _ in 0..j {
                        self.redo();
                    }
                }
            }
        }

        if self.show_help {
            let mut open = true;
            egui::Window::new("How to use FrameMap Studio").open(&mut open).default_width(460.0).show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                    ui.strong("Building a framework");
                    ui.label("• Pick a shape on the left, then click the canvas to place it (hold Shift to place several).");
                    ui.label("• Double-click empty canvas to add a box; double-click a shape or connector to edit its text.");
                    ui.label("• Hover a shape and drag from a blue dot to another shape to connect them. Drop on empty canvas to create and connect a new shape in one go.");
                    ui.label("• Select a connector to change it: straight / curved / elbow, single or double-sided arrow, arrow type, width, colour, dashes and a label.");
                    ui.label("• Flexible lines: drag the hollow dots on a selected connector to bend it. To remove a bend, click its solid dot and press Delete, double-click it, or right-click it. 'Remove all bends' (Shift+B) straightens the whole line.");
                    ui.label("• Moving lines: drag a selected line to move it anywhere, use the arrow keys (Shift ×10, Ctrl ×50) or the arrow buttons in the panel. Drag a green end onto empty canvas to free it, or onto a shape to attach it. The Free arrow tool (A) draws arrows anywhere.");
                    ui.label("• Slanted lines: 'Make horizontal' (Shift+H) or 'Make vertical' (Shift+V) straightens a line, moving the end shape into line if needed. 'Right angles' turns every slanted segment of a bent line into horizontal/vertical pieces. Hold Shift while dragging to snap to 0°, 45° or 90°.");
                    ui.add_space(6.0);
                    ui.strong("Crossing lines");
                    ui.label("Connectors may meet at shapes but are not meant to cross. Use Connectors ▸ Minimise crossings to re-route them automatically. Any crossing that cannot be avoided is drawn with a hop (bridge) so readers can see the lines are not joined. Choose hop, gap or plain in the Connectors menu.");
                    ui.add_space(6.0);
                    ui.strong("Groups");
                    ui.label("Select shapes (Shift-click or drag a box around them) and press Ctrl+G. Clicking any member selects the whole group so it moves together. Alt-click selects one member. Ctrl+Shift+G ungroups.");
                    ui.add_space(6.0);
                    ui.strong("Shortcuts (Cmd instead of Ctrl on Mac)");
                    egui::Grid::new("keys").striped(true).show(ui, |ui| {
                        for (k, d) in [
                            ("Ctrl+Z / Ctrl+Y", "Undo / redo (Edit ▸ History to jump to any step)"),
                            ("Ctrl+C / V / X / D", "Copy / paste / cut / duplicate"),
                            ("Ctrl+G / Ctrl+Shift+G", "Group / ungroup"),
                            ("Delete", "Delete selection"),
                            ("Arrow keys", "Move selection (Shift = 10 px, Ctrl = 50 px)"),
                            ("V / C / A", "Select / connect / free-arrow tool"),
                            ("Shift+H / Shift+V", "Make connector horizontal / vertical"),
                            ("Shift+B", "Remove all bends from connector"),
                            ("Right-click", "Menu for a shape, line or bend"),
                            ("F2", "Edit text of selected shape"),
                            ("Ctrl+scroll", "Zoom; scroll or middle-drag or Space+drag to pan"),
                            ("Ctrl+0", "Fit diagram to window"),
                            ("Ctrl+S / Ctrl+O", "Save / open (.fmap)"),
                            ("Ctrl+E", "Export to Word"),
                            ("Shift while resizing", "Keep it square"),
                        ] {
                            ui.monospace(k);
                            ui.label(d);
                            ui.end_row();
                        }
                    });
                });
            });
            self.show_help = open;
        }

        self.update_windows(ctx);

        if self.show_about {
            let mut open = true;
            egui::Window::new("About").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
                ui.heading(format!("FrameMap Studio {}", env!("CARGO_PKG_VERSION")));
                ui.label("Conceptual framework and systems map builder.");
                ui.add_space(6.0);
                ui.strong("Author and owner: Muruhukye Jacob");
                ui.label("Biostatistician/Programmer and Researcher");
                ui.label("© 2026 Muruhukye Jacob. All rights reserved.");
                ui.label(
                    egui::RichText::new("This software may not be copied, modified or redistributed without the author's written permission.")
                        .weak()
                        .size(11.0),
                );
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Built with Rust. Includes open-source components (egui, tiny-skia, ab_glyph, DejaVu fonts and others) under their own licences.").weak().size(10.0));
            });
            self.show_about = open;
        }
    }
}
