// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! The reason of a Browser hand-off: carried whole to the person, or refused with the limit.

/// The most characters a hand-off's reason may have. The person reads all of it on the card, so it is never cut: an
/// over-long one is refused with the limit, and the agent shortens it.
pub(crate) const HANDOFF_REASON_MAX: usize = 600;

/// The reason for a hand-off, whole, or why it is refused.
pub(crate) fn handoff_reason(reason: &str) -> Result<String, String> {
    let n = reason.chars().count();
    if n == 0 {
        return Err("say what the user should do (`reason`)".to_string());
    }
    if n > HANDOFF_REASON_MAX {
        return Err(format!(
            "the reason is {n} characters and the most a hand-off carries is {HANDOFF_REASON_MAX}: the person reads all of it on the card, so it is not cut. Shorten it to what they must do"
        ));
    }
    Ok(reason.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-off's reason reaches the person whole: a long one is refused with the limit, never cut.
    #[test]
    fn a_handoff_reason_is_carried_whole_or_refused_with_the_limit() {
        let long_but_ok = "Upload the file on the form, then press Done. ".repeat(12);
        assert!(long_but_ok.chars().count() > 300 && long_but_ok.chars().count() <= HANDOFF_REASON_MAX);
        assert_eq!(handoff_reason(&long_but_ok).unwrap(), long_but_ok, "past the old 300-character cut, carried whole");
        let exactly = "x".repeat(HANDOFF_REASON_MAX);
        assert_eq!(handoff_reason(&exactly).unwrap(), exactly);
        let over = "x".repeat(HANDOFF_REASON_MAX + 1);
        let why = handoff_reason(&over).unwrap_err();
        assert!(why.contains(&(HANDOFF_REASON_MAX + 1).to_string()) && why.contains(&HANDOFF_REASON_MAX.to_string()) && why.contains("not cut"), "{why}");
        assert!(handoff_reason("").unwrap_err().contains("what the user should do"));
        // Characters, not bytes: a reason in another script is not refused early.
        assert!(handoff_reason(&"é".repeat(HANDOFF_REASON_MAX)).is_ok());
    }
}
