// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for `shell.list`, Settings' default-shell picker
//! (`backend::shells`, docs/specs/SPEC_DEFAULT_SHELL_SETTING_2026_10_13.md).

use serde::{Deserialize, Serialize};

/// One shell installed on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ShellInfo {
    /// The executable, as `term:localshellpath` would hold it.
    pub path: String,
    /// What people call it: "PowerShell", "Git Bash", "Zsh".
    pub name: String,
    /// `pwsh`, `powershell`, `cmd`, `gitbash`, `bash`, `zsh`, `fish`, `nu`, `sh` or `other`.
    pub kind: String,
    /// What it said when asked, e.g. `7.6.6`; absent if it didn't answer in time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version: Option<String>,
}

/// The shell a new local terminal starts with now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct EffectiveShell {
    pub path: String,
    /// `setting` (`term:localshellpath`), `automatic` (none set), or `fallback`
    /// (the one set wasn't found, so the automatic one is used).
    pub source: String,
    /// The configured shell that wasn't found, for `fallback`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub missing: Option<String>,
}

/// `shell.list`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct ShellList {
    /// The shells found, most familiar first (the login shell first on macOS and Linux).
    pub shells: Vec<ShellInfo>,
    /// What "Automatic" resolves to on this machine now.
    pub automatic: String,
    pub effective: EffectiveShell,
}
