// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Vault's Reveal and Copy
// (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.3).
//
// Both start by asking the OS to confirm the person is at the keyboard
// (Windows Hello). Only then does the host ask srv for the value, as the host.
// Copy never hands the value to the page: the host puts it on the clipboard
// itself and clears it 30 seconds later if it is still there. Reveal returns
// the value to the pane, which shows it for a few seconds.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::state::AppState;

/// How long a copied value stays on the clipboard.
const CLIPBOARD_CLEAR_AFTER: Duration = Duration::from_secs(30);

/// Asks the OS to confirm the person. `Ok(())` only if they were confirmed.
///
/// Runs on a thread of its own that initialises WinRT: the async runtime's
/// workers never do, and `UserConsentVerifier` fails with CO_E_NOTINITIALIZED
/// on a thread that hasn't.
#[cfg(target_os = "windows")]
pub fn verify_person(message: &str) -> Result<(), String> {
    use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};
    let message = message.to_string();
    std::thread::Builder::new()
        .name("vault-hello".into())
        .spawn(move || {
            // Ok (or S_FALSE) means this call initialised it and must undo it;
            // RPC_E_CHANGED_MODE means the thread already was, in another mode.
            let initialised = unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok();
            let result = verify_person_here(&message);
            if initialised {
                unsafe { RoUninitialize() };
            }
            result
        })
        .map_err(|e| format!("couldn't start the Windows Hello prompt: {e}"))?
        .join()
        .map_err(|_| "the Windows Hello prompt stopped unexpectedly".to_string())?
}

#[cfg(target_os = "windows")]
fn verify_person_here(message: &str) -> Result<(), String> {
    use windows::core::{factory, HSTRING};
    use windows::Security::Credentials::UI::{UserConsentVerificationResult as R, UserConsentVerifier, UserConsentVerifierAvailability as A};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::WinRT::IUserConsentVerifierInterop;
    use windows_future::IAsyncOperation;

    let available = UserConsentVerifier::CheckAvailabilityAsync()
        .and_then(|op| op.get())
        .map_err(|e| format!("couldn't ask Windows whether Windows Hello is set up: {e}"))?;
    if available != A::Available {
        return Err(match available {
            A::DeviceNotPresent => "this computer has no Windows Hello device (PIN, fingerprint or face) to confirm you".to_string(),
            A::NotConfiguredForUser => {
                "set up Windows Hello (Settings, Accounts, Sign-in options) so AgentMux can confirm it is you before it shows a secret".to_string()
            }
            A::DisabledByPolicy => "Windows Hello is turned off by your organisation's policy".to_string(),
            _ => "Windows Hello is busy; try again in a moment".to_string(),
        });
    }
    let hwnd = foreground_own_window().ok_or("no AgentMux window to show the Windows Hello prompt over")?;
    let interop = factory::<UserConsentVerifier, IUserConsentVerifierInterop>().map_err(|e| format!("Windows Hello isn't available: {e}"))?;
    let op: IAsyncOperation<R> = unsafe { interop.RequestVerificationForWindowAsync(HWND(hwnd as *mut _), &HSTRING::from(message)) }
        .map_err(|e| format!("Windows Hello couldn't start: {e}"))?;
    match op.get().map_err(|e| format!("Windows Hello failed: {e}"))? {
        R::Verified => Ok(()),
        R::Canceled => Err("cancelled".to_string()),
        R::RetriesExhausted => Err("Windows Hello refused: too many wrong tries".to_string()),
        R::DeviceBusy => Err("Windows Hello is busy; try again in a moment".to_string()),
        R::DeviceNotPresent | R::NotConfiguredForUser | R::DisabledByPolicy => Err("Windows Hello isn't available".to_string()),
        _ => Err("Windows Hello didn't confirm it is you".to_string()),
    }
}

/// The AgentMux window the person is using, else any of ours.
#[cfg(target_os = "windows")]
fn foreground_own_window() -> Option<isize> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    unsafe {
        let fg = GetForegroundWindow();
        if !fg.is_null() {
            let mut pid = 0u32;
            GetWindowThreadProcessId(fg, &mut pid);
            if pid == std::process::id() {
                return Some(fg as isize);
            }
        }
    }
    // No AgentMux window is in front: any of ours will do for the prompt.
    let any = unsafe { super::window::find_own_top_level_window() };
    (!any.is_null()).then_some(any as isize)
}

#[cfg(not(target_os = "windows"))]
pub fn verify_person(_message: &str) -> Result<(), String> {
    Err("Reveal and Copy need Windows Hello for now: Touch ID and the desktop's prompt aren't built yet".to_string())
}

/// The part of the request every Reveal and Copy names.
fn target(args: &Value) -> Result<(String, String), String> {
    let id = args.get("id").and_then(Value::as_str).unwrap_or("").trim();
    let part = args.get("part").and_then(Value::as_str).unwrap_or("").trim();
    if id.is_empty() || part.is_empty() {
        return Err("a Vault entry and the part to show are needed".into());
    }
    Ok((id.to_string(), part.to_string()))
}

/// Asks srv for one part's value, as the host, after the person is confirmed.
async fn fetch_value(state: &Arc<AppState>, id: &str, part: &str, action: &str) -> Result<zeroize::Zeroizing<String>, String> {
    let body = json!({ "op": "reveal", "id": id, "part": part, "action": action });
    let web_endpoint = state.backend_endpoints.lock().web_endpoint.clone();
    let auth_key = state.auth_key.lock().clone();
    let ipc_token = state.ipc_token.clone();
    let data = tokio::task::spawn_blocking(move || crate::client::backend_vault(&web_endpoint, &auth_key, &ipc_token, &body))
        .await
        .map_err(|e| format!("vault: {e}"))??;
    data.get("value").and_then(Value::as_str).map(|s| zeroize::Zeroizing::new(s.to_string())).ok_or_else(|| "srv sent no value".to_string())
}

async fn confirmed(message: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || verify_person(&message)).await.map_err(|e| format!("vault: {e}"))?
}

/// `vault_reveal`: confirm the person, then return one part's value to the pane.
pub async fn reveal(state: &Arc<AppState>, args: &Value) -> Result<Value, String> {
    let (id, part) = target(args)?;
    let what = args.get("label").and_then(Value::as_str).unwrap_or(&id).chars().take(80).collect::<String>();
    confirmed(format!("Show {what} in the AgentMux Vault")).await?;
    tracing::info!("[vault] reveal confirmed for an entry (value not logged)");
    let value = fetch_value(state, &id, &part, "revealed").await?;
    Ok(json!({ "value": value.as_str() }))
}

/// The value a copy put on the clipboard, until it is cleared. One at a time: a
/// newer copy replaces it, so an older copy's timer can't clear the newer one.
static PENDING_CLEAR: std::sync::Mutex<Option<(u64, zeroize::Zeroizing<String>)>> = std::sync::Mutex::new(None);
static COPY_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Held from writing the clipboard to registering the copy as pending, so two
/// copies can't interleave and leave the later one's value on the clipboard
/// with no timer for it.
static COPY_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// How often a clear that couldn't get the clipboard is tried again.
const CLEAR_RETRY_EVERY: Duration = Duration::from_secs(2);
const CLEAR_RETRIES: u32 = 15;

/// Clears the clipboard if it still holds the pending copy. The timer passes
/// its own copy's number; shutdown passes `None` and clears whatever is pending.
/// The copy stays pending until the clear succeeds (the clipboard may be held
/// by another program for a moment), so a failure can be tried again.
fn try_clear(only: Option<u64>) -> Result<(), String> {
    let (n, value) = {
        let p = PENDING_CLEAR.lock().unwrap_or_else(|e| e.into_inner());
        match &*p {
            Some((n, v)) if only.map_or(true, |want| want == *n) => (*n, v.clone()),
            _ => return Ok(()),
        }
    };
    super::clipboard::clear_if_holds(&value)?;
    let mut p = PENDING_CLEAR.lock().unwrap_or_else(|e| e.into_inner());
    if matches!(&*p, Some((m, _)) if *m == n) {
        *p = None;
    }
    Ok(())
}

/// Called as the host's message loop ends: a secret the Vault copied doesn't
/// stay on the clipboard once AgentMux is gone.
pub fn clear_clipboard_on_shutdown() {
    if let Err(e) = try_clear(None) {
        tracing::warn!("[vault] couldn't clear the clipboard at shutdown: {e}");
    }
}

/// `vault_copy`: confirm the person, then copy one part's value to the
/// clipboard without it passing through the page, and clear it later.
pub async fn copy(state: &Arc<AppState>, args: &Value) -> Result<Value, String> {
    let (id, part) = target(args)?;
    let what = args.get("label").and_then(Value::as_str).unwrap_or(&id).chars().take(80).collect::<String>();
    confirmed(format!("Copy {what} from the AgentMux Vault")).await?;
    let value = fetch_value(state, &id, &part, "copied").await?;
    let n = {
        let _one_at_a_time = COPY_LOCK.lock().await;
        super::clipboard::write_secret_text(&value)?;
        let n = COPY_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        *PENDING_CLEAR.lock().unwrap_or_else(|e| e.into_inner()) = Some((n, value));
        n
    };
    tracing::info!("[vault] copy confirmed for an entry (value not logged)");
    tokio::spawn(async move {
        tokio::time::sleep(CLIPBOARD_CLEAR_AFTER).await;
        for attempt in 0..CLEAR_RETRIES {
            // Only if no newer copy replaced this one, and the clipboard still
            // holds what we put there: the person may have copied something
            // else since.
            match tokio::task::spawn_blocking(move || try_clear(Some(n))).await {
                Ok(Ok(())) => return,
                Ok(Err(e)) if attempt + 1 == CLEAR_RETRIES => tracing::warn!("[vault] couldn't clear the clipboard: {e}"),
                _ => tokio::time::sleep(CLEAR_RETRY_EVERY).await,
            }
        }
    });
    Ok(json!({ "clears_in_secs": CLIPBOARD_CLEAR_AFTER.as_secs() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WinRT works on the prompt's own thread: asking whether Windows Hello is set
    /// up (no prompt is shown) must not fail with CO_E_NOTINITIALIZED. Run by hand
    /// (`cargo test -p agentmux-cef vault -- --ignored`): CI machines have no Hello.
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "asks Windows about Hello; machine-dependent"]
    fn windows_hello_can_be_asked_about_from_the_prompts_thread() {
        use windows::Security::Credentials::UI::UserConsentVerifier;
        use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};
        let answer = std::thread::spawn(|| {
            let initialised = unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok();
            let r = UserConsentVerifier::CheckAvailabilityAsync().and_then(|op| op.get());
            if initialised {
                unsafe { RoUninitialize() };
            }
            r
        })
        .join()
        .unwrap();
        println!("Windows Hello availability: {answer:?}");
        assert!(answer.is_ok(), "{answer:?}");
    }

    #[test]
    fn a_reveal_names_an_entry_and_a_part() {
        assert_eq!(target(&json!({ "id": " ssn-alex ", "part": "value" })).unwrap(), ("ssn-alex".to_string(), "value".to_string()));
        assert!(target(&json!({ "id": "ssn-alex" })).is_err());
        assert!(target(&json!({ "part": "value" })).is_err());
        assert!(target(&json!({})).is_err());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn where_nothing_confirms_the_person_nothing_is_shown() {
        assert!(verify_person("Show it").unwrap_err().contains("Windows Hello"));
    }
}
