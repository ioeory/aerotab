use terminal_handoff_spike::single_core::{CanonicalSession, Error, View};
#[test]
fn combined_input_outboxes_stay_bounded_and_target_failure_never_destroys_core() {
    let mut s = CanonicalSession::new(1, 80, 24);
    let identity = s.core_identity();
    s.attach(View::Native).unwrap();
    s.attach(View::Webview).unwrap();
    let frame = s.frame(View::Webview).unwrap().unwrap();
    s.ack(View::Webview, frame.epoch, frame.revision).unwrap();
    s.first_frame(View::Webview, frame.epoch, frame.revision)
        .unwrap();
    s.input(&[1; 100]).unwrap();
    s.begin_commit(View::Webview, 0, 2).unwrap();
    assert_eq!(s.input(&[2; 29]), Err(Error::Backpressure));
    s.input(&[2; 28]).unwrap();
    s.detach(View::Webview).unwrap();
    assert_eq!(s.active(), View::Native);
    assert_eq!(s.take_input().len(), 128);
    assert_eq!(s.core_identity(), identity);
    s.output(b"\x1b[6n").unwrap();
    assert_eq!(s.take_responses(), vec!["\x1b[1;1R"]);
    s.attach(View::Webview).unwrap();
    let frame = s.frame(View::Webview).unwrap().unwrap();
    assert_eq!(frame.output_seq, 1);
    assert_eq!(frame.wire_seq, 2);
    assert!(frame.full);
    assert!(s.take_responses().is_empty());
    assert_eq!(s.feed_count(), 1);
}
