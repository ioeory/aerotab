use terminal_handoff_spike::{Broker, Consumer::*, Error, Event};

#[test]
fn cancelled_delivery_can_retry_same_record_and_stale_ack_cannot_skip() {
    let mut b = Broker::new(6, 1024);
    b.try_append(Event::Bytes(b"first".to_vec())).unwrap();
    b.try_append(Event::Eof).unwrap();
    let first = b.next(Native).unwrap().unwrap();
    assert_eq!(b.ack(Native, 2), Err(Error::Undelivered));
    b.cancel_delivery(Native, first.seq).unwrap();
    assert_eq!(b.next(Native).unwrap().unwrap(), first);
    b.ack(Native, first.seq).unwrap();
    assert_eq!(b.ack(Native, first.seq), Err(Error::Undelivered));
    assert_eq!(b.next(Native).unwrap().unwrap().event, Event::Eof);
    assert!(b.retained_bytes() > 0);
}

#[test]
fn cannot_remove_fixed_authority_even_when_it_is_hidden() {
    let mut b = Broker::new(6, 1024);
    let switch = b.begin_switch(ModeledXterm).unwrap();
    b.first_frame(switch, 0).unwrap();
    b.commit(switch).unwrap();
    assert_eq!(b.remove(Native), Err(Error::AuthorityConsumer));
    assert_eq!(b.active(), ModeledXterm);
}

#[test]
fn response_batches_cannot_skip_an_earlier_applied_event() {
    let mut b = Broker::new(6, 1024);
    b.try_append(Event::Bytes(b"query".to_vec())).unwrap();
    b.try_append(Event::Bytes(b"query2".to_vec())).unwrap();
    for _ in 0..2 {
        let record = b.next(Native).unwrap().unwrap();
        b.ack(Native, record.seq).unwrap();
    }
    assert_eq!(
        b.route_responses(Native, 2, vec![]),
        Err(Error::Undelivered)
    );
    assert_eq!(b.route_responses(Native, 1, vec![]), Ok(vec![]));
    assert_eq!(b.route_responses(Native, 2, vec![]), Ok(vec![]));
}
