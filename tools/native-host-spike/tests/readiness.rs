use native_host_spike::Terminal;
use native_host_spike::handoff::{PageReady, Stamp, draw_ack_matches, frame};
#[test]
fn delayed_ready_and_reload_reject_stale_generation() {
    let mut p = PageReady::default();
    let a = p.attach("doc-a");
    assert!(!p.ready);
    assert!(!p.confirm("wrong", a));
    assert!(p.confirm("doc-a", a));
    p.invalidate();
    assert!(!p.ready);
    let b = p.attach("doc-b");
    assert!(!p.confirm("doc-a", a));
    assert!(p.confirm("doc-b", b));
    assert!(b > a);
}
#[test]
fn delayed_startup_does_not_consume_renderer_timeout_budget() {
    let mut p = PageReady::default();
    let g = p.attach("slow");
    let mut ticks = 0;
    for _ in 0..100 {
        if p.ready {
            ticks += 1;
        }
    }
    assert_eq!(ticks, 0);
    assert!(p.confirm("slow", g));
    assert_eq!(ticks, 0);
}
#[test]
fn independent_pending_pane_leaves_other_parser_running() {
    use native_host_spike::handoff::{Switch, View};
    let mut switches = [Switch::default(), Switch::default()];
    let mut terminals = [Terminal::new(32, 2), Terminal::new(32, 2)];
    switches[0].begin(View::Web, 0, 0);
    for i in 0..2 {
        if switches[i].pending().is_none() {
            terminals[i].feed(b"INDEPENDENT_MARKER");
        }
    }
    assert!(
        terminals[1]
            .snapshot()
            .cells
            .iter()
            .map(|c| c.text.as_str())
            .collect::<String>()
            .contains("INDEPENDENT_MARKER")
    );
    switches[0].fail();
    assert_eq!(switches[0].active, View::Native);
}
#[test]
fn only_public_exact_drawn_ticket_ack_matches() {
    let f = frame(
        &Terminal::new(8, 2).snapshot(),
        Stamp {
            epoch: 3,
            revision: 4,
            output_seq: 5,
        },
        0,
    );
    let mut a = serde_json::json!({"version":1,"session":"0","view":"webview","epoch":"3","revision":"4","output_seq":"5","wire_seq":4,"phase":"drawn","evidence":"canvas-raf","physical_present":false});
    assert!(draw_ack_matches(&f, &a));
    a["phase"] = "applied".into();
    assert!(!draw_ack_matches(&f, &a));
    a["phase"] = "drawn".into();
    a["session"] = "1".into();
    assert!(!draw_ack_matches(&f, &a));
    for (key, value) in [
        ("session", serde_json::json!("wrong")),
        ("view", serde_json::json!("native")),
        ("epoch", serde_json::json!("2")),
        ("revision", serde_json::json!("3")),
        ("output_seq", serde_json::json!("4")),
        ("version", serde_json::json!(2)),
        ("wire_seq", serde_json::json!(3)),
        ("physical_present", serde_json::json!(true)),
        ("evidence", serde_json::json!("native-draw-callback")),
    ] {
        let mut bad = serde_json::json!({"version":1,"session":"0","view":"webview","epoch":"3","revision":"4","output_seq":"5","wire_seq":4,"phase":"drawn","evidence":"canvas-raf","physical_present":false});
        bad[key] = value;
        assert!(!draw_ack_matches(&f, &bad), "{key}");
    }
}
