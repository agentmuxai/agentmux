// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Deep links: one address for every place AgentMux can show
//! (docs/specs/SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md). The registry of destinations
//! (`crates/common/deeplinks/destinations.json`) is generated into `registry.rs`; `parse` reads a link
//! against it. srv's guard and the `Pane` tool read both; the app has the same pair in `frontend/app/deeplinks/`.

// Generated: its layout is the generator's, and `--check` compares it byte for byte.
#[rustfmt::skip]
mod registry;
pub mod parse;
pub mod types;

#[cfg(test)]
mod conformance_tests;

pub use parse::{format_link, parse_link, path_problem, LinkError, ParsedLink, Value};
pub use registry::REGISTRY;
pub use types::{AgentRule, Effect, Kind, Place, Registry, Segment, Sensitivity, View};

/// The label key of a parsed link's place, for the words a person sees: the enum value's own label when the place
/// is one enum segment (`settings.sections.recording`), else the place's label; None for the view as it is.
pub fn place_label_key(link: &ParsedLink) -> Option<&'static str> {
    let place = link.place?;
    if let Some(key) = place.label_key {
        return Some(key);
    }
    place.segments.iter().find_map(|s| match s {
        Segment::Var(v) if v.kind == Kind::Enum => {
            let held = link.values.get(v.name)?.to_string();
            v.values.iter().find(|e| e.id == held).map(|e| e.label_key)
        }
        _ => None,
    })
}
