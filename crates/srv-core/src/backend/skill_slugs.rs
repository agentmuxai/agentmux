// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The folder name each skill gets under an agent's `.claude/skills/`. `build_config_files` writes a skill's
//! `SKILL.md` there, and a computer-use skill's other files must land in the same folder, so both ask here.

use std::collections::HashSet;

use crate::backend::agent_config::{unique_skill_slug, SKILL_TYPE_AGENT_SKILL};
use crate::backend::storage::AgentSkill;

/// The folder name (`.claude/skills/<slug>`) each skill gets: `Some` for an Agent Skill with a body, `None` for the
/// rest. Slugs are made unique in the order given, so whatever writes a skill's files beside its `SKILL.md` must use
/// this and not work the slug out again.
pub(crate) fn agent_skill_slugs(skills: &[AgentSkill]) -> Vec<Option<String>> {
    let mut used: HashSet<String> = HashSet::new();
    skills
        .iter()
        .map(|s| (!s.content.is_empty() && s.skill_type == SKILL_TYPE_AGENT_SKILL).then(|| unique_skill_slug(&s.name, &mut used)))
        .collect()
}
