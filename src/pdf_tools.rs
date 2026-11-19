//! The PDF tools that fit a form: each is a spec (files accepted + fields) that `build_flags` turns into the
//! flags `converter_core::pdf_cli` expects. The tools that need a page canvas (sign, edit, redact, forms) come later.
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq)]
pub enum Accepts { Pdf, Images }

pub enum FieldKind { Text { secret: bool }, Choice(&'static [(&'static str, &'static str)]) }

pub struct FieldSpec { pub key: &'static str, pub label: &'static str, pub hint: &'static str, pub kind: FieldKind, pub optional: bool, pub default: &'static str }

pub struct ToolSpec {
    pub op: &'static str,
    pub name: &'static str,
    pub desc: &'static str,
    pub accepts: Accepts,
    pub min_files: usize,
    pub multi: bool,
    pub fields: &'static [FieldSpec],
}

const PAGES: FieldSpec = FieldSpec { key: "pages", label: "Pages", hint: "e.g. 1,3-5", kind: FieldKind::Text { secret: false }, optional: false, default: "" };
const PAGES_OPT: FieldSpec = FieldSpec { key: "pages", label: "Only these pages (optional)", hint: "e.g. 1,3-5 — empty means all", kind: FieldKind::Text { secret: false }, optional: true, default: "" };
const PASSWORD: FieldSpec = FieldSpec { key: "password", label: "Password", hint: "", kind: FieldKind::Text { secret: true }, optional: false, default: "" };

pub static TOOLS: &[ToolSpec] = &[
    ToolSpec { op: "merge", name: "Merge", desc: "Combine PDFs into one, in the order listed.", accepts: Accepts::Pdf, min_files: 2, multi: true, fields: &[] },
    ToolSpec { op: "split", name: "Split", desc: "Cut a PDF into parts of N pages.", accepts: Accepts::Pdf, min_files: 1, multi: false,
        fields: &[FieldSpec { key: "every", label: "Pages per part", hint: "", kind: FieldKind::Text { secret: false }, optional: false, default: "1" }] },
    ToolSpec { op: "extract", name: "Extract pages", desc: "Keep only the pages you name.", accepts: Accepts::Pdf, min_files: 1, multi: false, fields: &[PAGES] },
    ToolSpec { op: "remove", name: "Remove pages", desc: "Delete the pages you name.", accepts: Accepts::Pdf, min_files: 1, multi: false, fields: &[PAGES] },
    ToolSpec { op: "organize", name: "Reorder pages", desc: "Give every page number once, in the new order.", accepts: Accepts::Pdf, min_files: 1, multi: false,
        fields: &[FieldSpec { key: "order", label: "New order", hint: "e.g. 3,1,2", kind: FieldKind::Text { secret: false }, optional: false, default: "" }] },
    ToolSpec { op: "rotate", name: "Rotate", desc: "Turn pages clockwise or counter-clockwise.", accepts: Accepts::Pdf, min_files: 1, multi: false,
        fields: &[FieldSpec { key: "degrees", label: "Rotation", hint: "", kind: FieldKind::Choice(&[("90", "90° right"), ("180", "180°"), ("-90", "90° left")]), optional: false, default: "90" }, PAGES_OPT] },
    ToolSpec { op: "crop", name: "Crop", desc: "Trim margins off every page.", accepts: Accepts::Pdf, min_files: 1, multi: false,
        fields: &[NUM_FIELD_LEFT, NUM_FIELD_RIGHT, NUM_FIELD_TOP, NUM_FIELD_BOTTOM, PAGES_OPT] },
    ToolSpec { op: "compress", name: "Compress", desc: "Make the file smaller.", accepts: Accepts::Pdf, min_files: 1, multi: false,
        fields: &[FieldSpec { key: "level", label: "Strength", hint: "", kind: FieldKind::Choice(&[("low", "Light"), ("medium", "Balanced"), ("high", "Strong")]), optional: false, default: "medium" }] },
    ToolSpec { op: "protect", name: "Add password", desc: "Encrypt the PDF so it needs a password to open.", accepts: Accepts::Pdf, min_files: 1, multi: false, fields: &[PASSWORD] },
    ToolSpec { op: "unlock", name: "Remove password", desc: "Open a protected PDF and save an unprotected copy.", accepts: Accepts::Pdf, min_files: 1, multi: false, fields: &[PASSWORD] },
    ToolSpec { op: "repair", name: "Repair", desc: "Try to rebuild a damaged PDF.", accepts: Accepts::Pdf, min_files: 1, multi: false, fields: &[] },
    ToolSpec { op: "watermark", name: "Watermark", desc: "Stamp text across the pages.", accepts: Accepts::Pdf, min_files: 1, multi: false,
        fields: &[FieldSpec { key: "text", label: "Text", hint: "e.g. CONFIDENTIAL", kind: FieldKind::Text { secret: false }, optional: false, default: "" }, PAGES_OPT] },
    ToolSpec { op: "page-numbers", name: "Page numbers", desc: "Number every page.", accepts: Accepts::Pdf, min_files: 1, multi: false,
        fields: &[FieldSpec { key: "position", label: "Position", hint: "", kind: FieldKind::Choice(&[("bottom-left", "Left"), ("bottom-center", "Centre"), ("bottom-right", "Right")]), optional: false, default: "bottom-center" },
                  FieldSpec { key: "start", label: "Start at", hint: "", kind: FieldKind::Text { secret: false }, optional: false, default: "1" }] },
    ToolSpec { op: "scan-to-pdf", name: "Images to PDF", desc: "One page per image (PNG or JPG), in the order listed.", accepts: Accepts::Images, min_files: 1, multi: true, fields: &[] },
    ToolSpec { op: "ocr", name: "Make searchable (OCR)", desc: "Add a text layer to a scanned PDF.", accepts: Accepts::Pdf, min_files: 1, multi: false,
        fields: &[FieldSpec { key: "force", label: "If it already has text", hint: "", kind: FieldKind::Choice(&[("false", "Leave it"), ("true", "Redo OCR")]), optional: false, default: "false" }] },
];

const fn margin(key: &'static str, label: &'static str) -> FieldSpec {
    FieldSpec { key, label, hint: "points (1 pt = 1/72 inch)", kind: FieldKind::Text { secret: false }, optional: true, default: "0" }
}
const NUM_FIELD_LEFT: FieldSpec = margin("left", "Left");
const NUM_FIELD_RIGHT: FieldSpec = margin("right", "Right");
const NUM_FIELD_TOP: FieldSpec = margin("top", "Top");
const NUM_FIELD_BOTTOM: FieldSpec = margin("bottom", "Bottom");

/// Turns the form into pdf_cli flags. Optional empty fields are left out; a missing required one is an error
/// worded for the person filling the form.
pub fn build_flags(tool: &ToolSpec, values: &HashMap<&'static str, String>) -> Result<HashMap<String, String>, String> {
    let mut out = HashMap::new();
    for f in tool.fields {
        let v = values.get(f.key).map(|s| s.trim()).unwrap_or("");
        if v.is_empty() {
            if f.optional { continue; }
            return Err(format!("\"{}\" is required.", f.label));
        }
        out.insert(f.key.to_string(), v.to_string());
    }
    Ok(out)
}

pub fn accepts_file(tool: &ToolSpec, ext: &str) -> bool {
    match tool.accepts { Accepts::Pdf => ext == ".pdf", Accepts::Images => matches!(ext, ".png" | ".jpg" | ".jpeg") }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tool(op: &str) -> &'static ToolSpec { TOOLS.iter().find(|t| t.op == op).unwrap() }
    fn vals(p: &[(&'static str, &str)]) -> HashMap<&'static str, String> { p.iter().map(|(k, v)| (*k, v.to_string())).collect() }

    #[test] fn every_tool_is_a_real_core_operation() {
        // Keep this list in step with `converter_core::pdf_cli::run`: a typo here would only show up as a runtime error.
        let known = ["merge", "split", "extract", "remove", "organize", "rotate", "crop", "compress", "protect", "unlock", "repair", "watermark", "page-numbers", "scan-to-pdf", "ocr"];
        assert_eq!(TOOLS.len(), known.len());
        for t in TOOLS { assert!(known.contains(&t.op), "{}", t.op); }
        let ops: std::collections::HashSet<_> = TOOLS.iter().map(|t| t.op).collect();
        assert_eq!(ops.len(), TOOLS.len(), "no duplicate tools");
    }
    #[test] fn required_fields_are_enforced_with_a_readable_message() {
        assert_eq!(build_flags(tool("extract"), &vals(&[])).unwrap_err(), "\"Pages\" is required.");
        assert_eq!(build_flags(tool("protect"), &vals(&[("password", "  ")])).unwrap_err(), "\"Password\" is required.");
        assert!(build_flags(tool("repair"), &vals(&[])).unwrap().is_empty());
    }
    #[test] fn optional_fields_are_omitted_when_empty() {
        let f = build_flags(tool("rotate"), &vals(&[("degrees", "90"), ("pages", "")])).unwrap();
        assert_eq!(f.get("degrees").map(String::as_str), Some("90"));
        assert!(!f.contains_key("pages"));
        let f = build_flags(tool("crop"), &vals(&[("left", "12"), ("top", " ")])).unwrap();
        assert_eq!(f.len(), 1);
    }
    #[test] fn choices_use_values_the_core_understands() {
        for t in TOOLS { for f in t.fields {
            if let FieldKind::Choice(opts) = &f.kind {
                assert!(opts.iter().any(|(v, _)| *v == f.default), "{}.{} default must be one of its options", t.op, f.key);
            }
        } }
        let pos = tool("page-numbers").fields.iter().find(|f| f.key == "position").unwrap();
        if let FieldKind::Choice(o) = &pos.kind { assert!(o.iter().all(|(v, _)| v.starts_with("bottom-"))); }
    }
    #[test] fn file_filters() {
        assert!(accepts_file(tool("merge"), ".pdf") && !accepts_file(tool("merge"), ".png"));
        assert!(accepts_file(tool("scan-to-pdf"), ".jpeg") && !accepts_file(tool("scan-to-pdf"), ".pdf"));
        assert!(tool("merge").min_files == 2 && tool("merge").multi && !tool("split").multi);
    }
}
