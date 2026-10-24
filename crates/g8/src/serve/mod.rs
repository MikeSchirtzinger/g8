//! `g8 serve` — a live, local web dashboard for the convergence space.
//!
//! A dependency-light HTTP server (`tiny_http`, synchronous, no async runtime)
//! that serves:
//!   - `GET /`            embedded single-page dashboard
//!   - `GET /api/health`  crate version
//!   - `GET /api/graph`   the convergence graph (nodes + edges + stats)
//!   - `GET /events`      Server-Sent Events; pushes on any `.g8/store.db` change
//!   - `GET /api/version` monotonic change counter (SSE poll fallback)
//!   - `POST /api/plan`               create a plan (runs the planner fit-check)
//!   - `POST /api/plan/<id>/<action>` lifecycle transition
//!   - `POST /api/substrate`          register/update a WIP budget
//!
//! Live shared state, not async clipboard: a `notify` watcher on the store file
//! bumps a version counter, the SSE stream wakes every connected client, and
//! the UI's controls *do* the action (POST → store mutation → watcher → SSE →
//! refetch). An external `g8` CLI edit to the same store reflects in the
//! dashboard within a frame, and vice-versa.
//!
//! ## Hardening (this module only ships behind the `serve` feature — see
//! `crates/g8/Cargo.toml`)
//!
//! `g8 serve` has no TLS and no real auth, so three compensating controls
//! keep it safe for its actual use case (a local dev-loop dashboard):
//!   1. **Host guard** (`is_loopback_host`): refuses to bind anything but
//!      loopback unless `--allow-external-unsafe` is passed.
//!   2. **Ephemeral token**: generated at startup, printed to stderr, and
//!      required (header `X-G8-Token` or `?token=`) on every mutating
//!      (`POST`) route. GET routes are unaffected.
//!   3. **No CORS.** No route sends an `Access-Control-*` header, so a
//!      browser page on any other origin cannot read the dashboard's data
//!      or send it a mutating request.

mod actions;
mod graph;

use std::io::{Cursor, Write};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::cli::ServeArgs;
use crate::ctx::Ctx;

const INDEX_HTML: &str = include_str!("index.html");

/// Placeholder substituted with the real per-run token when `GET /` is served
/// (see [`respond_html`]). Keeps the token out of the compiled-in HTML asset
/// and out of shell/browser history — the page picks it up from its own body,
/// not a URL.
const TOKEN_PLACEHOLDER: &str = "__GOVERN_TOKEN__";

/// Shared server state. The version counter is bumped by the file watcher and
/// awaited by SSE handlers via the condvar.
struct State {
    store_path: PathBuf,
    /// Ephemeral per-run token gating all `POST` routes.
    token: String,
    version: Mutex<u64>,
    cond: Condvar,
}

impl State {
    fn bump(&self) {
        {
            let mut v = self.version.lock().unwrap();
            *v += 1;
        }
        self.cond.notify_all();
    }
    fn current(&self) -> u64 {
        *self.version.lock().unwrap()
    }
}

pub fn run(ctx: &Ctx, args: &ServeArgs) -> Result<i32> {
    if !ctx.store_path.exists() {
        bail!(
            "no store at {}: run `g8 init` (and `g8 scan`) first",
            ctx.store_path.display()
        );
    }

    if !args.allow_external_unsafe && !is_loopback_host(&args.host) {
        bail!(
            "refusing to bind non-loopback host '{host}': g8 serve protects \
             its mutating routes with an ephemeral token printed to this \
             terminal, not TLS or real auth, so exposing it beyond localhost \
             hands anyone who can reach the port a shot at that token. Pass \
             --allow-external-unsafe if you understand and accept that risk \
             (e.g. a trusted LAN, or you're fronting it with your own \
             auth/TLS proxy).",
            host = args.host
        );
    }

    let token = generate_token();

    let state = Arc::new(State {
        store_path: ctx.store_path.clone(),
        token: token.clone(),
        version: Mutex::new(0),
        cond: Condvar::new(),
    });

    // ── File watcher: bump the version on any change to the store dir ────────
    // Watch the containing `.g8/` directory (not just the file) so SQLite's
    // -wal/-shm sidecars and atomic-rename writes are all caught.
    let watch_dir = ctx
        .store_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let _watcher = spawn_watcher(watch_dir, state.clone());

    // ── HTTP server ──────────────────────────────────────────────────────────
    let addr = format!("{}:{}", args.host, args.port);
    let server = Server::http(&addr)
        .map_err(|e| anyhow::anyhow!("binding {addr}: {e}"))
        .with_context(|| format!("could not start dashboard on {addr}"))?;
    let server = Arc::new(server);

    let url = format!("http://{}:{}", display_host(&args.host), args.port);
    eprintln!("g8 dashboard live at {url}");
    eprintln!("  graph    {url}/api/graph");
    eprintln!("  token    {token}   (required on POST routes: header `X-G8-Token` or `?token=`)");
    eprintln!("  Ctrl-C to stop.");

    // The token travels to the browser inside the served HTML (see
    // respond_html), never in this URL — no need to smuggle it through
    // shell/browser history for `--no-open` to still work by hand.
    if !args.no_open {
        open_browser(&url);
    }

    loop {
        match server.recv() {
            Ok(request) => {
                let st = state.clone();
                thread::spawn(move || {
                    if let Err(e) = handle(request, st) {
                        tracing::debug!("request handler error: {e:#}");
                    }
                });
            }
            Err(e) => {
                tracing::warn!("server recv error: {e}");
            }
        }
    }
}

// ── Routing ─────────────────────────────────────────────────────────────────

fn handle(mut request: Request, state: Arc<State>) -> Result<()> {
    let method = request.method().clone();
    let raw_url = request.url().to_string();
    let (path, query) = split_url(&raw_url);

    // CORS preflight: a bare 204 with no Access-Control-* header, which makes
    // browsers refuse to send the real cross-origin request. Non-browser
    // callers (curl, the g8 CLI, other agents) don't consult CORS at all —
    // the token is the real gate on mutations, this just stops browser-JS
    // misuse.
    if method == Method::Options {
        return request.respond(Response::empty(204)).map_err(Into::into);
    }

    match (&method, path.as_str()) {
        (Method::Get, "/") => respond_html(request, INDEX_HTML, &state.token),

        (Method::Get, "/api/health") => {
            let body = serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
            });
            respond_json(request, 200, &body)
        }

        (Method::Get, "/api/version") => {
            let body = serde_json::json!({ "version": state.current() });
            respond_json(request, 200, &body)
        }

        (Method::Get, "/api/graph") => match build_graph_json(&state) {
            Ok(v) => respond_json(request, 200, &v),
            Err(e) => respond_error(request, 500, e),
        },

        (Method::Get, "/events") => serve_sse(request, state),

        (Method::Post, "/api/plan") => {
            if let Some(resp) = check_token(&request, &query, &state) {
                return request.respond(resp).map_err(Into::into);
            }
            let body = read_body(&mut request);
            let result = serde_json::from_str(&body)
                .context("parsing request body")
                .and_then(|req| actions::create_plan(&state.store_path, req));
            respond_result(request, result)
        }

        (Method::Post, "/api/substrate") => {
            if let Some(resp) = check_token(&request, &query, &state) {
                return request.respond(resp).map_err(Into::into);
            }
            let body = read_body(&mut request);
            let result = serde_json::from_str(&body)
                .context("parsing request body")
                .and_then(|req| actions::add_substrate(&state.store_path, req));
            respond_result(request, result)
        }

        (Method::Post, p) if p.starts_with("/api/plan/") => {
            if let Some(resp) = check_token(&request, &query, &state) {
                return request.respond(resp).map_err(Into::into);
            }
            let segs: Vec<&str> = p.trim_start_matches('/').split('/').collect();
            // ["api","plan",<id>,<action>]
            if segs.len() != 4 {
                return respond_error(
                    request,
                    400,
                    anyhow::anyhow!("expected /api/plan/<id>/<action>"),
                );
            }
            let (id, action) = (segs[2].to_string(), segs[3].to_string());
            let body = read_body(&mut request);
            let req: actions::ActionReq = serde_json::from_str(&body).unwrap_or_default();
            let result = actions::plan_action(&state.store_path, &id, &action, req);
            respond_result(request, result)
        }

        _ => respond_error(request, 404, anyhow::anyhow!("not found: {path}")),
    }
}

fn build_graph_json(state: &State) -> Result<serde_json::Value> {
    let g = graph::Graph::build(&state.store_path, g8_core::now_millis().as_i64())?;
    Ok(serde_json::to_value(&g)?)
}

// ── Server-Sent Events ────────────────────────────────────────────────────────

fn serve_sse(request: Request, state: Arc<State>) -> Result<()> {
    // Raw socket access (tiny_http `into_writer` is CGI-style — we frame the
    // whole HTTP response ourselves). On any write error the client has gone,
    // so we drop the writer and the thread exits.
    let mut w = request.into_writer();
    write!(
        w,
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/event-stream\r\n\
         Cache-Control: no-cache\r\n\
         Connection: close\r\n\
         \r\n"
    )?;
    let mut last = state.current();
    write!(w, "event: hello\ndata: {{\"v\":{last}}}\n\n")?;
    w.flush()?;

    loop {
        // Wait up to 15s for a version bump; on timeout send a keep-alive comment.
        let newv = {
            let guard = state.version.lock().unwrap();
            let (guard, res) = state
                .cond
                .wait_timeout(guard, Duration::from_secs(15))
                .unwrap();
            let v = *guard;
            drop(guard);
            let _ = res;
            v
        };
        let write_res = if newv != last {
            last = newv;
            write!(w, "event: update\ndata: {{\"v\":{last}}}\n\n").and_then(|_| w.flush())
        } else {
            write!(w, ": ping\n\n").and_then(|_| w.flush())
        };
        if write_res.is_err() {
            break; // client disconnected
        }
    }
    Ok(())
}

// ── File watcher ──────────────────────────────────────────────────────────────

fn spawn_watcher(dir: PathBuf, state: Arc<State>) -> Option<notify::RecommendedWatcher> {
    use notify::{Event, RecursiveMode, Watcher};

    let st = state.clone();
    let mut watcher = match notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(event) = res {
            use notify::EventKind;
            if matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
            ) {
                st.bump();
            }
        }
    }) {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!("file watcher unavailable ({e}); live updates disabled");
            return None;
        }
    };
    if let Err(e) = watcher.watch(&dir, RecursiveMode::NonRecursive) {
        tracing::warn!(
            "watching {} failed ({e}); live updates disabled",
            dir.display()
        );
        return None;
    }
    Some(watcher)
}

// ── Security: host guard, token auth ─────────────────────────────────────────

/// True if `host` is a loopback interface — `127.0.0.1`/`127.x.x.x`, `::1`, or
/// the literal `localhost` alias. Anything else (`0.0.0.0`, a LAN/public IP, a
/// real hostname) needs `--allow-external-unsafe`.
///
/// Pure string/IP parsing, no DNS resolution: it can't block on a slow
/// resolver and can't be fooled by DNS rebinding after the check runs (the
/// exact string `tiny_http::Server::http` will itself bind against is what we
/// inspect here).
fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

/// Generate the ephemeral per-run token gating all `POST` routes.
///
/// `serve` is `g8` — outside the deterministic zone (SPEC.md D11) — so
/// real randomness is fine here. `nanoid` costs nothing extra in the
/// dependency tree: it's already resolved transitively via g8-core's ID
/// generation (`g8_core::ids`), this just uses it directly for a longer,
/// higher-entropy token appropriate for something guarding write access.
fn generate_token() -> String {
    nanoid::nanoid!(32)
}

/// Constant-time string comparison — the token check shouldn't leak how many
/// leading bytes of a guess were correct via response-time side channels.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// True if `expected` matches the token supplied via the `X-G8-Token` header
/// or a `?token=` query parameter. Pure over already-extracted strings (no
/// request/socket access), so it's unit-testable without a live server.
fn token_authorized(expected: &str, header_token: Option<&str>, query: &str) -> bool {
    header_token.is_some_and(|h| constant_time_eq(h, expected))
        || query_value(query, "token").is_some_and(|q| constant_time_eq(q, expected))
}

/// Enforce the token on a mutating route. `None` means proceed; `Some(resp)`
/// is the 403 to send back (no CORS header, like every route; see module
/// docs).
fn check_token(request: &Request, query: &str, state: &State) -> Option<Response<Cursor<Vec<u8>>>> {
    let header_token = header_value(request, "X-G8-Token");
    if token_authorized(&state.token, header_token, query) {
        return None;
    }
    let body = serde_json::json!({ "ok": false, "error": "missing or invalid X-G8-Token" });
    Some(
        Response::from_string(serde_json::to_string(&body).unwrap_or_default())
            .with_status_code(403)
            .with_header(header("Content-Type", "application/json; charset=utf-8")),
    )
}

/// Case-insensitive lookup of a request header's value.
fn header_value<'a>(request: &'a Request, name: &'static str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str())
}

// ── Response helpers ──────────────────────────────────────────────────────────

fn header(key: &str, value: &str) -> Header {
    Header::from_bytes(key.as_bytes(), value.as_bytes()).expect("static header")
}

/// Serve the dashboard HTML with the real per-run token spliced in — see
/// [`TOKEN_PLACEHOLDER`]. No CORS header: the page is meant to be navigated
/// to directly, not fetched cross-origin.
fn respond_html(request: Request, html: &str, token: &str) -> Result<()> {
    let page = html.replacen(TOKEN_PLACEHOLDER, token, 1);
    let resp =
        Response::from_string(page).with_header(header("Content-Type", "text/html; charset=utf-8"));
    request.respond(resp).map_err(Into::into)
}

/// JSON response with no CORS header — same-origin only.
fn respond_json(request: Request, status: u16, body: &serde_json::Value) -> Result<()> {
    let resp = Response::from_string(serde_json::to_string(body).unwrap_or_default())
        .with_status_code(status)
        .with_header(header("Content-Type", "application/json; charset=utf-8"));
    request.respond(resp).map_err(Into::into)
}

fn respond_error(request: Request, status: u16, err: anyhow::Error) -> Result<()> {
    let body = serde_json::json!({ "ok": false, "error": format!("{err:#}") });
    respond_json(request, status, &body)
}

/// Map a handler `Result<Value>` to an HTTP response. Hard errors → 400 with a
/// uniform `{ok:false,error}` envelope; success values pass through (they may
/// themselves carry `ok:false` for soft refusals like a WIP-cap block).
fn respond_result(request: Request, result: Result<serde_json::Value>) -> Result<()> {
    match result {
        Ok(v) => respond_json(request, 200, &v),
        Err(e) => respond_error(request, 400, e),
    }
}

fn read_body(request: &mut Request) -> String {
    let mut s = String::new();
    let _ = request.as_reader().read_to_string(&mut s);
    s
}

// ── URL + misc helpers ────────────────────────────────────────────────────────

fn split_url(raw: &str) -> (String, String) {
    match raw.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (raw.to_string(), String::new()),
    }
}

/// Parse `key=value` out of a raw (un-percent-decoded) query string — fine
/// here since the only value ever read this way is the opaque token.
fn query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then_some(v)
    })
}

fn display_host(host: &str) -> &str {
    if host == "0.0.0.0" {
        "localhost"
    } else {
        host
    }
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let cmd = ("open", url);
    #[cfg(target_os = "linux")]
    let cmd = ("xdg-open", url);
    #[cfg(target_os = "windows")]
    let cmd = ("explorer", url);
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let cmd: (&str, &str) = ("", url);

    if cmd.0.is_empty() {
        return;
    }
    let _ = std::process::Command::new(cmd.0).arg(cmd.1).spawn();
}

// ── Unit tests ─────────────────────────────────────────────────────────────────
// Pure predicates only — no live socket/server here (that's covered, where
// feasible, by the black-box tests in crates/g8/tests/serve.rs).

#[cfg(test)]
mod security_tests {
    use super::*;

    #[test]
    fn loopback_host_accepts_only_loopback_forms() {
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("127.5.5.5"));
        assert!(is_loopback_host("::1"));
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("LOCALHOST"));
    }

    #[test]
    fn loopback_host_rejects_everything_else() {
        assert!(!is_loopback_host("0.0.0.0"), "INADDR_ANY is not loopback");
        assert!(!is_loopback_host("192.168.1.5"));
        assert!(!is_loopback_host("10.0.0.1"));
        assert!(!is_loopback_host("8.8.8.8"));
        assert!(!is_loopback_host("::"));
        assert!(!is_loopback_host("example.com"));
        assert!(!is_loopback_host(""));
    }

    #[test]
    fn constant_time_eq_matches_only_identical_strings() {
        assert!(constant_time_eq("", ""));
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ABC"));
        assert!(!constant_time_eq("abc", "abcd"), "different lengths");
        assert!(
            !constant_time_eq("abcd", "abc"),
            "different lengths, reversed"
        );
    }

    #[test]
    fn token_authorized_via_header() {
        assert!(token_authorized("secret", Some("secret"), ""));
        assert!(!token_authorized("secret", Some("wrong"), ""));
        assert!(!token_authorized("secret", None, ""));
    }

    #[test]
    fn token_authorized_via_query_param() {
        assert!(token_authorized("secret", None, "token=secret"));
        assert!(token_authorized("secret", None, "foo=bar&token=secret"));
        assert!(token_authorized("secret", None, "token=secret&foo=bar"));
        assert!(!token_authorized("secret", None, "token=wrong"));
        assert!(!token_authorized("secret", None, "nope=secret"));
    }

    #[test]
    fn token_authorized_rejects_missing_entirely() {
        assert!(!token_authorized("secret", None, ""));
        assert!(!token_authorized("secret", None, "foo=bar"));
    }

    #[test]
    fn query_value_parses_key_value_pairs() {
        assert_eq!(query_value("token=abc", "token"), Some("abc"));
        assert_eq!(query_value("a=1&token=abc&b=2", "token"), Some("abc"));
        assert_eq!(query_value("", "token"), None);
        assert_eq!(query_value("a=1", "token"), None);
    }

    #[test]
    fn generated_tokens_are_long_and_distinct() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 32);
        assert_eq!(b.len(), 32);
        assert_ne!(a, b, "two calls must not collide in practice");
    }
}
