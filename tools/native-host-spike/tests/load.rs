use native_host_spike::load::{payload, percentile, verify};
#[test]
fn deterministic_payload_and_loss_detection() {
    let p = payload(1, 2);
    assert!(p.len() > 256 * 1024);
    assert!(verify(&p, 1, 2));
    let mut truncated = p.clone();
    truncated.remove(300);
    assert!(!verify(&truncated, 1, 2));
    assert!(!verify(&p, 0, 2));
}
#[test]
fn percentiles_are_nearest_rank_not_cpu_time() {
    assert_eq!(percentile(&[], 95), None);
    assert_eq!(percentile(&[4, 1, 3, 2], 50), Some(2));
    assert_eq!(percentile(&(1..=100).collect::<Vec<_>>(), 99), Some(99));
}
