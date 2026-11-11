// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the host does while the app's own page can't be loaded (a dev frontend
//! server that is down, the page server gone).
//!
//! The error page used to reload the failed address every 1.2 s for ever: a
//! refused load every second (323 in five minutes in one sighting), Chromium's
//! raw "This site can't be reached" page flashing between them, and every
//! reload counted as a window that was still starting, so the window pool kept
//! being refilled and new windows kept being opened onto the same dead address.
//!
//! Now a failed app page is shown once and left alone. The host checks the
//! address itself (a TCP connect, no page load) with a growing pause, and
//! reloads the window only once something answers, at most `MAX_AUTO_RELOADS`
//! times until a page has loaded. While the page can't be reached no window is
//! opened and the pools are not refilled ([`check_reachable`],
//! [`pool_spawn_blocked`]). A dev build whose page stays gone quits, so a dev
//! app does not outlive its dev server (a job timeout that kills the server
//! and leaves the app behind was the second sighting).

use std::collections::{HashMap, HashSet};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use cef::{rc::Rc, CefString, ImplBrowser, ImplFrame, ImplTask, Task, WrapTask};
use parking_lot::Mutex;

use crate::state::AppState;

/// Reloads of one window the host does on its own until a page has loaded. A
/// server that accepts connections but still fails the load must not loop.
pub(crate) const MAX_AUTO_RELOADS: u32 = 6;
const FIRST_CHECK_MS: u64 = 1_200;
const MAX_CHECK_MS: u64 = 10_000;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(400);
/// How long a dev build keeps running without its page before it quits.
pub(crate) const DEV_EXIT_AFTER: Duration = Duration::from_secs(300);
/// The error page's Quit button navigates here; the host quits instead.
pub(crate) const QUIT_URL: &str = "agentmux-error-page:quit";

/// The pause before the `round`th check of the address: doubling, capped.
pub(crate) fn check_delay_ms(round: u32) -> u64 {
    FIRST_CHECK_MS.saturating_mul(1u64 << round.min(8)).min(MAX_CHECK_MS)
}

/// The window a failed URL belongs to: its `windowLabel`, else the main window.
pub(crate) fn label_of(url: &str) -> String {
    let query = url.split('#').next().unwrap_or(url);
    query
        .split_once('?')
        .map(|(_, q)| q)
        .unwrap_or("")
        .split('&')
        .find_map(|p| p.strip_prefix("windowLabel="))
        .filter(|l| !l.is_empty())
        .unwrap_or("main")
        .to_string()
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// The host checks the address and reloads when it answers.
    Watching,
    /// No more automatic reloads for this window; the page's Retry stays.
    GaveUp,
}

/// The state, kept apart from CEF and the clock so it is tested on its own.
#[derive(Default)]
pub(crate) struct Core {
    /// Window label to the URL it failed to load.
    watches: HashMap<String, String>,
    /// Automatic reloads done per window since a page last loaded.
    reloads: HashMap<String, u32>,
    /// The windows whose page failed and has not loaded since.
    failed: HashSet<String>,
    /// When the first failure of this stretch happened.
    since: Option<Instant>,
}

impl Core {
    pub(crate) fn fail(&mut self, label: &str, url: &str, watch: bool, now: Instant) -> Failure {
        self.since.get_or_insert(now);
        self.failed.insert(label.to_string());
        if !watch || self.reloads.get(label).copied().unwrap_or(0) >= MAX_AUTO_RELOADS {
            self.watches.remove(label);
            return Failure::GaveUp;
        }
        self.watches.insert(label.to_string(), url.to_string());
        Failure::Watching
    }

    /// The watches whose address answers now: each is removed and counted as a reload.
    pub(crate) fn due(&mut self, answers: impl Fn(&str) -> bool) -> Vec<(String, String)> {
        let ready: Vec<String> = self.watches.iter().filter(|(_, url)| answers(url)).map(|(l, _)| l.clone()).collect();
        ready
            .into_iter()
            .filter_map(|label| {
                *self.reloads.entry(label.clone()).or_insert(0) += 1;
                self.watches.remove(&label).map(|url| (label, url))
            })
            .collect()
    }

    /// A page loaded: the stretch is over.
    pub(crate) fn loaded(&mut self) {
        *self = Core::default();
    }

    pub(crate) fn is_unreachable(&self) -> bool {
        !self.failed.is_empty()
    }

    pub(crate) fn failed_labels(&self) -> Vec<String> {
        self.failed.iter().cloned().collect()
    }

    /// Forget the failed windows `alive` says are gone. A closed window can't load a
    /// page, so it must not keep pools and new windows blocked; with none left the
    /// stretch is over.
    pub(crate) fn prune(&mut self, alive: impl Fn(&str) -> bool) {
        let gone: Vec<String> = self.failed.iter().filter(|l| !alive(l)).cloned().collect();
        for label in gone {
            self.failed.remove(&label);
            self.watches.remove(&label);
            self.reloads.remove(&label);
        }
        if self.failed.is_empty() {
            *self = Core::default();
        }
    }

    pub(crate) fn dev_exit_due(&self, now: Instant, after: Duration) -> bool {
        self.since.is_some_and(|s| now.duration_since(s) >= after)
    }

    /// Nothing left for the checking thread to do.
    fn idle(&self, dev: bool) -> bool {
        self.watches.is_empty() && !(dev && self.is_unreachable())
    }
}

fn core() -> &'static Mutex<Core> {
    static CORE: OnceLock<Mutex<Core>> = OnceLock::new();
    CORE.get_or_init(|| Mutex::new(Core::default()))
}

static CHECKING: AtomicBool = AtomicBool::new(false);

fn is_dev() -> bool {
    std::env::var("AGENTMUX_DEV").is_ok()
}

/// Drop the failed windows that have been closed. Level-triggered on purpose: no
/// close path has to remember to tell this module, so the block can't outlive its window.
fn prune_closed(state: &AppState) {
    let alive: HashSet<String> = core().lock().failed_labels().into_iter().filter(|l| state.get_browser(l).is_some()).collect();
    core().lock().prune(|l| alive.contains(l));
}

/// True while the app page can't be loaded: the pools are not refilled.
pub(crate) fn pool_spawn_blocked(state: &AppState) -> bool {
    prune_closed(state);
    core().lock().is_unreachable()
}

/// `Err` while the app page can't be loaded: a new window would load the same
/// dead address.
pub(crate) fn check_reachable(state: &AppState, caller: &str) -> Result<(), String> {
    if !pool_spawn_blocked(state) {
        return Ok(());
    }
    tracing::warn!(target: "wfr:gate", "[wfr:gate] {caller} refused — the app page can't be loaded");
    Err("the app page can't be loaded".to_string())
}

/// A frontend script ran in some window, so the app page loads again.
pub(crate) fn note_loaded() {
    core().lock().loaded();
}

/// An app window failed to load its page (`watch`: false for a failure no
/// retry can clear). Starts the checking thread when there is something to do.
pub(crate) fn on_failure(state: &Arc<AppState>, failed_url: &str, watch: bool) {
    let label = label_of(failed_url);
    let outcome = core().lock().fail(&label, failed_url, watch, Instant::now());
    tracing::warn!(target: "wfr:gate", label = %label, outcome = ?outcome, "[load-error] the app page can't be loaded — no windows opened or pools refilled until it answers");
    if (watch || is_dev()) && !CHECKING.swap(true, Ordering::SeqCst) {
        let state = Arc::clone(state);
        std::thread::spawn(move || check_loop(state));
    }
}

fn check_loop(state: Arc<AppState>) {
    let dev = is_dev();
    let mut round = 0u32;
    loop {
        std::thread::sleep(Duration::from_millis(check_delay_ms(round)));
        round = round.saturating_add(1);
        prune_closed(&state);
        let (due, exit) = {
            let mut c = core().lock();
            let due = c.due(address_answers);
            let exit = dev && c.dev_exit_due(Instant::now(), DEV_EXIT_AFTER);
            if due.is_empty() && !exit && c.idle(dev) {
                CHECKING.store(false, Ordering::SeqCst);
                return;
            }
            (due, exit)
        };
        for (label, url) in due {
            tracing::info!(target: "wfr:gate", label = %label, "[load-error] the app page answers — reloading the window");
            let mut task = ReloadTask::new(Arc::clone(&state), label, url);
            cef::post_task(cef::ThreadId::UI, Some(&mut task));
        }
        if exit {
            tracing::error!("[load-error] dev build: the app page has been unreachable for {} min — quitting", DEV_EXIT_AFTER.as_secs() / 60);
            CHECKING.store(false, Ordering::SeqCst);
            let _ = crate::commands::window::quit_app(&state);
            return;
        }
    }
}

cef::wrap_task! {
    struct ReloadTask {
        state: Arc<AppState>,
        label: String,
        url: String,
    }

    impl Task {
        fn execute(&self) {
            if let Some(frame) = self.state.get_browser(&self.label).and_then(|b| b.main_frame()) {
                frame.load_url(Some(&CefString::from(self.url.as_str())));
            }
        }
    }
}

/// Whether something accepts connections at the host and port of `url`. A
/// connect only: no request, no page load, nothing for Chromium to report.
pub(crate) fn address_answers(url: &str) -> bool {
    let Some((host, port)) = host_and_port(url) else { return false };
    (host.as_str(), port)
        .to_socket_addrs()
        .map(|mut addrs| addrs.any(|a| TcpStream::connect_timeout(&a, CONNECT_TIMEOUT).is_ok()))
        .unwrap_or(false)
}

fn host_and_port(url: &str) -> Option<(String, u16)> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?;
    let default = match scheme.to_ascii_lowercase().as_str() {
        "http" => 80,
        "https" => 443,
        _ => return None,
    };
    if let Some(v6) = authority.strip_prefix('[') {
        let (host, after) = v6.split_once(']')?;
        return Some((host.to_string(), after.strip_prefix(':').and_then(|p| p.parse().ok()).unwrap_or(default)));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => Some((host.to_string(), port.parse().ok()?)),
        _ if authority.is_empty() => None,
        _ => Some((authority.to_string(), default)),
    }
}

/// The error page's help text for an app window whose page can't be reached.
pub(crate) fn hint_html() -> String {
    let dev_note = if is_dev() {
        format!(" This dev build closes by itself if that is still so after {} minutes.", DEV_EXIT_AFTER.as_secs() / 60)
    } else {
        String::new()
    };
    format!(
        "<p>Make sure the Vite dev server is running:<br><code>task dev</code> or <code>npx vite</code></p>\
         <p>This window opens the page by itself as soon as the address answers.{dev_note}</p>\
         <p><button class=\"retry\" onclick=\"location.href='{QUIT_URL}'\">Quit</button></p>"
    )
}

/// From `RequestHandler::on_before_browse`: the error page's Quit button.
/// True (cancel the navigation) when this was it. Only a host-made `data:` page
/// may ask: the frontend and web pages can't quit the app this way.
pub(crate) fn quit_requested(state: &Arc<AppState>, frame: Option<&cef::Frame>, request: Option<&cef::Request>) -> bool {
    use cef::ImplRequest;
    let Some(request) = request else { return false };
    if CefString::from(&request.url()).to_string() != QUIT_URL {
        return false;
    }
    let from_error_page = frame.is_some_and(|f| f.is_main() == 1 && CefString::from(&f.url()).to_string().starts_with("data:"));
    if from_error_page {
        // Off this thread: quit_app saves the session over HTTP.
        let state = Arc::clone(state);
        std::thread::spawn(move || {
            tracing::info!("[load-error] Quit on the error page — quitting");
            let _ = crate::commands::window::quit_app(&state);
        });
    }
    from_error_page
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://localhost:5300/?ipc_port=1";

    fn fail(c: &mut Core, label: &str) -> Failure {
        c.fail(label, URL, true, Instant::now())
    }

    #[test]
    fn the_checks_slow_down_and_stop_at_a_cap() {
        let delays: Vec<u64> = (0..6).map(check_delay_ms).collect();
        assert_eq!(delays, [1_200, 2_400, 4_800, 9_600, 10_000, 10_000]);
        assert_eq!(check_delay_ms(u32::MAX), 10_000);
    }

    /// The incident: a closed port. The host settles after the one failure: any number of
    /// checks that find nothing reload nothing, and no window or pool refill is allowed.
    #[test]
    fn a_closed_port_settles_with_no_reload_and_no_new_windows() {
        let mut c = Core::default();
        assert_eq!(fail(&mut c, "main"), Failure::Watching);
        for _ in 0..1_000 {
            assert!(c.due(|_| false).is_empty());
        }
        assert!(c.is_unreachable(), "pools and new windows stay blocked");
        assert_eq!(c.reloads.get("main"), None, "not one reload");
    }

    #[test]
    fn a_pooled_window_that_fails_is_watched_on_its_own_label() {
        let mut c = Core::default();
        c.fail("main", "http://localhost:5300/?ipc_port=1", true, Instant::now());
        c.fail("window-pool-ab", "http://localhost:5300/?windowLabel=window-pool-ab&pool=1", true, Instant::now());
        assert_eq!(c.watches.len(), 2);
    }

    #[test]
    fn when_the_address_answers_the_window_is_reloaded_once() {
        let mut c = Core::default();
        fail(&mut c, "main");
        assert_eq!(c.due(|_| true), vec![("main".to_string(), URL.to_string())]);
        assert!(c.due(|_| true).is_empty(), "the watch is spent");
    }

    #[test]
    fn a_server_that_answers_but_fails_the_load_is_reloaded_a_bounded_number_of_times() {
        let mut c = Core::default();
        let mut reloads = 0;
        for _ in 0..100 {
            if fail(&mut c, "main") == Failure::GaveUp {
                break;
            }
            reloads += c.due(|_| true).len();
        }
        assert_eq!(reloads as u32, MAX_AUTO_RELOADS);
        assert!(c.watches.is_empty());
        assert!(c.is_unreachable());
    }

    #[test]
    fn a_failure_no_retry_can_clear_is_not_watched() {
        let mut c = Core::default();
        assert_eq!(c.fail("main", "http://localhost:6000/", false, Instant::now()), Failure::GaveUp);
        assert!(c.watches.is_empty());
        assert!(c.is_unreachable());
    }

    #[test]
    fn a_loaded_page_ends_the_stretch() {
        let mut c = Core::default();
        fail(&mut c, "main");
        c.due(|_| true);
        c.loaded();
        assert!(!c.is_unreachable());
        assert_eq!(fail(&mut c, "main"), Failure::Watching, "the reload budget starts again");
    }

    /// The review's case: the failed window is closed, then the address answers. Nothing
    /// is left to load a page, so nothing may stay blocked.
    #[test]
    fn a_closed_window_does_not_keep_the_block() {
        let mut c = Core::default();
        fail(&mut c, "main");
        fail(&mut c, "window-pool-ab");
        c.prune(|l| l == "main");
        assert!(c.is_unreachable(), "main is still failed");
        assert_eq!(c.failed_labels(), vec!["main".to_string()]);
        c.prune(|_| false);
        assert!(!c.is_unreachable());
        assert!(c.watches.is_empty() && c.reloads.is_empty() && c.since.is_none(), "a clean slate");
    }

    #[test]
    fn a_gave_up_window_that_closes_ends_the_block_too() {
        let mut c = Core::default();
        for _ in 0..=MAX_AUTO_RELOADS {
            fail(&mut c, "main");
            c.due(|_| true);
        }
        assert_eq!(fail(&mut c, "main"), Failure::GaveUp);
        c.prune(|_| false);
        assert!(!c.is_unreachable());
    }

    #[test]
    fn a_dev_build_quits_after_its_page_stayed_gone() {
        let mut c = Core::default();
        let t0 = Instant::now();
        assert!(!c.dev_exit_due(t0, DEV_EXIT_AFTER), "nothing failed");
        c.fail("main", URL, true, t0);
        assert!(!c.dev_exit_due(t0 + Duration::from_secs(299), DEV_EXIT_AFTER));
        assert!(c.dev_exit_due(t0 + DEV_EXIT_AFTER, DEV_EXIT_AFTER));
        assert!(!c.idle(true), "a dev build keeps counting");
    }

    #[test]
    fn a_window_is_known_by_its_label_in_the_url() {
        assert_eq!(label_of("http://localhost:5300/?ipc_port=1&windowLabel=window-pool-ab&pool=1"), "window-pool-ab");
        assert_eq!(label_of("http://localhost:5300/?windowLabel=window-1#x"), "window-1");
        assert_eq!(label_of("http://localhost:5300/?ipc_port=1"), "main");
        assert_eq!(label_of("http://localhost:5300/?windowLabel="), "main");
        assert_eq!(label_of("http://localhost:5300/#windowLabel=x"), "main");
    }

    #[test]
    fn an_address_is_read_into_host_and_port() {
        assert_eq!(host_and_port("http://localhost:5300/?a=1"), Some(("localhost".to_string(), 5300)));
        assert_eq!(host_and_port("http://127.0.0.1:62876"), Some(("127.0.0.1".to_string(), 62876)));
        assert_eq!(host_and_port("http://[::1]:5173/x"), Some(("::1".to_string(), 5173)));
        assert_eq!(host_and_port("https://example.com/"), Some(("example.com".to_string(), 443)));
        assert_eq!(host_and_port("http://user@localhost:9/"), Some(("localhost".to_string(), 9)));
        assert_eq!(host_and_port("data:text/html;base64,AAAA"), None);
        assert_eq!(host_and_port("http:///x"), None);
    }

    #[test]
    fn a_listening_port_answers_and_a_closed_one_does_not() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(address_answers(&format!("http://127.0.0.1:{port}/?ipc_port=1")));
        drop(listener);
        assert!(!address_answers(&format!("http://127.0.0.1:{port}/?ipc_port=1")));
        assert!(!address_answers("not a url"));
    }

    #[test]
    fn the_hint_names_the_server_and_offers_quit() {
        let hint = hint_html();
        assert!(hint.contains("Make sure the Vite dev server is running"));
        assert!(hint.contains(QUIT_URL));
        assert!(!QUIT_URL.contains('\''), "it sits inside a single-quoted attribute");
    }
}
