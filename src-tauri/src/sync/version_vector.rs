//! Version vector for sync conflict resolution.
//!
//! Each replica is identified by a stable UUID (`device_id`). The vector maps
//! `device_id -> u64 counter`. On every local write the writer increments its
//! own counter and merges the prior vector. Conflict semantics follow
//! `docs/sync-protocol.md`:
//!
//! - `a` **dominates** `b` iff for every device `d`, `a[d] >= b[d]` and at
//!   least one entry is strictly greater.
//! - `a` and `b` are **concurrent** iff neither dominates the other.
//! - Merge takes the elementwise max.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionVector {
    /// BTreeMap keeps the serialised form deterministic.
    entries: BTreeMap<Uuid, u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ordering {
    /// `self == other`.
    Equal,
    /// `self` strictly dominates `other`.
    Dominates,
    /// `other` strictly dominates `self`.
    DominatedBy,
    /// Neither dominates: concurrent edits.
    Concurrent,
}

impl VersionVector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, device: &Uuid) -> u64 {
        self.entries.get(device).copied().unwrap_or(0)
    }

    /// Bumps the counter for `device` and returns the new value.
    pub fn bump(&mut self, device: Uuid) -> u64 {
        let entry = self.entries.entry(device).or_insert(0);
        *entry = entry.saturating_add(1);
        *entry
    }

    /// In-place elementwise max merge.
    pub fn merge(&mut self, other: &VersionVector) {
        for (&device, &counter) in &other.entries {
            let slot = self.entries.entry(device).or_insert(0);
            if counter > *slot {
                *slot = counter;
            }
        }
    }

    pub fn compare(&self, other: &VersionVector) -> Ordering {
        let mut self_greater = false;
        let mut other_greater = false;
        // Iterate union of keys.
        let mut keys: Vec<&Uuid> = self.entries.keys().chain(other.entries.keys()).collect();
        keys.sort();
        keys.dedup();
        for k in keys {
            let a = self.get(k);
            let b = other.get(k);
            if a > b {
                self_greater = true;
            } else if b > a {
                other_greater = true;
            }
            if self_greater && other_greater {
                return Ordering::Concurrent;
            }
        }
        match (self_greater, other_greater) {
            (false, false) => Ordering::Equal,
            (true, false) => Ordering::Dominates,
            (false, true) => Ordering::DominatedBy,
            (true, true) => Ordering::Concurrent,
        }
    }

    pub fn dominates(&self, other: &VersionVector) -> bool {
        matches!(self.compare(other), Ordering::Dominates)
    }

    pub fn concurrent_with(&self, other: &VersionVector) -> bool {
        matches!(self.compare(other), Ordering::Concurrent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(b: u8) -> Uuid {
        let mut bytes = [0u8; 16];
        bytes[0] = b;
        Uuid::from_bytes(bytes)
    }

    #[test]
    fn empty_vectors_equal() {
        let a = VersionVector::new();
        let b = VersionVector::new();
        assert_eq!(a.compare(&b), Ordering::Equal);
    }

    #[test]
    fn single_bump_dominates_empty() {
        let mut a = VersionVector::new();
        a.bump(uuid(1));
        let b = VersionVector::new();
        assert!(a.dominates(&b));
        assert_eq!(b.compare(&a), Ordering::DominatedBy);
    }

    #[test]
    fn linear_history_dominates() {
        let mut a = VersionVector::new();
        a.bump(uuid(1));
        let mut b = a.clone();
        b.bump(uuid(1));
        b.bump(uuid(2));
        assert!(b.dominates(&a));
    }

    #[test]
    fn divergent_history_is_concurrent() {
        let mut a = VersionVector::new();
        a.bump(uuid(1));
        let mut b = VersionVector::new();
        b.bump(uuid(2));
        assert!(a.concurrent_with(&b));
        assert!(b.concurrent_with(&a));
    }

    #[test]
    fn merge_produces_join() {
        let mut a = VersionVector::new();
        a.bump(uuid(1));
        let mut b = VersionVector::new();
        b.bump(uuid(2));
        let mut joined = a.clone();
        joined.merge(&b);
        assert!(joined.dominates(&a));
        assert!(joined.dominates(&b));
    }

    #[test]
    fn merge_takes_elementwise_max() {
        let mut a = VersionVector::new();
        a.bump(uuid(1));
        a.bump(uuid(1));
        a.bump(uuid(1));
        let mut b = VersionVector::new();
        b.bump(uuid(1));
        a.merge(&b);
        assert_eq!(a.get(&uuid(1)), 3);
    }

    #[test]
    fn serde_roundtrip() {
        let mut a = VersionVector::new();
        a.bump(uuid(1));
        a.bump(uuid(2));
        let json = serde_json::to_string(&a).unwrap();
        let back: VersionVector = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
    }
}
