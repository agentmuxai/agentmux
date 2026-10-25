// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The text of each skill in a bundle import's preview: shown whole, so the
//! person reads what an agent will be told to follow before anything is
//! imported, and cut only past a size cap, with the cut flagged.

use super::bundle_import::ParsedSkill;

/// Most characters of one skill's text the preview echoes. A skill is text an
/// agent will be told to follow, so the preview shows it whole; this only
/// bounds a pathological one. Same size as the instructions preview.
pub const MAX_SKILL_PREVIEW_CHARS: usize = 50_000;

/// Most skill text one preview response carries across all its skills. With
/// `MAX_IMPORTED_SKILLS` skills at [`MAX_SKILL_PREVIEW_CHARS`] each, the
/// per-skill cap alone would allow a response of several megabytes.
pub const MAX_SKILLS_PREVIEW_TOTAL_CHARS: usize = 1_000_000;

/// How much of one skill's text a preview shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillTextPreview {
    /// The skill's text as it would be imported, up to what was shown.
    pub text: String,
    /// True when `text` is not the whole skill: the per-skill or the total cap
    /// cut it, so the reader has not seen all of it.
    pub truncated: bool,
    pub total_chars: usize,
}

/// Each skill's text for the preview, in the order given: whole while it fits
/// under [`MAX_SKILL_PREVIEW_CHARS`] and what is left of
/// [`MAX_SKILLS_PREVIEW_TOTAL_CHARS`], else cut and flagged `truncated` so the
/// UI can say the text was not fully shown. The text is the value the commit
/// writes (`ParsedSkill::content`), so what is shown is what is imported.
pub fn bounded_skill_previews(skills: &[ParsedSkill]) -> Vec<SkillTextPreview> {
    let mut left = MAX_SKILLS_PREVIEW_TOTAL_CHARS;
    skills
        .iter()
        .map(|skill| {
            let total_chars = skill.content.chars().count();
            let shown = total_chars.min(MAX_SKILL_PREVIEW_CHARS).min(left);
            left -= shown;
            let text = if shown == total_chars { skill.content.clone() } else { skill.content.chars().take(shown).collect() };
            SkillTextPreview { text, truncated: shown < total_chars, total_chars }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill_with(content: &str) -> ParsedSkill {
        ParsedSkill { source_dir: "skills/x".into(), slug: "x".into(), description: String::new(), content: content.into() }
    }

    /// A skill is shown whole under the per-skill cap, counted in characters
    /// not bytes; past it, the text is cut, flagged, and keeps its real length.
    #[test]
    fn a_skill_is_shown_whole_and_one_that_is_cut_is_flagged() {
        let accented = "é".repeat(MAX_SKILL_PREVIEW_CHARS);
        let long = "x".repeat(MAX_SKILL_PREVIEW_CHARS + 7);
        let got = bounded_skill_previews(&[skill_with("short"), skill_with(&accented), skill_with(&long)]);

        assert_eq!(got[0], SkillTextPreview { text: "short".into(), truncated: false, total_chars: 5 });
        assert!(!got[1].truncated && got[1].text == accented, "a skill exactly at the cap is whole, whatever its byte size");
        assert!(got[2].truncated);
        assert_eq!((got[2].text.chars().count(), got[2].total_chars), (MAX_SKILL_PREVIEW_CHARS, MAX_SKILL_PREVIEW_CHARS + 7));
    }

    /// The response carries a bounded amount of skill text in all: skills past
    /// the total are flagged cut (with nothing shown) instead of growing it.
    #[test]
    fn the_total_budget_stops_the_text() {
        let each = "y".repeat(MAX_SKILL_PREVIEW_CHARS);
        let fits = MAX_SKILLS_PREVIEW_TOTAL_CHARS / MAX_SKILL_PREVIEW_CHARS;
        let skills: Vec<ParsedSkill> = (0..fits + 3).map(|_| skill_with(&each)).collect();
        let got = bounded_skill_previews(&skills);

        assert!(got[..fits].iter().all(|s| !s.truncated));
        assert!(got[fits..].iter().all(|s| s.truncated && s.text.is_empty() && s.total_chars == MAX_SKILL_PREVIEW_CHARS));
        assert!(got.iter().map(|s| s.text.chars().count()).sum::<usize>() <= MAX_SKILLS_PREVIEW_TOTAL_CHARS);
    }
}
