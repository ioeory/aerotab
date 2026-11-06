//! Bounded, synchronous session broker experiment. The transport must retry rejected
//! events BEFORE reading more PTY bytes; resize and EOF use that same ordered lane.
//! Both parsers must exist from session creation, including while hidden. There is
//! intentionally no late subscription/reconstruction from a truncated byte ring.
//! `ModeledXterm` is a protocol role, NOT an xterm.js implementation or proof.
//! Only acknowledgements permit log reclamation. Consumer parser/scrollback and
//! caller-owned in-flight records have separate budgets; no renderer is included.
pub mod single_core;

use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Consumer {
    Native,
    ModeledXterm,
}
impl Consumer {
    fn index(self) -> usize {
        match self {
            Self::Native => 0,
            Self::ModeledXterm => 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Bytes(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Eof,
}
impl Event {
    fn cost(&self) -> usize {
        // Conservative per-entry accounting bounds count as well as payload.
        // Capacity matters: a tiny slice in an oversized allocation isn't tiny.
        64_usize.saturating_add(match self {
            Self::Bytes(bytes) => bytes.capacity(),
            _ => 0,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub seq: u64,
    pub event: Event,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Backpressure,
    Oversize,
    Ended,
    InvalidSize,
    Removed,
    ActiveConsumer,
    AuthorityConsumer,
    Undelivered,
    InFlight,
    StaleSwitch,
    NotReady,
    NotAuthority,
    DuplicateResponse,
    SequenceExhausted,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Switch {
    generation: u64,
    target: Consumer,
    barrier: u64,
}

#[derive(Default)]
struct Subscriber {
    ack: u64,
    delivered: Option<u64>,
    removed: bool,
    pending_utf8: u8,
}

pub struct Broker {
    session: u64,
    budget: usize,
    retained: usize,
    log: VecDeque<Record>,
    subscribers: [Subscriber; 2],
    tail: u64,
    ended: bool,
    active: Consumer,
    generation: u64,
    pending: Option<Switch>,
    presented: Option<u64>,
    response_ack: u64,
}

impl Broker {
    pub fn new(session: u64, budget: usize) -> Self {
        assert!(budget >= 64, "budget must fit at least a control event");
        Self {
            session,
            budget,
            retained: 0,
            log: VecDeque::new(),
            subscribers: Default::default(),
            tail: 0,
            ended: false,
            active: Consumer::Native,
            generation: 0,
            pending: None,
            presented: None,
            response_ack: 0,
        }
    }

    pub fn session_id(&self) -> u64 {
        self.session
    }
    pub fn active(&self) -> Consumer {
        self.active
    }
    pub fn retained_bytes(&self) -> usize {
        self.retained
    }

    /// Returns ownership on rejection so the single producer can retry exactly.
    /// Backpressure is not permission to read ahead or drop the oldest record.
    pub fn try_append(&mut self, event: Event) -> Result<u64, (Error, Event)> {
        let cost = event.cost();
        let error = if self.ended {
            Some(Error::Ended)
        } else if matches!(
            event,
            Event::Resize { cols: 0, .. } | Event::Resize { rows: 0, .. }
        ) {
            Some(Error::InvalidSize)
        } else if cost > self.budget {
            Some(Error::Oversize)
        } else if cost > self.budget - self.retained {
            Some(Error::Backpressure)
        } else if self.tail == u64::MAX {
            Some(Error::SequenceExhausted)
        } else {
            None
        };
        if let Some(error) = error {
            return Err((error, event));
        }
        self.tail += 1;
        self.ended = matches!(event, Event::Eof);
        self.retained += cost;
        self.log.push_back(Record {
            seq: self.tail,
            event,
        });
        Ok(self.tail)
    }

    /// One record per consumer may be in flight. ACK means parser application,
    /// not just receipt by an IPC channel. No ACK on a failed parser application.
    pub fn next(&mut self, consumer: Consumer) -> Result<Option<Record>, Error> {
        let subscriber = &mut self.subscribers[consumer.index()];
        if subscriber.removed {
            return Err(Error::Removed);
        }
        if subscriber.delivered.is_some() {
            return Err(Error::InFlight);
        }
        let record = self.log.iter().find(|r| r.seq > subscriber.ack).cloned();
        subscriber.delivered = record.as_ref().map(|r| r.seq);
        Ok(record)
    }

    /// Call only when delivery was cancelled BEFORE parser application. Once a
    /// parser has applied bytes, it must ACK rather than replay them a second time.
    pub fn cancel_delivery(&mut self, consumer: Consumer, seq: u64) -> Result<(), Error> {
        let subscriber = &mut self.subscribers[consumer.index()];
        if subscriber.removed {
            return Err(Error::Removed);
        }
        if subscriber.delivered != Some(seq) {
            return Err(Error::Undelivered);
        }
        subscriber.delivered = None;
        Ok(())
    }

    pub fn ack(&mut self, consumer: Consumer, seq: u64) -> Result<(), Error> {
        self.ack_decoded(consumer, seq, 0)
    }

    /// Consumption ACK permits reclaiming bytes held in the bounded decoder.
    /// pending_utf8 != 0 forbids presentation/switch commit: not fully parsed yet.
    pub fn ack_decoded(
        &mut self,
        consumer: Consumer,
        seq: u64,
        pending_utf8: u8,
    ) -> Result<(), Error> {
        if pending_utf8 > 3 {
            return Err(Error::NotReady);
        }
        let subscriber = &mut self.subscribers[consumer.index()];
        if subscriber.removed {
            return Err(Error::Removed);
        }
        if subscriber.delivered != Some(seq) {
            return Err(Error::Undelivered);
        }
        subscriber.ack = seq;
        subscriber.pending_utf8 = pending_utf8;
        subscriber.delivered = None;
        self.collect();
        Ok(())
    }

    fn collect(&mut self) {
        let floor = self
            .subscribers
            .iter()
            .filter(|s| !s.removed)
            .map(|s| s.ack)
            .min()
            .unwrap_or(0);
        while self.log.front().is_some_and(|r| r.seq <= floor) {
            let record = self.log.pop_front().unwrap();
            self.retained -= record.event.cost();
        }
        // VecDeque can retain its high-water allocation, bounded by budget / 64
        // entries. Shrink when empty to avoid retaining that allocation forever.
        if self.log.is_empty() {
            self.log.shrink_to_fit();
        }
    }

    /// Removal is permanent. Without a full parser-state checkpoint there is no
    /// safe rejoin after old records are collected. The old visible view stays.
    pub fn remove(&mut self, consumer: Consumer) -> Result<(), Error> {
        if self.active == consumer {
            return Err(Error::ActiveConsumer);
        }
        if consumer == Consumer::Native {
            return Err(Error::AuthorityConsumer);
        }
        self.subscribers[consumer.index()].removed = true;
        self.subscribers[consumer.index()].delivered = None;
        if self.pending.is_some_and(|p| p.target == consumer) {
            self.pending = None;
            self.presented = None;
        }
        self.collect();
        Ok(())
    }

    pub fn begin_switch(&mut self, target: Consumer) -> Result<Switch, Error> {
        if self.subscribers[target.index()].removed {
            return Err(Error::Removed);
        }
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(Error::SequenceExhausted)?;
        let switch = Switch {
            generation: self.generation,
            target,
            barrier: self.tail,
        };
        self.pending = Some(switch);
        self.presented = None;
        Ok(switch)
    }

    fn validate(&self, switch: Switch) -> Result<(), Error> {
        if self.pending == Some(switch) {
            Ok(())
        } else {
            Err(Error::StaleSwitch)
        }
    }

    /// The real host must call this only after presentation of a frame carrying
    /// this exact applied sequence. This prototype cannot attest GPU presentation.
    pub fn first_frame(&mut self, switch: Switch, applied_seq: u64) -> Result<(), Error> {
        self.validate(switch)?;
        if self.subscribers[switch.target.index()].pending_utf8 != 0
            || applied_seq < switch.barrier
            || applied_seq > self.subscribers[switch.target.index()].ack
        {
            return Err(Error::NotReady);
        }
        self.presented = Some(applied_seq);
        Ok(())
    }

    /// Atomic visibility commit at the CURRENT tail, not a stale initial barrier.
    /// A busy source can delay commit; the adapter may pause ingress briefly.
    pub fn commit(&mut self, switch: Switch) -> Result<(), Error> {
        self.validate(switch)?;
        if self.subscribers[switch.target.index()].pending_utf8 != 0
            || self.subscribers[switch.target.index()].ack != self.tail
            || self.presented != Some(self.tail)
        {
            return Err(Error::NotReady);
        }
        self.active = switch.target;
        self.pending = None;
        self.presented = None;
        Ok(())
    }

    pub fn cancel_switch(&mut self, switch: Switch) -> Result<(), Error> {
        self.validate(switch)?;
        self.pending = None;
        self.presented = None;
        Ok(())
    }

    /// Fixed native parser authority, independent of view. Secondary consumers
    /// must suppress ALL device/clipboard/title side effects at their adapter.
    /// Bundle every response from one applied event into one call, including empty
    /// batches. Return value is forwarded once by the session transport; retries
    /// across transport failure/crash are not an exactly-once I/O guarantee.
    pub fn route_responses(
        &mut self,
        source: Consumer,
        seq: u64,
        bytes: Vec<u8>,
    ) -> Result<Vec<u8>, Error> {
        if source != Consumer::Native {
            return Err(Error::NotAuthority);
        }
        if seq == 0 || seq > self.subscribers[0].ack {
            return Err(Error::Undelivered);
        }
        if seq <= self.response_ack {
            return Err(Error::DuplicateResponse);
        }
        if self.response_ack.checked_add(1) != Some(seq) {
            return Err(Error::Undelivered);
        }
        self.response_ack = seq;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unacknowledged_output_is_retained_not_silently_discarded() {
        let mut broker = Broker::new(42, 128);
        assert_eq!(broker.try_append(Event::Bytes(b"hello".to_vec())), Ok(1));
        assert!(
            broker.retained_bytes() >= 5,
            "both persistent consumers must retain unacknowledged bytes"
        );
    }
}
