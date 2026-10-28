// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `gh` shell function of an agent whose GitHub access goes through AgentMux Cloud.
//!
//! srv puts it in the agent's environment under the name bash imports exported functions from,
//! so it reaches every bash the agent runs, whichever provider CLI started it.

/// The name bash gives an exported `gh` function.
pub const GH_FUNCTION_NAME: &str = "BASH_FUNC_gh%%";

/// `gh`, for an agent whose person connected GitHub through AgentMux Cloud (srv sets
/// `AGENTMUX_GITHUB_BROKER` for those): the command runs with the GitHub App's token for the
/// repository it is about, or with no token at all, never with one the environment happened to
/// carry. Without the variable it is plain `gh`.
pub const GH_FUNCTION_BODY: &str = "() {  if [ -n \"${AGENTMUX_GITHUB_BROKER:-}\" ]; then\n    local _amx_t;\n    _amx_t=\"$(agentmux-bashwrap github-token -- \"$@\")\";\n    if [ -n \"$_amx_t\" ]; then\n      GH_TOKEN=\"$_amx_t\" GITHUB_TOKEN=\"$_amx_t\" command gh \"$@\";\n    else\n      GH_TOKEN= GITHUB_TOKEN= command gh \"$@\";\n    fi;\n  else\n    command gh \"$@\";\n  fi\n}";
