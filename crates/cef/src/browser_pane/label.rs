// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A browser pane's host label: `browser-pane-<block id>-<seq>`.
//!
//! The reducer makes one for each pane it creates. `seq` is never reused, so a
//! pane closed and made again for the same block has a new label. The label is
//! the key of the pane's browser in host state, so "is this a pane" and "which
//! block is it" are asked of a label in many places: they ask here, so the
//! format is written once.

const PREFIX: &str = "browser-pane-";

/// The label of the pane made for `block_id` with sequence number `seq`.
pub(crate) fn pane_label(block_id: &str, seq: u64) -> String {
    format!("{PREFIX}{block_id}-{seq}")
}

/// Whether `label` names a browser pane, and not a window, a pool window or a
/// floating pane.
pub(crate) fn is_pane_label(label: &str) -> bool {
    label.starts_with(PREFIX)
}

/// The block a pane label was made for. `None` for a label that is no pane's,
/// or that has no sequence number after the block id.
pub(crate) fn pane_block_id(label: &str) -> Option<&str> {
    let (block_id, _seq) = label.strip_prefix(PREFIX)?.rsplit_once('-')?;
    Some(block_id)
}

/// Whether `label` is a pane label made for `block_id`, whatever its sequence
/// number.
pub(crate) fn is_pane_label_of(label: &str, block_id: &str) -> bool {
    pane_block_id(label) == Some(block_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_gives_its_block_back() {
        let label = pane_label("3f2a-77b1", 12);
        assert_eq!(label, "browser-pane-3f2a-77b1-12");
        assert!(is_pane_label(&label));
        assert_eq!(pane_block_id(&label), Some("3f2a-77b1"));
    }

    #[test]
    fn other_labels_are_no_panes() {
        for label in ["main", "window-7", "window-pool-3", "floating-pool-x", "floating-9", ""] {
            assert!(!is_pane_label(label), "{label}");
            assert_eq!(pane_block_id(label), None, "{label}");
        }
    }

    #[test]
    fn a_label_with_no_sequence_number_is_a_pane_with_no_block() {
        assert!(is_pane_label("browser-pane-orphan"));
        assert_eq!(pane_block_id("browser-pane-orphan"), None);
    }

    #[test]
    fn a_label_belongs_to_its_own_block_only() {
        let label = pane_label("abc", 4);
        assert!(is_pane_label_of(&label, "abc"));
        assert!(!is_pane_label_of(&label, "ab"));
        // A block whose id starts with another block's id and a dash.
        assert!(!is_pane_label_of(&pane_label("abc-def", 4), "abc"));
        assert!(!is_pane_label_of("window-abc-4", "abc"));
    }
}
