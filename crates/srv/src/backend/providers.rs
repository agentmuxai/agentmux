// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Static provider registry — Rust equivalent of
//! `frontend/app/view/agent/providers/index.ts`.
//!
//! All string data is `&'static str` / `&'static [&'static str]` so lookups
//! are zero-allocation.  The registry is initialised once via `LazyLock` and
//! then read-only for the lifetime of the process.

use std::collections::HashMap;
use std::sync::LazyLock;

// ─── Controller type ─────────────────────────────────────────────────────────

/// How the provider process is managed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerType {
    /// A single long-running process; input is streamed to stdin.
    Persistent,
    /// A fresh subprocess is spawned for every turn; prior sessions are
    /// resumed via `resume_flag`.
    Subprocess,
    /// Agent Client Protocol (ACP): JSON-RPC 2.0 over stdio.
    /// Sessions are managed by the protocol — no resume flags needed.
    Acp,
    /// Provider-specific App Server protocol over a persistent JSONL stdio
    /// transport. Protocol semantics are selected by [`AppServerKind`].
    #[allow(dead_code)] // Off by default until SPEC_CODEX_APP_SERVER_FIRST_CLASS_PROVIDER_2026_09_12.md §15 Slice 6.
    AppServer,
}

/// Protocol spoken by a provider-neutral App Server controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppServerKind {
    Codex,
}

/// Optional App Server capability advertised by a provider. Presence does not
/// select the controller: `controller_type` remains the rollout switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppServerConfig {
    pub kind: AppServerKind,
    pub launch_args: &'static [&'static str],
    pub fallback_controller: ControllerType,
    pub fallback_output_format: &'static str,
}

// ─── ProviderConfig ──────────────────────────────────────────────────────────

/// All configuration needed to launch, authenticate, and stream output from
/// a provider CLI.
#[derive(Debug)]
pub struct ProviderConfig {
    /// Canonical provider identifier (e.g. `"claude"`).
    pub id: &'static str,
    /// Executable name on PATH (e.g. `"claude"`).
    pub cli_command: &'static str,
    /// Whether the provider keeps a persistent subprocess or spawns per turn.
    pub controller_type: ControllerType,
    /// Rich-client App Server capability, when the provider exposes one.
    /// Kept separate from `controller_type` so the transport can land behind a
    /// disabled-by-default rollout gate.
    pub app_server: Option<AppServerConfig>,
    /// Complete CLI args for a single-turn (subprocess) invocation.
    /// The user prompt is written to the process's stdin.
    pub launch_args: &'static [&'static str],
    /// CLI args for persistent (long-running) mode.
    /// `None` when `controller_type` is `Subprocess`.
    pub persistent_launch_args: Option<&'static [&'static str]>,
    /// Flag passed to resume a prior session, e.g. `"--resume"`.
    /// `None` when the provider does not support simple-flag resume.
    pub resume_flag: Option<&'static str>,
    /// JSON field name in the CLI's init event that carries the session /
    /// thread ID, e.g. `"session_id"` or `"thread_id"`.
    pub session_id_field: &'static str,
    /// Output format produced by the CLI in styled / streaming mode.
    pub styled_output_format: &'static str,
    // ── Auth isolation ───────────────────────────────────────────────────────
    /// Environment variable that redirects the provider's config / auth
    /// directory, e.g. `"CLAUDE_CONFIG_DIR"`.
    pub auth_config_dir_env_var: &'static str,
    /// Sub-directory name for this provider's auth/config dir, e.g. `"claude"`.
    /// Resolved under `shared/providers/<name>/` (the default, account-wide and
    /// instance-independent) and `shared/identities/<bundle>/<name>/` (per-identity)
    /// — see `DataPaths::provider_auth_dir` and `identity_dir`.
    pub auth_dir_name: &'static str,
    /// Sub-directory (relative to this provider's config/auth dir) that
    /// actually holds its native session/transcript history — e.g.
    /// Claude's `"projects"`, Codex's `"sessions"`, Gemini's `"history"`.
    /// `None` for providers with no simple directory-based history to
    /// isolate/link (ACP providers — openclaw/pi/copilot — expose history
    /// through the live protocol stream instead, not a fixed on-disk
    /// path). Per-provider, filesystem-verified table:
    /// `docs/specs/SPEC_UNIFIED_AGENT_HISTORY_STORE_2026-06-10.md` §2.1.
    /// Used by `identity_auth_dirs.rs` to redirect an isolated identity's
    /// history to the always-global
    /// `DataPaths::identity_history_dir` — reagentx P1 on PR #2605 caught
    /// that hardcoding `"projects"` there only worked for `claude`,
    /// silently leaving every other OAuth-class provider's history
    /// channel-local and unlinked.
    pub history_native_subdir: Option<&'static str>,
    /// Extra environment variables required for auth isolation.
    /// Each entry is a `(key, value)` pair.
    pub auth_extra_env: &'static [(&'static str, &'static str)],
    /// Environment variables that must be *unset* before launching the CLI
    /// (guards against nested-session issues, etc.).
    pub unset_env: &'static [&'static str],
    // ── npm install ──────────────────────────────────────────────────────────
    /// npm package name used for local installation, e.g.
    /// `"@anthropic-ai/claude-code"`.
    pub npm_package: &'static str,
    /// Version string passed to `npm install`, e.g. `"latest"` or `"0.116.0"`.
    pub pinned_version: &'static str,
    // ── Model vendor (harness vs. model vendor decoupling) ──────────────────
    /// Environment variable this harness reads to redirect its model vendor
    /// backend (e.g. Claude Code's `ANTHROPIC_BASE_URL`, pointing it at a
    /// proxy/Bedrock/OpenRouter instead of Anthropic's default endpoint).
    /// `None` when a harness isn't confirmed to support redirection — the
    /// harness (CLI driving the session) and the model vendor (LLM backend
    /// actually serving responses) are independent dimensions; this is the
    /// declared capability side of that split. Set only where independently
    /// verified, not guessed. See docs/specs for the harness/vendor concept.
    pub base_url_env_var: Option<&'static str>,
    /// Intelligence model vendors this harness talks to, most-default-first
    /// (e.g. claude → `&["anthropic"]`, openclaw → `&["openai", "anthropic",
    /// "google"]` since it's model-agnostic). Purely descriptive/display
    /// data — drives the dual-icon vendor badge and the picker's default
    /// vendor inference (`resolveEffectiveVendor` on the frontend); does not
    /// gate anything at spawn time. The one thing that DOES gate spawn-time
    /// behavior is `base_url_env_var` above — a provider can be redirected
    /// to a custom endpoint independent of whether that endpoint's vendor
    /// is even listed here.
    pub supported_vendors: &'static [&'static str],
    /// Path (relative to the agent's working directory) this provider
    /// natively auto-discovers its startup instructions from — e.g.
    /// `"CLAUDE.md"`, `"AGENTS.md"`, `".pi/APPEND_SYSTEM.md"`. `None` when
    /// no native file-based convention is confirmed to exist (currently
    /// only `kimi`) — `build_config_files` skips writing the instructions
    /// file entirely in that case rather than writing inert content nobody
    /// reads. Set only where independently verified against the provider's
    /// own docs, not guessed — same discipline as `base_url_env_var` above.
    /// See docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §2.
    pub startup_instructions_filename: Option<&'static str>,
    /// Every path (relative to the agent's working directory) this provider
    /// **reads** as project instructions — which is not the same question as
    /// `startup_instructions_filename` above, and is usually a superset of it.
    ///
    /// That field names the one file AgentMux *writes*: the canonical target
    /// it picked per provider. This one names what the provider will pick up
    /// on its own, whether AgentMux put it there or the repository did.
    /// Copilot is the clearest case — it reads
    /// `.github/copilot-instructions.md`, `CLAUDE.md` and `GEMINI.md` besides
    /// the `AGENTS.md` chosen as the write target — and pi reads
    /// `.pi/SYSTEM.md` as well as the `.pi/APPEND_SYSTEM.md` written to.
    ///
    /// Phase 3 of `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md`
    /// resolves this set to answer "what will this agent actually read", which
    /// the write target alone cannot (Codex, PR #3144). Nothing writes these
    /// paths — tracking is read-only by design, and the ownership protection
    /// in `SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md` still governs
    /// the one file AgentMux does write.
    ///
    /// **Same evidence rule as the field above: listed only where verified
    /// against the provider's own docs, never guessed.** Where the only
    /// confirmed source is the write target, that is the whole list — an
    /// under-complete list means Phase 3 shows the operator less than the
    /// truth, which is recoverable; a guessed one means it shows them
    /// something false, which is not.
    pub native_instruction_sources: &'static [&'static str],
    /// Directories a provider scans for instruction files, for the providers
    /// whose convention is a pattern rather than a fixed path.
    ///
    /// Copilot's `.github/instructions/**/*.instructions.md` is the reason
    /// this exists: those are path-scoped instructions that genuinely reach
    /// the agent, so a resolver reading only `native_instruction_sources`
    /// above would produce an incomplete answer for any repository using them
    /// (Codex, PR #3154), verified in
    /// `docs/archive/research-cli-context-files-2026-04-23-copilot-verification.md`
    /// claim 3.
    ///
    /// Structured rather than a glob string, because there is no glob engine
    /// in the tree and one dependency for one pattern shape is a poor trade.
    /// It also forces the two things a glob leaves implicit — whether to
    /// descend, and what actually qualifies — to be stated.
    ///
    /// Same evidence rule as the two fields above.
    pub native_instruction_dirs: &'static [InstructionDirScan],
}

/// A directory a provider reads instruction files out of.
#[derive(Debug, Clone, Copy)]
pub struct InstructionDirScan {
    /// Working-directory-relative, e.g. `".github/instructions"`.
    pub dir: &'static str,
    /// Only files ending in this qualify, e.g. `".instructions.md"`.
    pub suffix: &'static str,
    /// Whether subdirectories are scanned too — Copilot's `**` says yes.
    pub recursive: bool,
}

/// The model a freshly opened pane of this provider starts on, or `None` for a
/// provider whose model the runtime menu does not wire (it passes no `--model`).
///
/// Mirrors the `default: true` row of the frontend catalog
/// (`frontend/app/view/agent/providers/catalog.ts`), which is where the menu's
/// default comes from; `providers/runtime-defaults-consistency.test.ts` fails when the two disagree.
/// srv needs its own copy because `agent.open` builds a pane without going
/// through the frontend, and a pane opened with no `--model` runs on the CLI's
/// own default while its menu reads this one.
pub fn default_model_for(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        "claude" => Some("sonnet"),
        "codex" => Some("gpt-5.5"),
        // One of the ids `agy models` lists (agy 1.1.11).
        "antigravity" => Some("gemini-3.8-flash-medium"),
        _ => None,
    }
}

impl ProviderConfig {
    /// Return the controller type as the string used in block metadata.
    pub fn controller_type_str(&self) -> &'static str {
        match self.controller_type {
            ControllerType::Persistent => "persistent",
            ControllerType::Subprocess => "subprocess",
            ControllerType::Acp => "acp",
            ControllerType::AppServer => "app-server",
        }
    }

    /// Return the provider-specific continuation argv strategy stored in pane
    /// metadata. Codex uses an `exec resume` subcommand; existing providers
    /// append their configured flag.
    pub fn resume_strategy_str(&self) -> &'static str {
        if self.id == "codex" {
            "codex-exec"
        } else if self.resume_flag.is_some() {
            "flag"
        } else {
            "none"
        }
    }

    /// Packages installed next to `npm_package`, in the same `npm install` and
    /// the same pinned dir: `(name, version)`. Pi's adapter (`pi-acp`) runs
    /// the `pi` CLI, which is its own package (`@earendil-works/...`; the old
    /// `@mariozechner/...` name stops at 0.73.1).
    pub fn companion_npm_packages(&self) -> &'static [(&'static str, &'static str)] {
        match self.id {
            "pi" => &[("@earendil-works/pi-coding-agent", "1.0.4")],
            _ => &[],
        }
    }

    /// Every `name@version` an install of this provider passes to npm: the
    /// pinned package (at `pin`, the effective pin) and its companions.
    pub fn npm_install_specs(&self, pin: &str) -> Vec<String> {
        let main = if pin.is_empty() {
            self.npm_package.to_string()
        } else {
            format!("{}@{}", self.npm_package, pin)
        };
        std::iter::once(main)
            .chain(self.companion_npm_packages().iter().map(|(n, v)| format!("{n}@{v}")))
            .collect()
    }

    /// The flag that passes the prompt as an argument, for a CLI that doesn't
    /// read it from stdin: `agy` ignores stdin and needs `-p <prompt>`
    /// (SPEC_ANTIGRAVITY_HARNESS_REAL_CLI_2026_10_06.md). `None`, the prompt
    /// goes on stdin, for every other provider. Stored in pane meta as
    /// `agent:prompt_arg_flag`; the frontend's `promptArgFlag` mirrors it.
    pub fn prompt_arg_flag(&self) -> Option<&'static str> {
        (self.id == "antigravity").then_some("-p")
    }
}

// ─── Provider definitions ────────────────────────────────────────────────────
//
// One file per provider under providers/, so adding a provider adds a file.

mod claude;
mod codex;
mod gemini;
mod qwen;
mod kimi;
mod openclaw;
mod pi;
mod muxcode;
mod copilot;
mod antigravity;
mod cline;
mod kilo;
mod opencode;

use claude::CLAUDE;
use codex::CODEX;
use gemini::GEMINI;
use qwen::QWEN;
use kimi::KIMI;
use openclaw::OPENCLAW;
use pi::PI;
use muxcode::MUX_CODE;
use copilot::COPILOT;
use antigravity::ANTIGRAVITY;
use cline::CLINE;
use kilo::KILO;
use opencode::OPENCODE;

// ─── Static registry ─────────────────────────────────────────────────────────

static REGISTRY: LazyLock<HashMap<&'static str, &'static ProviderConfig>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    m.insert(CLAUDE.id, &CLAUDE);
    m.insert(CODEX.id, &CODEX);
    m.insert(GEMINI.id, &GEMINI);
    m.insert(QWEN.id, &QWEN);
    m.insert(KIMI.id, &KIMI);
    m.insert(OPENCLAW.id, &OPENCLAW);
    m.insert(PI.id, &PI);
    m.insert(COPILOT.id, &COPILOT);
    m.insert(MUX_CODE.id, &MUX_CODE);
    m.insert(ANTIGRAVITY.id, &ANTIGRAVITY);
    m.insert(KILO.id, &KILO);
    m.insert(CLINE.id, &CLINE);
    m.insert(OPENCODE.id, &OPENCODE);
    m
});

// Aliases for provider IDs from older databases or alternate naming.
static ALIASES: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    m.insert("claude-code", "claude");
    m.insert("claude_code", "claude");
    m.insert("codex-cli", "codex");
    m.insert("gemini-cli", "gemini");
    m.insert("qwen-code", "qwen");
    m.insert("qwen3-coder", "qwen");
    m.insert("kimi-cli", "kimi");
    m.insert("kimi_code", "kimi");
    m.insert("openclaw-cli", "openclaw");
    m.insert("open-claw", "openclaw");
    m.insert("copilot-cli", "copilot");
    m.insert("github-copilot", "copilot");
    m.insert("copilot_cli", "copilot");
    m.insert("mux-code", "muxcode");
    m.insert("mux_code", "muxcode");
    m.insert("agy", "antigravity");
    m.insert("antigravity-cli", "antigravity");
    m.insert("antigravity_cli", "antigravity");
    m.insert("kilocode", "kilo");
    m.insert("kilo-code", "kilo");
    m.insert("cline-cli", "cline");
    m.insert("opencode-ai", "opencode");
    m.insert("open-code", "opencode");
    m
});

// ─── Public API ──────────────────────────────────────────────────────────────

/// Resolve a provider alias to its canonical ID.
///
/// Returns `id` unchanged if it is not a known alias.
pub fn resolve_provider_alias(id: &str) -> &'static str {
    ALIASES.get(id).copied().unwrap_or_else(|| {
        // If the id itself is a canonical key return the interned key, otherwise
        // return a best-effort static ref. The caller should treat the return
        // value as a lookup key only.
        REGISTRY
            .get_key_value(id)
            .map(|(k, _)| *k)
            .unwrap_or("") // unknown — get_provider will return None
    })
}

/// Where a PATH-only provider's own installer puts its CLI, for the case
/// where that folder is not on this process's PATH yet. Antigravity's
/// installer adds its folder to the user's PATH, but an AgentMux that is
/// already running keeps the PATH it started with, so without this the CLI
/// would read as missing until AgentMux restarts.
pub fn known_install_paths(provider_id: &str) -> Vec<std::path::PathBuf> {
    match resolve_provider_alias(provider_id) {
        "antigravity" => antigravity_install_paths(
            std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from),
            dirs::home_dir(),
        ),
        _ => Vec::new(),
    }
}

/// `%LOCALAPPDATA%\agy\bin\agy.exe` on Windows and `~/.local/bin/agy`
/// elsewhere: the default `TARGET_DIR` of Google's installers,
/// https://antigravity.google/cli/install.ps1 and the `.sh` one beside it.
fn antigravity_install_paths(
    local_app_data: Option<std::path::PathBuf>,
    home: Option<std::path::PathBuf>,
) -> Vec<std::path::PathBuf> {
    if cfg!(windows) {
        local_app_data
            .map(|d| d.join("agy").join("bin").join("agy.exe"))
            .into_iter()
            .collect()
    } else {
        home.map(|h| h.join(".local").join("bin").join("agy"))
            .into_iter()
            .collect()
    }
}

/// The `pi` installed next to `cmd` when `cmd` is pi-acp, for
/// `PI_ACP_PI_COMMAND` (blockcontroller/acp.rs) and the auth check (`run_auth_check`). Without it pi-acp runs whatever `pi` is on PATH, often
/// none (the managed install's `.bin` isn't on PATH) or an older one.
/// SPEC_PI_HARNESS_VIA_PI_ACP_2026_10_06.md.
pub fn pi_beside_pi_acp(cmd: &str) -> Option<String> {
    if !is_pi_acp(cmd) {
        return None;
    }
    let path = std::path::Path::new(cmd);
    let dir = path.parent()?;
    let names: &[&str] = if cfg!(windows) { &["pi.cmd", "pi.exe", "pi"] } else { &["pi"] };
    names
        .iter()
        .map(|n| dir.join(n))
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}

/// Whether `cmd` (a path or bare name) is the pi-acp adapter.
pub fn is_pi_acp(cmd: &str) -> bool {
    std::path::Path::new(cmd)
        .file_stem()
        .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case("pi-acp"))
}

/// Look up a provider by canonical ID or alias.
///
/// Returns `None` when the ID (and any resolved alias) does not match a known
/// provider.
pub fn get_provider(id: &str) -> Option<&'static ProviderConfig> {
    // Direct lookup first.
    if let Some(p) = REGISTRY.get(id) {
        return Some(p);
    }
    // Fall back to alias resolution.
    let canonical = ALIASES.get(id).copied()?;
    REGISTRY.get(canonical).copied()
}

/// Every registered provider, for callers that must act on all of them
/// rather than look one up by id — e.g. a migration backfilling
/// conversation history for every provider that has a linkable native
/// history directory ([`ProviderConfig::history_native_subdir`]), so a
/// future provider gains that behavior automatically instead of needing
/// a follow-up patch to an id list maintained elsewhere.
pub fn all_providers() -> impl Iterator<Item = &'static ProviderConfig> {
    REGISTRY.values().copied()
}

/// True when `dir` resolves to `provider`'s literal ambient home directory
/// (e.g. `~/.claude` for Claude Code) rather than an AgentMux-isolated dir.
/// Used to block identity bindings/spawns from ever pointing a spawned
/// agent at the operator's own global CLI login — a real, currently-live
/// instance of exactly this was found in this repo's own data (an account's
/// `secret_ref.dir` set to the literal ambient path — see
/// `docs/status/STATUS_IDENTITY_ISOLATION_GATE_NOT_ENFORCING_2026_08_20.md`
/// §8), which nothing in the codebase previously validated against. See
/// `docs/specs/SPEC_BLOCK_AMBIENT_HOME_DIR_IDENTITY_BINDING_2026_08_25.md`.
///
/// Canonicalizes both sides when possible (defeats `..`, symlink, and
/// Windows `\\?\`-prefix tricks — same approach `identity::cleanup`'s
/// containment check already uses); falls back to a normalized lexical
/// comparison when either path doesn't exist yet on disk (e.g. validating a
/// not-yet-materialized dir at account-creation time), so a not-yet-created
/// ambient path can't slip past the guard just by not existing at check
/// time.
pub fn is_provider_ambient_home_dir(provider: &ProviderConfig, dir: &str) -> bool {
    let ambient = crate::backend::base::get_home_dir().join(format!(".{}", provider.auth_dir_name));
    paths_resolve_to_same_dir(&ambient, dir)
}

/// Placeholder written by `seed_claude_md_placeholder_if_missing` — explains
/// itself so an operator who opens the file isn't confused by an unexplained
/// empty one.
const CLAUDE_MD_ISOLATION_PLACEHOLDER: &str = "<!--\n\
AgentMux: intentionally empty.\n\
\n\
This is an isolated Claude Code config directory (CLAUDE_CONFIG_DIR),\n\
separate from your personal ~/.claude/CLAUDE.md, so this agent never\n\
silently inherits your personal global instructions. To give every\n\
agent shared instructions, use Memory -> Global instead --\n\
those compose into this agent's own project-level CLAUDE.md at launch,\n\
not this file. See SPEC_ISOLATE_HOST_CLAUDE_MD_2026_08_31.md.\n\
-->\n";

/// Seeds an empty placeholder `CLAUDE.md` into an isolated Claude Code
/// `CLAUDE_CONFIG_DIR` the first time it's used, so Claude Code CLI's own
/// user-level `CLAUDE.md` discovery always finds *something* there instead
/// of silently falling through to the operator's real
/// `$HOME/.claude/CLAUDE.md`. `CLAUDE_CONFIG_DIR` relocates credential/
/// session/project storage but AgentMux never wrote a `CLAUDE.md` into the
/// relocated dir — verified live on a real machine: 18+ isolated `claude`
/// config dirs, none with a `CLAUDE.md`, and a session whose
/// `CLAUDE_CONFIG_DIR` was a genuinely separate (non-ambient) dir still
/// received the real host file's contents. See
/// `docs/specs/SPEC_ISOLATE_HOST_CLAUDE_MD_2026_08_31.md`.
///
/// Scoped to the `claude` provider only (`auth_dir_name == "claude"`) —
/// this is a Claude Code CLI-specific fallback behavior; whether any other
/// provider's CLI has an equivalent gap is unverified. Never overwrites an
/// existing `CLAUDE.md` — Global Bundle (or anything else) may have
/// legitimately placed real content at that path. Returns `Ok(true)` only
/// when it actually wrote the placeholder.
pub fn seed_claude_md_placeholder_if_missing(
    provider: &ProviderConfig,
    config_dir: &str,
) -> std::io::Result<bool> {
    if provider.auth_dir_name != "claude" {
        return Ok(false);
    }
    // Codex P2 on PR #2854: a blank config_dir would resolve
    // `Path::new("").join("CLAUDE.md")` relative to the server process's
    // own CWD (`create_dir_all("")` succeeds, silently no-op) instead of
    // erroring — writing an unrelated file while leaving the actual
    // (empty-string) CLAUDE_CONFIG_DIR still exposed to ambient fallback.
    // Reject before touching the filesystem so the caller's fail-closed
    // handling (both call sites now block spawn on any Err here) covers
    // this case too, rather than silently succeeding at nothing.
    if config_dir.trim().is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "config_dir is empty",
        ));
    }
    let path = std::path::Path::new(config_dir).join("CLAUDE.md");
    if path.exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(config_dir)?;
    std::fs::write(&path, CLAUDE_MD_ISOLATION_PLACEHOLDER)?;
    Ok(true)
}

/// Days Claude Code keeps session transcripts in an AgentMux-owned config dir
/// (SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md §4.1).
pub const CLAUDE_TRANSCRIPT_RETENTION_DAYS: u32 = 180;

/// Seeds `cleanupPeriodDays` into an isolated Claude Code config dir's
/// `settings.json` when it isn't set. Claude Code's default is 30 days, and
/// its startup sweep deletes transcripts under `<config_dir>/projects`, which
/// AgentMux junctions to the account's shared history — so any channel that
/// launched an agent without this setting deleted every channel's month-old
/// conversations, leaving `--resume` nothing to find.
///
/// Claude only. Leaves an explicit `cleanupPeriodDays` alone, and never
/// rewrites a `settings.json` it can't parse as a JSON object. Callers must
/// only pass a dir already confirmed not to be the operator's ambient home.
/// Returns `Ok(true)` only when it wrote the setting.
pub fn seed_transcript_retention_if_missing(
    provider: &ProviderConfig,
    config_dir: &str,
) -> std::io::Result<bool> {
    if provider.auth_dir_name != "claude" {
        return Ok(false);
    }
    if config_dir.trim().is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "config_dir is empty",
        ));
    }
    let path = std::path::Path::new(config_dir).join("settings.json");
    let mut settings = match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(serde_json::Value::Object(map)) => map,
            _ => return Ok(false),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::Map::new(),
        Err(e) => return Err(e),
    };
    if settings.contains_key("cleanupPeriodDays") {
        return Ok(false);
    }
    settings.insert(
        "cleanupPeriodDays".to_string(),
        serde_json::json!(CLAUDE_TRANSCRIPT_RETENTION_DAYS),
    );
    let body = serde_json::to_string_pretty(&serde_json::Value::Object(settings))
        .map_err(std::io::Error::other)?;
    std::fs::create_dir_all(config_dir)?;
    let tmp = path.with_extension("json.agentmux-tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, &path)?;
    Ok(true)
}

/// `provider`'s DEFAULT auth/config dir: `~/.agentmux/shared/providers/<auth_dir_name>/`
/// — account-wide and version/channel-independent, so one login is shared by
/// every instance (the structural fix for the per-channel validate-spin
/// regression). A per-account auth dir (identity_handlers) still wins
/// for explicit multi-account. Shared by agent open and `provider.ensureauthdir`.
pub fn default_auth_dir(provider: &ProviderConfig) -> Result<String, String> {
    if let Some(paths) = agentmux_common::DataPaths::from_env() {
        return Ok(paths.provider_auth_dir(provider.auth_dir_name).to_string_lossy().into_owned());
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "cannot determine home directory".to_string())?;
    Ok(format!("{}/.agentmux/shared/providers/{}", home, provider.auth_dir_name))
}

/// Prepare an isolated provider auth/config directory for use as
/// `CLAUDE_CONFIG_DIR` (or a provider's equivalent): create it, then apply
/// every isolation guarantee that directory needs before a CLI is pointed
/// at it.
///
/// Exists so callers cannot obtain a usable auth dir WITHOUT its isolation
/// guarantees — the spawn paths call this instead of `create_dir_all` +
/// a separately-skippable seed step. Deleting the isolation from a spawn
/// path now means deleting the directory preparation too, which fails
/// loudly instead of silently reopening the leak. See
/// `docs/specs/SPEC_ISOLATE_HOST_CLAUDE_MD_2026_08_31.md` and
/// `docs/reports/REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md`
/// (the live three-arm experiment proving the leak is real and this closes
/// it).
pub fn prepare_provider_auth_dir(
    provider: &ProviderConfig,
    auth_dir: &str,
) -> std::io::Result<()> {
    if auth_dir.trim().is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "auth_dir is empty",
        ));
    }
    std::fs::create_dir_all(auth_dir)?;
    seed_claude_md_placeholder_if_missing(provider, auth_dir)?;
    if let Err(e) = seed_transcript_retention_if_missing(provider, auth_dir) {
        tracing::warn!(auth_dir, error = %e, "could not set Claude transcript retention");
    }
    Ok(())
}

/// The actual comparison, split out with the reference side pre-resolved
/// (not calling `get_home_dir()` internally) so it's directly testable
/// against a tempdir instead of the real `$HOME`/`%USERPROFILE%` — same
/// "inject the path, don't resolve it internally" pattern
/// `read_claude_global_config` already uses (`agent_handlers/bundle.rs`).
fn paths_resolve_to_same_dir(reference: &std::path::Path, candidate: &str) -> bool {
    let candidate_path = std::path::Path::new(candidate);

    if let (Ok(canon_reference), Ok(canon_candidate)) =
        (std::fs::canonicalize(reference), std::fs::canonicalize(candidate_path))
    {
        return canon_reference == canon_candidate;
    }

    normalize_path_lexically(reference) == normalize_path_lexically(candidate_path)
}

/// Lexically normalizes a path with NO filesystem access (unlike
/// `canonicalize`, safe to call on a path that doesn't exist). Collapses
/// `.` and `..` components (codex P1, PR #2802: without this, a not-yet-
/// created dir like `$HOME/.claude/.` compared unequal to `$HOME/.claude`
/// under plain string comparison, bypassing the guard entirely on a fresh
/// machine — a `..` segment is normalized the same way for the same
/// reason). Never pops past a root/prefix/leading `..` — a leading `..`
/// with nothing to cancel stays literal, standard lexical-normalization
/// semantics.
fn normalize_path_lexically(p: &std::path::Path) -> String {
    use std::path::Component;

    let mut normalized: Vec<Component> = Vec::new();
    for component in p.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(normalized.last(), Some(Component::Normal(_))) {
                    normalized.pop();
                } else {
                    normalized.push(component);
                }
            }
            other => normalized.push(other),
        }
    }
    let joined: std::path::PathBuf = normalized.into_iter().collect();
    let as_str = joined.to_string_lossy().replace('\\', "/");
    let trimmed = as_str.trim_end_matches('/');

    // codex P2, PR #2802: case-fold only on platforms whose default
    // filesystem is case-insensitive (Windows, macOS). Unconditionally
    // lowercasing made a not-yet-created `$HOME/.CLAUDE` compare equal to
    // `$HOME/.claude` on Linux (case-sensitive ext4) — once both existed,
    // canonicalize would correctly tell them apart, so the guard's
    // verdict depended on creation order instead of being deterministic.
    if cfg!(target_os = "windows") || cfg!(target_os = "macos") {
        trimmed.to_lowercase()
    } else {
        trimmed.to_string()
    }
}

/// Whether `path` matches ANY registered provider's
/// `startup_instructions_filename` — used by the "click Launch" RPC write
/// path (`editor_handlers.rs`'s `WriteAgentConfig` handler), which receives
/// an already-built file list from the frontend with no accompanying
/// provider ID, so it can't otherwise tell "this is the startup
/// instructions file" from "this is some other config file" without either
/// re-deriving the mapping itself (drift risk) or checking membership here
/// (single source of truth). See
/// docs/specs/SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md §3.
pub fn is_known_startup_instructions_filename(path: &str) -> bool {
    REGISTRY
        .values()
        .any(|p| p.startup_instructions_filename == Some(path))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;

#[cfg(test)]
mod native_instruction_sources_tests;
