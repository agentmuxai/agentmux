// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Vault's export and import files
// (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §8.1).
//
// The person picks the file in the host's native dialog, and only that path is
// given to srv, which reads or writes the file itself: the page never supplies a
// path, and no MuxAPI route starts an export or import, so an agent can't export
// the Vault to a file it then reads.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::state::AppState;

/// The file the person last picked to import, by the id the page was given.
/// The page then names the id, never the path, to unlock or inspect it again.
static PICKED: Mutex<Option<(String, PathBuf)>> = Mutex::new(None);

/// A file name the save dialog may suggest: letters, digits, `.`, `_` and `-`.
fn suggested_name(raw: &str) -> String {
    let clean: String = raw.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')).take(80).collect();
    let stem = clean.trim_end_matches(".vault").trim_matches('.');
    format!("{}.vault", if stem.is_empty() { "agentmux" } else { stem })
}

/// Relays one Vault operation to srv, as the host.
async fn relay(state: &Arc<AppState>, body: Value) -> Result<Value, String> {
    let web_endpoint = state.backend_endpoints.lock().web_endpoint.clone();
    let auth_key = state.auth_key.lock().clone();
    let ipc_token = state.ipc_token.clone();
    tokio::task::spawn_blocking(move || crate::client::backend_vault(&web_endpoint, &auth_key, &ipc_token, &body))
        .await
        .map_err(|e| format!("vault: {e}"))?
}

/// `vault_export`: ask where to save, then have srv write the encrypted file there.
/// Answers `{ cancelled: true }` if the person closes the dialog.
pub async fn export(state: &Arc<AppState>, args: &Value) -> Result<Value, String> {
    let name = suggested_name(args.get("default_name").and_then(Value::as_str).unwrap_or(""));
    let path = tokio::task::spawn_blocking(move || {
        let _guard = super::platform::host_dialog_guard();
        rfd::FileDialog::new().set_title("Export the Vault").set_file_name(name).add_filter("AgentMux Vault", &["vault"]).save_file()
    })
    .await
    .map_err(|e| format!("vault_export: {e}"))?;
    let Some(mut path) = path else { return Ok(json!({ "cancelled": true })) };
    if path.extension().is_none() {
        path.set_extension("vault");
    }
    tracing::info!("[vault] export to a file the person chose");
    relay(
        state,
        json!({
            "op": "export",
            "path": path.to_string_lossy(),
            "ids": args.get("ids"),
            "passphrase": args.get("passphrase").and_then(Value::as_str).unwrap_or(""),
        }),
    )
    .await
}

/// `vault_import`: with no `pick_id`, ask which file, then have srv read it; with
/// one, read the same file again (after the person typed its passphrase). Answers
/// `{ pick_id, file_name, needs_passphrase }` or `{ pick_id, file_name, preview }`,
/// or `{ cancelled: true }`.
pub async fn import(state: &Arc<AppState>, args: &Value) -> Result<Value, String> {
    let wanted = args.get("pick_id").and_then(Value::as_str).unwrap_or("").to_string();
    let (pick_id, path) = if wanted.is_empty() {
        let picked = tokio::task::spawn_blocking(|| {
            let _guard = super::platform::host_dialog_guard();
            rfd::FileDialog::new().set_title("Import into the Vault").add_filter("AgentMux Vault", &["vault"]).pick_file()
        })
        .await
        .map_err(|e| format!("vault_import: {e}"))?;
        let Some(path) = picked else { return Ok(json!({ "cancelled": true })) };
        let id = uuid::Uuid::new_v4().to_string();
        *PICKED.lock().unwrap_or_else(|e| e.into_inner()) = Some((id.clone(), path.clone()));
        tracing::info!("[vault] import of a file the person chose");
        (id, path)
    } else {
        let held = PICKED.lock().unwrap_or_else(|e| e.into_inner()).clone();
        match held {
            Some((id, path)) if id == wanted => (id, path),
            _ => return Err("choose the file again".into()),
        }
    };
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut answer = relay(
        state,
        json!({
            "op": "import_inspect",
            "path": path.to_string_lossy(),
            "passphrase": args.get("passphrase").and_then(Value::as_str),
        }),
    )
    .await?;
    if let Some(o) = answer.as_object_mut() {
        o.insert("pick_id".into(), json!(pick_id));
        o.insert("file_name".into(), json!(file_name));
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_suggested_file_name_is_plain_and_ends_in_vault() {
        assert_eq!(suggested_name("agentmux-2026-10-20"), "agentmux-2026-10-20.vault");
        assert_eq!(suggested_name("../../evil name.vault"), "evilname.vault");
        assert_eq!(suggested_name(""), "agentmux.vault");
        assert_eq!(suggested_name("a/b\\c:d"), "abcd.vault");
        assert!(suggested_name(&"x".repeat(500)).len() <= 86);
    }
}
