// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Move the colours stored for the 16-slot palette to the 14 slots that
//! replace it: 12 hues, Black and White
//! (`docs/specs/SPEC_SHARED_PALETTE_12_HUES_BLACK_WHITE_2026_11_16.md` §5).
//!
//! The same two stores as `m0045_palette_snap`, by the same code: every
//! agent's `ui:color` (`agent_content`) and every browser profile's colour
//! (`browser-profiles.json`). What changed is what the palette says a colour
//! means (`backend::palette::snap_hex`): one of the 16 slots' own hexes goes to
//! the slot the palette file's `retired` table names (four pairs merge, Azure
//! becomes Blue and Pink becomes Rose), a grey is Black or White, and any other
//! colour goes to the hue nearest it. `m0045` is recorded as done on every
//! store that has the 16, so the move needs a migration of its own.
//!
//! A window tab's `tab:color`, a pane's `frame:color` / `frame:hue`,
//! `pane:colors` and a backdrop's built-in id are left as they are: they are
//! read as their slot, a retired id through the `retired` table, and become
//! current values the next time they are written.
//!
//! Idempotent: a value that is already one of the 14 hexes (Black and White
//! among them) is left alone, and keeps its `updated_at`.

use super::m0045_palette_snap::{snap_agent_colors, snap_profiles_file};
use super::{Migration, MigrationContext, MigrationError, MigrationScope};
use crate::backend::browser_profiles_store::profiles_file_path;

pub struct M0051Palette12Snap;

impl Migration for M0051Palette12Snap {
    fn id(&self) -> &'static str {
        "0051_palette_12_snap"
    }
    fn scope(&self) -> MigrationScope {
        MigrationScope::Channel
    }
    fn description(&self) -> &'static str {
        "Move agent and browser profile colours from the 16-slot palette to its 12 hues, Black and White"
    }

    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError> {
        if ctx.channel_store_path.exists() {
            snap_agent_colors(&ctx.channel_store_path)?;
        }
        if let Some(path) = profiles_file_path() {
            // A corrupt or hand-edited profiles file must not block this migration and every later one.
            if let Err(e) = snap_profiles_file(&path) {
                tracing::warn!(error = %e, "palette_12_snap: leaving browser profiles as they are");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "m0051_palette_12_snap_tests.rs"]
mod tests;
