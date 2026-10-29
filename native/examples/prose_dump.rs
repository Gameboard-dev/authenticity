//! What the AI check sends for a note (a developer's check, not part of the release): each unit's text exactly as sent
//! (prose.rs, ai::sections, ai::units, white space made single as sloptotal::paragraphs does), with the raw Markdown
//! of the paragraphs it covers, as JSON lines.
//!
//! `cargo run --release --example prose_dump -- NOTE.md`

fn main() {
    let path = std::env::args().nth(1).expect("the note");
    let note = std::fs::read_to_string(&path).expect("the note");
    let prose = authenticity_core::prose::extract(&note);
    let (paras, boundary) = authenticity_core::ai::sections(&note, authenticity_core::ai::paragraphs(&prose));
    let units = authenticity_core::ai::units(&paras, &boundary, authenticity_core::sloptotal::SHORT_WORDS);
    let utf16: Vec<u16> = note.encode_utf16().collect();
    for u in units {
        let (from, to) = (paras[u.span.start].from as usize, paras[u.span.end - 1].to as usize);
        let raw = String::from_utf16_lossy(&utf16[from.min(utf16.len())..to.min(utf16.len())]);
        let sent = u.text.split_whitespace().collect::<Vec<_>>().join(" ");
        println!("{}", serde_json::json!({ "span": [u.span.start, u.span.end], "members": u.members, "sent": sent, "raw": raw }));
    }
}
