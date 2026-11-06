use terminal_handoff_spike::{Broker, Consumer::*, Error, Event};
#[test]
fn decoder_consumption_ack_reclaims_log_but_cannot_present_or_commit_partial_scalar() {
    let mut broker = Broker::new(1, 128);
    broker.try_append(Event::Bytes(vec![0xe4])).unwrap();
    for consumer in [Native, ModeledXterm] {
        let record = broker.next(consumer).unwrap().unwrap();
        broker.ack_decoded(consumer, record.seq, 1).unwrap();
    }
    assert_eq!(broker.retained_bytes(), 0);
    let switch = broker.begin_switch(ModeledXterm).unwrap();
    assert_eq!(broker.first_frame(switch, 1), Err(Error::NotReady));
    assert_eq!(broker.commit(switch), Err(Error::NotReady));
    broker.try_append(Event::Bytes(vec![0xb8, 0xad])).unwrap();
    for consumer in [Native, ModeledXterm] {
        let record = broker.next(consumer).unwrap().unwrap();
        broker.ack_decoded(consumer, record.seq, 0).unwrap();
    }
    broker.first_frame(switch, 2).unwrap();
    broker.commit(switch).unwrap();
}
