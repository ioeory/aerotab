//! Reproducible export of the EXACT per-char policy used by alacritty 0.26.
//! Run without flags to regenerate, --check to reject drift. No string/grapheme
//! width folding: alacritty Term::input calls UnicodeWidthChar::width per scalar.
use serde_json::json;
use std::path::PathBuf;
use unicode_width::UnicodeWidthChar;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut ranges: Vec<[u32; 3]> = Vec::new();
    for cp in 0..=0x10ffff {
        // Control/invalid scalar widths map to 0; controls are dispatched by VT,
        // and surrogate values never originate from valid UTF-8 decoding.
        let width = char::from_u32(cp)
            .and_then(UnicodeWidthChar::width)
            .unwrap_or(0) as u32;
        if let Some(last) = ranges.last_mut().filter(|r| r[2] == width) {
            last[1] = cp;
        } else {
            ranges.push([cp, cp, width]);
        }
    }
    let canonical = serde_json::to_vec(&ranges)?;
    use sha2::{Digest, Sha256};
    let hash = format!("{:x}", Sha256::digest(&canonical));
    let document = json!({"metadata":{"crate":"unicode-width=0.2.3","unicode":unicode_width::UNICODE_VERSION,
        "generator":"cargo run --locked --bin unicode-table","policy":"UnicodeWidthChar::width().unwrap_or(0); non-CJK; per scalar; no grapheme folding",
        "hashEncoding":"SHA256(JSON.stringify(ranges))","sha256":hash},"ranges":ranges});
    let mut data = serde_json::to_vec_pretty(&document)?;
    data.push(b'\n');
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("generated/unicode-width-0.2.3.json");
    if std::env::args().any(|arg| arg == "--check") {
        if std::fs::read(path)? != data {
            return Err("generated Unicode table drift".into());
        }
    } else {
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, data)?;
    }
    println!("unicode-table sha256={hash}");
    Ok(())
}
