use native_host_spike::handoff::{Stamp, Switch, View};
#[test]
fn only_exact_ack_commits_and_failure_retains_old() {
    let mut s = Switch::default();
    let a = s.begin(View::Web, 4, 8);
    assert_eq!(s.active, View::Native);
    assert!(!s.ack(Stamp { revision: 5, ..a }));
    assert!(!s.ack(Stamp { epoch: 0, ..a }));
    assert!(s.ack(a));
    assert_eq!(s.active, View::Web);
    let b = s.begin(View::Native, 5, 9);
    s.fail();
    assert!(!s.ack(b));
    assert_eq!(s.active, View::Web);
}
#[test]
fn superseded_epoch_never_commits() {
    let mut s = Switch::default();
    let a = s.begin(View::Web, 1, 2);
    let b = s.begin(View::Native, 1, 2);
    assert!(!s.ack(a));
    assert!(s.ack(b));
}
