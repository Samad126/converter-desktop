//! The page editor's data and maths, kept free of UI code so it can be tested: annotations live in PDF points with a
//! top-left origin (the convention of the core's edit/sign/redact functions); the screen shows them scaled.
use converter_core::pdf::python_ops::Area;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool { Text, Rect, Ellipse, Line, Signature, Redact }

impl Tool {
    pub const ALL: [(Tool, &'static str); 6] = [(Tool::Text, "Text"), (Tool::Rect, "Rectangle"), (Tool::Ellipse, "Ellipse"), (Tool::Line, "Line"),
        (Tool::Signature, "Signature"), (Tool::Redact, "Redact")];
    /// Click-to-place tools take their text from the field; the others are dragged out.
    pub fn is_placed(self) -> bool { matches!(self, Tool::Text | Tool::Signature) }
}

pub const COLORS: [&str; 6] = ["black", "red", "blue", "green", "yellow", "orange"];

#[derive(Debug, Clone, PartialEq)]
pub struct Item { pub page: usize, pub tool: Tool, pub x0: f64, pub y0: f64, pub x1: f64, pub y1: f64, pub text: String, pub color: &'static str }

/// A drag in any direction → (x, y, width, height) with a non-negative size.
pub fn norm_rect(x0: f64, y0: f64, x1: f64, y1: f64) -> (f64, f64, f64, f64) { (x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs()) }

/// Window position → PDF points on a page shown at `scale` px per point, clamped to the page.
pub fn screen_to_pt(pos: (f32, f32), origin: (f32, f32), scale: f32, page: (f64, f64)) -> (f64, f64) {
    let x = ((pos.0 - origin.0) / scale) as f64;
    let y = ((pos.1 - origin.1) / scale) as f64;
    (x.clamp(0.0, page.0), y.clamp(0.0, page.1))
}

/// Smallest drag that counts as a shape (anything less is a stray click).
pub const MIN_DRAG: f64 = 4.0;

/// What a finished gesture becomes, or `None` for a stray click / an empty text.
pub fn make_item(page: usize, tool: Tool, color: &'static str, start: (f64, f64), end: (f64, f64), text: &str, page_size: (f64, f64)) -> Option<Item> {
    let (x, y, w, h) = norm_rect(start.0, start.1, end.0, end.1);
    match tool {
        Tool::Text => (!text.trim().is_empty()).then(|| Item { page, tool, x0: start.0, y0: start.1, x1: start.0, y1: start.1, text: text.into(), color }),
        Tool::Signature => {
            if text.trim().is_empty() { return None; }
            // A drag sets the box; a plain click drops a default-sized one, kept inside the page.
            let (bw, bh) = if w >= MIN_DRAG && h >= MIN_DRAG { (w, h) } else { (200.0, 50.0) };
            let (bx, by) = if w >= MIN_DRAG && h >= MIN_DRAG { (x, y) } else { (start.0.min(page_size.0 - bw).max(0.0), start.1.min(page_size.1 - bh).max(0.0)) };
            Some(Item { page, tool, x0: bx, y0: by, x1: bx + bw, y1: by + bh, text: text.into(), color })
        }
        Tool::Line => ((w * w + h * h).sqrt() >= MIN_DRAG).then(|| Item { page, tool, x0: start.0, y0: start.1, x1: end.0, y1: end.1, text: String::new(), color }),
        Tool::Rect | Tool::Ellipse | Tool::Redact => (w >= MIN_DRAG && h >= MIN_DRAG).then(|| Item { page, tool, x0: x, y0: y, x1: x + w, y1: y + h, text: String::new(), color }),
    }
}

/// The three kinds of request the core understands, built from the annotations (pages become 1-based).
pub struct Requests { pub redact: Vec<Area>, pub edit: Value, pub sign: Value }

pub fn to_requests(items: &[Item]) -> Requests {
    let mut r = Requests { redact: vec![], edit: json!([]), sign: json!([]) };
    for it in items {
        let page = it.page + 1;
        let (x, y, w, h) = norm_rect(it.x0, it.y0, it.x1, it.y1);
        match it.tool {
            Tool::Redact => r.redact.push(Area { page, x, y, width: w, height: h }),
            Tool::Text => r.edit.as_array_mut().unwrap().push(json!({"type":"text","page":page,"x":it.x0,"y":it.y0,"value":it.text,"fontSize":14,"color":it.color})),
            Tool::Rect => r.edit.as_array_mut().unwrap().push(json!({"type":"rectangle","page":page,"x":x,"y":y,"width":w,"height":h,"color":it.color,"strokeWidth":2})),
            Tool::Ellipse => r.edit.as_array_mut().unwrap().push(json!({"type":"ellipse","page":page,"x":x,"y":y,"width":w,"height":h,"color":it.color,"strokeWidth":2})),
            Tool::Line => r.edit.as_array_mut().unwrap().push(json!({"type":"line","page":page,"x1":it.x0,"y1":it.y0,"x2":it.x1,"y2":it.y1,"color":it.color,"strokeWidth":2})),
            Tool::Signature => r.sign.as_array_mut().unwrap().push(json!({"type":"signature","page":page,"x":x,"y":y,"width":w,"height":h,"value":it.text,"fontStyle":"cursive","color":sign_color(it.color)})),
        }
    }
    r
}

/// Signatures only come in four colours.
fn sign_color(c: &str) -> &'static str { match c { "red" => "red", "blue" => "blue", "green" => "green", _ => "black" } }

#[cfg(test)]
mod tests {
    use super::*;
    use converter_core::pdf::overlay::{EditElement, SignElement};
    const PAGE: (f64, f64) = (612.0, 792.0);

    #[test] fn drags_are_normalised_in_every_direction() {
        assert_eq!(norm_rect(100.0, 80.0, 40.0, 20.0), (40.0, 20.0, 60.0, 60.0));
        assert_eq!(norm_rect(1.0, 2.0, 3.0, 5.0), (1.0, 2.0, 2.0, 3.0));
    }
    #[test] fn screen_positions_map_to_points_and_clamp_to_the_page() {
        assert_eq!(screen_to_pt((300.0, 200.0), (100.0, 50.0), 0.5, PAGE), (400.0, 300.0));
        assert_eq!(screen_to_pt((0.0, 0.0), (100.0, 50.0), 0.5, PAGE), (0.0, 0.0), "above/left of the page clamps to 0");
        assert_eq!(screen_to_pt((9999.0, 9999.0), (0.0, 0.0), 1.0, PAGE), PAGE, "beyond the page clamps to its size");
    }
    #[test] fn stray_clicks_make_no_shape_but_text_and_signature_are_placed() {
        assert!(make_item(0, Tool::Rect, "red", (10.0, 10.0), (12.0, 11.0), "", PAGE).is_none());
        assert!(make_item(0, Tool::Redact, "red", (10.0, 10.0), (50.0, 11.0), "", PAGE).is_none(), "zero height is not a box");
        assert!(make_item(0, Tool::Line, "red", (10.0, 10.0), (11.0, 11.0), "", PAGE).is_none());
        let t = make_item(0, Tool::Text, "blue", (30.0, 40.0), (30.0, 40.0), "Hello", PAGE).unwrap();
        assert_eq!((t.x0, t.y0), (30.0, 40.0));
        assert!(make_item(0, Tool::Text, "blue", (30.0, 40.0), (30.0, 40.0), "  ", PAGE).is_none(), "empty text places nothing");
    }
    #[test] fn a_signature_click_gets_a_default_box_that_stays_on_the_page() {
        let s = make_item(0, Tool::Signature, "blue", (600.0, 780.0), (600.0, 780.0), "Ann", PAGE).unwrap();
        assert_eq!((s.x1 - s.x0, s.y1 - s.y0), (200.0, 50.0));
        assert!(s.x1 <= PAGE.0 && s.y1 <= PAGE.1, "{s:?}");
        let d = make_item(0, Tool::Signature, "blue", (100.0, 100.0), (260.0, 150.0), "Ann", PAGE).unwrap();
        assert_eq!((d.x0, d.y0, d.x1, d.y1), (100.0, 100.0, 260.0, 150.0), "a drag defines the box");
    }
    #[test] fn requests_split_by_kind_and_use_one_based_pages() {
        let items = vec![
            make_item(1, Tool::Redact, "black", (50.0, 50.0), (150.0, 90.0), "", PAGE).unwrap(),
            make_item(0, Tool::Rect, "red", (10.0, 10.0), (60.0, 40.0), "", PAGE).unwrap(),
            make_item(0, Tool::Text, "green", (20.0, 30.0), (20.0, 30.0), "Hi", PAGE).unwrap(),
            make_item(2, Tool::Signature, "orange", (70.0, 80.0), (70.0, 80.0), "Ann", PAGE).unwrap(),
        ];
        let r = to_requests(&items);
        assert_eq!((r.redact.len(), r.edit.as_array().unwrap().len(), r.sign.as_array().unwrap().len()), (1, 2, 1));
        assert_eq!(r.redact[0].page, 2);
        assert_eq!(r.sign[0]["page"], 3);
        assert_eq!(r.sign[0]["color"], "black", "orange is not a signature colour");
    }
    #[test] fn generated_json_is_accepted_by_the_core_types() {
        // The editor never constructs core types directly; this proves the JSON it builds deserialises into them.
        let items: Vec<Item> = Tool::ALL.iter().filter(|(t, _)| *t != Tool::Redact).filter_map(|(t, _)| make_item(0, *t, "blue", (20.0, 20.0), (120.0, 90.0), "x", PAGE)).collect();
        assert_eq!(items.len(), 5);
        let r = to_requests(&items);
        let edits: Vec<EditElement> = serde_json::from_value(r.edit).unwrap();
        let signs: Vec<SignElement> = serde_json::from_value(r.sign).unwrap();
        assert_eq!((edits.len(), signs.len()), (4, 1));
    }
}
