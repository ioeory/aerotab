use terminal_handoff_spike::single_core::{CanonicalSession, Error, View};

fn present(s: &mut CanonicalSession, view: View) {
    let frame = s.frame(view).unwrap().unwrap();
    s.ack(view, frame.epoch, frame.revision).unwrap();
    s.first_frame(view, frame.epoch, frame.revision).unwrap();
}
#[test]
fn real_single_core_split_stream_survives_full_damage_handoff_without_reparse() {
    let mut s = CanonicalSession::new(42, 80, 24);
    s.attach(View::Native).unwrap();
    present(&mut s, View::Native);
    let pointer = s.core_identity();
    let fixture =
        "main\r\n中e\u{301}\x1b[31mR\x1b]0;title\x07\x1b[?1049hvim\x1b[3;5H\x1b[?25l\x1b[6n";
    for byte in fixture.as_bytes() {
        s.output(&[*byte]).unwrap();
    }
    s.attach(View::Webview).unwrap();
    let full = s.frame(View::Webview).unwrap().unwrap();
    assert!(full.full);
    assert_eq!(full.snapshot, s.snapshot());
    s.output(b"tail").unwrap();
    s.ack(View::Webview, full.epoch, full.revision).unwrap();
    assert_eq!(s.begin_commit(View::Webview, 10, 20), Err(Error::NotReady));
    let damage = s.frame(View::Webview).unwrap().unwrap();
    assert!(!damage.full);
    assert_eq!(damage.base_revision, Some(full.revision));
    assert!(!damage.changed_rows.is_empty());
    s.ack(View::Webview, damage.epoch, damage.revision).unwrap();
    assert_eq!(s.begin_commit(View::Webview, 10, 20), Err(Error::NotReady));
    s.first_frame(View::Webview, damage.epoch, damage.revision)
        .unwrap();
    s.begin_commit(View::Webview, 10, 20).unwrap();
    s.commit(11).unwrap();
    assert_eq!(s.active(), View::Webview);
    assert_eq!(s.core_identity(), pointer);
    assert_eq!(s.session_id(), 42);
    assert_eq!(s.feed_count(), fixture.len() as u64 + 1);
    assert_eq!(s.take_responses(), vec!["\x1b[3;5R".to_string()]);
    s.output(b"\x1b[?1049l\x1b[?25h").unwrap();
    assert!(s.snapshot().rows.iter().any(|r| r.contains("main")));
    assert!(s.take_responses().is_empty());
}
#[test]
fn bounded_frames_input_freeze_timeout_cancel_and_stale_epoch_keep_old_view() {
    let mut s = CanonicalSession::new(9, 80, 24);
    s.attach(View::Native).unwrap();
    present(&mut s, View::Native);
    s.attach(View::Webview).unwrap();
    present(&mut s, View::Webview);
    s.begin_commit(View::Webview, 0, 5).unwrap();
    s.input(b"queued").unwrap();
    assert_eq!(s.input(&[0; 129]), Err(Error::Backpressure));
    s.output(b"still reads output").unwrap();
    assert_eq!(s.commit(1), Err(Error::NotReady));
    assert_eq!(s.commit(6), Err(Error::Timeout));
    assert_eq!(s.active(), View::Native);
    assert_eq!(s.take_input(), b"queued");
    let old = s.frame(View::Webview).unwrap().unwrap();
    s.detach(View::Webview).unwrap();
    s.attach(View::Webview).unwrap();
    assert_eq!(
        s.ack(View::Webview, old.epoch, old.revision),
        Err(Error::Stale)
    );
    assert!(s.frame(View::Webview).unwrap().unwrap().full);
    assert_eq!(s.detach(View::Native), Err(Error::ActiveView));
    assert!(s.retained_frames() <= 2);
}
#[test]
fn high_output_resize_request_full_eof_never_restarts_core_or_replays_queries() {
    let mut s = CanonicalSession::new(11, 80, 24);
    s.attach(View::Native).unwrap();
    present(&mut s, View::Native);
    s.attach(View::Webview).unwrap();
    let old = s.frame(View::Webview).unwrap().unwrap();
    for _ in 0..4000 {
        s.output(
            b"01234567890123456789012345678901234567890123456789012345678901234567890123456789\r\n",
        )
        .unwrap();
    }
    s.resize(93, 30).unwrap();
    assert!(s.snapshot().history > 1000);
    assert_eq!(s.output_seq(), 4001);
    assert!(s.retained_frames() <= 2);
    s.ack(View::Webview, old.epoch, old.revision).unwrap();
    s.request_full(View::Webview).unwrap();
    let full = s.frame(View::Webview).unwrap().unwrap();
    assert!(full.full);
    assert_eq!(full.snapshot, s.snapshot());
    s.ack(View::Webview, full.epoch, full.revision).unwrap();
    s.first_frame(View::Webview, full.epoch, full.revision)
        .unwrap();
    s.begin_commit(View::Webview, 0, 10).unwrap();
    s.cancel().unwrap();
    assert_eq!(s.active(), View::Native);
    s.eof();
    assert_eq!(s.output(b"late"), Err(Error::Ended));
    assert!(s.snapshot().ended);
}
