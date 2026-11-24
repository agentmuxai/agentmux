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
/// The real `gh` it runs is told (`AGENTMUX_GH_SHIM_DONE`) that these steps are done: in a container's image
/// `gh` on PATH is `agentmux-bashwrap` too (`crates/bashwrap/src/gh_shim.rs`, for `timeout gh`, `xargs gh`, scripts),
/// and it does the same steps, once, for a caller that did not come through this function.
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
      GH_TOKEN="$_amx_t" GITHUB_TOKEN="$_amx_t" AGENTMUX_GH_SHIM_DONE=1 command gh "$@";
    else
      GH_TOKEN= GITHUB_TOKEN= AGENTMUX_GH_SHIM_DONE=1 command gh "$@";
    fi;
  else
    command gh "$@";
  fi
}"#;

/// The name bash gives an exported `git` function.
pub const GIT_FUNCTION_NAME: &str = "BASH_FUNC_git%%";

/// The variable that makes the `git` function keep a folder's own diff programs from running (srv sets it for a
/// host agent in a folder a container agent can write).
pub const GIT_NO_EXT_DIFF_ENV: &str = "AGENTMUX_GIT_NO_EXT_DIFF";

/// The step of the `git` function that [`GIT_NO_EXT_DIFF_ENV`] turns on. A `diff.<driver>.textconv` and a
/// `filter.<name>.clean`, `smudge` and `process` of the folder's own configuration are set to `cat` (the `process` to
/// nothing, and the filter not required), and a `merge.<name>.driver` to `false`, as `GIT_CONFIG_KEY_<n>` and
/// `GIT_CONFIG_VALUE_<n>` entries added to those the environment holds (which every command git starts inherits, so
/// `range-diff` is covered, and which take the whole key, so a name with an `=` in it is covered too). After any leading `-C <dir>`, `-c <name=value>` and other global options, every command
/// gets `--no-pager` (so a pager the folder's own configuration names does not run); `diff`, `log`, `show`,
/// `format-patch`, `whatchanged`, `blame` and `reflog` (after its `show`, or with no subcommand, and not for another
/// one) also get `--no-ext-diff` and `--no-textconv` (so the external diff program, a diff driver's `command` and a
/// `textconv` of the folder's own configuration do not run); `fetch`, `pull`, `ls-remote` and `clone` get
/// `--upload-pack=git-upload-pack` and `push` gets `--receive-pack=git-receive-pack` (so a remote's `uploadpack` and
/// `receivepack` of the folder's configuration do not run), and `fetch` and `pull` get `fetch.all` false; `fetch` and `pull` of all remotes, several or a group, and
/// `remote update`, `show`, `prune`, `set-head` and `add -f`, are refused while the folder's configuration names such a
/// program for a remote. Every other option is git's own, and
/// the function goes on with the arguments so changed.
macro_rules! no_ext_diff_step {
    () => {
        r#"  if [ -n "${AGENTMUX_GIT_NO_EXT_DIFF:-}" ]; then
    local _amx_g=() _amx_r=() _amx_e=() _amx_s _amx_t _amx_c _amx_k _amx_b _amx_n _amx_i _amx_w _amx_z _amx_x=() _amx_y=() _amx_u=;
    while [ $# -gt 0 ]; do
      case "$1" in
        -C|-c|--git-dir|--work-tree|--namespace|--super-prefix|--config-env|--attr-source) _amx_g+=("$1"); shift; if [ $# -gt 0 ]; then _amx_g+=("$1"); shift; fi;;
        -*) _amx_g+=("$1"); shift;;
        *) break;;
      esac;
    done;
    while IFS=$'\t' read -r _amx_c _amx_k; do
      case "$_amx_c" in global|system) continue;; esac;
      case "$_amx_k" in
        *.textconv) _amx_e+=("$_amx_k" cat);;
        merge.*.driver) _amx_e+=("$_amx_k" false);;
        filter.*.clean|filter.*.smudge) _amx_e+=("$_amx_k" cat "${_amx_k%.*}.required" false);;
        filter.*.process) _amx_e+=("$_amx_k" "" "${_amx_k%.*}.required" false);;
        remotes.*) _amx_r+=("${_amx_k#remotes.}");;
        *.uploadpack|*.receivepack) _amx_u=1;;
      esac;
    done < <(command git ${_amx_g[@]+"${_amx_g[@]}"} config --show-scope --name-only --get-regexp '^(diff\..*\.textconv|merge\..*\.driver|filter\..*\.(clean|smudge|process)|remotes\..*|remote\..*\.(uploadpack|receivepack))$' 2>/dev/null);
    _amx_n=${GIT_CONFIG_COUNT:-0}; case "$_amx_n" in ''|*[!0-9]*) _amx_n=0;; esac;
    for ((_amx_i=0; _amx_i<${#_amx_e[@]}; _amx_i+=2)); do
      local -x "GIT_CONFIG_KEY_$_amx_n=${_amx_e[_amx_i]}" "GIT_CONFIG_VALUE_$_amx_n=${_amx_e[_amx_i+1]}"; _amx_n=$((_amx_n+1));
    done;
    local -x GIT_CONFIG_COUNT=$_amx_n;
    _amx_w=${1:-};
    for _amx_i in 1 2 3 4 5; do
      _amx_z=;
      case "$_amx_w" in diff|log|show|format-patch|whatchanged|blame|reflog|fetch|pull|ls-remote|clone|push|remote|"") break;; esac;
      _amx_t=$(command git ${_amx_g[@]+"${_amx_g[@]}"} config --show-scope --get "alias.$_amx_w" 2>/dev/null) || break;
      _amx_c=${_amx_t%%$'\t'*}; _amx_b=${_amx_t#*$'\t'}; read -r _amx_w _amx_t <<< "$_amx_b";
      read -ra _amx_y <<< "$_amx_t"; _amx_x=(${_amx_y[@]+"${_amx_y[@]}"} ${_amx_x[@]+"${_amx_x[@]}"});
      case "$_amx_c:$_amx_w" in
        global:*|system:*) ;;
        *:'!'*|*:-*) echo "git: an alias of this folder's configuration that runs a program or starts with an option is not run" >&2; return 1;;
        *) case "$_amx_b" in *'"'*|*"'"*|*'\'*) echo "git: an alias of this folder's configuration that quotes a word is not run" >&2; return 1;; esac;;
      esac;
      _amx_z=1;
    done;
    if [ -n "$_amx_z" ]; then
      case "$_amx_w" in
        diff|log|show|format-patch|whatchanged|blame|reflog|fetch|pull|ls-remote|clone|push|remote|"") ;;
        *) if _amx_t=$(command git ${_amx_g[@]+"${_amx_g[@]}"} config --show-scope --get "alias.$_amx_w" 2>/dev/null); then
             case "${_amx_t%%$'\t'*}" in global|system) ;; *) echo "git: an alias chain of this folder's configuration that is too long to follow is not run" >&2; return 1;; esac;
           fi;;
      esac;
    fi;
    case "$_amx_w" in
      remote)
        if [ -n "$_amx_u" ]; then
          _amx_s=$1; shift; _amx_t=;
          for _amx_k in ${_amx_x[@]+"${_amx_x[@]}"} "$@"; do case "$_amx_k" in -*) ;; *) _amx_t=$_amx_k; break;; esac; done;
          case "$_amx_t" in
            update|show|prune|set-head) echo "git remote $_amx_t is not run while this folder's configuration names an upload or receive program for a remote; use git fetch or git ls-remote" >&2; return 1;;
            add)
              _amx_b=;
              for _amx_k in ${_amx_x[@]+"${_amx_x[@]}"} "$@"; do
                case "$_amx_k" in
                  --?*) _amx_c=${_amx_k#--}; _amx_c=${_amx_c%%=*}; case "fetch" in "$_amx_c"*) _amx_b=1;; esac;;
                  -*f*) _amx_b=1;;
                esac;
              done;
              if [ -n "$_amx_b" ]; then echo "git remote add with a fetch is not run while this folder's configuration names an upload or receive program for a remote" >&2; return 1; fi;;
          esac;
          set -- "$_amx_s" "$@";
        fi;;
      diff|log|show|format-patch|whatchanged|blame) _amx_s=$1; shift; set -- "$_amx_s" --no-ext-diff --no-textconv "$@";;
      reflog)
        _amx_s=$1; shift;
        case "${1:-}" in
          show) _amx_t=$1; shift; set -- "$_amx_s" "$_amx_t" --no-ext-diff --no-textconv "$@";;
          ""|-*) set -- "$_amx_s" --no-ext-diff --no-textconv "$@";;
          *) set -- "$_amx_s" "$@";;
        esac;;
      fetch|pull)
        for _amx_k in ${_amx_x[@]+"${_amx_x[@]}"} "$@"; do
          case "$_amx_k" in
            --?*) _amx_c=${_amx_k#--}; _amx_c=${_amx_c%%=*}; case "recurse-submodules" in "$_amx_c"*) echo "git $1 $_amx_k is not run: a submodule's own configuration can name a program for a remote; fetch the submodule on its own" >&2; return 1;; esac;;
          esac;
        done;
        if [ -n "$_amx_u" ]; then
          for _amx_k in ${_amx_x[@]+"${_amx_x[@]}"} "$@"; do
            _amx_b=;
            case "$_amx_k" in
              --?*) _amx_c=${_amx_k#--}; _amx_c=${_amx_c%%=*}; case "all" in "$_amx_c"*) _amx_b=1;; esac; case "multiple" in "$_amx_c"*) _amx_b=1;; esac;;
            esac;
            if [ -n "$_amx_b" ]; then echo "git $1 $_amx_k is not run while this folder's configuration names an upload or receive program for a remote; name one remote" >&2; return 1; fi;
            for _amx_t in ${_amx_r[@]+"${_amx_r[@]}"}; do
              if [ "$_amx_k" = "$_amx_t" ]; then echo "git $1 of the group $_amx_k is not run while this folder's configuration names an upload or receive program for a remote; name one remote" >&2; return 1; fi;
            done;
          done;
        fi;
        _amx_s=$1; shift; _amx_g+=(-c fetch.all=false); set -- "$_amx_s" --no-recurse-submodules --upload-pack=git-upload-pack "$@";;
      ls-remote|clone) _amx_s=$1; shift; set -- "$_amx_s" --upload-pack=git-upload-pack "$@";;
      push) _amx_s=$1; shift; set -- "$_amx_s" --receive-pack=git-receive-pack "$@";;
    esac;
    set -- --no-pager ${_amx_g[@]+"${_amx_g[@]}"} "$@";
  fi;
"#
    };
}

/// `git` for an agent that has nothing else of the function: only the diff step, then plain `git`.
pub const GIT_FLAGS_FUNCTION_BODY: &str = concat!("() {\n", no_ext_diff_step!(), "  command git \"$@\"\n}");

/// `git`, for the same agents: a push or a command that makes commits first asks
/// `agentmux-bashwrap git-env`, which refuses what would skip the hooks (`git push --no-verify`) and
/// names the App's bot as the commit's author. Any other git command is plain `git`. It also has the diff step
/// of [`GIT_FLAGS_FUNCTION_BODY`].
pub const GIT_FUNCTION_BODY: &str = concat!(
    "() {\n",
    no_ext_diff_step!(),
    r#"  if [ -n "${AGENTMUX_GITHUB_BROKER:-}" ]; then
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
}"#
);
