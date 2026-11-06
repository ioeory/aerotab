use terminal_handoff_spike::{Broker, Consumer::*, Error, Event};

fn drain(b: &mut Broker, c: terminal_handoff_spike::Consumer) -> Vec<Event> {
    let mut out = vec![];
    while let Some(record) = b.next(c).unwrap() {
        out.push(record.event);
        b.ack(c, record.seq).unwrap();
    }
    out
}

#[test]
fn bounded_backpressure_retry_retains_order_and_eof() {
    let mut b = Broker::new(77, 256);
    let mut accepted = vec![];
    loop {
        let event = Event::Bytes(vec![accepted.len() as u8; 32]);
        match b.try_append(event.clone()) {
            Ok(_) => accepted.push(event),
            Err((Error::Backpressure, retry)) => {
                assert_eq!(retry, event);
                assert_eq!(drain(&mut b, Native), accepted);
                assert!(matches!(
                    b.try_append(event.clone()),
                    Err((Error::Backpressure, _))
                ));
                assert_eq!(drain(&mut b, ModeledXterm), accepted);
                assert_eq!(b.retained_bytes(), 0);
                b.try_append(retry.clone()).unwrap();
                b.try_append(Event::Resize { cols: 91, rows: 30 }).unwrap();
                b.try_append(Event::Eof).unwrap();
                let expected = vec![retry, Event::Resize { cols: 91, rows: 30 }, Event::Eof];
                assert_eq!(drain(&mut b, Native), expected);
                assert_eq!(drain(&mut b, ModeledXterm), expected);
                assert!(matches!(b.try_append(Event::Eof), Err((Error::Ended, _))));
                assert_eq!(b.session_id(), 77);
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(b.retained_bytes() <= 256);
    }
}

#[test]
fn barrier_requires_applied_and_presented_ack_and_rejects_stale_switches() {
    let mut b = Broker::new(99, 1024);
    b.try_append(Event::Bytes(b"one".to_vec())).unwrap();
    let stale = b.begin_switch(ModeledXterm).unwrap();
    let current = b.begin_switch(ModeledXterm).unwrap();
    assert_eq!(b.commit(stale), Err(Error::StaleSwitch));
    assert_eq!(b.commit(current), Err(Error::NotReady));
    assert_eq!(b.first_frame(current, 1), Err(Error::NotReady));
    drain(&mut b, ModeledXterm);
    assert_eq!(b.commit(current), Err(Error::NotReady));
    b.first_frame(current, 1).unwrap();
    // New bytes after the barrier must also be applied and presented at commit.
    b.try_append(Event::Bytes(b"two".to_vec())).unwrap();
    assert_eq!(b.commit(current), Err(Error::NotReady));
    assert_eq!(b.active(), Native);
    drain(&mut b, ModeledXterm);
    b.first_frame(current, 2).unwrap();
    b.commit(current).unwrap();
    assert_eq!(b.active(), ModeledXterm);
    assert_eq!(b.session_id(), 99);
    let back = b.begin_switch(Native).unwrap();
    b.cancel_switch(back).unwrap();
    assert_eq!(b.commit(back), Err(Error::StaleSwitch));
    assert_eq!(b.active(), ModeledXterm);
}

#[test]
fn failed_target_removal_unblocks_producer_without_changing_old_view() {
    let mut b = Broker::new(5, 100);
    let seq = b.try_append(Event::Bytes(vec![1; 32])).unwrap();
    let pending = b.begin_switch(ModeledXterm).unwrap();
    assert_eq!(b.ack(Native, seq), Err(Error::Undelivered));
    let record = b.next(Native).unwrap().unwrap();
    assert_eq!(b.next(Native), Err(Error::InFlight));
    b.ack(Native, record.seq).unwrap();
    assert!(matches!(
        b.try_append(Event::Eof),
        Err((Error::Backpressure, _))
    ));
    b.remove(ModeledXterm).unwrap();
    assert_eq!(b.commit(pending), Err(Error::StaleSwitch));
    assert_eq!(b.active(), Native);
    assert_eq!(b.remove(Native), Err(Error::ActiveConsumer));
    assert_eq!(b.begin_switch(ModeledXterm), Err(Error::Removed));
    assert_eq!(b.next(ModeledXterm), Err(Error::Removed));
    assert_eq!(b.retained_bytes(), 0);
    b.try_append(Event::Eof).unwrap();
    assert_eq!(drain(&mut b, Native), vec![Event::Eof]);
    assert!(matches!(
        b.try_append(Event::Bytes(vec![0; 101])),
        Err((Error::Ended, _))
    ));
}

#[test]
fn oversize_and_invalid_resize_never_enter_log() {
    let mut b = Broker::new(1, 100);
    assert!(matches!(
        b.try_append(Event::Bytes(vec![0; 101])),
        Err((Error::Oversize, _))
    ));
    assert!(matches!(
        b.try_append(Event::Resize { cols: 0, rows: 1 }),
        Err((Error::InvalidSize, _))
    ));
    assert_eq!(b.retained_bytes(), 0);
    assert_eq!(b.ack(Native, 800), Err(Error::Undelivered));
}

#[test]
fn fixed_protocol_authority_does_not_follow_visible_consumer() {
    let mut b = Broker::new(1, 1024);
    b.try_append(Event::Bytes(b"\x1b[6n".to_vec())).unwrap();
    drain(&mut b, Native);
    drain(&mut b, ModeledXterm);
    let token = b.begin_switch(ModeledXterm).unwrap();
    b.first_frame(token, 1).unwrap();
    b.commit(token).unwrap();
    assert_eq!(
        b.route_responses(ModeledXterm, 1, b"duplicate".to_vec()),
        Err(Error::NotAuthority)
    );
    assert_eq!(
        b.route_responses(Native, 1, b"\x1b[1;1R".to_vec()),
        Ok(b"\x1b[1;1R".to_vec())
    );
    assert_eq!(
        b.route_responses(Native, 1, vec![]),
        Err(Error::DuplicateResponse)
    );
}
