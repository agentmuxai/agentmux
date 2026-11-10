// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    COMMAND_WRITE_AGENT_CONFIG,
    CommandWriteAgentConfigData, CommandWriteAgentConfigResult,
};
use crate::backend::agent_config::BUNDLE_SECTION_HEADING;
use crate::backend::base::expand_home_dir_safe;
use crate::backend::rpc_types::{
    UnwatchMediaDirReq, WatchEditorFileReq, WatchMediaDirReq,
    CommandReadEditorFileData, CommandReadEditorFileResult, CommandWriteEditorFileData,
    DirEntry, EditorDrive, EditorRootsReq, GetEditorHomeResult, GetEditorRootsResult,
    ListEditorDirReq, ListEditorDirResult,
    CreateEditorDirReq, CreateEditorDirResult, CreateEditorFileReq, CreateEditorFileResult,
    CreateScratchFileReq, CreateScratchFileResult, DeleteEditorFileReq, MoveScratchFileReq,
    MoveScratchFileResult, OpenInShellReq, RenameEditorFileReq, RenameEditorFileResult,
};
use crate::backend::storage::store::Store;

use super::AppState;

mod agent_config;
mod file_io;
mod scratch;

// Per-process token written into scratch claim files. Stale claim files from a
// previous process run have a different token and are ignored during reuse scans,
// allowing crash-recovery (the scratch becomes reclaimable on restart).
static SCRATCH_SESSION_TOKEN: std::sync::OnceLock<String> = std::sync::OnceLock::new();

fn scratch_session_token() -> &'static str {
    SCRATCH_SESSION_TOKEN.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

/// The editor changes files only under the user's home folder (an allowlist).
/// A WSL distro's file through `\\wsl.localhost` counts when it is under a
/// distro user's home folder (`remote::wsl_fs::in_home`): the same rule, in
/// the distro. Its `/mnt/<letter>` is a Windows drive and never counts here.
fn editor_may_change(canonical: &std::path::Path, canonical_home: &std::path::Path) -> bool {
    canonical.starts_with(canonical_home) || in_a_distro_home(canonical)
}

/// A home folder itself, the user's or a distro user's: never renamed or deleted.
fn is_a_home(canonical: &std::path::Path, canonical_home: &std::path::Path) -> bool {
    canonical == canonical_home
        || crate::backend::remote::wsl_fs::share_of(canonical)
            .is_some_and(|s| crate::backend::remote::wsl_fs::is_home(&s.linux))
}

fn in_a_distro_home(path: &std::path::Path) -> bool {
    crate::backend::remote::wsl_fs::share_of(path)
        .is_some_and(|s| crate::backend::remote::wsl_fs::in_home(&s.linux))
}

/// Enumerate drives/mounts to surface as editor file-tree roots alongside
/// $HOME (via the `geteditorroots` RPC). On **macOS** this returns nothing, so
/// the file-tree's roots are limited to $HOME — `/`, `/Volumes`, and external
/// mounts aren't offered as starting points (root scoping for the tree, not a
/// sandbox; see the note at the macOS arm). On Linux/Windows it exposes the
/// filesystem root + mounts.
/// Spec: docs/specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md (multi-root follow-up).
/// Also the drives section of `fs.places` (fs_handlers.rs), so the editor
/// tree and the Files pane offer the same roots.
#[cfg(target_os = "windows")]
pub(super) fn list_drives() -> Vec<EditorDrive> {
    let mut drives = Vec::new();
    for letter in b'A'..=b'Z' {
        let path = format!("{}:\\", letter as char);
        if std::path::Path::new(&path).exists() {
            drives.push(EditorDrive {
                name: format!("{}:", letter as char),
                path,
            });
        }
    }
    drives
}

// macOS: scope the editor file-tree ROOTS to the user's home. HOME is the sole
// root from GetEditorRoots, so the tree doesn't offer `/`, `/Volumes`, or
// external mounts as starting points (no nudging the user toward volumes they
// didn't choose). NOTE: this is root scoping, not a sandbox — listeditordir /
// readeditorfile still serve any absolute path the frontend sends, and a
// symlink under HOME can resolve outside it; macOS TCC remains the actual gate
// for protected locations.
#[cfg(target_os = "macos")]
pub(super) fn list_drives() -> Vec<EditorDrive> {
    Vec::new()
}

// Linux (and any other non-Windows, non-macOS unix): filesystem root + mounts.
#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
pub(super) fn list_drives() -> Vec<EditorDrive> {
    let mut drives = vec![EditorDrive { name: "/".to_string(), path: "/".to_string() }];
    for mount_dir in ["/mnt", "/media", "/Volumes"] {
        if let Ok(entries) = std::fs::read_dir(mount_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    drives.push(EditorDrive {
                        name: entry.file_name().to_string_lossy().to_string(),
                        path: path.to_string_lossy().to_string(),
                    });
                }
            }
        }
    }
    drives
}

/// Prepend Global Memory, then `agent_block` (the agent's picked bundles,
/// `format_agent_bundle_block`), into the `BUNDLE_SECTION_HEADING` section of
/// a CLAUDE.md string, mirroring the injection done by
/// `write_agent_config_files` in the `agent.open` RPC path.  If the file has
/// no such section, a new one is inserted before `# Available Skills` (or at
/// the end of the file).
pub(crate) fn inject_global_bundles(claude_md: &str, id_store: &Arc<Store>, agent_mode: &str, agent_block: &str) -> String {
    let bundles = crate::backend::operator_config_seed::global_bundles_for_agent(
        id_store.bundle_list_global().unwrap_or_default(),
        agent_mode,
    );
    let bundle_block = crate::backend::storage::join_startup_blocks(
        &crate::backend::storage::format_global_bundle_block(&bundles),
        agent_block,
    );
    if bundle_block.is_empty() {
        return claude_md.to_string();
    }

    // Inject after the `BUNDLE_SECTION_HEADING` line if it exists.
    let heading_line = format!("\n{BUNDLE_SECTION_HEADING}\n");
    if let Some(pos) = claude_md.find(&heading_line) {
        let insert_at = pos + heading_line.len();
        let (before, after) = claude_md.split_at(insert_at);
        format!("{}{}\n\n---\n\n{}", before, bundle_block, after)
    } else {
        // No bundle section — insert one before `# Available Skills` or append.
        let anchor = "\n# Available Skills\n";
        if let Some(pos) = claude_md.find(anchor) {
            let (before, after) = claude_md.split_at(pos);
            format!("{before}{heading_line}{bundle_block}{after}")
        } else {
            format!("{claude_md}{heading_line}{bundle_block}\n")
        }
    }
}

use crate::backend::fs_ops::remote as remote_fs;

pub fn register_editor_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    register_watchers(engine, state);
    register_file_io(engine, state);
    register_tree_mutations(engine);
    register_scratch(engine);
    register_agent_config(engine, state);
}

/// The user's home folder, or the command's own message when there is none.
fn home_or(message: &str) -> Result<std::path::PathBuf, String> {
    dirs::home_dir().ok_or_else(|| message.to_string())
}

/// Live-reload watching of editor files (spec: docs/specs/SPEC_EDITOR_LIVE_FILE_RELOAD_2026_07_18.md) and
/// live-update watching of Media panes pointed at a directory (docs/specs/SPEC_MEDIA_PANE_2026_07_26.md). Both
/// watchers always exist since the move onto the shared FsWatchPool
/// (SPEC_SHARED_FS_WATCHER_FRAMEWORK_2026_08_07.md): a construction-time failure degrades into
/// `FsWatchPool::health()`'s degraded_paths, so there's nothing to check here.
fn register_watchers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // watcheditorfile starts watching a (path, block_id) pair once a tab's content has loaded.
    let watcher = state.editor_file_watcher.clone();
    engine.register_typed("watcheditorfile", move |cmd: WatchEditorFileReq, _ctx| {
        let watcher = watcher.clone();
        async move {
            watcher.watch_path(expand_home_dir_safe(&cmd.path).as_path(), &cmd.block_id);
            Ok(())
        }
    });
    // unwatcheditorfile stops it, on tab close / pane dispose. The same Req type as `watcheditorfile`, on
    // purpose (see `WatchEditorFileReq`): an unwatch that cannot name exactly what the watch named leaks a watcher.
    let watcher = state.editor_file_watcher.clone();
    engine.register_typed("unwatcheditorfile", move |cmd: WatchEditorFileReq, _ctx| {
        let watcher = watcher.clone();
        async move {
            watcher.unwatch_path(expand_home_dir_safe(&cmd.path).as_path(), &cmd.block_id);
            Ok(())
        }
    });
    // watchmediadir starts it for a directory target (a fixed single file has nothing new to watch for; it is
    // re-fetched on demand); unwatchmediadir stops it when the target moves away, or on dispose.
    let watcher = state.media_file_watcher.clone();
    engine.register_typed("watchmediadir", move |cmd: WatchMediaDirReq, _ctx| {
        let watcher = watcher.clone();
        async move {
            watcher.watch_directory(expand_home_dir_safe(&cmd.path).as_path(), &cmd.block_id, &cmd.extensions);
            Ok(())
        }
    });
    let watcher = state.media_file_watcher.clone();
    engine.register_typed("unwatchmediadir", move |cmd: UnwatchMediaDirReq, _ctx| {
        let watcher = watcher.clone();
        async move {
            watcher.unwatch_directory(expand_home_dir_safe(&cmd.path).as_path(), &cmd.block_id);
            Ok(())
        }
    });
}

/// Reading, writing and listing for the editor pane, and the roots its file tree starts from.
fn register_file_io(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let auth_key = state.auth_key.clone();
    engine.register_typed("readeditorfile", move |cmd: CommandReadEditorFileData, _ctx| {
        let auth_key = auth_key.clone();
        async move {
            match remote_fs::ssh_connection(cmd.connection.as_deref()) {
                Some(conn) => file_io::read_remote_file(conn, &cmd, &auth_key).await,
                None => file_io::read_local_file(&cmd.path),
            }
        }
    });
    let auth_key = state.auth_key.clone();
    engine.register_typed("writeeditorfile", move |cmd: CommandWriteEditorFileData, _ctx| {
        let auth_key = auth_key.clone();
        async move {
            file_io::check_write_size(&cmd)?;
            match remote_fs::ssh_connection(cmd.connection.as_deref()) {
                Some(conn) => file_io::write_remote_file(conn, &cmd, &auth_key).await,
                None => file_io::write_local_file(&home_or("writeeditorfile: cannot determine home directory")?, &cmd),
            }
        }
    });
    engine.register_typed("listeditordir", |cmd: ListEditorDirReq, _ctx| async move { file_io::list_dir(&cmd) });
    // geteditorhome is the OS home directory, the editor file-tree's default root. The request is an `Option<_>`
    // (see `EditorRootsReq`): it is what makes both encodings of "no argument" deserialize.
    engine.register_typed("geteditorhome", |_req: Option<EditorRootsReq>, _ctx| async move {
        let home = home_or("geteditorhome: cannot determine home directory")?;
        Ok(GetEditorHomeResult { home: home.to_string_lossy().into_owned() })
    });
    // geteditorroots is home plus (Linux/Windows) the filesystem root and mounts, rendered as sibling top-level
    // roots. On macOS `list_drives` returns none, so the file tree is scoped to $HOME only.
    // Spec: docs/specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md (multi-root follow-up)
    engine.register_typed("geteditorroots", |_req: Option<EditorRootsReq>, _ctx| async move {
        let home = home_or("geteditorroots: cannot determine home directory")?;
        // WSL distros after the drives, at their roots through the share.
        let mut drives = list_drives();
        drives.extend(crate::backend::remote::wsl::list_cached().await.into_iter().map(|d| EditorDrive {
            path: crate::backend::remote::wsl_fs::share_path(&d, "/"),
            name: format!("{d} (WSL)"),
        }));
        Ok(GetEditorRootsResult { home: home.to_string_lossy().into_owned(), drives })
    });
}

/// The file tree's context-menu mutations, each confined to the user's home folder.
fn register_tree_mutations(engine: &Arc<WshRpcEngine>) {
    engine.register_typed("openinshell", |cmd: OpenInShellReq, _ctx| async move {
        file_io::reveal_in_shell(&home_or("openinshell: cannot determine home directory")?, &cmd)
    });
    engine.register_typed("renameeditorfile", |cmd: RenameEditorFileReq, _ctx| async move {
        file_io::rename_file(&home_or("renameeditorfile: cannot determine home")?, &cmd)
    });
    engine.register_typed("createeditorfile", |cmd: CreateEditorFileReq, _ctx| async move {
        file_io::create_file(&home_or("createeditorfile: cannot determine home")?, &cmd)
    });
    engine.register_typed("createeditordir", |cmd: CreateEditorDirReq, _ctx| async move {
        file_io::create_dir(&home_or("createeditordir: cannot determine home")?, &cmd)
    });
    engine.register_typed("deleteeditorfile", |cmd: DeleteEditorFileReq, _ctx| async move {
        file_io::delete_file(&home_or("deleteeditorfile: cannot determine home")?, &cmd)
    });
}

fn register_scratch(engine: &Arc<WshRpcEngine>) {
    engine.register_typed("createscratchfile", |cmd: CreateScratchFileReq, _ctx| async move {
        scratch::create_scratch_file(&home_or("createscratchfile: cannot determine home directory")?, cmd)
    });
    engine.register_typed("movescratchfile", |cmd: MoveScratchFileReq, _ctx| async move {
        scratch::move_scratch_file(&home_or("movescratchfile: cannot determine home")?, &cmd)
    });
}

fn register_agent_config(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let stores = Arc::new(agent_config::ConfigStores {
        id_store: state.id_store.clone(),
        mstore: state.mstore.clone(),
        identity_store: state.identity_store.clone(),
    });
    engine.register_typed(COMMAND_WRITE_AGENT_CONFIG, move |cmd: CommandWriteAgentConfigData, ctx| {
        let stores = stores.clone();
        // Only the window, proven by its host, gets an agent's keys; an agent writes only its own (agent_config_guard).
        let hands_out_keys = ctx.operator_only("only the AgentMux window writes an agent's signing keys").is_ok();
        async move { agent_config::write_agent_config(&stores, &ctx, hands_out_keys, cmd) }
    });
}

#[cfg(test)]
mod tests {
    use crate::backend::rpc_types::AgentConfigFile;
    use crate::backend::rpc_types::{
        CommandReadEditorFileResult, CreateScratchFileReq, DeleteEditorFileReq, DirEntry,
    };

    /// The picker launch path puts the agent's picked bundles after Global
    /// Memory under `# Memory`, as `agent.open` does, and still adds them
    /// when there is no Global Memory.
    #[test]
    fn the_startup_file_gets_global_memory_then_the_picked_bundles() {
        use crate::backend::storage::store::{Bundle, Store};
        let id_store = std::sync::Arc::new(Store::open_in_memory().unwrap());
        let file = "# Agent\n\n# Memory\nown notes\n";
        let picks = "# [Bundle] Writer\n\nwrite plainly";

        let only_picks = super::inject_global_bundles(file, &id_store, "host", picks);
        assert_eq!(only_picks, "# Agent\n\n# Memory\n# [Bundle] Writer\n\nwrite plainly\n\n---\n\nown notes\n");

        id_store
            .bundle_upsert(&Bundle {
                id: "g1".into(),
                name: "Rules".into(),
                description: String::new(),
                is_blank: false,
                is_global: true,
                provider: String::new(),
                model: String::new(),
                instructions: "global rule".into(),
                instructions_by_provider: "{}".into(),
                context_files: "[]".into(),
                mcp_servers: "[]".into(),
                skills: "[]".into(),
                sort_order: 0,
                created_at: 0,
                updated_at: 0,
                is_system: false,
            })
            .unwrap();
        let both = super::inject_global_bundles(file, &id_store, "host", picks);
        let global = both.find("# [Workspace] Rules").unwrap();
        let writer = both.find("# [Bundle] Writer").unwrap();
        let own = both.find("own notes").unwrap();
        assert!(global < writer && writer < own, "{both}");
        assert_eq!(super::inject_global_bundles(file, &id_store, "host", ""), both.replace("\n\n---\n\n# [Bundle] Writer\n\nwrite plainly", ""));
    }

    /// The editor's "only under your home folder" rule, in a WSL distro: under
    /// a distro user's home, never its system folders or the Windows drives
    /// mounted in it, and never a home folder itself. Windows only: elsewhere
    /// such a path is an ordinary one (`wsl_fs::share_of`).
    #[cfg(windows)]
    #[test]
    fn the_editor_may_change_files_under_a_distro_home_and_nowhere_else_in_it() {
        use std::path::Path;
        let home = Path::new(r"C:\Users\me");
        let may = |p: &str| super::editor_may_change(Path::new(p), home);
        assert!(may(r"\\?\UNC\wsl.localhost\Ubuntu\home\u\proj\main.rs"));
        assert!(may(r"\\wsl.localhost\Ubuntu\root\notes.md"));
        assert!(may(r"\\wsl.localhost\Ubuntu\home\u"), "a parent for a new file");
        assert!(!may(r"\\wsl.localhost\Ubuntu\etc\passwd"));
        assert!(!may(r"\\wsl.localhost\Ubuntu\srv\app\x"));
        assert!(
            !may(r"\\wsl.localhost\Ubuntu\mnt\c\Windows\x"),
            "a Windows drive through the distro is not a way around the home rule"
        );
        assert!(super::is_a_home(Path::new(r"\\wsl.localhost\Ubuntu\home\u"), home));
        assert!(super::is_a_home(Path::new(r"\\wsl.localhost\Ubuntu\root"), home));
        assert!(!super::is_a_home(Path::new(r"\\wsl.localhost\Ubuntu\home\u\x"), home));
    }

    /// The four watchers record their types too, and `writeagentconfig` migrates
    /// in the same slice as its own agent.ts stub -- so this file is finished:
    /// it has no untyped registrations left at all.
    ///
    /// `watcheditorfile` and `unwatcheditorfile` share one request type on
    /// purpose. They are not merely same-shaped: an unwatch that cannot name
    /// exactly what the watch named leaks a watcher, so sharing makes a future
    /// divergence a compile error instead. `watchmediadir` and
    /// `unwatchmediadir` do NOT share, because the extension filter is
    /// genuinely not part of the registration key.
    #[tokio::test]
    async fn register_typed_records_the_four_watchers() {
        let state = crate::server::tests::test_state();
        let (engine, _rx) = crate::backend::rpc::engine::WshRpcEngine::new();
        super::register_editor_handlers(&engine, &state);
        let schema = engine.schema_json();
        let rows = schema.as_array().unwrap();
        let find = |cmd: &str| {
            rows.iter()
                .find(|r| r["command"] == cmd)
                .unwrap_or_else(|| panic!("{cmd} missing from the schema"))
        };

        for (cmd, req) in [
            ("watcheditorfile", "WatchEditorFileReq"),
            ("unwatcheditorfile", "WatchEditorFileReq"),
            ("watchmediadir", "WatchMediaDirReq"),
            ("unwatchmediadir", "UnwatchMediaDirReq"),
        ] {
            let row = find(cmd);
            assert_eq!(row["requestName"], req, "{cmd} request");
            assert_eq!(row["responseName"], "()", "{cmd} answers nothing");
        }

        // `writeagentconfig` migrated too, in the same slice as its agent.ts
        // stub -- so editor_handlers.rs now has no untyped registrations at
        // all, and this asserts the file is finished rather than that one
        // command is still pending.
        let wac = find(crate::backend::rpc_types::COMMAND_WRITE_AGENT_CONFIG);
        assert_eq!(wac["requestName"], "CommandWriteAgentConfigData");
        assert_eq!(wac["responseName"], "CommandWriteAgentConfigResult");
    }

    /// The five read commands record their request and response types too. With
    /// the four watchers and `writeagentconfig` covered by the test above,
    /// nothing in this file is left untyped.
    #[tokio::test]
    async fn register_typed_records_the_editor_read_commands() {
        let state = crate::server::tests::test_state();
        let (engine, _rx) = crate::backend::rpc::engine::WshRpcEngine::new();
        super::register_editor_handlers(&engine, &state);
        let schema = engine.schema_json();
        let rows = schema.as_array().unwrap();
        let find = |cmd: &str| {
            rows.iter()
                .find(|r| r["command"] == cmd)
                .unwrap_or_else(|| panic!("{cmd} missing from the schema"))
        };

        for (cmd, resp) in [
            ("readeditorfile", "CommandReadEditorFileResult"),
            ("writeeditorfile", "()"),
            ("listeditordir", "ListEditorDirResult"),
            ("geteditorhome", "GetEditorHomeResult"),
            ("geteditorroots", "GetEditorRootsResult"),
        ] {
            assert_eq!(find(cmd)["responseName"], resp, "{cmd} response");
        }
        assert_eq!(find("readeditorfile")["requestName"], "CommandReadEditorFileData");
        assert_eq!(find("writeeditorfile")["requestName"], "CommandWriteEditorFileData");
        assert_eq!(find("listeditordir")["requestName"], "ListEditorDirReq");
    }

    /// `geteditorhome` and `geteditorroots` take no argument, and the two
    /// encodings of that both reach the server -- the stub sends `{}`, a client
    /// that omits `data` sends `null`. `Option<EditorRootsReq>` takes both;
    /// a bare struct rejects the second and `()` rejects the first.
    #[tokio::test]
    async fn the_no_argument_read_commands_accept_both_encodings() {
        let state = crate::server::tests::test_state();
        let (engine, mut rx) = crate::backend::rpc::engine::WshRpcEngine::new();
        super::register_editor_handlers(&engine, &state);

        for cmd in ["geteditorhome", "geteditorroots"] {
            for (i, payload) in [serde_json::json!({}), serde_json::Value::Null]
                .into_iter()
                .enumerate()
            {
                engine.handle_message(crate::backend::rpc_types::RpcMessage {
                    command: cmd.to_string(),
                    reqid: format!("{cmd}-{i}"),
                    data: Some(payload.clone()),
                    ..Default::default()
                });
                let resp = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
                    .await
                    .unwrap()
                    .unwrap();
                assert!(
                    resp.error.is_empty(),
                    "{cmd} should accept {payload}, got error: {}",
                    resp.error,
                );
            }
        }
    }

    /// `readeditorfile`'s four encoding fields were typed `?:` by hand "for
    /// back-compat; absent ⇒ treat as UTF-8", but the server has never omitted
    /// them: `decode_file` returns all four unconditionally. Pin that, because
    /// the generated type now says they are required and this is the only
    /// thing that would notice if the handler started omitting one.
    #[test]
    fn readeditorfile_always_reports_the_encoding_fields() {
        let decoded = crate::backend::text_encoding::decode_file(b"hello");
        let v = serde_json::to_value(CommandReadEditorFileResult {
            content: decoded.content,
            encoding: decoded.encoding,
            bom: decoded.bom.to_string(),
            line_ending: decoded.line_ending.to_string(),
            had_decode_errors: decoded.had_decode_errors,
            read_only: false,
        })
        .unwrap();
        for key in ["content", "encoding", "bom", "line_ending", "had_decode_errors", "read_only"] {
            assert!(v.get(key).is_some(), "{key} must always be present, got {v}");
        }
        assert_eq!(v["encoding"], "UTF-8");
        assert_eq!(v["bom"], "none");
        assert_eq!(v["line_ending"], "lf");
    }

    /// `DirEntry.size` / `.mtime` really are absent rather than zero: a
    /// directory has no meaningful size, and `0` would be indistinguishable
    /// from a real empty file. The generated TS says `size?: number`, and this
    /// is the half of that claim the TS cannot check.
    #[test]
    fn a_directory_entry_omits_size_rather_than_sending_zero() {
        let dir = DirEntry {
            name: "src".to_string(),
            is_dir: true,
            is_symlink: false,
            size: None,
            mtime: None,
        };
        let v = serde_json::to_value(&dir).unwrap();
        assert!(v.get("size").is_none(), "expected size omitted, got {v}");
        assert!(v.get("mtime").is_none(), "expected mtime omitted, got {v}");

        let file = DirEntry {
            name: "main.rs".to_string(),
            is_dir: false,
            is_symlink: false,
            size: Some(0),
            mtime: Some(1_700_000_000_000),
        };
        let v = serde_json::to_value(&file).unwrap();
        assert_eq!(v["size"], 0, "a real empty file still reports its size");
        assert_eq!(v["mtime"], 1_700_000_000_000u64);
    }

    /// The seven file-tree mutation commands record their request and response
    /// types.
    ///
    /// Each of them used to deserialize into a `struct Cmd` declared inside its
    /// own closure, with the response built by an inline `json!({..})`. Neither
    /// can be named, so neither could be generated from — which is exactly why
    /// the frontend restated all fourteen shapes by hand. A command that falls
    /// back to `register_handler` disappears from the registry rather than
    /// being recorded wrong, and that absence is what this catches.
    #[tokio::test]
    async fn register_typed_records_the_editor_mutation_commands() {
        let state = crate::server::tests::test_state();
        let (engine, _rx) = crate::backend::rpc::engine::WshRpcEngine::new();
        super::register_editor_handlers(&engine, &state);
        let schema = engine.schema_json();
        let rows = schema.as_array().unwrap();
        let find = |cmd: &str| {
            rows.iter()
                .find(|r| r["command"] == cmd)
                .unwrap_or_else(|| panic!("{cmd} missing from the schema — did it fall back to register_handler?"))
        };

        for (cmd, req, resp) in [
            ("openinshell", "OpenInShellReq", "()"),
            ("renameeditorfile", "RenameEditorFileReq", "RenameEditorFileResult"),
            ("createeditorfile", "CreateEditorFileReq", "CreateEditorFileResult"),
            ("createeditordir", "CreateEditorDirReq", "CreateEditorDirResult"),
            ("deleteeditorfile", "DeleteEditorFileReq", "()"),
            ("createscratchfile", "CreateScratchFileReq", "CreateScratchFileResult"),
            ("movescratchfile", "MoveScratchFileReq", "MoveScratchFileResult"),
        ] {
            let row = find(cmd);
            assert_eq!(row["requestName"], req, "{cmd} request");
            assert_eq!(row["responseName"], resp, "{cmd} response");
        }
    }

    /// `recursive` is required on purpose, and this is the assertion that keeps
    /// it that way.
    ///
    /// Adding `#[serde(default)]` would compile, pass every gate, and quietly
    /// turn "a caller that forgot the field" into "delete a directory tree it
    /// never asked to delete" — the generated TS would go from `recursive:
    /// boolean` to still-required (ts-rs cannot express the default anyway), so
    /// nothing downstream would flag it either. The destructive choice has to
    /// stay explicit on the wire.
    #[test]
    fn deleteeditorfile_will_not_deserialize_without_recursive() {
        let err = serde_json::from_value::<DeleteEditorFileReq>(
            serde_json::json!({ "path": "/home/u/x" }),
        )
        .expect_err("recursive must not be defaultable");
        assert!(
            err.to_string().contains("recursive"),
            "expected the error to name the field, got {err}",
        );

        // Both explicit values still work, so this is about absence only.
        for recursive in [true, false] {
            let req: DeleteEditorFileReq = serde_json::from_value(
                serde_json::json!({ "path": "/home/u/x", "recursive": recursive }),
            )
            .unwrap();
            assert_eq!(req.recursive, recursive);
        }
    }

    /// `createscratchfile`'s two fields are genuinely optional — the stub
    /// defaults its argument to `{}` — and they are `Option<T>` in Rust, so
    /// ts-rs can say `display_name?: string` without the derive-a-narrower-type
    /// dance the `serde(default)`-on-`String` fields elsewhere need.
    #[test]
    fn createscratchfile_accepts_an_empty_request() {
        let req: CreateScratchFileReq = serde_json::from_value(serde_json::json!({}))
            .expect("both fields are optional");
        assert!(req.display_name.is_none());
        assert!(req.exclude_scratch_ids.is_none());

        let full: CreateScratchFileReq = serde_json::from_value(serde_json::json!({
            "display_name": "Notes",
            "exclude_scratch_ids": ["a", "b"],
        }))
        .unwrap();
        assert_eq!(full.display_name.as_deref(), Some("Notes"));
        assert_eq!(full.exclude_scratch_ids.unwrap().len(), 2);
    }

    /// reagent P1, PR #2322: `writeagentconfig` (this file) is the actual
    /// "click Launch" path, distinct from `agent.open`'s
    /// `write_agent_config_files` (agent_open.rs) which already had
    /// stale-skill-file cleanup. This exercises the exact integration point
    /// the handler uses -- `crate::backend::rpc_types::block::AgentConfigFile`
    /// has a `path` field (not `filename`, unlike `agent_config::AgentConfigFile`)
    /// -- to catch a field-name mismatch silently no-op'ing the cleanup, which
    /// a test only calling `managed_skill_file_paths` with hand-built strings
    /// would not catch.
    #[test]
    fn writeagentconfig_files_produce_the_expected_managed_skill_paths() {
        let files = vec![
            AgentConfigFile { path: "CLAUDE.md".to_string(), content: "soul".to_string() },
            AgentConfigFile {
                path: ".claude/commands/deploy.md".to_string(),
                content: "deploy".to_string(),
            },
            AgentConfigFile {
                path: ".claude/skills/review/SKILL.md".to_string(),
                content: "review".to_string(),
            },
            AgentConfigFile { path: ".mcp.json".to_string(), content: "{}".to_string() },
        ];
        let managed = crate::backend::agent_config::managed_skill_file_paths(
            files.iter().map(|f| f.path.as_str()),
        );
        assert_eq!(managed.len(), 2, "expected exactly the two skill-derived paths: {managed:?}");
        assert!(managed.contains(".claude/commands/deploy.md"));
        assert!(managed.contains(".claude/skills/review/SKILL.md"));
        assert!(!managed.contains("CLAUDE.md"));
        assert!(!managed.contains(".mcp.json"));
    }

    /// End-to-end proof (real filesystem, no RPC engine needed) that the
    /// three shared calls this handler makes -- compute managed paths,
    /// clean up stale ones from a prior manifest, write the new manifest --
    /// actually remove a stale skill artifact between two materializations,
    /// using this handler's own `AgentConfigFile{path, content}` shape.
    #[test]
    fn stale_skill_file_is_removed_across_two_materializations() {
        let work_dir = tempfile::tempdir().unwrap();
        let base_path = work_dir.path();

        // "Launch 1": agent-skill format.
        let launch1 = vec![AgentConfigFile {
            path: ".claude/skills/deploy/SKILL.md".to_string(),
            content: "body".to_string(),
        }];
        let managed1 = crate::backend::agent_config::managed_skill_file_paths(
            launch1.iter().map(|f| f.path.as_str()),
        );
        crate::backend::agent_config::cleanup_stale_managed_skill_files(base_path, &managed1);
        for f in &launch1 {
            let p = base_path.join(&f.path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, &f.content).unwrap();
        }
        crate::backend::agent_config::write_managed_skill_file_manifest(base_path, &managed1);
        let skill_md = base_path.join(".claude/skills/deploy/SKILL.md");
        assert!(skill_md.exists());

        // "Launch 2": switched to prompt format -- same skill, different path.
        let launch2 = vec![AgentConfigFile {
            path: ".claude/commands/deploy.md".to_string(),
            content: "body".to_string(),
        }];
        let managed2 = crate::backend::agent_config::managed_skill_file_paths(
            launch2.iter().map(|f| f.path.as_str()),
        );
        crate::backend::agent_config::cleanup_stale_managed_skill_files(base_path, &managed2);
        for f in &launch2 {
            let p = base_path.join(&f.path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, &f.content).unwrap();
        }
        crate::backend::agent_config::write_managed_skill_file_manifest(base_path, &managed2);

        assert!(
            !skill_md.exists(),
            "stale .claude/skills/deploy/SKILL.md must be removed by the writeagentconfig path too"
        );
        assert!(base_path.join(".claude/commands/deploy.md").exists());
    }

    /// docs/specs/REPORT_JEKT_SIGNING_KEY_INJECTION_GAP_2026_08_16.md: before
    /// this fix, `WriteAgentConfig` (the actual "click Launch" path) never
    /// injected jekt/LAN signing keys at all -- only `agent.open` did, a
    /// path most normal launches don't go through. This exercises the exact
    /// slug-extraction-then-injection sequence the real handler runs,
    /// against the real `rpc_types::AgentConfigFile{path, content}` shape
    /// a caller of this RPC sends (only
    /// `AGENTMUX_AGENT_ID` set -- no jekt fields), the same
    /// field-name-mismatch class of bug the test above this one guards
    /// against for `managed_skill_file_paths`.
    #[test]
    fn writeagentconfig_extracts_slug_and_injects_signing_keys_into_the_real_mcp_json_shape() {
        let store = crate::backend::storage::store::Store::open_in_memory().unwrap();
        let (_wan_dir, wan) = crate::backend::storage::wan_identity::attach_temp_wan_identity(&store);
        let files = vec![AgentConfigFile {
            path: ".mcp.json".to_string(),
            content: serde_json::to_string(&serde_json::json!({
                "mcpServers": {
                    "agentmux": {
                        "type": "stdio",
                        "command": "agentmux-mcp",
                        "args": [],
                        "env": { "AGENTMUX_AGENT_ID": "aria" }
                    }
                }
            }))
            .unwrap(),
        }];

        let pos = files.iter().position(|f| f.path == ".mcp.json").unwrap();
        let agent_slug = serde_json::from_str::<serde_json::Value>(&files[pos].content)
            .ok()
            .and_then(|v| {
                v.pointer("/mcpServers/agentmux/env/AGENTMUX_AGENT_ID")
                    .and_then(|s| s.as_str())
                    .map(str::to_string)
            })
            .expect("must extract the agent slug from the real .mcp.json shape");
        assert_eq!(agent_slug, "aria");

        let rewritten = crate::backend::agent_config::inject_jekt_signing_keys_into_mcp_json(
            &files[pos].content,
            &store,
            &agent_slug,
        )
        .expect("keys must ensure successfully against a fresh in-memory store");
        let parsed: serde_json::Value = serde_json::from_str(&rewritten).unwrap();
        let env = &parsed["mcpServers"]["agentmux"]["env"];
        assert_eq!(env["AGENTMUX_AGENT_ID"], "aria");
        assert!(env["AGENTMUX_JEKT_KEY"].is_string() && !env["AGENTMUX_JEKT_KEY"].as_str().unwrap().is_empty());
        assert!(env["AGENTMUX_LAN_KEY"].is_string() && !env["AGENTMUX_LAN_KEY"].as_str().unwrap().is_empty());
        // No WAN key on this path either (SPEC_WAN_JEKT_SIGNED_BY_SRV_2026_10_14.md):
        // srv signs WAN jekts, and the key it uses is minted in wan.db here.
        assert!(env.get("AGENTMUX_WAN_KEY").is_none());
        assert!(env.get("AGENTMUX_HOST_LABEL").is_none());
        assert!(wan.agent_key_load("aria").unwrap().is_some());

    }
}

#[cfg(test)]
#[path = "editor_handlers_keys_tests.rs"]
mod write_agent_config_keys_tests;
