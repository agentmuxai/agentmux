// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The shape of the deep-link registry, as `registry.rs` holds it. The app's twin is
//! `frontend/app/deeplinks/registry-types.ts`; both are generated from
//! `crates/common/deeplinks/destinations.json` by `scripts/gen-deeplinks.mjs`.

/// What a variable segment, a parameter or an anchor holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Enum,
    Id,
    Slug,
    Int,
    Opaque,
    Path,
    Name,
}

/// What showing a view does beyond showing (G1): a view whose effect is not `None` is a worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    None,
    Starts,
    Network,
}

/// What a view shows, which decides who may open and capture it (G5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sensitivity {
    Public,
    Private,
    Secret,
    /// Decided by the path (G7).
    Path,
    /// As the widget declares.
    Widget,
}

/// Whether an agent may open a view by MuxAPI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRule {
    Yes,
    /// Only its own place (the var or parameter marked `agent_own` names the caller).
    Own,
    /// Only an open pane is revealed; nothing is started (G1).
    Reveal,
    /// Only a package the person approved.
    Approved,
    No,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnumValue {
    pub id: &'static str,
    pub label_key: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Var {
    pub name: &'static str,
    pub kind: Kind,
    pub values: &'static [EnumValue],
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub agent_own: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Segment {
    Lit(&'static str),
    Var(Var),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Param {
    pub name: &'static str,
    pub kind: Kind,
    pub label_key: &'static str,
    pub values: &'static [EnumValue],
    pub min: Option<i64>,
    pub max: Option<i64>,
    /// Allowed only when the place's enum var (`.0`) holds one of these values (`.1`).
    pub only_with: Option<(&'static str, &'static [&'static str])>,
    pub agent_own: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor {
    pub label_key: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    pub id: &'static str,
    /// None when the place is one enum segment: each value is labelled instead.
    pub label_key: Option<&'static str>,
    pub segments: &'static [Segment],
    /// The enum var is the pane's section (`<view>:section`, R14).
    pub sections: bool,
    pub anchor: Option<Anchor>,
    pub params: &'static [Param],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct View {
    pub id: &'static str,
    /// The pane view it opens (block `meta.view`); None for a view that is not a pane.
    pub pane_view: Option<&'static str>,
    pub label_key: &'static str,
    pub icon: &'static str,
    pub build: u32,
    pub effect: Effect,
    pub sensitivity: Sensitivity,
    pub agent: AgentRule,
    pub surfaces: &'static [&'static str],
    pub command: Option<&'static str>,
    /// The link's view is `<id>.<package>` (a widget).
    pub prefix: bool,
    pub requires_place: bool,
    pub places: &'static [Place],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Alias {
    pub from: &'static [&'static str],
    pub to: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unlinked {
    pub view: &'static str,
    pub reason: &'static str,
}

#[derive(Debug)]
pub struct Registry {
    pub grammar: u32,
    /// The build this AgentMux has: a view of a later build is declared and opens nothing.
    pub build: u32,
    pub max_bytes: usize,
    pub separator: &'static str,
    pub views: &'static [View],
    pub aliases: &'static [Alias],
    pub unlinked: &'static [Unlinked],
    pub errors: &'static [(&'static str, &'static str)],
}

impl Registry {
    pub fn view(&self, id: &str) -> Option<&'static View> {
        self.views.iter().find(|v| v.id == id)
    }

    /// The sentence a refusal of `code` says.
    pub fn reason(&self, code: &str) -> &'static str {
        self.errors.iter().find(|(c, _)| *c == code).map(|(_, r)| *r).unwrap_or("the link is refused")
    }
}
