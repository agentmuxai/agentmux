// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Snap the colours stored before the shared palette to their nearest slot
//! (`docs/specs/SPEC_SHARED_PERCEPTUAL_PALETTE_16_2026_11_01.md` §5.3, §7): the
//! agents' `ui:color` (`agent_content`) and the browser profiles' colours
//! (`browser-profiles.json`). Both were picked from older lists (14 Tailwind
//! hexes, 8 profile hexes, or an HSL pick), so they are not slot hexes yet;
//! the frontend already draws them in their nearest slot, and this makes the
//! stored value the slot's own hex, which is what Kanban, the mobile app and
//! the "least-used colour" pick for new agents read.
//!
//! A window tab's `tab:color` and a pane's `frame:hue` / `pane:colors` are
//! left as they are: they are read as their nearest slot and become slot
//! values the next time the user picks one.
//!
//! Like `m0020`, it attaches the global definition registry itself so it sees
//! agents that live only there. Idempotent: a value that is already a slot's
//! hex is left alone (and keeps its `updated_at`).

use std::sync::Arc;

use crate::backend::browser_profiles_store::{profiles_file_path, read_profiles, write_profiles, BrowserProfile};
use crate::backend::palette::snap_hex;
use crate::backend::storage::store::{AgentContent, Store};
use crate::registry::{resolve_shared_definitions_dir, DefinitionStore};

use super::{Migration, MigrationContext, MigrationError, MigrationScope};

pub struct M0045PaletteSnap;

/// The slot hex a stored colour snaps to, or `None` when it already is one or is not a colour.
fn snapped(stored: &str) -> Option<String> {
    let stored = stored.trim();
    let slot = snap_hex(stored)?;
    (!slot.hex.eq_ignore_ascii_case(stored)).then(|| slot.hex.clone())
}

/// Snap every profile's colour; whether anything changed.
fn snap_profiles(profiles: &mut [BrowserProfile]) -> bool {
    let mut changed = false;
    for p in profiles {
        if let Some(hex) = snapped(&p.color) {
            p.color = hex;
            changed = true;
        }
    }
    changed
}

impl Migration for M0045PaletteSnap {
    fn id(&self) -> &'static str {
        "0045_palette_snap"
    }
    fn scope(&self) -> MigrationScope {
        MigrationScope::Channel
    }
    fn description(&self) -> &'static str {
        "Snap agent and browser profile colours to the shared palette's nearest slot"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if ctx.channel_store_path.exists() {
            snap_agent_colors(&ctx.channel_store_path)?;
        }
        if let Some(path) = profiles_file_path() {
            let mut profiles =
                read_profiles(&path).map_err(|e| MigrationError(format!("palette_snap: read profiles: {e}")))?;
            if snap_profiles(&mut profiles) {
                write_profiles(&path, &profiles)
                    .map_err(|e| MigrationError(format!("palette_snap: write profiles: {e}")))?;
            }
        }
        Ok(())
    }
}

fn snap_agent_colors(store_path: &std::path::Path) -> Result<(), MigrationError> {
    let mstore = Arc::new(
        Store::open(store_path).map_err(|e| MigrationError(format!("palette_snap: open mstore: {e}")))?,
    );
    // Best-effort, as m0020: without the registry only this channel's agents are seen.
    if let Some(def_dir) = resolve_shared_definitions_dir() {
        match DefinitionStore::open(def_dir) {
            Ok(def_store) => mstore.set_def_registry(Arc::new(def_store)),
            Err(e) => tracing::warn!(error = %e, "palette_snap: failed to open global def registry, snapping local-only"),
        }
    }
    let defs = mstore
        .agent_def_list()
        .map_err(|e| MigrationError(format!("palette_snap: list defs: {e}")))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    for def in defs {
        let Some(existing) = mstore
            .agent_content_get(&def.id, "ui:color")
            .map_err(|e| MigrationError(format!("palette_snap: get {}: {e}", def.id)))?
        else {
            continue;
        };
        let Some(hex) = snapped(&existing.content) else { continue };
        mstore
            .agent_content_set(&AgentContent {
                agent_id: def.id.clone(),
                content_type: "ui:color".to_string(),
                content: hex,
                updated_at: now,
            })
            .map_err(|e| MigrationError(format!("palette_snap: set {}: {e}", def.id)))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::palette::slots;
    use crate::backend::storage::agents::AgentDefinition;
    use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_GUARD;

    /// Run `f` with the home override on a fresh temp dir, so the migration
    /// never reads the machine's real registry or profiles (see m0020's tests).
    fn with_isolated_home<T>(f: impl FnOnce() -> T) -> T {
        let _guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let home_tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", home_tmp.path().to_str().unwrap());
        let result = f();
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        result
    }

    fn ctx_for(path: &std::path::Path) -> MigrationContext {
        MigrationContext {
            home: std::env::temp_dir(),
            data_dir: std::env::temp_dir(),
            shared_store_path: std::env::temp_dir().join("unused-store.db"),
            channel_store_path: path.to_path_buf(),
        }
    }

    fn insert_def(mstore: &Store, name: &str) -> String {
        let mut def = AgentDefinition {
            seat: None,
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
            id: format!("test-{name}"),
            slug: String::new(),
            name: name.to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
        };
        mstore.agent_def_insert(&mut def).unwrap();
        def.id
    }

    fn set_color(mstore: &Store, id: &str, color: &str, at: i64) {
        mstore
            .agent_content_set(&AgentContent {
                agent_id: id.to_string(),
                content_type: "ui:color".to_string(),
                content: color.to_string(),
                updated_at: at,
            })
            .unwrap();
    }

    #[test]
    fn an_old_colour_snaps_and_a_slot_hex_or_junk_is_left_alone() {
        assert_eq!(snapped("#14b8a6").as_deref(), Some("#00b89e"));
        assert_eq!(snapped(" #14B8A6 ").as_deref(), Some("#00b89e"));
        assert_eq!(snapped(&slots()[3].hex), None);
        assert_eq!(snapped(&slots()[3].hex.to_uppercase()), None);
        assert_eq!(snapped("junk"), None);
        assert_eq!(snapped(""), None);
    }

    #[test]
    fn profiles_snap_and_report_whether_anything_changed() {
        let profile = |color: &str| BrowserProfile {
            id: "p-1".into(),
            name: "Work".into(),
            color: color.into(),
            created_at: 0,
            agents_allowed: false,
        };
        let mut ps = vec![profile("#ec4899"), profile(&slots()[0].hex)];
        assert!(snap_profiles(&mut ps));
        assert_eq!(ps[0].color, "#e276a0");
        assert_eq!(ps[1].color, slots()[0].hex);
        assert!(!snap_profiles(&mut ps), "a second run changes nothing");
    }

    #[test]
    fn agent_colours_snap_once_and_an_agent_without_one_stays_without() {
        with_isolated_home(|| {
            let tmp = tempfile::NamedTempFile::new().unwrap();
            let mstore = Store::open(tmp.path()).unwrap();
            let old = insert_def(&mstore, "old");
            let current = insert_def(&mstore, "current");
            let none = insert_def(&mstore, "none");
            set_color(&mstore, &old, "#3b82f6", 7);
            set_color(&mstore, &current, &slots()[5].hex, 42);

            M0045PaletteSnap.up(&ctx_for(tmp.path())).unwrap();

            let snapped_old = mstore.agent_content_get(&old, "ui:color").unwrap().unwrap();
            assert_eq!(snapped_old.content, "#53a3f2", "Tailwind blue is Azure");
            let kept = mstore.agent_content_get(&current, "ui:color").unwrap().unwrap();
            assert_eq!((kept.content.as_str(), kept.updated_at), (slots()[5].hex.as_str(), 42));
            assert!(mstore.agent_content_get(&none, "ui:color").unwrap().is_none());

            M0045PaletteSnap.up(&ctx_for(tmp.path())).unwrap();
            let again = mstore.agent_content_get(&old, "ui:color").unwrap().unwrap();
            assert_eq!(again.updated_at, snapped_old.updated_at, "idempotent");
        });
    }

    #[test]
    fn a_missing_channel_store_is_a_noop() {
        with_isolated_home(|| {
            M0045PaletteSnap.up(&ctx_for(std::path::Path::new("Z:/does/not/exist/objects.db"))).unwrap();
        });
    }
}
