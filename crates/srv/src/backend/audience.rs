// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who a thing is for: one answer to "does this reach a native agent, a container agent, or only
//! the app?", used for srv's routes (`route_audience`), catalog MCP servers
//! (`storage::mcp_servers`) and Operator Config entries (`operator_config_seed`).
//!
//! The operator's rule (2026-10-28): a container agent can do everything a native agent can, except
//! write to, run on or read from the host directly. So [`Audience::Everyone`] is the normal answer.
//! Anything narrower names the host capability that is the reason and says what the caller should do
//! instead, so a container agent is never refused without a reason and a next step.

/// A thing only a process on the host can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostCap {
    /// Run a command or launch a program on the host.
    Exec,
    /// Change files or the app on the host.
    Write,
    /// Read files on the host outside the container's own mounts.
    Read,
}

impl HostCap {
    pub fn name(self) -> &'static str {
        match self {
            HostCap::Exec => "host_exec",
            HostCap::Write => "host_write",
            HostCap::Read => "host_read",
        }
    }
}

/// What a runtime may do on the host. A native agent has all three; a container agent has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeCaps {
    pub host_exec: bool,
    pub host_write: bool,
    pub host_read: bool,
}

impl RuntimeCaps {
    pub const NATIVE: RuntimeCaps = RuntimeCaps { host_exec: true, host_write: true, host_read: true };
    pub const CONTAINER: RuntimeCaps = RuntimeCaps { host_exec: false, host_write: false, host_read: false };

    pub fn of(container: bool) -> RuntimeCaps {
        if container { Self::CONTAINER } else { Self::NATIVE }
    }

    pub fn has(self, cap: HostCap) -> bool {
        match cap {
            HostCap::Exec => self.host_exec,
            HostCap::Write => self.host_write,
            HostCap::Read => self.host_read,
        }
    }
}

/// Who gets a thing. The `&'static str` of the narrower kinds is what the refused caller is told:
/// the reason, and what to do instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    /// Every agent, native or container. The normal answer.
    Everyone,
    /// Only a runtime that has `cap` on the host.
    NativeOnly(HostCap, &'static str),
    /// The AgentMux app's and srv's own plumbing: not a container agent's to call. (Native agents
    /// are not held back by this tag; the app's own calls carry the host key.)
    AppOnly(&'static str),
    /// Only container agents (nothing uses this yet; the Operator Config manifest can say it).
    ContainerOnly,
}

impl Audience {
    /// Whether an agent whose runtime has `caps` gets the thing.
    pub fn permits(&self, caps: RuntimeCaps) -> bool {
        match *self {
            Audience::Everyone => true,
            Audience::NativeOnly(cap, _) => caps.has(cap),
            Audience::AppOnly(_) => caps == RuntimeCaps::NATIVE,
            Audience::ContainerOnly => caps == RuntimeCaps::CONTAINER,
        }
    }

    /// What a refused caller is told, `None` when `caps` is let in.
    pub fn refusal(&self, caps: RuntimeCaps) -> Option<&'static str> {
        if self.permits(caps) {
            return None;
        }
        match *self {
            Audience::NativeOnly(_, why) | Audience::AppOnly(why) => Some(why),
            Audience::ContainerOnly => Some("this is for container agents."),
            Audience::Everyone => None,
        }
    }

    /// `host_exec`, `host_write` or `host_read` for a [`NativeOnly`](Audience::NativeOnly), else `-`:
    /// the column a reviewer reads to see the boundary.
    pub fn cap_name(&self) -> &'static str {
        match *self {
            Audience::NativeOnly(cap, _) => cap.name(),
            _ => "-",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everyone_reaches_both_runtimes() {
        assert!(Audience::Everyone.permits(RuntimeCaps::NATIVE));
        assert!(Audience::Everyone.permits(RuntimeCaps::CONTAINER));
        assert_eq!(Audience::Everyone.refusal(RuntimeCaps::CONTAINER), None);
    }

    #[test]
    fn a_native_only_thing_needs_its_capability_and_a_container_has_none() {
        for cap in [HostCap::Exec, HostCap::Write, HostCap::Read] {
            let a = Audience::NativeOnly(cap, "why");
            assert!(a.permits(RuntimeCaps::NATIVE));
            assert!(!a.permits(RuntimeCaps::CONTAINER));
            assert_eq!(a.refusal(RuntimeCaps::CONTAINER), Some("why"));
        }
        // A runtime with only some capabilities (a later WSL or remote one) gets what it can use.
        let read_only = RuntimeCaps { host_exec: false, host_write: false, host_read: true };
        assert!(Audience::NativeOnly(HostCap::Read, "r").permits(read_only));
        assert!(!Audience::NativeOnly(HostCap::Exec, "e").permits(read_only));
    }

    #[test]
    fn app_only_and_container_only_split_the_two_runtimes() {
        assert!(!Audience::AppOnly("app").permits(RuntimeCaps::CONTAINER));
        assert!(Audience::AppOnly("app").permits(RuntimeCaps::NATIVE));
        assert!(Audience::ContainerOnly.permits(RuntimeCaps::CONTAINER));
        assert!(!Audience::ContainerOnly.permits(RuntimeCaps::NATIVE));
    }
}
