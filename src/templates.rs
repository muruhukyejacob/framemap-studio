//! Starter templates.

use crate::model::*;

pub const TEMPLATES: [(&str, &str); 4] = [
    ("Blank canvas", "blank"),
    ("Conceptual framework (IV → DV)", "conceptual"),
    ("Systems map (causal loops)", "systems"),
    ("Logic model / theory of change", "logic"),
];

fn style_header(n: &mut Node, fill: Rgba, stroke: Rgba) {
    n.fill = fill;
    n.stroke = stroke;
    n.text_color = Rgba::WHITE;
    n.bold = true;
    n.font_size = 15.0;
}

pub fn make(kind: &str) -> Diagram {
    let mut d = Diagram::default();
    match kind {
        "conceptual" => {
            d.title = "Conceptual framework".into();
            let blue = (Rgba(30, 64, 175, 255), Rgba(30, 58, 138, 255));
            let green = (Rgba(4, 120, 87, 255), Rgba(6, 95, 70, 255));
            let amber = (Rgba(180, 83, 9, 255), Rgba(146, 64, 14, 255));
            // Independent variable column
            let g1 = d.new_id();
            let h1 = d.add_node(Shape::Rectangle, P::new(160.0, 80.0), "INDEPENDENT VARIABLES");
            let b1 = d.add_node(
                Shape::Rectangle,
                P::new(160.0, 210.0),
                "Household factors\n• Income level\n• Education of head\n• Household size\n• Access to credit",
            );
            for (id, hdr) in [(h1, true), (b1, false)] {
                let n = d.node_mut(id).unwrap();
                n.w = 240.0;
                n.group = Some(g1);
                if hdr {
                    n.h = 50.0;
                    n.pos = P::new(40.0, 55.0);
                    style_header(n, blue.0, blue.1);
                } else {
                    n.h = 180.0;
                    n.pos = P::new(40.0, 105.0);
                    n.fill = Rgba(219, 234, 254, 255);
                    n.stroke = blue.1;
                }
            }
            // Dependent variable column
            let g2 = d.new_id();
            let h2 = d.add_node(Shape::Rectangle, P::new(0.0, 0.0), "DEPENDENT VARIABLE");
            let b2 = d.add_node(Shape::Rectangle, P::new(0.0, 0.0), "Climate resilience\n• Food security\n• Asset retention\n• Recovery time");
            for (id, hdr) in [(h2, true), (b2, false)] {
                let n = d.node_mut(id).unwrap();
                n.w = 240.0;
                n.group = Some(g2);
                if hdr {
                    n.h = 50.0;
                    n.pos = P::new(560.0, 55.0);
                    style_header(n, green.0, green.1);
                } else {
                    n.h = 180.0;
                    n.pos = P::new(560.0, 105.0);
                    n.fill = Rgba(209, 250, 229, 255);
                    n.stroke = green.1;
                }
            }
            // Intervening variables
            let g3 = d.new_id();
            let h3 = d.add_node(Shape::Rectangle, P::new(0.0, 0.0), "INTERVENING VARIABLES");
            let b3 = d.add_node(Shape::Rectangle, P::new(0.0, 0.0), "• Government policy\n• NGO interventions\n• Rainfall variability");
            for (id, hdr) in [(h3, true), (b3, false)] {
                let n = d.node_mut(id).unwrap();
                n.w = 240.0;
                n.group = Some(g3);
                if hdr {
                    n.h = 50.0;
                    n.pos = P::new(300.0, 370.0);
                    style_header(n, amber.0, amber.1);
                } else {
                    n.h = 120.0;
                    n.pos = P::new(300.0, 420.0);
                    n.fill = Rgba(254, 243, 199, 255);
                    n.stroke = amber.1;
                }
            }
            let e1 = d.add_edge(b1, b2);
            let e2 = d.add_edge(h3, b2);
            if let Some(e) = d.edge_mut(e1) {
                e.width = 3.0;
                e.head_size = 16.0;
            }
            if let Some(e) = d.edge_mut(e2) {
                e.route = Route::Elbow;
                e.elbow_vertical = Some(true);
                e.dash = Dash::Dashed;
            }
            let src = d.add_node(Shape::Text, P::new(420.0, 600.0), "Source: Researcher's own construct (2026)");
            if let Some(n) = d.node_mut(src) {
                n.w = 420.0;
                n.pos.x = 210.0;
                n.font_size = 12.0;
                n.text_color = Rgba(75, 85, 99, 255);
            }
        }
        "systems" => {
            d.title = "Systems map: waste, drainage and flooding".into();
            let pts = [
                ("Waste dumped in channels", 120.0, 100.0),
                ("Blocked drainage", 470.0, 100.0),
                ("Flooding", 620.0, 320.0),
                ("Waterborne disease", 400.0, 500.0),
                ("Household income", 100.0, 470.0),
                ("Community clean-ups", 300.0, 290.0),
            ];
            let ids: Vec<u64> = pts
                .iter()
                .map(|(t, x, y)| {
                    let id = d.add_node(Shape::Ellipse, P::new(*x, *y), t);
                    let n = d.node_mut(id).unwrap();
                    n.fill = Rgba(237, 233, 254, 255);
                    n.stroke = Rgba(91, 33, 182, 255);
                    id
                })
                .collect();
            let links = [(0, 1, "+"), (1, 2, "+"), (2, 3, "+"), (3, 4, "−"), (4, 0, "−"), (5, 0, "−"), (2, 5, "+")];
            for (a, b, l) in links {
                let e = d.add_edge(ids[a], ids[b]);
                let ed = d.edge_mut(e).unwrap();
                ed.route = Route::Curved;
                ed.label = l.into();
                ed.label_size = 16.0;
                ed.color = if l == "+" { Rgba(29, 78, 216, 255) } else { Rgba(185, 28, 28, 255) };
            }
            // bend each loop edge outward a little
            let cx = 380.0;
            let cy = 300.0;
            for e in d.edges.clone() {
                let (a, b) = (d.node(e.from).unwrap().center(), d.node(e.to).unwrap().center());
                let mid = a.lerp(b, 0.5);
                let out = mid.sub(P::new(cx, cy)).norm();
                if let Some(ed) = d.edge_mut(e.id) {
                    ed.bends = vec![mid.add(out.mul(35.0))];
                }
            }
            let r = d.add_node(Shape::Circle, P::new(380.0, 400.0), "R1");
            let n = d.node_mut(r).unwrap();
            n.w = 50.0;
            n.h = 50.0;
            n.fill = Rgba::WHITE;
            n.bold = true;
        }
        "logic" => {
            d.title = "Logic model".into();
            let stages = [
                ("Inputs", "Funds, staff, data"),
                ("Activities", "Training, workshops"),
                ("Outputs", "People trained, maps produced"),
                ("Outcomes", "Improved drainage practice"),
                ("Impact", "Reduced flood risk"),
            ];
            let colors = [
                Rgba(219, 234, 254, 255),
                Rgba(209, 250, 229, 255),
                Rgba(254, 243, 199, 255),
                Rgba(254, 226, 226, 255),
                Rgba(237, 233, 254, 255),
            ];
            let mut prev = None;
            for (i, (h, b)) in stages.iter().enumerate() {
                let id = d.add_node(Shape::RoundedRect, P::new(110.0 + i as f32 * 210.0, 160.0), &format!("{h}\n{b}"));
                let n = d.node_mut(id).unwrap();
                n.h = 110.0;
                n.fill = colors[i];
                if let Some(p) = prev {
                    let e = d.add_edge(p, id);
                    d.edge_mut(e).unwrap().route = Route::Elbow;
                }
                prev = Some(id);
            }
            let a = d.add_node(Shape::Note, P::new(320.0, 340.0), "Assumptions: community participation, stable funding");
            d.node_mut(a).unwrap().w = 260.0;
            let k = d.add_node(Shape::Note, P::new(740.0, 340.0), "External factors: rainfall, policy changes");
            d.node_mut(k).unwrap().w = 260.0;
        }
        _ => {}
    }
    d
}
