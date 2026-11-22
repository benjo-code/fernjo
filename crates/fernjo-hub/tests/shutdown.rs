//! SIGTERM and SIGINT resolve `shutdown_signal` (Unix only: it sends real signals to this
//! process, so it lives in its own test binary with a single test).
#![cfg(unix)]

use std::time::Duration;

use fernjo_hub::shutdown_signal;

fn send(signal: &str) {
    let status = std::process::Command::new("kill")
        .args([&format!("-{signal}"), &std::process::id().to_string()])
        .status()
        .expect("kill is available");
    assert!(status.success());
}

#[tokio::test(flavor = "current_thread")]
async fn sigterm_and_sigint_end_the_wait() {
    for signal in ["TERM", "INT"] {
        let waiting = tokio::spawn(shutdown_signal());
        // give the task time to register its handlers before the signal arrives
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!waiting.is_finished(), "must not resolve without a signal");
        send(signal);
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .unwrap_or_else(|_| panic!("SIG{signal} did not end the wait"))
            .unwrap();
    }
}
