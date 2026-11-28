//! When a test that needs the SSH test server may be skipped. Shared by the `fernjo-ssh`
//! integration tests and the session tests of the app.

/// A missing server is fine on a developer machine and an error in CI, where it would let every
/// test pass by skipping. `no_server` is `FERNJO_TEST_NO_SSH_SERVER`, set by the Windows CI job,
/// which has no OpenSSH container. It is honoured on Windows only, so the flag can never turn a
/// Linux or macOS run green.
pub fn skip_allowed(windows: bool, ci: bool, no_server: bool) -> bool {
    !ci || (windows && no_server)
}
