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
/// Where the person chose to save an unencrypted export, by the id the page was given: the page
/// sees the place and says so, then confirms with that id, and srv writes only to this path.
static PLAIN_PICKED: Mutex<Option<(String, PathBuf)>> = Mutex::new(None);
/// The key file the person chose for the database in `PICKED` (a KeePass database may need one as
/// well as, or instead of, its password), by the same id. Only its path is held: the bytes are read
/// when srv is asked, and go to srv only.
static KEY_PICKED: Mutex<Option<(String, PathBuf)>> = Mutex::new(None);
/// The most a key file may be (srv refuses more).
const MAX_KEY_FILE: u64 = 1024 * 1024;

fn key_file_for(pick_id: &str) -> Option<PathBuf> {
    let held = KEY_PICKED.lock().unwrap_or_else(|e| e.into_inner());
    held.as_ref().filter(|(id, _)| id == pick_id).map(|(_, p)| p.clone())
}

/// A file name the save dialog may suggest: letters, digits, `.`, `_` and `-`.
fn suggested_name(raw: &str, extension: &str) -> String {
    let clean: String = raw.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')).take(80).collect();
    let stem = clean.trim_end_matches(".vault").trim_end_matches(".kdbx").trim_end_matches(".csv").trim_end_matches(".json").trim_matches('.');
    format!("{}.{extension}", if stem.is_empty() { "agentmux" } else { stem })
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
    if matches!(args.get("format").and_then(Value::as_str), Some("csv" | "json")) {
        return export_plain(state, args).await;
    }
    // A `.vault` file, or a KeePass database.
    let (format, extension, filter) = match args.get("format").and_then(Value::as_str) {
        Some("kdbx") => ("kdbx", "kdbx", "KeePass database"),
        _ => ("vault", "vault", "AgentMux Vault"),
    };
    let name = suggested_name(args.get("default_name").and_then(Value::as_str).unwrap_or(""), extension);
    let path = tokio::task::spawn_blocking(move || {
        let _guard = super::platform::host_dialog_guard();
        rfd::FileDialog::new().set_title("Export the Vault").set_file_name(name).add_filter(filter, &[extension]).save_file()
    })
    .await
    .map_err(|e| format!("vault_export: {e}"))?;
    let Some(mut path) = path else { return Ok(json!({ "cancelled": true })) };
    if path.extension().is_none() {
        path.set_extension(extension);
    }
    tracing::info!("[vault] export to a file the person chose");
    relay(
        state,
        json!({
            "op": "export",
            "format": format,
            "path": path.to_string_lossy(),
            "ids": args.get("ids"),
            "passphrase": args.get("passphrase").and_then(Value::as_str).unwrap_or(""),
        }),
    )
    .await
}

/// The unencrypted CSV or JSON export, in two steps so the person sees where the file will go
/// before anything is written (spec §8.3): with no `pick_id`, ask where to save and answer with the
/// place; with the `pick_id` and `confirmed`, have the OS confirm the person (as Reveal does) and
/// then have srv write the file. `cancel` forgets the place.
async fn export_plain(state: &Arc<AppState>, args: &Value) -> Result<Value, String> {
    let format = if args.get("format").and_then(Value::as_str) == Some("json") { "json" } else { "csv" };
    let wanted = args.get("pick_id").and_then(Value::as_str).unwrap_or("");
    if args.get("cancel").and_then(Value::as_bool) == Some(true) {
        *PLAIN_PICKED.lock().unwrap_or_else(|e| e.into_inner()) = None;
        return Ok(json!({ "cancelled": true }));
    }
    if wanted.is_empty() {
        // Checked first, so the person isn't taken through the steps to a refusal.
        if !super::vault::CAN_CONFIRM {
            return Err("Exporting the Vault unencrypted needs this computer to confirm it is you first, and it can't yet on this system. Export a .vault file instead.".into());
        }
        let name = suggested_name(args.get("default_name").and_then(Value::as_str).unwrap_or(""), format);
        let path = tokio::task::spawn_blocking(move || {
            let _guard = super::platform::host_dialog_guard();
            rfd::FileDialog::new().set_title("Export the Vault, unencrypted").set_file_name(name).add_filter(if format == "json" { "JSON" } else { "CSV" }, &[format]).save_file()
        })
        .await
        .map_err(|e| format!("vault_export: {e}"))?;
        let Some(mut path) = path else { return Ok(json!({ "cancelled": true })) };
        if path.extension().is_none() {
            path.set_extension(format);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let answer = json!({
            "pick_id": id,
            "file_name": path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            "folder": path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
            "synced_folder": synced_folder(&path),
        });
        *PLAIN_PICKED.lock().unwrap_or_else(|e| e.into_inner()) = Some((id, path));
        return Ok(answer);
    }
    if args.get("confirmed").and_then(Value::as_bool) != Some(true) {
        return Err("the person hasn't confirmed the export".into());
    }
    let held = PLAIN_PICKED.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let path = match held {
        Some((id, path)) if id == wanted => path,
        _ => return Err("choose where to save the file again".into()),
    };
    super::vault::confirmed("Export every secret in the AgentMux Vault, unencrypted".to_string()).await?;
    // One export per choice of place.
    *PLAIN_PICKED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    tracing::info!("[vault] unencrypted export to a file the person chose");
    relay(state, json!({ "op": "export", "format": format, "path": path.to_string_lossy(), "ids": args.get("ids") })).await
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
            // Every export AgentMux reads: its own, other managers' CSV and JSON, .env.
            rfd::FileDialog::new()
                .set_title("Import into the Vault")
                .add_filter("Vault and password exports", &["vault", "kdbx", "1pux", "csv", "json", "env", "txt"])
                .add_filter("AgentMux Vault", &["vault"])
                .add_filter("All files", &["*"])
                .pick_file()
        })
        .await
        .map_err(|e| format!("vault_import: {e}"))?;
        let Some(path) = picked else { return Ok(json!({ "cancelled": true })) };
        let id = uuid::Uuid::new_v4().to_string();
        *PICKED.lock().unwrap_or_else(|e| e.into_inner()) = Some((id.clone(), path.clone()));
        // A new file starts with no key file.
        *KEY_PICKED.lock().unwrap_or_else(|e| e.into_inner()) = None;
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
    // The key file: chosen or removed here, in the host's own dialog, never by the page or an agent.
    let flag = |k: &str| args.get(k).and_then(Value::as_bool) == Some(true);
    if flag("clear_key_file") {
        *KEY_PICKED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
    if flag("choose_key_file") {
        let picked = tokio::task::spawn_blocking(|| {
            let _guard = super::platform::host_dialog_guard();
            rfd::FileDialog::new()
                .set_title("Choose the database's key file")
                .add_filter("KeePass key files", &["keyx", "key"])
                .add_filter("All files", &["*"])
                .pick_file()
        })
        .await
        .map_err(|e| format!("vault_import: {e}"))?;
        if let Some(key) = picked {
            *KEY_PICKED.lock().unwrap_or_else(|e| e.into_inner()) = Some((pick_id.clone(), key));
            tracing::info!("[vault] a key file the person chose for an import");
        }
    }
    let key_path = key_file_for(&pick_id);
    let key_file_name = key_path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
    if flag("choose_key_file") || flag("clear_key_file") {
        // Only the choice changed: back to the password step, with the key file's name.
        return Ok(json!({ "pick_id": pick_id, "file_name": file_name, "needs_passphrase": true, "key_file_name": key_file_name }));
    }
    let keyfile = match &key_path {
        Some(p) => {
            let p = p.clone();
            let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
                let meta = std::fs::metadata(&p).map_err(|e| format!("couldn't read the key file: {e}"))?;
                if meta.len() > MAX_KEY_FILE {
                    return Err("that key file is too large to be a KeePass key file".into());
                }
                std::fs::read(&p).map_err(|e| format!("couldn't read the key file: {e}"))
            })
            .await
            .map_err(|e| format!("vault_import: {e}"))??;
            use base64::Engine as _;
            Some(base64::engine::general_purpose::STANDARD.encode(bytes))
        }
        None => None,
    };
    let mut answer = relay(
        state,
        json!({
            "op": "import_inspect",
            "path": path.to_string_lossy(),
            "passphrase": args.get("passphrase").and_then(Value::as_str),
            "mapping": args.get("mapping"),
            "keyfile": keyfile,
        }),
    )
    .await?;
    if let Some(o) = answer.as_object_mut() {
        o.insert("pick_id".into(), json!(pick_id));
        o.insert("file_name".into(), json!(file_name));
        o.insert("key_file_name".into(), json!(key_file_name));
        // A copy may already be in the cloud, and in the service's file history.
        o.insert("synced_folder".into(), json!(synced_folder(&path)));
    }
    Ok(answer)
}

/// The cloud-synced folder a file is in (OneDrive, Dropbox, iCloud Drive, Google
/// Drive), if it is in one: a plaintext export left there may already be in the
/// cloud (spec §8.6).
pub fn synced_folder(path: &std::path::Path) -> Option<&'static str> {
    for part in path.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()) {
        let part = part.as_str();
        if part.starts_with("onedrive") {
            return Some("OneDrive");
        }
        if part.starts_with("dropbox") {
            return Some("Dropbox");
        }
        if part == "icloud drive" || part == "iclouddrive" || part == "com~apple~clouddocs" {
            return Some("iCloud Drive");
        }
        if part == "google drive" || part == "googledrive" || part == "my drive" {
            return Some("Google Drive");
        }
    }
    None
}

/// `vault_import_delete`: deletes the file the person picked to import, after they
/// pressed **Delete the file**. It doesn't promise secure erasure: on an SSD an
/// overwrite doesn't reliably reach the old blocks (spec §8.6).
pub async fn delete_picked(args: &Value) -> Result<Value, String> {
    let wanted = args.get("pick_id").and_then(Value::as_str).unwrap_or("");
    let path = {
        let held = PICKED.lock().unwrap_or_else(|e| e.into_inner());
        match &*held {
            Some((id, path)) if !wanted.is_empty() && id == wanted => path.clone(),
            _ => return Err("choose the file again".into()),
        }
    };
    tokio::task::spawn_blocking(move || std::fs::remove_file(&path).map_err(|e| format!("couldn't delete the file: {e}")))
        .await
        .map_err(|e| format!("vault_import_delete: {e}"))??;
    *PICKED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    tracing::info!("[vault] deleted an import file the person chose");
    Ok(json!({ "deleted": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_in_a_synced_folder_is_named_so_the_person_can_be_warned() {
        use std::path::Path;
        assert_eq!(synced_folder(Path::new("C:/Users/a/OneDrive/Documents/export.csv")), Some("OneDrive"));
        assert_eq!(synced_folder(Path::new("C:/Users/a/OneDrive - Contoso/x.csv")), Some("OneDrive"));
        assert_eq!(synced_folder(Path::new("/home/a/Dropbox/x.csv")), Some("Dropbox"));
        assert_eq!(synced_folder(Path::new("/Users/a/Library/Mobile Documents/com~apple~CloudDocs/x.csv")), Some("iCloud Drive"));
        assert_eq!(synced_folder(Path::new("G:/My Drive/x.csv")), Some("Google Drive"));
        assert_eq!(synced_folder(Path::new("C:/Users/a/Downloads/x.csv")), None);
        assert_eq!(synced_folder(Path::new("C:/Users/a/dropbox_notes_not/x.csv")), Some("Dropbox"), "a name that merely starts so is a false alarm, which only warns");
    }

    #[tokio::test]
    async fn only_the_file_the_person_picked_is_deleted_and_only_by_its_id() {
        let dir = std::env::temp_dir().join(format!("vault-del-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let picked = dir.join("export.csv");
        let other = dir.join("other.csv");
        std::fs::write(&picked, "a").unwrap();
        std::fs::write(&other, "b").unwrap();
        *PICKED.lock().unwrap() = Some(("pick-1".to_string(), picked.clone()));
        assert!(delete_picked(&json!({ "pick_id": "someone-elses" })).await.is_err());
        assert!(delete_picked(&json!({})).await.is_err());
        assert!(picked.exists() && other.exists(), "nothing was deleted by a wrong id");
        assert_eq!(delete_picked(&json!({ "pick_id": "pick-1" })).await.unwrap(), json!({ "deleted": true }));
        assert!(!picked.exists() && other.exists(), "only the picked file");
        assert!(delete_picked(&json!({ "pick_id": "pick-1" })).await.is_err(), "and only once");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_plain_export_is_only_for_the_place_the_person_chose() {
        *PLAIN_PICKED.lock().unwrap() = Some(("plain-1".to_string(), PathBuf::from("x.csv")));
        let state = Arc::new(crate::state::AppState::default());
        let run = |args: Value| tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(export_plain(&state, &args));
        // Without the person's confirmation, or for another id, nothing is written and the place is kept.
        assert!(run(json!({ "format": "csv", "pick_id": "plain-1" })).unwrap_err().contains("hasn't confirmed"));
        assert!(run(json!({ "format": "csv", "pick_id": "someone-elses", "confirmed": true })).unwrap_err().contains("again"));
        assert!(PLAIN_PICKED.lock().unwrap().is_some());
        // Cancelling forgets it.
        assert_eq!(run(json!({ "format": "csv", "cancel": true })).unwrap(), json!({ "cancelled": true }));
        assert!(PLAIN_PICKED.lock().unwrap().is_none());
        assert!(run(json!({ "format": "csv", "pick_id": "plain-1", "confirmed": true })).unwrap_err().contains("again"));
    }

    #[test]
    fn a_suggested_file_name_is_plain_and_ends_in_vault() {
        assert_eq!(suggested_name("agentmux-2026-10-20", "vault"), "agentmux-2026-10-20.vault");
        assert_eq!(suggested_name("../../evil name.vault", "vault"), "evilname.vault");
        assert_eq!(suggested_name("", "vault"), "agentmux.vault");
        assert_eq!(suggested_name("a/b\\c:d", "vault"), "abcd.vault");
        // A KeePass database takes `.kdbx`, and drops either extension it was already given.
        assert_eq!(suggested_name("agentmux-2026-10-20", "kdbx"), "agentmux-2026-10-20.kdbx");
        assert_eq!(suggested_name("backup.vault", "kdbx"), "backup.kdbx");
        assert!(suggested_name(&"x".repeat(500), "vault").len() <= 86);
    }
}
