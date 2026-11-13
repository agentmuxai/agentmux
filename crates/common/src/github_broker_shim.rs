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
/// carry. What a `gh pr`, `gh issue`, `gh release` or `gh api` command sends is scanned for
/// credentials first (`github-text-guard`: a hit refuses the command, a stdin body reaches `gh` as a
/// file), and `gh pr create` and `gh pr edit` carry the owner tag in the PR's body. `gh auth status`
/// says so, instead of "not logged into any GitHub hosts" (the bare command only: with flags such as
/// `--json` it is gh's own output a caller parses). A container's `gh pr merge`, `gh release` and `gh api` first go through
/// `github-container`: srv does the merge for it, and the API writes its token cannot make are refused with what to do
/// (`crates/bashwrap/src/container_verbs.rs`). The command word is read after a leading `-R`/`--repo` (`gh -R o/r pr merge 7`),
/// for the container step, the text check and the owner tag alike. Without the variable it is plain `gh`.
pub const GH_FUNCTION_BODY: &str = r#"() {  if [ -n "${AGENTMUX_GITHUB_BROKER:-}" ]; then
    local _amx_t _amx_x _amx_n _amx_r _amx_c0 _amx_c1 _amx_a=();
    if [ "$1" = auth ] && [ "$2" = status ] && [ "$#" -eq 2 ]; then
      printf '%s
' 'github.com' '  AgentMux signs gh in per repository: each gh command in a repository gets a short-lived token for it' '  (the AgentMux GitHub App, acting for your user). There is no stored login, so gh auth does not list one.';
      return 0;
    fi;
    _amx_c0=$1; _amx_c1=${2:-};
    case "$1" in
      -R|--repo) _amx_c0=${3:-}; _amx_c1=${4:-};;
      -R?*|--repo=*) _amx_c0=${2:-}; _amx_c1=${3:-};;
    esac;
    if [ "${AGENTMUX_IN_CONTAINER:-}" = 1 ] && { [ "$_amx_c0" = api ] || [ "$_amx_c0" = release ] || { [ "$_amx_c0" = pr ] && [ "$_amx_c1" = merge ]; }; }; then
      if agentmux-bashwrap github-container -- "$@"; then return 0; else _amx_r=$?; fi;
      [ "$_amx_r" -eq 3 ] || return "$_amx_r";
    fi;
    if [ "$_amx_c0" = pr ] || [ "$_amx_c0" = issue ] || [ "$_amx_c0" = release ] || [ "$_amx_c0" = api ]; then
      while IFS= read -r -d '' _amx_x; do _amx_a+=("$_amx_x"); done < <(agentmux-bashwrap github-text-guard -- "$@");
      _amx_n=${#_amx_a[@]};
      if [ "$_amx_n" -eq 0 ]; then
        echo 'agentmux: this gh command was not run: the text it sends could not be checked.' >&2;
        return 1;
      fi;
      _amx_x="${_amx_a[$((_amx_n - 1))]}";
      [ "$_amx_x" = ok ] || return 1;
      unset "_amx_a[$((_amx_n - 1))]";
      set -- "${_amx_a[@]}";
      _amx_a=();
    fi;
    _amx_t="$(agentmux-bashwrap github-token -- "$@")";
    if [ "$_amx_c0" = pr ] && { [ "$_amx_c1" = create ] || [ "$_amx_c1" = new ] || [ "$_amx_c1" = edit ]; }; then
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
          ( export "${_amx_v[@]}"; command git "$@" );
          return $?;
        fi;;
    esac;
  fi;
  command git "$@"
}"#;
