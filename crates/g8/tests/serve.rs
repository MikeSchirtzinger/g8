//! Integration tests for `g8 serve` feature-gating + hardening.
//!
//! Split from `cli.rs` because almost everything here only makes sense (or
//! only compiles) under `--features serve` — see crates/g8/Cargo.toml
//! and crates/g8/src/serve/mod.rs for the design this exercises.
//!
//! # Coverage
//!
//! - Default build: `--help` never mentions `serve` (feature is off by
//!   default, so the subcommand doesn't exist).
//! - Feature build: `--help` DOES mention `serve`.
//! - Feature build: the host guard rejects a non-loopback `--host` without
//!   `--allow-external-unsafe` — fast, since the process `bail!`s before ever
//!   binding a socket.
//! - Feature build: a live server on loopback rejects a `POST` without a
//!   token, rejects one with the wrong token, and accepts one with the
//!   correct token — a genuine end-to-end HTTP round-trip over a raw socket
//!   against a real `g8 serve` subprocess, not a mock.
//!
//! # What this does NOT cover (honesty per the task brief)
//!
//! - CORS header scoping end-to-end (would need to send a real `Origin`
//!   header and inspect `Access-Control-Allow-Origin` on the response; the
//!   *decision logic* — `origin_of`, `cors_scoped`'s match arm — is covered
//!   by unit tests in `serve/mod.rs` instead, since it's pure).
//! - The `/events` SSE stream.
//! - The `?token=` query-param path for auth (header path is covered here
//!   end-to-end; the query-param path is covered by a unit test on
//!   `token_authorized` in `serve/mod.rs`, since both paths share the same
//!   pure check function and only one needed a live-socket proof).
//! - Actually binding a non-loopback host: deliberately never exercised here,
//!   even with `--allow-external-unsafe` — binding `0.0.0.0` can trigger an
//!   OS firewall permission prompt on some machines, which would hang a test
//!   run. The "flag permits it" half of the guard is covered by the
//!   `is_loopback_host` unit test instead.

use std::process::Command;

use assert_cmd::prelude::*;
use predicates::prelude::*;

fn g8() -> Command {
    Command::cargo_bin("g8").expect("g8 binary not found")
}

// ── Default build: serve is invisible ───────────────────────────────────────

#[cfg(not(feature = "serve"))]
#[test]
fn help_has_no_serve_in_default_build() {
    g8().arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("serve").not());
}

// ── Feature build ────────────────────────────────────────────────────────────
// Scoped in one module (rather than per-item #[cfg]) so every helper below —
// process spawning, raw sockets — simply doesn't exist in the default build.
// That's what keeps `cargo clippy --workspace --all-targets -- -D warnings`
// clean without a default build: it'd otherwise flag them as dead code.
#[cfg(feature = "serve")]
mod feature_gated {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::process::{Child, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;
    use tempfile::TempDir;

    fn init_temp_project() -> (TempDir, PathBuf) {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().to_path_buf();
        g8().current_dir(&path)
            .args(["init", "--no-claude-import", "--no-subagents"])
            .assert()
            .success();
        (dir, path)
    }

    #[test]
    fn help_lists_serve_when_feature_enabled() {
        g8().arg("--help")
            .assert()
            .success()
            .stdout(predicate::str::contains("serve"));
    }

    #[test]
    fn host_guard_rejects_nonloopback_without_flag() {
        let (_dir, path) = init_temp_project();
        // Never actually binds: the guard bail!s first, so this exits fast
        // and doesn't touch the network (see module docs on why we avoid
        // ever binding 0.0.0.0 in a test, even with the override flag).
        g8().current_dir(&path)
            .args(["serve", "--host", "0.0.0.0", "--no-open"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("allow-external-unsafe"));
    }

    /// Kills the child on drop so a failing assertion mid-test (which unwinds
    /// past the normal end of the function) can never leak a live `g8 serve`
    /// process holding the port open.
    struct KillOnDrop(Child);

    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// An OS-assigned free loopback port. Small TOCTOU race (something else
    /// could grab it before `g8 serve` binds) — acceptable for a local test.
    fn free_port() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        l.local_addr().unwrap().port()
    }

    /// Read child stderr lines until the startup banner's token line (`  token
    /// <value>   (...)`) shows up, or time out.
    fn wait_for_token(stderr: std::process::ChildStderr, timeout: Duration) -> String {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                let mut parts = line.split_whitespace();
                if parts.next() == Some("token") {
                    if let Some(tok) = parts.next() {
                        let _ = tx.send(tok.to_string());
                        return;
                    }
                }
            }
        });
        rx.recv_timeout(timeout)
            .expect("g8 serve never printed its token to stderr within timeout")
    }

    /// Minimal raw-socket HTTP/1.1 POST — returns the response status code.
    /// No HTTP client dependency needed for one request/response pair.
    fn http_post(port: u16, path: &str, token: Option<&str>, body: &str) -> u16 {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        let mut req = format!(
            "POST {path} HTTP/1.1\r\n\
             Host: 127.0.0.1:{port}\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n",
            body.len()
        );
        if let Some(t) = token {
            req.push_str(&format!("X-G8-Token: {t}\r\n"));
        }
        req.push_str("\r\n");
        req.push_str(body);

        stream.write_all(req.as_bytes()).expect("write request");
        stream.shutdown(std::net::Shutdown::Write).ok();

        let mut resp = Vec::new();
        stream.read_to_end(&mut resp).expect("read response");
        let text = String::from_utf8_lossy(&resp);
        text.lines()
            .next()
            .and_then(|status_line| status_line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .unwrap_or(0)
    }

    #[test]
    fn post_routes_require_the_printed_token() {
        let (_dir, path) = init_temp_project();
        let port = free_port();

        let mut child = g8()
            .current_dir(&path)
            .args(["serve", "--port", &port.to_string(), "--no-open"])
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn g8 serve");
        let stderr = child.stderr.take().expect("piped stderr");
        let _guard = KillOnDrop(child); // keeps the child alive + reaped

        let token = wait_for_token(stderr, Duration::from_secs(5));

        let status = http_post(port, "/api/plan", None, r#"{"title":"t"}"#);
        assert_eq!(status, 403, "missing token must be rejected");

        let status = http_post(port, "/api/plan", Some("not-the-token"), r#"{"title":"t"}"#);
        assert_eq!(status, 403, "wrong token must be rejected");

        let status = http_post(port, "/api/plan", Some(&token), r#"{"title":"t"}"#);
        assert_eq!(
            status, 200,
            "correct token must let the request through to the handler"
        );
    }
}
