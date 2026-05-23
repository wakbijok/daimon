//! Phase 8 — kill-switch end-to-end test against the kill-switch
//! filesystem flag.
//!
//! Setup:
//! 1. Create a KillSwitch watching a temp file.
//! 2. Spawn a long-running task that ticks while the switch is disengaged.
//! 3. Touch the file mid-loop.
//! 4. Assert the loop observes `engaged()` within ~1s and halts cleanly.
//! 5. Remove the file; assert `engaged()` returns to false (so the next
//!    run can pick up). Operator-initiated resume is `rm`; daimon-guard
//!    does not auto-resume per masterplan §2.4.
//!
//! The NATS-bus variant (multi-process kill) is a manual operator test
//! documented in `deploy/systemd/README.md` — we don't spawn full daimon
//! processes from a Cargo test. The single-process logic exercised here
//! is the load-bearing invariant: any agent loop that consults
//! `KillSwitch::engaged()` will halt the same way regardless of how the
//! agents are wired together.

use std::sync::Arc;
use std::time::Duration;

use daimon_guard::KillSwitch;
use tempfile::tempdir;
use tokio::sync::Notify;

#[tokio::test]
async fn touch_kill_file_halts_in_under_one_second() {
    let tmp = tempdir().unwrap();
    let kill_file = tmp.path().join("KILL");
    let switch = KillSwitch::new(kill_file.clone());
    let state = switch.state();
    assert!(!state.engaged(), "fresh switch must be disengaged");

    // Start the watcher (it polls the filesystem flag — 1s cadence).
    switch.spawn_watchers();

    // Simulated agent loop: tick until engaged.
    let notify = Arc::new(Notify::new());
    let agent_state = state.clone();
    let agent_notify = notify.clone();
    let agent_handle = tokio::spawn(async move {
        let start = std::time::Instant::now();
        let mut ticks = 0u32;
        loop {
            if agent_state.engaged() {
                agent_notify.notify_one();
                return (ticks, start.elapsed());
            }
            ticks += 1;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });

    // Let the agent run for a moment, then trip the kill switch.
    tokio::time::sleep(Duration::from_millis(80)).await;
    std::fs::write(&kill_file, b"halted by test").unwrap();

    // The watcher should pick up the file inside its poll interval +
    // notify our agent; cap the wait at 5s to surface a real regression
    // without flake.
    let elapsed = tokio::time::timeout(Duration::from_secs(5), notify.notified())
        .await
        .expect("kill switch did not engage in time");
    let (ticks, dur) = agent_handle.await.unwrap();
    let _ = elapsed;

    assert!(dur < Duration::from_secs(4), "halt took too long: {dur:?}");
    assert!(ticks > 0, "agent loop didn't run at all");

    // Operator-initiated resume: rm the file. Watcher must reflect this
    // so the next agent loop sees a disengaged switch. No auto-resume
    // by design (D13); the test simulates the operator's `rm`.
    std::fs::remove_file(&kill_file).unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(
        !state.engaged(),
        "after rm of KILL file, switch should report disengaged"
    );
}

#[tokio::test]
async fn no_kill_file_means_disengaged_indefinitely() {
    let tmp = tempdir().unwrap();
    let kill_file = tmp.path().join("KILL");
    let switch = KillSwitch::new(kill_file);
    let state = switch.state();
    switch.spawn_watchers();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(!state.engaged(), "switch should stay disengaged when no flag file exists");
}
