use native_host_spike::handoff::{
    InputContext, InputFifo, TimeoutEvidence, View, startup_negative_control,
};
fn ctx() -> InputContext {
    InputContext {
        generation: 2,
        focus_generation: 7,
        epoch: 3,
        view: View::Native,
        focused: true,
    }
}
#[test]
fn bounded_fifo_counts_utf8_and_replays_once_after_commit_or_cancel() {
    for cancel in [false, true] {
        let mut q = InputFifo::new(8);
        let c = ctx();
        assert_eq!(q.accept(c, c, b"a", 99), Ok(1));
        assert_eq!(q.accept(c, c, "中".as_bytes(), 99), Ok(2));
        assert_eq!(q.bytes(), 4);
        assert_eq!(q.accept(c, c, b"12345", 99), Err("backpressure"));
        assert_eq!(q.bytes(), 4);
        let mut writes = Vec::new();
        q.replay(99, |b| {
            writes.push(b.to_vec());
            Ok(())
        })
        .unwrap();
        q.replay(99, |_| panic!("duplicate")).unwrap();
        assert_eq!(writes, vec![b"a".to_vec(), "中".as_bytes().to_vec()]);
        assert_eq!(q.bytes(), 0);
        let mut switch = native_host_spike::handoff::Switch::default();
        let stamp = switch.begin(View::Web, 0, 0);
        if cancel {
            switch.fail();
            assert_eq!(switch.active, View::Native);
        } else {
            assert!(switch.ack(stamp));
            assert_eq!(switch.active, View::Web);
        }
    }
}
#[test]
fn native_and_web_share_budget_stale_focus_page_epoch_rejected() {
    let mut q = InputFifo::new(4);
    for view in [View::Native, View::Web] {
        let c = InputContext { view, ..ctx() };
        assert!(q.accept(c, c, b"aa", 99).is_ok());
    }
    assert_eq!(q.bytes(), 4);
    let c = ctx();
    for bad in [
        InputContext { generation: 1, ..c },
        InputContext {
            focus_generation: 6,
            ..c
        },
        InputContext { epoch: 2, ..c },
        InputContext {
            view: View::Web,
            ..c
        },
        InputContext {
            focused: false,
            ..c
        },
    ] {
        assert_eq!(q.accept(bad, c, b"x", 99), Err("stale_or_inactive"));
    }
    assert_eq!(q.accept(c, c, b"x", 99), Err("backpressure"));
    assert!(q.replay(100, |_| panic!("different PTY")).is_err());
    assert_eq!(q.bytes(), 4);
}
#[test]
fn write_failure_keeps_unwritten_fifo_and_timeout_requires_complete_evidence() {
    let mut q = InputFifo::new(9);
    q.accept(ctx(), ctx(), b"ok", 99).unwrap();
    assert!(
        q.replay(99, |_| Err(std::io::Error::other("blocked")))
            .is_err()
    );
    assert_eq!(q.bytes(), 2);
    let mut e = TimeoutEvidence::default();
    assert!(!e.complete());
    e.event = true;
    e.old_view = true;
    e.other_marker = true;
    e.queue_replayed = true;
    assert!(!e.complete());
    e.cleanup = true;
    assert!(e.complete());
}
#[test]
fn startup_negative_control_reproduces_old_timeout_without_mutating_live_policy() {
    let e = startup_negative_control(2400, 1800);
    assert_eq!(e.legacy_early_frames, 1);
    assert_eq!(e.legacy_timeouts, 1);
    assert_eq!(e.ready_early_frames, 0);
    assert_eq!(e.ready_timeouts, 0);
    assert_eq!(e.retries, 0);
}
