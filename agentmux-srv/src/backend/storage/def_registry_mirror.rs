// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Global agent-definition registry mirror.
//!
//! Mirrors `db_agent_definitions` mutations (with the agent's content +
//! skills) into the GLOBAL definition store at
//! `~/.agentmux/shared/agents/definitions/` so an agent created in one
//! channel is visible in every channel. SQLite remains authoritative for
//! the local channel; the global store is the cross-channel view.
//! Best-effort: every failure is logged, never propagated. Sibling of
//! `registry_mirror.rs` (the named-instance mirror).
//!
//! Cross-channel agent persistence, P0.2b
//! (`docs/specs/SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md`).

use crate::registry::{
    DefContentBlob, DefSkillBlob, DefinitionRecord, DefinitionRecordV1, DEF_MAX_SUPPORTED_SCHEMA,
};

use super::agents::AgentDefinition;
use super::content::AgentContent;
use super::error::StoreError;
use super::skills::AgentSkill;
use super::store::Store;

/// Build a global definition record from a definition row + its content and
/// skills (which live in separate tables and must travel with the
/// definition for a cross-channel agent to launch with its instructions).
pub(super) fn agent_definition_to_record(
    def: &AgentDefinition,
    content: &[AgentContent],
    skills: &[AgentSkill],
) -> DefinitionRecord {
    let data = DefinitionRecordV1 {
        id: def.id.clone(),
        slug: def.slug.clone(),
        name: def.name.clone(),
        icon: def.icon.clone(),
        provider: def.provider.clone(),
        description: def.description.clone(),
        working_directory: def.working_directory.clone(),
        shell: def.shell.clone(),
        provider_flags: def.provider_flags.clone(),
        auto_start: def.auto_start,
        restart_on_crash: def.restart_on_crash,
        idle_timeout_minutes: def.idle_timeout_minutes,
        created_at: def.created_at,
        agent_type: def.agent_type.clone(),
        environment: def.environment.clone(),
        agent_bus_id: def.agent_bus_id.clone(),
        is_seeded: def.is_seeded,
        accounts: def.accounts.clone(),
        parent_id: def.parent_id.clone(),
        branch_label: def.branch_label.clone(),
        updated_at: def.updated_at,
        user_hidden: def.user_hidden,
        container_image: def.container_image.clone(),
        container_volumes: def.container_volumes.clone(),
        container_name: def.container_name.clone(),
        use_ambient_login: def.use_ambient_login,
        auto_continue_enabled: def.auto_continue_enabled,
        content: content
            .iter()
            .map(|c| DefContentBlob {
                content_type: c.content_type.clone(),
                content: c.content.clone(),
            })
            .collect(),
        skills: skills
            .iter()
            .map(|s| DefSkillBlob {
                id: s.id.clone(),
                name: s.name.clone(),
                trigger: s.trigger.clone(),
                skill_type: s.skill_type.clone(),
                description: s.description.clone(),
                content: s.content.clone(),
            })
            .collect(),
    };
    DefinitionRecord {
        schema_version: DEF_MAX_SUPPORTED_SCHEMA,
        data,
    }
}

/// Reconstruct an `AgentDefinition` from a global record (inverse of
/// [`agent_definition_to_record`]) — the read path for a cross-channel
/// agent whose definition isn't in the local channel's SQLite. Content +
/// skills are carried separately on the record and surfaced via the
/// content/skills read fallbacks.
pub(super) fn record_to_agent_definition(rec: &DefinitionRecord) -> AgentDefinition {
    let d = &rec.data;
    AgentDefinition {
        id: d.id.clone(),
        slug: d.slug.clone(),
        name: d.name.clone(),
        icon: d.icon.clone(),
        provider: d.provider.clone(),
        description: d.description.clone(),
        working_directory: d.working_directory.clone(),
        shell: d.shell.clone(),
        provider_flags: d.provider_flags.clone(),
        auto_start: d.auto_start,
        restart_on_crash: d.restart_on_crash,
        idle_timeout_minutes: d.idle_timeout_minutes,
        created_at: d.created_at,
        agent_type: d.agent_type.clone(),
        environment: d.environment.clone(),
        agent_bus_id: d.agent_bus_id.clone(),
        is_seeded: d.is_seeded,
        accounts: d.accounts.clone(),
        parent_id: d.parent_id.clone(),
        branch_label: d.branch_label.clone(),
        updated_at: d.updated_at,
        user_hidden: d.user_hidden,
        container_image: d.container_image.clone(),
        container_volumes: d.container_volumes.clone(),
        container_name: d.container_name.clone(),
        use_ambient_login: d.use_ambient_login,
        // Deliberately not carried by DefinitionRecordV1 (see its module
        // doc) — a model vendor override is channel-local only for now;
        // a cross-channel agent reopened from the global registry starts
        // with the harness's default vendor. Known limitation, not a bug.
        model_vendor_base_url: String::new(),
        auto_continue_enabled: d.auto_continue_enabled,
        // Same known limitation as model_vendor_base_url just above —
        // DefinitionRecordV1 doesn't carry it yet, so a cross-channel
        // reopen starts unbound. m0021's backfill only ever sees the LOCAL
        // channel's SQLite for this reason; a genuinely global backfill
        // needs this field added to the registry wire format first.
        memory_id: String::new(),
    }
}

impl Store {
    /// Mirror a definition (by id) into the global store, reading its full
    /// row + content + skills from SQLite. Best-effort: a missing global
    /// store or any failure is logged, never propagated (SQLite stays
    /// authoritative). No-op when the definition no longer exists.
    ///
    /// `upsert` itself refuses to resurrect a tombstoned id, so a stale
    /// mirror call for a deleted agent won't un-delete it.
    pub(super) fn registry_def_upsert(&self, def_id: &str) {
        let Some(reg) = self.shared_def_registry() else {
            return;
        };
        let def = match self.agent_def_get(def_id) {
            Ok(Some(d)) => d,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!(def_id, error = %e, "def registry: read definition failed, skipping mirror");
                return;
            }
        };
        // Only USER agents go to the global cross-channel store. Seeded
        // templates are manifest-managed and re-seeded per channel (and
        // their hide flag is per-channel UI state), so they stay local.
        if def.is_seeded != 0 {
            return;
        }
        // LOCAL-only reads: the mirror operates on a local agent, so it must
        // never read the global record (that would re-mirror just-deleted
        // content/skills back, defeating the deletion). (reagent P1 on #1385.)
        let content = self.agent_content_get_all_local(def_id).unwrap_or_default();
        let skills = self.agent_skill_list_local(def_id).unwrap_or_default();
        let rec = agent_definition_to_record(&def, &content, &skills);
        if let Err(e) = reg.upsert(&rec) {
            tracing::warn!(def_id, error = %e, "def registry: mirror upsert failed");
        }
    }

    /// Mirror a user-agent deletion as a global tombstone (`retired/`) so
    /// another channel's stale SQLite can't resurrect it via `upsert`.
    /// Best-effort.
    pub(super) fn registry_def_retire(&self, def_id: &str) -> bool {
        let Some(reg) = self.shared_def_registry() else {
            return false;
        };
        // Whether an ACTIVE global record existed — lets the delete path
        // report success for a cross-channel agent that has no local SQLite
        // row. (codex P1 on #1385.)
        let existed = reg.exists(def_id);
        if let Err(e) = reg.retire(def_id) {
            tracing::warn!(def_id, error = %e, "def registry: mirror retire failed");
            return false;
        }
        existed
    }

    /// Apply a definition edit directly to the global record's definition
    /// fields, PRESERVING its existing content + skills. Used when editing a
    /// cross-channel agent that has no local SQLite row to update. Returns
    /// whether a global record was found + updated. Best-effort. Tombstoned
    /// (retired) agents are not resurrected — `get` reads only the active tree.
    pub(super) fn registry_def_update_definition_fields(&self, agent: &AgentDefinition) -> bool {
        let Some(reg) = self.shared_def_registry() else {
            return false;
        };
        let mut rec = match reg.get(&agent.id) {
            Ok(Some(r)) => r,
            _ => return false,
        };
        // Keep the record's content + skills; replace the definition fields.
        let kept_content = std::mem::take(&mut rec.data.content);
        let kept_skills = std::mem::take(&mut rec.data.skills);
        let mut new_rec = agent_definition_to_record(agent, &[], &[]);
        new_rec.data.content = kept_content;
        new_rec.data.skills = kept_skills;
        if let Err(e) = reg.upsert(&new_rec) {
            tracing::warn!(def_id = %agent.id, error = %e, "def registry: cross-channel update failed");
            return false;
        }
        true
    }

    /// Update a single content blob in the global registry record for a
    /// cross-channel agent, preserving all other definition fields, content, and
    /// skills. Used when the local channel has no `db_agent_definitions` row for
    /// the agent (FK would prevent a local SQLite write).
    ///
    /// `value = Some(s)` upserts the blob; `value = None` removes it.
    /// Returns `Err` if the global registry is unavailable or the agent is absent.
    pub(super) fn registry_def_update_content_field(
        &self,
        agent_id: &str,
        content_type: &str,
        value: Option<&str>,
    ) -> Result<(), StoreError> {
        let Some(reg) = self.shared_def_registry() else {
            return Err(StoreError::Other("no global registry available".into()));
        };
        let mut rec = match reg.get(agent_id) {
            Ok(Some(r)) => r,
            Ok(None) => {
                return Err(StoreError::Other(format!(
                    "agent {agent_id} not in global registry"
                )))
            }
            Err(e) => {
                return Err(StoreError::Other(format!(
                    "global registry read failed: {e}"
                )))
            }
        };
        match value {
            Some(v) => {
                if let Some(blob) = rec.data.content.iter_mut().find(|b| b.content_type == content_type) {
                    blob.content = v.to_string();
                } else {
                    rec.data.content.push(DefContentBlob {
                        content_type: content_type.to_string(),
                        content: v.to_string(),
                    });
                }
            }
            None => {
                rec.data.content.retain(|b| b.content_type != content_type);
            }
        }
        reg.upsert(&rec)
            .map_err(|e| StoreError::Other(format!("global registry write failed: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{
        DefContentBlob, DefSkillBlob, DefinitionRecord, DefinitionRecordV1, DefinitionStore,
        DEF_MAX_SUPPORTED_SCHEMA,
    };
    use std::sync::Arc;

    fn global_user_agent(id: &str, name: &str) -> DefinitionRecord {
        DefinitionRecord {
            schema_version: DEF_MAX_SUPPORTED_SCHEMA,
            data: DefinitionRecordV1 {
                id: id.to_string(),
                name: name.to_string(),
                provider: "claude".to_string(),
                is_seeded: 0,
                content: vec![DefContentBlob {
                    content_type: "agentmd".to_string(),
                    content: "be helpful".to_string(),
                }],
                skills: vec![DefSkillBlob {
                    id: "sk1".to_string(),
                    name: "greet".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        }
    }

    #[test]
    fn read_first_surfaces_cross_channel_agent_with_content_and_skills() {
        let store = Store::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let def_store = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());
        // A user agent that exists only in the global store (another channel).
        def_store
            .upsert(&global_user_agent("remote-1", "Remote"))
            .unwrap();
        store.set_def_registry(def_store);

        // Roster includes the cross-channel agent (it's not in local SQLite).
        let list = store.agent_def_list().unwrap();
        assert!(
            list.iter().any(|d| d.id == "remote-1" && d.name == "Remote"),
            "agent_def_list must include the global-only user agent"
        );

        // Content + skills fall back to the global record (local SQLite empty),
        // so the cross-channel agent launches with its instructions.
        let content = store.agent_content_get_all("remote-1").unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0].content, "be helpful");
        let skills = store.agent_skill_list("remote-1").unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "greet");
    }

    // reagent P0 on PR #2505: agent_def_list's global overlay unconditionally
    // inserted the registry-derived def over the local one for every user
    // agent id — including a same-channel agent that's ALSO mirrored into
    // the global store (the normal case: every agent.define write
    // auto-mirrors via registry_def_upsert). Since DefinitionRecordV1 never
    // carries model_vendor_base_url (deliberately channel-local only), the
    // overlay's always-"" value silently erased the local override on every
    // read — including agent.open's spawn-time resolution — making the
    // whole feature a no-op in default single-instance operation, not just
    // the documented genuinely-cross-channel case.
    #[test]
    fn agent_def_list_preserves_local_model_vendor_base_url_over_the_global_overlay() {
        let store = Store::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let def_store = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());

        let mut def = local_def("agent-1", "Vendor Agent");
        def.model_vendor_base_url = "https://my-proxy.example.com".to_string();
        store.agent_def_insert(&mut def).unwrap();
        // Same id also present in the global store — the normal case, not
        // the cross-channel-only case the earlier (buggy) comment assumed.
        def_store.upsert(&global_user_agent("agent-1", "Vendor Agent")).unwrap();
        store.set_def_registry(def_store);

        let list = store.agent_def_list().unwrap();
        let found = list.iter().find(|d| d.id == "agent-1").expect("agent must be listed");
        assert_eq!(
            found.model_vendor_base_url, "https://my-proxy.example.com",
            "the global overlay must not wipe a same-channel agent's vendor override"
        );
    }

    // P0 regression test (ReAgent review on PR #2587 round 6): the
    // IDENTICAL bug as the model_vendor_base_url test just above,
    // for memory_id — DefinitionRecordV1 doesn't carry memory_id either
    // (same documented gap), so record_to_agent_definition always
    // returns "" for it, and the overlay used to overwrite the local
    // row's real memory_id with that empty default on every read. Since
    // every local write auto-mirrors into the global registry, this
    // fired for virtually every agent whenever a shared store is
    // configured — silently defeating agent_open.rs's spawn-time bundle
    // resolution and m0021's memory_id-empty backfill filter.
    #[test]
    fn agent_def_list_preserves_local_memory_id_over_the_global_overlay() {
        let store = Store::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let def_store = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());

        let mut def = local_def("agent-1", "Bundle Agent");
        store.agent_def_insert(&mut def).unwrap();
        store.agent_def_set_memory_id_if_empty("agent-1", "bundle-123").unwrap();
        // Same id also present in the global store — the normal case.
        def_store.upsert(&global_user_agent("agent-1", "Bundle Agent")).unwrap();
        store.set_def_registry(def_store);

        let list = store.agent_def_list().unwrap();
        let found = list.iter().find(|d| d.id == "agent-1").expect("agent must be listed");
        assert_eq!(
            found.memory_id, "bundle-123",
            "the global overlay must not wipe a same-channel agent's bundle binding"
        );
    }

    fn local_def(id: &str, name: &str) -> AgentDefinition {
        AgentDefinition {
            id: id.to_string(),
            slug: id.to_string(),
            name: name.to_string(),
            icon: "✦".to_string(),
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
            auto_continue_enabled: 0,
            memory_id: String::new(),
        }
    }

    fn skill(id: &str, agent_id: &str, name: &str) -> AgentSkill {
        AgentSkill {
            id: id.to_string(),
            agent_id: agent_id.to_string(),
            name: name.to_string(),
            trigger: String::new(),
            skill_type: "prompt".to_string(),
            description: String::new(),
            content: String::new(),
            created_at: 1,
        }
    }

    #[test]
    fn local_skill_delete_does_not_resurrect_from_global() {
        let store = Store::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let def_store = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());
        store.set_def_registry(def_store.clone());

        // Create a LOCAL user agent with one skill (mirrored to global).
        let mut def = local_def("local-1", "Local");
        store.agent_def_insert(&mut def).unwrap();
        store
            .agent_skill_insert(&skill("sk-1", "local-1", "greet"))
            .unwrap();
        assert_eq!(
            def_store.get("local-1").unwrap().unwrap().data.skills.len(),
            1,
            "skill mirrored to global"
        );

        // Delete the skill in this (local) channel.
        store.agent_skill_delete("sk-1").unwrap();

        // Local read must NOT resurrect the deleted skill from the global
        // record (the agent is local, just skill-less now). (reagent P1.)
        assert!(
            store.agent_skill_list("local-1").unwrap().is_empty(),
            "deleted local skill must not reappear from the global record"
        );
        // And the deletion propagated to the global record (the mirror used a
        // local-only read, so it didn't re-write the stale skill).
        assert!(
            def_store.get("local-1").unwrap().unwrap().data.skills.is_empty(),
            "deletion must propagate cross-channel"
        );
    }

    #[test]
    fn local_content_delete_propagates_to_global() {
        let store = Store::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let def_store = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());
        store.set_def_registry(def_store.clone());

        // LOCAL user agent with a per-agent ui:zoom blob (mirrored to global).
        let mut def = local_def("local-z", "LocalZ");
        store.agent_def_insert(&mut def).unwrap();
        store
            .agent_content_set(&AgentContent {
                agent_id: "local-z".into(),
                content_type: "ui:zoom".into(),
                content: "1.5".into(),
                updated_at: 1,
            })
            .unwrap();
        assert!(
            def_store
                .get("local-z")
                .unwrap()
                .unwrap()
                .data
                .content
                .iter()
                .any(|c| c.content_type == "ui:zoom"),
            "content mirrored to global"
        );

        // Reset-to-default deletes the local row.
        store.agent_content_delete("local-z", "ui:zoom").unwrap();

        // Local read is clean (no resurrection from the global record)...
        assert!(
            store.agent_content_get("local-z", "ui:zoom").unwrap().is_none(),
            "deleted local content must not reappear from the global record"
        );
        // ...and the deletion propagated to the global record, so a
        // cross-channel/other-instance reopen won't restore the stale zoom.
        // (reviewer P1 on #1700 — agent_content_delete must re-mirror, like set.)
        assert!(
            def_store
                .get("local-z")
                .unwrap()
                .unwrap()
                .data
                .content
                .iter()
                .all(|c| c.content_type != "ui:zoom"),
            "content deletion must propagate cross-channel"
        );
    }

    #[test]
    fn delete_tombstones_a_cross_channel_only_agent() {
        let store = Store::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let def_store = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());
        // A user agent that exists ONLY in the global store (no local SQLite row).
        def_store
            .upsert(&global_user_agent("remote-1", "Remote"))
            .unwrap();
        store.set_def_registry(def_store.clone());
        // It appears in the roster via the global overlay.
        assert!(store.agent_def_list().unwrap().iter().any(|d| d.id == "remote-1"));

        // Deleting it (rows == 0 locally) must tombstone the global record and
        // report success — not silently leave it to reappear. (codex P1.)
        let deleted = store.agent_def_delete("remote-1").unwrap();
        assert!(deleted, "cross-channel delete must report success");
        assert!(!def_store.exists("remote-1"), "global record tombstoned");
        assert!(
            !store.agent_def_list().unwrap().iter().any(|d| d.id == "remote-1"),
            "deleted cross-channel agent must not reappear in the roster"
        );
    }

    #[test]
    fn update_edits_a_cross_channel_only_agent_preserving_content_skills() {
        let store = Store::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let def_store = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());
        // Global-only agent WITH content + skills.
        def_store
            .upsert(&global_user_agent("remote-1", "Remote"))
            .unwrap();
        store.set_def_registry(def_store.clone());

        // Edit it (no local SQLite row → UPDATE affects 0 rows).
        let mut edited = record_to_agent_definition(&global_user_agent("remote-1", "Remote"));
        edited.name = "Renamed".to_string();
        let ok = store.agent_def_update(&mut edited).unwrap();
        assert!(ok, "cross-channel update must report success, not 'not found'");

        // Global record reflects the edit; content + skills are preserved.
        let rec = def_store.get("remote-1").unwrap().unwrap();
        assert_eq!(rec.data.name, "Renamed");
        assert_eq!(rec.data.content.len(), 1, "content preserved");
        assert_eq!(rec.data.skills.len(), 1, "skills preserved");
        // The roster shows the new name.
        assert!(store
            .agent_def_list()
            .unwrap()
            .iter()
            .any(|d| d.id == "remote-1" && d.name == "Renamed"));
    }

    #[test]
    fn cross_channel_content_set_writes_to_global_registry() {
        // Regression test for #1700: agent_content_set for a cross-channel agent
        // (one absent from local db_agent_definitions) must write directly to the
        // global registry rather than failing with an FK violation.
        let store = Store::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let def_store = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());
        def_store
            .upsert(&global_user_agent("remote-1", "Remote"))
            .unwrap();
        store.set_def_registry(def_store.clone());

        // Write ui:zoom for the cross-channel agent — must not fail.
        store
            .agent_content_set(&AgentContent {
                agent_id: "remote-1".into(),
                content_type: "ui:zoom".into(),
                content: "1.4".into(),
                updated_at: 100,
            })
            .unwrap();

        // The global record must now carry the zoom (other content preserved).
        let rec = def_store.get("remote-1").unwrap().unwrap();
        let zoom_blob = rec.data.content.iter().find(|c| c.content_type == "ui:zoom");
        assert!(zoom_blob.is_some(), "ui:zoom must be in the global record");
        assert_eq!(zoom_blob.unwrap().content, "1.4");
        let agentmd_blob = rec.data.content.iter().find(|c| c.content_type == "agentmd");
        assert!(agentmd_blob.is_some(), "existing agentmd content must be preserved");

        // agent_content_get must surface it via the cross-channel fallback.
        let got = store.agent_content_get("remote-1", "ui:zoom").unwrap();
        assert!(got.is_some(), "agent_content_get must find the zoom via registry fallback");
        assert_eq!(got.unwrap().content, "1.4");
    }

    #[test]
    fn cross_channel_content_delete_removes_from_global_registry() {
        // Regression test for #1700: agent_content_delete for a cross-channel agent
        // must remove the field from the global registry without corrupting other content.
        let store = Store::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let def_store = Arc::new(DefinitionStore::open(tmp.path().join("definitions")).unwrap());
        // Start with a global record that already has ui:zoom set.
        let mut rec = global_user_agent("remote-1", "Remote");
        rec.data.content.push(DefContentBlob {
            content_type: "ui:zoom".into(),
            content: "1.4".into(),
        });
        def_store.upsert(&rec).unwrap();
        store.set_def_registry(def_store.clone());

        // Delete the zoom (reset-to-default path).
        store.agent_content_delete("remote-1", "ui:zoom").unwrap();

        // The zoom blob must be gone, other content intact.
        let after = def_store.get("remote-1").unwrap().unwrap();
        assert!(
            after.data.content.iter().all(|c| c.content_type != "ui:zoom"),
            "ui:zoom must be removed from global record"
        );
        assert!(
            after.data.content.iter().any(|c| c.content_type == "agentmd"),
            "agentmd content must be preserved"
        );

        // agent_content_get must return None now.
        assert!(
            store.agent_content_get("remote-1", "ui:zoom").unwrap().is_none(),
            "deleted zoom must not reappear"
        );
    }
}
