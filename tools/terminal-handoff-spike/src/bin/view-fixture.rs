//! Actual singleton core frames. Tests separately exercise large transport IDs.
use terminal_handoff_spike::single_core::{CanonicalSession, View};
fn main() {
    let mut s = CanonicalSession::new(42, 12, 3);
    s.output("A中\x1b[38;2;11;22;33mR\x1b[0m".as_bytes())
        .unwrap();
    s.attach(View::Webview).unwrap();
    let f = s.frame(View::Webview).unwrap().unwrap();
    println!("{}", f.to_wire(s.session_id(), View::Webview));
    s.ack(View::Webview, f.epoch, f.revision).unwrap();
    s.output(b"\x1b[2;1HDAMAGE").unwrap();
    let f = s.frame(View::Webview).unwrap().unwrap();
    println!("{}", f.to_wire(s.session_id(), View::Webview));
}
