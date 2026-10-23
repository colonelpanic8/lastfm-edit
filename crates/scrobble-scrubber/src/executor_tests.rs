use super::PassRateLimit;

#[test]
fn budget_pause_counts_decisions_against_the_cap() {
    let mut rl = PassRateLimit::default();
    assert!(rl.budget_pause(3));
    assert!(rl.budget_pause(3));
    assert!(rl.budget_pause(3));
    // Fourth decision exceeds the budget of 3.
    assert!(!rl.budget_pause(3));
}

#[test]
fn consecutive_pauses_grow_and_cap() {
    let mut rl = PassRateLimit::default();
    // First pause is at least the base minute; then doubles per consecutive pause.
    assert_eq!(rl.next_pause_secs(0), 60);
    assert_eq!(rl.next_pause_secs(0), 120);
    assert_eq!(rl.next_pause_secs(0), 240);
    assert_eq!(rl.next_pause_secs(0), 480);
    // Capped at 600s no matter how many consecutive pauses.
    assert_eq!(rl.next_pause_secs(0), 600);
    assert_eq!(rl.next_pause_secs(0), 600);
}

#[test]
fn a_longer_retry_after_wins_over_the_grown_floor() {
    let mut rl = PassRateLimit::default();
    // retry_after above the current floor is honored, still capped at 600.
    assert_eq!(rl.next_pause_secs(200), 200);
    // Next consecutive floor is 120, but retry_after of 1000 clamps to the 600 cap.
    assert_eq!(rl.next_pause_secs(1000), 600);
}

#[test]
fn pause_secs_is_non_mutating() {
    let mut rl = PassRateLimit::default();
    // Reading the cadence repeatedly must not advance the growth counter.
    assert_eq!(rl.pause_secs(0), 60);
    assert_eq!(rl.pause_secs(0), 60);
    assert_eq!(rl.pause_secs(0), 60);
    // Only next_pause_secs advances it.
    assert_eq!(rl.next_pause_secs(0), 60);
    assert_eq!(rl.pause_secs(0), 120);
}

#[test]
fn success_resets_the_backoff() {
    let mut rl = PassRateLimit::default();
    assert_eq!(rl.next_pause_secs(0), 60);
    assert_eq!(rl.next_pause_secs(0), 120);
    rl.succeeded();
    // Growth is back to the base; the budget count is untouched.
    assert_eq!(rl.next_pause_secs(0), 60);
}

#[test]
fn configured_pauses_start_at_the_base_and_stop_at_the_cap() {
    let mut rl = PassRateLimit::new(10, 30);
    assert_eq!(rl.next_pause_secs(0), 10);
    assert_eq!(rl.next_pause_secs(0), 20);
    assert_eq!(rl.next_pause_secs(0), 30);
    assert_eq!(rl.next_pause_secs(0), 30);
}
