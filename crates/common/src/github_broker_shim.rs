// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `gh` and `git` shell functions of an agent whose GitHub access goes through AgentMux Cloud.
//!
//! srv puts them in the agent's environment under the name bash imports exported functions from,
//! so they reach every bash the agent runs, whichever provider CLI started it. Both do their work
//! through `agentmux-bashwrap` (`crates/bashwrap/src/github.rs`, `git_hooks.rs`).

/// The name bash gives an exported `gh` function.
pub const GH_FUNCTION_NAME: &str = "BASH_FUNC_gh%%";

/// `gh`, for an agent whose person connected GitHub through AgentMux Cloud (srv sets
/// `AGENTMUX_GITHUB_BROKER` for those): the command runs with the GitHub App's token for the
/// repository it is about, or with no token at all, never with one the environment happened to
/// carry. `gh pr create` and `gh pr edit` carry the owner tag in the PR's body. Without the
/// variable it is plain `gh`.
pub const GH_FUNCTION_BODY: &str = r#"() {  if [ -n "${AGENTMUX_GITHUB_BROKER:-}" ]; then
    local _amx_t _amx_x _amx_a=();
    _amx_t="$(agentmux-bashwrap github-token -- "$@")";
    if [ "$1" = pr ] && { [ "$2" = create ] || [ "$2" = edit ]; }; then
      while IFS= read -r -d '' _amx_x; do _amx_a+=("$_amx_x"); done < <(agentmux-bashwrap github-owner-tag -- "$@" 2>/dev/null);
      if [ "${#_amx_a[@]}" -gt 0 ]; then set -- "${_amx_a[@]}"; fi;
    fi;
    if [ -n "$_amx_t" ]; then
      GH_TOKEN="$_amx_t" GITHUB_TOKEN="$_amx_t" command gh "$@";
    else
      GH_TOKEN= GITHUB_TOKEN= command gh "$@";
    fi;
  else
    command gh "$@";
  fi
}"#;

/// The name bash gives an exported `git` function.
pub const GIT_FUNCTION_NAME: &str = "BASH_FUNC_git%%";

/// `git`, for the same agents: a push or a command that makes commits first asks
/// `agentmux-bashwrap git-env`, which refuses what would skip the hooks (`git push --no-verify`) and
/// names the App's bot as the commit's author. Any other git command is plain `git`.
pub const GIT_FUNCTION_BODY: &str = r#"() {  if [ -n "${AGENTMUX_GITHUB_BROKER:-}" ]; then
    case " $* " in
      *" push "*|*" commit "*|*" merge "*|*" rebase "*|*" cherry-pick "*|*" am "*|*" revert "*|*" pull "*|*" stash "*)
        local _amx_e _amx_v=();
        _amx_e="$(agentmux-bashwrap git-env -- "$@")" || return $?;
        if [ -n "$_amx_e" ]; then
          IFS=$'\n' read -r -d '' -a _amx_v <<< "$_amx_e" || true;
          env "${_amx_v[@]}" git "$@";
          return $?;
        fi;;
    esac;
  fi;
  command git "$@"
}"#;
