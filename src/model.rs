//! Document model: nodes (shapes), edges (connectors) and groups.
//! Everything here is plain data so the whole diagram can be snapshotted for undo/redo
//! and serialised to a `.fmap` (JSON) file.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct P {
    pub x: f32,
    pub y: f32,
}

impl P {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    pub fn add(self, o: P) -> P {
        P::new(self.x + o.x, self.y + o.y)
    }
    pub fn sub(self, o: P) -> P {
        P::new(self.x - o.x, self.y - o.y)
    }
    pub fn mul(self, k: f32) -> P {
        P::new(self.x * k, self.y * k)
    }
    pub fn len(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    pub fn dist(self, o: P) -> f32 {
        self.sub(o).len()
    }
    pub fn norm(self) -> P {
        let l = self.len();
        if l < 1e-6 {
            P::new(1.0, 0.0)
        } else {
            self.mul(1.0 / l)
        }
    }
    pub fn perp(self) -> P {
        P::new(-self.y, self.x)
    }
    pub fn dot(self, o: P) -> f32 {
        self.x * o.x + self.y * o.y
    }
    pub fn lerp(self, o: P, t: f32) -> P {
        self.add(o.sub(self).mul(t))
    }
}

/// RGBA colour, 0-255.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub const WHITE: Rgba = Rgba(255, 255, 255, 255);
    pub const BLACK: Rgba = Rgba(0, 0, 0, 255);
    pub const NONE: Rgba = Rgba(0, 0, 0, 0);
    pub fn hex(&self) -> String {
        format!("{:02X}{:02X}{:02X}", self.0, self.1, self.2)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shape {
    Rectangle,
    RoundedRect,
    Ellipse,
    Circle,
    Diamond,
    Triangle,
    Parallelogram,
    Hexagon,
    Octagon,
    Cylinder,
    Document,
    Cloud,
    Note,
    Text,
}

impl Shape {
    pub const ALL: [Shape; 14] = [
        Shape::Rectangle,
        Shape::RoundedRect,
        Shape::Ellipse,
        Shape::Circle,
        Shape::Diamond,
        Shape::Triangle,
        Shape::Parallelogram,
        Shape::Hexagon,
        Shape::Octagon,
        Shape::Cylinder,
        Shape::Document,
        Shape::Cloud,
        Shape::Note,
        Shape::Text,
    ];
    pub fn name(&self) -> &'static str {
        match self {
            Shape::Rectangle => "Rectangle",
            Shape::RoundedRect => "Rounded rectangle",
            Shape::Ellipse => "Ellipse",
            Shape::Circle => "Circle",
            Shape::Diamond => "Diamond (decision)",
            Shape::Triangle => "Triangle",
            Shape::Parallelogram => "Parallelogram (input/output)",
            Shape::Hexagon => "Hexagon",
            Shape::Octagon => "Octagon",
            Shape::Cylinder => "Cylinder (data store)",
            Shape::Document => "Document",
            Shape::Cloud => "Cloud",
            Shape::Note => "Sticky note",
            Shape::Text => "Text only",
        }
    }
    pub fn default_size(&self) -> (f32, f32) {
        match self {
            Shape::Circle => (110.0, 110.0),
            Shape::Diamond => (150.0, 100.0),
            Shape::Triangle => (130.0, 110.0),
            Shape::Cylinder => (120.0, 110.0),
            Shape::Cloud => (170.0, 110.0),
            Shape::Note => (140.0, 120.0),
            Shape::Text => (160.0, 40.0),
            Shape::Hexagon | Shape::Octagon => (150.0, 90.0),
            _ => (160.0, 70.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dash {
    Solid,
    Dashed,
    Dotted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Head {
    None,
    Arrow,
    OpenArrow,
    Triangle,
    Diamond,
    Circle,
    Bar,
}

impl Head {
    pub const ALL: [Head; 7] = [
        Head::None,
        Head::Arrow,
        Head::OpenArrow,
        Head::Triangle,
        Head::Diamond,
        Head::Circle,
        Head::Bar,
    ];
    pub fn name(&self) -> &'static str {
        match self {
            Head::None => "None",
            Head::Arrow => "Arrow (filled)",
            Head::OpenArrow => "Arrow (open)",
            Head::Triangle => "Triangle (hollow)",
            Head::Diamond => "Diamond",
            Head::Circle => "Circle",
            Head::Bar => "Bar",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Route {
    /// Straight segments through the bend points.
    Straight,
    /// Smooth (flexible) curve through the bend points.
    Curved,
    /// Right-angled (orthogonal) connector.
    Elbow,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: u64,
    pub shape: Shape,
    pub pos: P, // top-left
    pub w: f32,
    pub h: f32,
    pub text: String,
    pub fill: Rgba,
    pub stroke: Rgba,
    pub stroke_width: f32,
    pub dash: Dash,
    pub text_color: Rgba,
    pub font_size: f32,
    pub bold: bool,
    #[serde(default)]
    pub group: Option<u64>,
    #[serde(default)]
    pub shadow: bool,
}

impl Node {
    pub fn center(&self) -> P {
        P::new(self.pos.x + self.w / 2.0, self.pos.y + self.h / 2.0)
    }
    pub fn contains(&self, p: P) -> bool {
        p.x >= self.pos.x && p.x <= self.pos.x + self.w && p.y >= self.pos.y && p.y <= self.pos.y + self.h
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub id: u64,
    pub from: u64,
    pub to: u64,
    pub route: Route,
    /// Waypoints the line passes through (Straight and Curved).
    pub bends: Vec<P>,
    /// Elbow: position of the middle segment (x for horizontal-first, y for vertical-first).
    #[serde(default)]
    pub elbow_mid: Option<f32>,
    /// Elbow: force vertical-first routing (None = automatic).
    #[serde(default)]
    pub elbow_vertical: Option<bool>,
    pub color: Rgba,
    pub width: f32,
    pub dash: Dash,
    pub start_head: Head,
    pub end_head: Head,
    pub head_size: f32,
    pub label: String,
    #[serde(default)]
    pub label_size: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HopStyle {
    /// Semicircular bridge (the standard "line jump").
    Arc,
    /// Break in the line.
    Gap,
    /// Draw crossings as-is.
    None,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Diagram {
    pub title: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub next_id: u64,
    pub hop_style: HopStyle,
    pub hop_size: f32,
    pub background: Rgba,
}

impl Default for Diagram {
    fn default() -> Self {
        Self {
            title: "Untitled framework".into(),
            nodes: vec![],
            edges: vec![],
            next_id: 1,
            hop_style: HopStyle::Arc,
            hop_size: 8.0,
            background: Rgba::WHITE,
        }
    }
}

pub const PALETTE: [Rgba; 12] = [
    Rgba(255, 255, 255, 255),
    Rgba(219, 234, 254, 255),
    Rgba(209, 250, 229, 255),
    Rgba(254, 243, 199, 255),
    Rgba(254, 226, 226, 255),
    Rgba(237, 233, 254, 255),
    Rgba(37, 99, 235, 255),
    Rgba(5, 150, 105, 255),
    Rgba(217, 119, 6, 255),
    Rgba(220, 38, 38, 255),
    Rgba(124, 58, 237, 255),
    Rgba(31, 41, 55, 255),
];

impl Diagram {
    pub fn new_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    pub fn node(&self, id: u64) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }
    pub fn node_mut(&mut self, id: u64) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }
    pub fn edge_mut(&mut self, id: u64) -> Option<&mut Edge> {
        self.edges.iter_mut().find(|e| e.id == id)
    }

    pub fn add_node(&mut self, shape: Shape, center: P, text: &str) -> u64 {
        let id = self.new_id();
        let (w, h) = shape.default_size();
        let (fill, stroke) = match shape {
            Shape::Note => (Rgba(254, 240, 138, 255), Rgba(202, 138, 4, 255)),
            Shape::Text => (Rgba::NONE, Rgba::NONE),
            _ => (Rgba(219, 234, 254, 255), Rgba(30, 64, 175, 255)),
        };
        self.nodes.push(Node {
            id,
            shape,
            pos: P::new(center.x - w / 2.0, center.y - h / 2.0),
            w,
            h,
            text: text.to_string(),
            fill,
            stroke,
            stroke_width: 2.0,
            dash: Dash::Solid,
            text_color: Rgba(17, 24, 39, 255),
            font_size: 15.0,
            bold: false,
            group: None,
            shadow: false,
        });
        id
    }

    pub fn add_edge(&mut self, from: u64, to: u64) -> u64 {
        let id = self.new_id();
        self.edges.push(Edge {
            id,
            from,
            to,
            route: Route::Straight,
            bends: vec![],
            elbow_mid: None,
            elbow_vertical: None,
            color: Rgba(31, 41, 55, 255),
            width: 2.0,
            dash: Dash::Solid,
            start_head: Head::None,
            end_head: Head::Arrow,
            head_size: 12.0,
            label: String::new(),
            label_size: 13.0,
        });
        id
    }

    /// Remove nodes (and any edges attached to them) plus the given edges.
    pub fn delete(&mut self, nodes: &[u64], edges: &[u64]) {
        self.nodes.retain(|n| !nodes.contains(&n.id));
        self.edges
            .retain(|e| !edges.contains(&e.id) && !nodes.contains(&e.from) && !nodes.contains(&e.to));
    }

    /// All node ids that share a group with `id` (including itself).
    pub fn group_members(&self, id: u64) -> Vec<u64> {
        match self.node(id).and_then(|n| n.group) {
            Some(g) => self.nodes.iter().filter(|n| n.group == Some(g)).map(|n| n.id).collect(),
            None => vec![id],
        }
    }

    pub fn bounds(&self) -> Option<(P, P)> {
        if self.nodes.is_empty() {
            return None;
        }
        let mut lo = P::new(f32::MAX, f32::MAX);
        let mut hi = P::new(f32::MIN, f32::MIN);
        for n in &self.nodes {
            lo.x = lo.x.min(n.pos.x);
            lo.y = lo.y.min(n.pos.y);
            hi.x = hi.x.max(n.pos.x + n.w);
            hi.y = hi.y.max(n.pos.y + n.h);
        }
        for e in &self.edges {
            for b in &e.bends {
                lo.x = lo.x.min(b.x);
                lo.y = lo.y.min(b.y);
                hi.x = hi.x.max(b.x);
                hi.y = hi.y.max(b.y);
            }
            if let Some(m) = e.elbow_mid {
                // conservative: elbow mid may stick out on one axis
                let _ = m;
            }
        }
        Some((lo, hi))
    }
}
