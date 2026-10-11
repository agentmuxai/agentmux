// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Armory Bundle Format (ABF) importer — Phase 2 of
//! `docs/specs/SPEC_ABF_V0_1_SINGLE_FILE_AND_IMPORTER_2026_08_01.md`.
//! Inverse of `bundle_export.rs`: takes a bundle's files (either a raw
//! `[{path, content}]` list or an unpacked `.abf` zip archive) and
//! validates + parses them into data ready for the `bundle.import` RPC
//! handler to write to the Store.
//!
//! Pure functions only — no I/O, no Store access, no account resolution
//! (account resolution needs `db_accounts`, which only the RPC handler has
//! access to; see the spec's §4.5). Mirrors `bundle_export.rs`'s shape and
//! quality bar deliberately: the same "warn, don't silently drop" and
//! "reject the whole thing on structural failure, but never partially
//! import on a per-entry problem" philosophy, since the two modules are
//! meant to round-trip against each other.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::bundle_export::sanitize_context_relative_path;

/// Bounds how many warnings a parse call accumulates, and how long each
/// individual warning string may be — Phase 3 spec
/// (`SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md`) §3.1, round 11: the cap must
/// live at the point warnings are actually PRODUCED, not only at a later
/// RPC-response-serialization boundary, or a hostile archive can still
/// force the parser itself to build an unbounded `Vec<String>` in memory
/// before any cap ever runs. `unbounded()` preserves today's existing
/// `bundle.import` route's real, already-shipped behavior exactly — this is
/// a capability the parser gains, not a behavior change forced onto Phase 2's
/// existing caller.
#[derive(Debug, Clone, Copy)]
pub struct WarningBudget {
    max_count: Option<usize>,
    max_len: Option<usize>,
}

impl WarningBudget {
    pub fn unbounded() -> Self {
        Self { max_count: None, max_len: None }
    }

    pub fn bounded(max_count: usize, max_len: usize) -> Self {
        Self { max_count: Some(max_count), max_len: Some(max_len) }
    }
}

/// Accumulates warnings against a [`WarningBudget`]. Exposes `.push(String)`
/// so every existing `warnings.push(format!(...))` call site in this module
/// keeps working unchanged after its binding's type changes from
/// `Vec<String>` to this — only construction (`WarningSink::new`) and
/// extraction (`.into_vec()`) differ.
#[derive(Debug, Clone)]
struct WarningSink {
    budget: WarningBudget,
    warnings: Vec<String>,
    dropped: usize,
}

impl WarningSink {
    fn new(budget: WarningBudget) -> Self {
        Self { budget, warnings: Vec::new(), dropped: 0 }
    }

    fn push(&mut self, message: String) {
        if let Some(max_count) = self.budget.max_count {
            if self.warnings.len() >= max_count {
                self.dropped += 1;
                return;
            }
        }
        let message = match self.budget.max_len {
            Some(max_len) => truncate_display(&message, max_len),
            None => message,
        };
        self.warnings.push(message);
    }

    fn into_vec(mut self) -> Vec<String> {
        if self.dropped > 0 {
            self.warnings.push(format!("... {} more warning(s) not shown", self.dropped));
        }
        self.warnings
    }
}

/// Fixed-character truncation shared by every bounded-display projection
/// this module (and its RPC callers, Phase 3 spec §3.1/§3.2) define —
/// stated once so `preview` and `commit` always apply IDENTICAL bounds to
/// the equivalent field, rather than independent per-endpoint copies
/// drifting apart (the exact class of gap codex found and re-found across
/// rounds 9 and 12).
pub fn truncate_display(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let mut truncated: String = s.chars().take(max_chars).collect();
        truncated.push_str("...");
        truncated
    }
}

/// Cap on `instructions_preview` (Phase 3 spec §3.1, round 5) — generous for
/// a glance-and-decide preview; bounds worst-case JSON-escaped response size
/// to roughly 300 KB rather than the ~300 MB a maximally-adversarial,
/// unbounded `instructions` string could otherwise force.
pub const MAX_INSTRUCTIONS_PREVIEW_CHARS: usize = 50_000;

/// Fixed-character display cap shared by every "meant to be short" bounded
/// field this spec defines: skill `description`/`slug` (rounds 10/12),
/// requirement `id`/`provider`/`env` (round 11), context-file
/// `display_path` (round 13), and bundle `description` (round 12 self-audit).
pub const MAX_DISPLAY_FIELD_CHARS: usize = 300;

/// Fixed-character display cap for an MCP server's projected `name`/
/// `command` (Phase 3 spec §3.1, round 7) — smaller than
/// [`MAX_DISPLAY_FIELD_CHARS`] since these are meant to be short
/// identifiers/executable names, not free-form text.
pub const MAX_MCP_DISPLAY_FIELD_CHARS: usize = 200;

/// Bounds `instructions_preview` for the RPC response — returns
/// `(preview, truncated, total_chars)`. The full, untruncated `instructions`
/// value is unaffected; this only bounds what's echoed back for display
/// (Phase 3 spec §3.1, rounds 5 and 8).
pub fn bounded_instructions_preview(instructions: &str) -> (String, bool, usize) {
    let total_chars = instructions.chars().count();
    if total_chars <= MAX_INSTRUCTIONS_PREVIEW_CHARS {
        (instructions.to_string(), false, total_chars)
    } else {
        (instructions.chars().take(MAX_INSTRUCTIONS_PREVIEW_CHARS).collect(), true, total_chars)
    }
}

/// Bounded `{name, command}` projection of an MCP server's full `config`
/// (Phase 3 spec §3.1, round 7) — the full `config` is never returned in
/// preview/commit responses, only this small, defensively-extracted
/// projection. Falls back to `null` for either field when absent or not a
/// string, since MCP JSON has no required shape (§3.0).
pub fn mcp_server_display(config: &Value) -> Value {
    let field = |key: &str| -> Value {
        match config.get(key).and_then(|v| v.as_str()) {
            Some(s) => Value::String(truncate_display(s, MAX_MCP_DISPLAY_FIELD_CHARS)),
            None => Value::Null,
        }
    };
    serde_json::json!({ "name": field("name"), "command": field("command") })
}

/// Every parsed skill slug that appears more than once across the WHOLE
/// bundle — the `"duplicate_in_bundle"` collision pass (Phase 3 spec §3.1,
/// codex P1 round 2), computed independently of any external global-catalog
/// state so it's pure and directly testable.
pub fn duplicate_in_bundle_slugs(skills: &[ParsedSkill]) -> HashSet<String> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut dupes: HashSet<String> = HashSet::new();
    for skill in skills {
        if !seen.insert(skill.slug.as_str()) {
            dupes.insert(skill.slug.clone());
        }
    }
    dupes
}

/// `"none"` / `"name_conflict"` / `"duplicate_in_bundle"` — Phase 3 spec
/// §3.1's two-pass skill collision classification. `global_slugs` is
/// whatever the caller already fetched from `skill.catalog.list`'s
/// underlying `skill_list_global()` (Store access lives in the RPC
/// handler, not here — this stays a pure function so it's testable with a
/// seeded fake global-skill list, per the spec's own §6 testing notes).
/// Pass 1 (global catalog) takes priority over pass 2 (intra-bundle
/// duplicate) when both apply, matching §3.1's stated precedence.
pub fn classify_skill_collision(slug: &str, global_slugs: &HashSet<String>, in_bundle_dupes: &HashSet<String>) -> &'static str {
    if global_slugs.contains(slug) {
        "name_conflict"
    } else if in_bundle_dupes.contains(slug) {
        "duplicate_in_bundle"
    } else {
        "none"
    }
}

/// One file read from an import source (zip or raw list), path relative
/// to the bundle root. Mirrors `bundle_export::BundleExportFile`.
#[derive(Debug, Clone)]
pub struct BundleImportFile {
    pub path: String,
    pub content: String,
}

/// A skill parsed out of a `skills/<slug>/SKILL.md` file — the inverse of
/// `agent_config::render_skill_md`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ParsedSkill {
    /// The exact `components.skills` directory reference that produced this
    /// entry (Phase 3 spec §3.0) — a stable, always-unique selection key,
    /// independent of `slug` (which two entries can share; see the
    /// `"duplicate_in_bundle"` collision case). Never displayed truncated;
    /// only used to match a preview row to a commit selection.
    pub source_dir: String,
    /// The slug from SKILL.md's own `name` field (Agent Skills spec:
    /// must match the parent directory) — reused as both the imported
    /// skill's display name and its `trigger`. Agent-skill-type skills
    /// don't materialize a slash-command file from `trigger` (see
    /// `agent_config.rs`'s `buildConfigFiles`), so this is a safe,
    /// deterministic default rather than a meaningful distinct value.
    pub slug: String,
    pub description: String,
    pub content: String,
}

/// An MCP server parsed out of a `components.mcpServers` reference — Phase
/// 3 spec §3.0. `source_path` is the stable, always-unique selection key
/// (the manifest path reference, distinct from whatever `"name"` field
/// happens to appear inside `config`'s arbitrary JSON, which has no
/// uniqueness guarantee and isn't even required to be present).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ParsedMcpServer {
    pub source_path: String,
    pub config: Value,
}

/// One `accounts/requirements.json` entry — mirrors the shape
/// `bundle_export.rs` writes (research report §5.3).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountRequirement {
    pub id: String,
    /// ABF v0.2 (SPEC_ABF_V0_2_PROVIDER_AWARE_COMPONENTS_AND_NATIVE_MEMORY_
    /// 2026_08_10.md §2.1): the wire key is `credentialProvider`, renamed
    /// from the ambiguous v0.1 `provider` (which collided with the
    /// unrelated harness/model-vendor "provider" concept components.
    /// instructions now uses — §2.2). `alias` keeps a v0.1-produced bundle
    /// (still carrying the old key) importing unchanged; new exports only
    /// ever write `credentialProvider`. The Rust field itself keeps its
    /// name — every internal caller (account-requirement resolution,
    /// `db_accounts.provider` matching) already reads it as "which
    /// credential service", which was always accurate; only the wire
    /// format needed disambiguating from the newer, unrelated sense.
    #[serde(rename = "credentialProvider", alias = "provider")]
    pub provider: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub env: String,
    #[serde(default)]
    pub optional: bool,
}

/// One `instructions/context/*` file, in the `[{path, content}]` shape
/// `db_bundles.context_files` stores.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ImportedContextFile {
    /// 0-based index within this parse's `context_files` list (Phase 3
    /// spec §3.1, round 13) — the stable selection key `bundle.import.commit`'s
    /// `include_context_files` uses. Deterministic and reusable across
    /// `preview`/`commit` because `expected_content_digest` already
    /// guarantees both calls parse identical content, so the same index
    /// always means the same entry. Never used for display.
    pub id: usize,
    pub path: String,
    pub content: String,
}

/// One `components.projectInstructions` entry, read back out of the bundle.
///
/// Phase 3 of `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md`. This is
/// a **record of what the SOURCE agent was reading**, not content to install:
/// no import path writes these files, and `owner` is the field that says why
/// (a `foreign` entry belongs to the source repository, and writing it into
/// the importing one is exactly what
/// `SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md` exists to prevent).
///
/// Carried through the parser rather than being reduced to a warning because
/// "surfaced" has to mean readable to be worth anything: `agent.
/// project_instructions` scans the IMPORTING agent's working directory, so it
/// is structurally unable to show what the source machine had (Codex P1,
/// PR #3163). Without this the import flow could tell you the component
/// existed and nothing more.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ImportedProjectInstruction {
    /// Working-directory-relative path on the SOURCE machine, e.g. `CLAUDE.md`.
    pub path: String,
    /// Where the snapshot lives inside the bundle (`instructions/project/...`).
    pub file: String,
    /// SHA-256 the exporter recorded for the source bytes. Kept verbatim and
    /// never recomputed here — a mismatch against `content` is itself
    /// information about the bundle, not something to paper over.
    pub content_hash: String,
    /// `"agentmux"` or `"foreign"`, verbatim from the manifest. Not parsed
    /// into an enum: an unrecognized value from a future/hand-written bundle
    /// must survive to the UI as-is rather than being silently coerced to
    /// `foreign`, which would understate AgentMux's own authorship.
    pub owner: String,
    /// The snapshot's content, resolved from the bundle's files.
    pub content: String,
}

/// Parsed, validated bundle content ready for the RPC handler to resolve
/// accounts against and write to the Store. `mcp_servers` still contains
/// whatever `${VAR}`-style placeholders the exporter's redaction left in
/// place — account resolution (§4.5, RPC-handler-side) substitutes real
/// account bindings where exactly one match is found and leaves the rest
/// untouched.
#[derive(Debug, Clone, Serialize)]
pub struct ParsedBundleImport {
    pub name: String,
    pub description: String,
    /// ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md §7.4.3: harness +
    /// vendor, carried through from the manifest's `suggestedFor` (v0.3) or
    /// top-level `provider`/`model` (v0.1–v0.2). Empty when it has neither
    /// (an older export predating this, or a bundle made with no hint). A
    /// hint only: it never sets an agent's provider.
    pub provider: String,
    pub model: String,
    pub instructions: String,
    /// ABF v0.2 §2.2: `{provider_id: content}` — every non-"default"
    /// variant found in a v0.2 `components.instructions` object, stored
    /// verbatim with no merge decision made here. Empty for a v0.1 bundle
    /// (flat array) or a v0.2 bundle with no provider-scoped variants.
    pub instructions_by_provider: HashMap<String, String>,
    pub context_files: Vec<ImportedContextFile>,
    pub mcp_servers: Vec<ParsedMcpServer>,
    pub skills: Vec<ParsedSkill>,
    pub skipped_skills: Vec<String>,
    pub requirements: Vec<AccountRequirement>,
    /// What the SOURCE agent was reading as project instructions. Read-only —
    /// see [`ImportedProjectInstruction`]. No caller writes these anywhere;
    /// `PROJECT_INSTRUCTIONS_NOT_APPLIED_WARNING` rides alongside them.
    pub project_instructions: Vec<ImportedProjectInstruction>,
    pub warnings: Vec<String>,
}

/// Normalize every input file's path and reduce to a first-wins,
/// deduped `(path -> content)` map — the exact effective representation
/// `parse_bundle_import` builds internally before doing anything else.
/// Extracted so the Phase 3 `files`-mode content digest (§3.0.5, round 6)
/// can compute over the IDENTICAL order-resolved representation the parser
/// itself uses, rather than a naive raw-input sort that could disagree
/// with which entry the parser's own first-wins rule actually keeps.
///
/// Also enforces the accounts/ allowlist (§4.3.5): only
/// `accounts/requirements.json` is ever readable from that directory,
/// checked against every file actually present (not just what the
/// manifest references, since a malformed bundle's `components` object
/// isn't a trustworthy inventory of its own contents), and normalized the
/// same way regardless of intake source (zip or raw `files` list) so
/// neither can bypass the check with a non-canonical spelling
/// (codex P1, PR #2379 rounds 1–2). A rejected file is excluded from the
/// returned map entirely, not merely warned about — otherwise a
/// `components.*` reference pointing at it (e.g.
/// `accounts/secrets.json`) would still resolve and leak its content into
/// the imported bundle.
fn dedup_files_by_path<'a>(files: &'a [BundleImportFile], warnings: &mut WarningSink) -> HashMap<String, &'a str> {
    let mut by_path: HashMap<String, &str> = HashMap::new();
    for f in files {
        let Some(safe_path) = sanitize_context_relative_path(&f.path) else {
            warnings.push(format!("{}: not a safe path; skipped", f.path));
            continue;
        };
        // reagent P1, PR #2379 round 4: case-insensitive on the DIRECTORY
        // check -- sanitize_context_relative_path never case-folds, so
        // `ACCOUNTS/secrets.json` (or any other-case variant) previously
        // sailed past the literal lowercase `starts_with` check while a
        // manifest reference using the identical casing would still
        // resolve it. The exception itself stays exact-match against the
        // canonical lowercase spelling the exporter always emits — an
        // other-case "requirements.json" is still inside the rejected
        // directory, just not recognized as the one allowed file.
        if safe_path.to_ascii_lowercase().starts_with("accounts/") && safe_path != "accounts/requirements.json" {
            warnings.push(format!(
                "{safe_path}: rejected — only accounts/requirements.json is ever read from the accounts/ directory"
            ));
            continue;
        }
        // reagent P2, PR #2379 round 5: `unzip_bundle_import` explicitly
        // detects and warns on a duplicate entry within a zip ("first
        // occurrence kept"), but the raw `files` RPC list reaches this
        // shared loop directly, bypassing that pass entirely — two input
        // entries normalizing to the same safe_path silently resolved
        // last-write-wins with no warning, so a caller inspecting the
        // input couldn't tell which content actually got imported.
        if by_path.contains_key(&safe_path) {
            warnings.push(format!("{safe_path}: duplicate path in input; first occurrence kept"));
            continue;
        }
        by_path.insert(safe_path, f.content.as_str());
    }
    by_path
}

/// Which of the three `bundle.import.preview`/`.commit` input fields
/// produced a given payload — mixed into the content-digest hash domain
/// itself (Phase 3 spec §3.0.5, round 7) so a `file_path` preview can never
/// be satisfied by a `zip_base64` commit of the identical underlying bytes
/// (both canonicalize to the same raw zip bytes and would otherwise hash
/// identically), closing the gap a bare byte-digest comparison left open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportInputMode {
    FilePath,
    ZipBase64,
    Files,
}

impl ImportInputMode {
    fn mode_byte(self) -> u8 {
        match self {
            ImportInputMode::FilePath => 0x01,
            ImportInputMode::ZipBase64 => 0x02,
            ImportInputMode::Files => 0x03,
        }
    }
}

/// SHA-256 content digest for `file_path`/`zip_base64` input — both
/// canonicalize to the same thing (raw zip bytes), differentiated only by
/// the mode tag mixed into the hash domain (§3.0.5, round 7).
pub fn content_digest_raw_bytes(mode: ImportInputMode, zip_bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update([mode.mode_byte()]);
    hasher.update(zip_bytes);
    hex::encode(hasher.finalize())
}

/// SHA-256 content digest for `files`-mode input (§3.0.5, round 6) — hashes
/// `parse_bundle_import`'s own effective, order-resolved representation
/// ([`dedup_files_by_path`]'s normalize-then-first-wins reduction, sorted
/// by normalized key), not the raw input array. This makes the digest
/// order-independent for genuinely equivalent inputs while remaining
/// sensitive to any reordering that would actually change which entry the
/// parser's first-wins rule keeps.
pub fn content_digest_files(files: &[BundleImportFile]) -> String {
    use sha2::{Digest, Sha256};
    let mut discard = WarningSink::new(WarningBudget::unbounded());
    let deduped = dedup_files_by_path(files, &mut discard);
    let mut entries: Vec<(&String, &&str)> = deduped.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    let mut hasher = Sha256::new();
    hasher.update([ImportInputMode::Files.mode_byte()]);
    for (path, content) in entries {
        hasher.update((path.len() as u64).to_le_bytes());
        hasher.update(path.as_bytes());
        hasher.update((content.len() as u64).to_le_bytes());
        hasher.update(content.as_bytes());
    }
    hex::encode(hasher.finalize())
}

/// Maximum number of distinct keys accepted in a v0.2
/// `components.instructions` object (`"default"` plus per-provider
/// variants) — ABF v0.2 §2.2. A real bundle needs at most one entry per
/// known harness (nine, as of `providers.rs`) plus `"default"`; generous
/// headroom for future providers without leaving this unbounded, mirroring
/// this module's other component-count caps (e.g.
/// [`MAX_ACCOUNT_REQUIREMENTS`]).
const MAX_INSTRUCTION_PROVIDER_VARIANTS: usize = 32;

/// The exact warning `parse_bundle_import_with_budget` pushes when
/// `components.memory` is present — ABF v0.2 §2.3. A shared constant
/// (rather than a literal duplicated at both the push site and
/// `bundle_import_for_agent_impl`'s filter site, reagent P1 PR #2527)
/// so the two can never drift apart: if this message ever changes, the
/// filter in `bundle.rs` keeps working because it references the same
/// constant, not a copy of the string.
/// Warning for an ABF carrying `components.projectInstructions`.
///
/// Phase 3 of `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md`. These
/// entries record what the *source* agent was reading. They are surfaced and
/// never installed — the files they describe belong to whatever repository the
/// bundle is being imported into, and an import that wrote them would be doing
/// what `SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md` exists to prevent.
///
/// Unlike [`MEMORY_COMPONENT_IGNORED_WARNING`] this has no filter site: memory
/// warns only on the agent-less path because `bundle.import_for_agent` really
/// does install it, whereas no import path installs these. The warning is
/// therefore unconditional and permanent, not a "use the other RPC" pointer.
pub(crate) const PROJECT_INSTRUCTIONS_NOT_APPLIED_WARNING: &str =
    "components.projectInstructions: recorded for reference, never installed — these files belong to the importing repository, not to the bundle. Inspect them with agent.project_instructions and apply anything you want deliberately";

/// Cap on how many `components.projectInstructions` entries are read back.
///
/// Matches `project_instructions::MAX_SCANNED_INSTRUCTION_FILES`, which is the
/// most AgentMux's own exporter can ever produce — so this can only ever bite
/// a hand-written or hostile bundle, where refusing to allocate unboundedly is
/// the point. Overflow warns rather than truncating silently, same rule the
/// rest of this parser follows.
pub const MAX_IMPORTED_PROJECT_INSTRUCTIONS: usize = 200;

pub(crate) const MEMORY_COMPONENT_IGNORED_WARNING: &str =
    "components.memory: present but ignored — memory requires an agent-scoped import (bundle.import_for_agent), not bundle.import";

/// Parse one `components.instructions`-shaped path array (either the v0.1
/// flat array itself, or one key's array within the v0.2 keyed-object
/// shape) into its joined instructions text. `label` is used only for
/// warning messages (`"instructions"` for the flat/default case,
/// `"instructions.<provider>"` for a variant) so every caller's warnings
/// are traceable to the specific key that produced them.
///
/// `seen_instruction_paths`/`duplicate_instruction_refs` are threaded
/// through BY THE CALLER, shared across every invocation within one
/// `parse_bundle_import_with_budget` call (default + every provider
/// variant) — reagent P1, PR #2523: a HashSet local to each call only
/// dedupes within its own component array, so the same `instructions/
/// context/*` path referenced from both `"default"` and a provider
/// variant was pushed into `context_files` twice, and — worse — a
/// manifest repeating one path across many provider keys (up to
/// [`MAX_INSTRUCTION_PROVIDER_VARIANTS`]) re-opened the exact
/// content-cloning amplification vector the original per-array dedup
/// existed to close (Codex P1, PR #2379 round 2, cited below).
///
/// `divert_context_files`: only `true` for the flat/default case.
/// reagent P1, PR #2523: diverting any `instructions/context/*`-prefixed
/// path applies to the raw file path unconditionally, regardless of which
/// provider variant is being parsed — so a provider literally named
/// `"context"` (whose exported path is `instructions/context/AGENTS.md`,
/// per `bundle_export.rs`'s `instructions/<provider>/AGENTS.md`
/// convention) would have its own content silently misrouted into
/// `context_files` on a later re-import, a reserved-word collision with
/// no validation guarding it. Context files are shared, not
/// provider-scoped, by design (§1's naming note) — a provider variant's
/// array never needs the diversion at all, so disabling it there closes
/// the collision structurally rather than special-casing the "context"
/// name.
fn parse_instruction_component_paths(
    arr: &[Value],
    label: &str,
    by_path: &HashMap<String, &str>,
    context_files: &mut Vec<ImportedContextFile>,
    divert_context_files: bool,
    seen_instruction_paths: &mut HashSet<String>,
    duplicate_instruction_refs: &mut u32,
    warnings: &mut WarningSink,
) -> String {
    let mut instructions_parts: Vec<String> = Vec::new();
    let paths = capped_component_array(Some(arr), label, warnings);
    for path_val in paths {
        let Some(raw_path) = path_val.as_str() else {
            warnings.push(format!("components.{label}: non-string entry skipped"));
            continue;
        };
        // codex P2, PR #2379 round 4: normalize the manifest's OWN
        // reference the same way `by_path`'s keys are normalized
        // (round 4's earlier fix) — otherwise a valid non-canonical
        // spelling here (e.g. `./instructions/AGENTS.md`) no longer
        // matches the now-canonicalized lookup key and the component
        // is reported missing even though the file is genuinely
        // present under an equivalent spelling.
        let Some(path) = sanitize_context_relative_path(raw_path) else {
            warnings.push(format!("components.{label}: \"{raw_path}\" is not a safe path; skipped"));
            continue;
        };
        // codex P1, PR #2379 round 6: accounts/requirements.json is
        // intentionally IN by_path (the dedicated parser below needs
        // to read it), but it must never be reachable through this
        // generic lookup — a manifest listing it here would otherwise
        // copy its raw JSON straight into `instructions`, later
        // written unredacted to `instructions/AGENTS.md` on export.
        if is_requirements_json(&path) {
            warnings.push(format!(
                "components.{label}: \"{path}\" is the accounts/ requirements file; not readable as instructions"
            ));
            continue;
        }
        // codex P1, PR #2379 round 6: dedup already avoids cloning
        // CONTENT per duplicate reference (round 3), but pushing one
        // warning STRING per duplicate is itself an amplification
        // vector — a permitted 10 MB manifest repeating one short
        // path hundreds of thousands of times could allocate hundreds
        // of megabytes of warning text serialized into the RPC
        // response. Count instead; a single summary warning is pushed
        // after the whole components.instructions parse completes
        // (ABF v0.2, §2.2: now after every array, not just this one —
        // see this function's own doc comment).
        if !seen_instruction_paths.insert(path.clone()) {
            *duplicate_instruction_refs += 1;
            continue;
        }
        let Some(content) = by_path.get(path.as_str()) else {
            warnings.push(format!("components.{label}: \"{path}\" not found among the bundle's files; skipped"));
            continue;
        };
        let diverted = divert_context_files && path.strip_prefix("instructions/context/").is_some();
        if diverted {
            let rel = path.strip_prefix("instructions/context/").unwrap();
            match sanitize_context_relative_path(rel) {
                Some(safe_rel) => context_files.push(ImportedContextFile {
                    id: context_files.len(),
                    path: safe_rel,
                    content: content.to_string(),
                }),
                None => warnings.push(format!(
                    "{path}: not a safe relative path under instructions/context/; skipped"
                )),
            }
        } else {
            instructions_parts.push(content.to_string());
        }
    }
    instructions_parts.join("\n\n---\n\n")
}

/// The manifest an ABF v0.3 archive carries
/// (SPEC_AGENT_BUNDLE_FORMAT_V0_3_2026_10_05.md §2.1).
pub const MANIFEST_FILE: &str = "bundle.json";
/// The manifest of ABF v0.1 and v0.2 archives, read forever.
pub const LEGACY_MANIFEST_FILE: &str = "armory.json";
/// The `$schema` an ABF v0.3 manifest names.
pub const SCHEMA_V0_3: &str = "https://docs.agentmux.ai/schemas/agent-bundle/v0.3/bundle.schema.json";

/// Which manifest file a set of archive paths carries: `bundle.json`, else
/// the legacy `armory.json`.
pub fn manifest_file_name(mut has: impl FnMut(&str) -> bool) -> Option<&'static str> {
    [MANIFEST_FILE, LEGACY_MANIFEST_FILE].into_iter().find(|name| has(name))
}

/// The ABF format version a manifest declares through its `$schema`, or
/// `None` when it names none this importer knows. The `version` field is
/// the bundle's own content version, not this.
pub fn abf_format_version(manifest: &Value) -> Option<&'static str> {
    let schema = manifest.get("$schema")?.as_str()?;
    if schema.contains("/agent-bundle/v0.3/") {
        Some("0.3")
    } else if schema.contains("/armory-bundle/v0.2/") {
        Some("0.2")
    } else if schema.contains("/armory-bundle/v0.1/") {
        Some("0.1")
    } else {
        None
    }
}

/// Parse and validate a bundle's files into [`ParsedBundleImport`].
/// Structural failures (no manifest, `bundle.json` or `armory.json`, or a
/// malformed one) reject the whole import — `Err` — since there is nothing safe to partially write.
/// Per-entry problems (a missing referenced file, an unsafe path, a
/// malformed SKILL.md) degrade to a warning and that entry is skipped —
/// matches `bundle_export.rs`'s own philosophy, and lets a lossy import
/// still produce a usable bundle rather than an all-or-nothing failure on
/// e.g. one corrupt skill among five good ones.
pub fn parse_bundle_import(files: &[BundleImportFile]) -> Result<ParsedBundleImport, String> {
    parse_bundle_import_with_budget(files, WarningBudget::unbounded())
}

/// Same as [`parse_bundle_import`], but with an explicit [`WarningBudget`]
/// enforced at every warning-push site inside this function (Phase 3 spec
/// §3.1, round 11) — the new `bundle.import.preview`/`.commit` RPC handlers
/// call this directly with a tight budget; [`parse_bundle_import`] passes
/// [`WarningBudget::unbounded`], preserving today's existing `bundle.import`
/// route's behavior exactly.
pub fn parse_bundle_import_with_budget(
    files: &[BundleImportFile],
    budget: WarningBudget,
) -> Result<ParsedBundleImport, String> {
    let mut warnings = WarningSink::new(budget);

    // Path normalization, first-wins dedup, and accounts/ allowlist
    // enforcement (§4.3.5) all live in dedup_files_by_path — see its own
    // doc comment.
    let by_path = dedup_files_by_path(files, &mut warnings);

    let manifest_name = manifest_file_name(|name| by_path.contains_key(name))
        .ok_or_else(|| format!("{MANIFEST_FILE}: missing from bundle (or {LEGACY_MANIFEST_FILE}, before ABF v0.3)"))?;
    if manifest_name == MANIFEST_FILE && by_path.contains_key(LEGACY_MANIFEST_FILE) {
        warnings.push(format!("{LEGACY_MANIFEST_FILE}: ignored; this bundle also has {MANIFEST_FILE}"));
    }
    let manifest_raw = by_path[manifest_name];
    let manifest: Value = serde_json::from_str(manifest_raw)
        .map_err(|e| format!("{manifest_name}: malformed JSON ({e})"))?;

    // Phase 3 spec §3.1, round 13: bound at the parse source, not at a
    // later response boundary — `name` is re-submitted verbatim as
    // `bundle_name` when a user doesn't edit the preview's suggested value
    // (§3.2, round 11), so `preview` and `commit` (which independently
    // re-parse) must converge on the identical canonical value
    // deterministically. There is no separate "full" name anywhere for a
    // display-only truncation to lose.
    let raw_name = manifest
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("imported-bundle");
    if raw_name.chars().count() > MAX_BUNDLE_NAME_CHARS {
        warnings.push(format!(
            "{manifest_name}: name exceeds {MAX_BUNDLE_NAME_CHARS} characters; truncated"
        ));
    }
    let name = bound_bundle_name(raw_name);
    let description = manifest
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // Who the bundle was made for, a hint only: v0.3's `suggestedFor`
    // (`vendor` is what `model` always held), or v0.1–v0.2's top-level
    // `provider` / `model`. Absent or non-string reads as "not set."
    let hint = |v03: &str, legacy: &str| {
        manifest
            .get("suggestedFor")
            .and_then(|s| s.get(v03))
            .or_else(|| manifest.get(legacy))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let provider = hint("provider", "provider");
    let model = hint("vendor", "model");

    // §4.3.2: schema/version are recorded (via warnings, since ParsedBundleImport
    // has no dedicated field for them yet — no schema registry exists to
    // validate against) but never block the import.
    if manifest.get("$schema").is_none() {
        warnings.push(format!("{manifest_name}: no $schema field present"));
    } else if abf_format_version(&manifest).is_none() {
        warnings.push(format!("{manifest_name}: $schema names no ABF version this AgentMux knows; reading it anyway"));
    }
    // `version` is the bundle's own content version, which says nothing
    // about the format (that's `$schema`, above), so it's never checked.

    let components = manifest.get("components").and_then(|v| v.as_object());

    // ------------------------------------------------------------------
    // instructions — every path in components.instructions, concatenated
    // in manifest order for AGENTS.md-shaped entries; instructions/context/*
    // paths become context_files entries instead.
    // ------------------------------------------------------------------
    let mut context_files: Vec<ImportedContextFile> = Vec::new();
    let mut instructions = String::new();
    let mut instructions_by_provider: HashMap<String, String> = HashMap::new();
    // ABF v0.2 §2.2: components.instructions is EITHER a flat array (v0.1
    // shape, treated as an implicit "default") OR an object keyed by
    // provider id (v0.2 shape, "default" plus zero or more provider-scoped
    // variants). No merge decision happens here — every variant is stored
    // verbatim; selecting one at launch time is a separate, not-yet-built
    // materializer's job (see the spec's non-goals).
    // Shared across every components.instructions array parsed below
    // (default + every provider variant) — see
    // parse_instruction_component_paths's doc comment for why this must
    // NOT be reset per-array (reagent P1, PR #2523).
    let mut seen_instruction_paths: HashSet<String> = HashSet::new();
    let mut duplicate_instruction_refs: u32 = 0;
    match components.and_then(|c| c.get("instructions")) {
        Some(Value::Array(arr)) => {
            instructions = parse_instruction_component_paths(
                arr, "instructions", &by_path, &mut context_files, true,
                &mut seen_instruction_paths, &mut duplicate_instruction_refs, &mut warnings,
            );
        }
        Some(Value::Object(obj)) => {
            if obj.len() > MAX_INSTRUCTION_PROVIDER_VARIANTS {
                warnings.push(format!(
                    "components.instructions: {} provider variants exceeds the limit ({MAX_INSTRUCTION_PROVIDER_VARIANTS}); only the first {MAX_INSTRUCTION_PROVIDER_VARIANTS} are used",
                    obj.len()
                ));
            }
            // "default" must be parsed FIRST, regardless of the manifest's
            // own key order (serde_json::Map without the preserve_order
            // feature iterates alphabetically — "claude" < "default" —
            // not insertion order). The shared dedup set means whichever
            // array reaches a given instructions/context/* path first
            // decides whether it's diverted; parsing a provider variant
            // first would let it silently claim a shared context-file
            // path as plain instructions text before "default" ever gets
            // a chance to divert it correctly.
            let mut ordered: Vec<(&String, &Value)> = Vec::with_capacity(obj.len());
            if let Some(default_entry) = obj.get_key_value("default") {
                ordered.push(default_entry);
            }
            ordered.extend(obj.iter().filter(|(k, _)| *k != "default"));
            for (key, val) in ordered.into_iter().take(MAX_INSTRUCTION_PROVIDER_VARIANTS) {
                let label = format!("instructions.{key}");
                let Some(arr) = val.as_array() else {
                    warnings.push(format!("components.{label}: expected an array; skipped"));
                    continue;
                };
                // Only the "default" array diverts instructions/context/*
                // references into context_files — a provider variant's
                // array never does (reagent P1, PR #2523; see this
                // function's own doc comment for the reserved-word
                // collision this closes).
                let divert_context_files = key == "default";
                let joined = parse_instruction_component_paths(
                    arr, &label, &by_path, &mut context_files, divert_context_files,
                    &mut seen_instruction_paths, &mut duplicate_instruction_refs, &mut warnings,
                );
                if key == "default" {
                    instructions = joined;
                } else {
                    instructions_by_provider.insert(key.clone(), joined);
                }
            }
        }
        Some(_) => warnings.push("components.instructions: expected an array or object; skipped".to_string()),
        None => {}
    }
    if duplicate_instruction_refs > 0 {
        warnings.push(format!(
            "components.instructions: {duplicate_instruction_refs} duplicate reference(s) skipped"
        ));
    }

    // ------------------------------------------------------------------
    // skills — every directory in components.skills, reading <dir>/SKILL.md
    // ------------------------------------------------------------------
    let mut skills: Vec<ParsedSkill> = Vec::new();
    let mut skipped_skills: Vec<String> = Vec::new();
    let mut seen_skill_dirs: HashSet<String> = HashSet::new();
    let mut duplicate_skill_refs: u32 = 0;
    {
        let dirs = capped_component_array(
            components.and_then(|c| c.get("skills")).and_then(|v| v.as_array()).map(|v| v.as_slice()),
            "skills",
            &mut warnings,
        );
        for dir_val in dirs {
            let Some(raw_dir) = dir_val.as_str() else {
                warnings.push("components.skills: non-string entry skipped".to_string());
                continue;
            };
            // codex P2, PR #2379 round 4: same normalization as
            // components.instructions above.
            let Some(dir) = sanitize_context_relative_path(raw_dir) else {
                warnings.push(format!("components.skills: \"{raw_dir}\" is not a safe path; skipped"));
                continue;
            };
            // codex P1, PR #2379 round 6: bounded the same way as
            // components.instructions above.
            if !seen_skill_dirs.insert(dir.clone()) {
                duplicate_skill_refs += 1;
                continue;
            }
            let skill_md_path = format!("{}/SKILL.md", dir.trim_end_matches('/'));
            let Some(content) = by_path.get(skill_md_path.as_str()) else {
                warnings.push(format!("components.skills: \"{skill_md_path}\" not found; skipped"));
                skipped_skills.push(dir.clone());
                continue;
            };
            match parse_skill_md(content) {
                Some(mut skill) => {
                    skill.source_dir = dir.clone();
                    skills.push(skill);
                }
                None => {
                    warnings.push(format!("{skill_md_path}: malformed SKILL.md frontmatter; skipped"));
                    skipped_skills.push(dir.clone());
                }
            }
        }
    }
    if duplicate_skill_refs > 0 {
        warnings.push(format!("components.skills: {duplicate_skill_refs} duplicate reference(s) skipped"));
    }
    // codex P1, PR #2379 round 7: bounds the RPC handler's WRITE side --
    // see MAX_IMPORTED_SKILLS's doc comment. The truncated skills still
    // count as "skipped" for reporting purposes, matching every other
    // skip reason in this loop.
    if skills.len() > MAX_IMPORTED_SKILLS {
        warnings.push(format!(
            "components.skills: {} skills exceeds the import limit ({MAX_IMPORTED_SKILLS}); only the first {MAX_IMPORTED_SKILLS} are imported",
            skills.len()
        ));
        skipped_skills.extend(skills.split_off(MAX_IMPORTED_SKILLS).into_iter().map(|s| s.slug));
    }

    // ------------------------------------------------------------------
    // mcp servers — every path in components.mcpServers, parsed as JSON
    // verbatim (still containing ${VAR} placeholders; resolution is the
    // RPC handler's job, §4.5).
    // ------------------------------------------------------------------
    let mut mcp_servers: Vec<ParsedMcpServer> = Vec::new();
    let mut seen_mcp_paths: HashSet<String> = HashSet::new();
    let mut duplicate_mcp_refs: u32 = 0;
    {
        let paths = capped_component_array(
            components.and_then(|c| c.get("mcpServers")).and_then(|v| v.as_array()).map(|v| v.as_slice()),
            "mcpServers",
            &mut warnings,
        );
        for path_val in paths {
            let Some(raw_path) = path_val.as_str() else {
                warnings.push("components.mcpServers: non-string entry skipped".to_string());
                continue;
            };
            // codex P2, PR #2379 round 4: same normalization as
            // components.instructions above.
            let Some(path) = sanitize_context_relative_path(raw_path) else {
                warnings.push(format!("components.mcpServers: \"{raw_path}\" is not a safe path; skipped"));
                continue;
            };
            // codex P1, PR #2379 round 6: same rejection as
            // components.instructions above -- requirements.json's raw
            // content is valid JSON and would otherwise parse cleanly
            // into an mcpServers entry, leaking it the same way.
            if is_requirements_json(&path) {
                warnings.push(format!(
                    "components.mcpServers: \"{path}\" is the accounts/ requirements file; not readable as an MCP server config"
                ));
                continue;
            }
            // codex P1, PR #2379 round 6: bounded the same way as
            // components.instructions above.
            if !seen_mcp_paths.insert(path.clone()) {
                duplicate_mcp_refs += 1;
                continue;
            }
            let Some(content) = by_path.get(path.as_str()) else {
                warnings.push(format!("components.mcpServers: \"{path}\" not found; skipped"));
                continue;
            };
            match serde_json::from_str::<Value>(content) {
                Ok(config) => mcp_servers.push(ParsedMcpServer { source_path: path.clone(), config }),
                Err(e) => warnings.push(format!("{path}: malformed JSON ({e}); skipped")),
            }
        }
    }
    if duplicate_mcp_refs > 0 {
        warnings.push(format!("components.mcpServers: {duplicate_mcp_refs} duplicate reference(s) skipped"));
    }

    // ------------------------------------------------------------------
    // accounts/requirements.json — read-only input to §4.5, never written
    // anywhere as-is.
    // ------------------------------------------------------------------
    let mut requirements: Vec<AccountRequirement> = Vec::new();
    if let Some(accounts_val) = components.and_then(|c| c.get("accounts")) {
        // reagent P2, PR #2379 round 7: every other component category
        // (instructions/skills/mcpServers non-string entries, an
        // unrecognized `version`) pushes an explicit warning when the
        // manifest's value has the wrong shape — this one silently
        // dropped a non-string `accounts` value with no warning at all,
        // inconsistent with the module's own "warn, don't silently drop"
        // philosophy.
        if let Some(raw_req_path) = accounts_val.as_str() {
            // reagent P2, PR #2379 round 6: this was the one remaining
            // manifest-reference lookup still comparing/using the raw,
            // un-normalized string -- the same bug class round 4 fixed for
            // components.instructions/skills/mcpServers, just missed here. A
            // valid non-canonical spelling (e.g. "./accounts/requirements.json")
            // failed the literal equality check, silently dropping legitimate
            // account requirements.
            let req_path = sanitize_context_relative_path(raw_req_path).filter(|p| is_requirements_json(p));
            if let Some(req_path) = &req_path {
                if let Some(content) = by_path.get(req_path.as_str()) {
                    #[derive(Deserialize)]
                    struct RequirementsDoc {
                        #[serde(default)]
                        requirements: Vec<AccountRequirement>,
                    }
                    match serde_json::from_str::<RequirementsDoc>(content) {
                        Ok(doc) => {
                            // codex P1, PR #2379 round 4: unbounded, the RPC
                            // handler's per-requirement account lookup becomes a
                            // synchronous store query per row — a permitted 10 MB
                            // JSON entry can hold tens/hundreds of thousands of
                            // (duplicate or distinct) requirements and keep that
                            // handler busy for a prolonged time. Bounding here
                            // protects the parse step itself; the handler
                            // separately dedupes by provider so its actual query
                            // count stays low even at this cap.
                            if doc.requirements.len() > MAX_ACCOUNT_REQUIREMENTS {
                                warnings.push(format!(
                                    "accounts/requirements.json: {} requirements exceeds the limit ({MAX_ACCOUNT_REQUIREMENTS}); only the first {MAX_ACCOUNT_REQUIREMENTS} are used",
                                    doc.requirements.len()
                                ));
                                requirements = doc.requirements.into_iter().take(MAX_ACCOUNT_REQUIREMENTS).collect();
                            } else {
                                requirements = doc.requirements;
                            }
                        }
                        Err(e) => warnings.push(format!("accounts/requirements.json: malformed JSON ({e}); ignored")),
                    }
                } else {
                    warnings.push("components.accounts references accounts/requirements.json, but it's not present in the bundle".to_string());
                }
            } else {
                warnings.push(format!(
                    "components.accounts: \"{raw_req_path}\" is not accounts/requirements.json; ignored per the accounts/ allowlist"
                ));
            }
        } else {
            warnings.push("components.accounts: non-string value skipped".to_string());
        }
    }

    // ABF v0.2 §2.3: `parse_bundle_import`/`parse_bundle_import_with_budget`
    // are called by every agent-LESS import path (bundle.import,
    // bundle.import.preview, bundle.import.commit) AND by
    // `bundle_import_for_agent_impl` (which reuses this same parser for
    // everything except memory, which it handles separately downstream).
    // A components.memory key gets a warning here so the three agent-less
    // callers can tell the memory component was present but requires
    // bundle.import_for_agent, not that it was missing from the source
    // bundle. reagent P1, PR #2527: `bundle_import_for_agent_impl` DOES
    // handle memory, so this warning is actively misleading there —
    // it must filter this exact string out of `parsed.warnings` before
    // merging into its own response (see MEMORY_COMPONENT_IGNORED_WARNING's
    // doc comment for why it's a shared constant, not a duplicated
    // literal, so the push site and the filter site can't drift apart).
    if components.and_then(|c| c.get("memory")).is_some() {
        warnings.push(MEMORY_COMPONENT_IGNORED_WARNING.to_string());
    }

    // Unlike memory, this warning has no filter site anywhere: NO import path
    // installs project instructions, agent-scoped or not. Those files belong
    // to the repository the bundle is being imported INTO, and writing them
    // would be exactly what SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md
    // exists to prevent — a portability feature turning into a back door
    // (SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md §5.2).
    //
    // The entries themselves ARE read back (Codex P1, PR #3163): a warning
    // saying "this bundle recorded some instructions" with no way to see them
    // is not surfacing anything, and `agent.project_instructions` can't fill
    // the gap because it scans the IMPORTING agent's working directory.
    // Reading is not applying — nothing downstream writes these.
    let mut project_instructions: Vec<ImportedProjectInstruction> = Vec::new();
    if let Some(raw) = components.and_then(|c| c.get("projectInstructions")) {
        warnings.push(PROJECT_INSTRUCTIONS_NOT_APPLIED_WARNING.to_string());
        match raw.as_array() {
            Some(entries) => {
                if entries.len() > MAX_IMPORTED_PROJECT_INSTRUCTIONS {
                    warnings.push(format!(
                        "components.projectInstructions: {} entries exceeds the \
                         {MAX_IMPORTED_PROJECT_INSTRUCTIONS}-entry cap; only the \
                         first {MAX_IMPORTED_PROJECT_INSTRUCTIONS} were read",
                        entries.len()
                    ));
                }
                for entry in entries.iter().take(MAX_IMPORTED_PROJECT_INSTRUCTIONS) {
                    let Some(obj) = entry.as_object() else {
                        warnings.push(
                            "components.projectInstructions: non-object entry skipped"
                                .to_string(),
                        );
                        continue;
                    };
                    let str_field = |k: &str| {
                        obj.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string()
                    };
                    let raw_file = str_field("file");
                    if raw_file.is_empty() {
                        warnings.push(
                            "components.projectInstructions: entry has no \"file\"; skipped"
                                .to_string(),
                        );
                        continue;
                    }
                    // Normalize the manifest's OWN reference the same way
                    // `by_path`'s keys are normalized, exactly as every other
                    // manifest-reference lookup in this file does (codex P2,
                    // PR #2379 round 4; reagent P2, round 6; reagent P2 again
                    // on PR #3163 — this is the third component to need it).
                    // Without it a valid non-canonical spelling
                    // (`./instructions/project/CLAUDE.md`, backslashes) misses
                    // the lookup and gets reported as "not found" even though
                    // the content is right there.
                    let Some(file_path) = sanitize_context_relative_path(&raw_file) else {
                        warnings.push(format!(
                            "components.projectInstructions: \"{raw_file}\" is not a \
                             safe path; skipped"
                        ));
                        continue;
                    };
                    // The accounts/ allowlist applies here for the same reason
                    // it applies to components.instructions: a manifest is not
                    // a trustworthy inventory of its own bundle, and a
                    // projectInstructions entry must not become a second way to
                    // read accounts/requirements.json back out as content.
                    // Checked on the NORMALIZED path so a non-canonical
                    // spelling can't slip past it either.
                    if is_requirements_json(&file_path) {
                        warnings.push(format!(
                            "components.projectInstructions: \"{raw_file}\" is the \
                             accounts/ requirements file; not readable as instructions"
                        ));
                        continue;
                    }
                    let Some(content) = by_path.get(file_path.as_str()) else {
                        warnings.push(format!(
                            "components.projectInstructions: \"{raw_file}\" not found \
                             among the bundle's files; skipped"
                        ));
                        continue;
                    };
                    project_instructions.push(ImportedProjectInstruction {
                        path: str_field("path"),
                        file: file_path,
                        content_hash: str_field("contentHash"),
                        owner: str_field("owner"),
                        content: content.to_string(),
                    });
                }
            }
            None => warnings.push(
                "components.projectInstructions: not an array; ignored".to_string(),
            ),
        }
    }

    Ok(ParsedBundleImport {
        name,
        description,
        provider,
        model,
        instructions,
        instructions_by_provider,
        context_files,
        mcp_servers,
        skills,
        skipped_skills,
        requirements,
        project_instructions,
        warnings: warnings.into_vec(),
    })
}

/// True if `path` is (case-insensitively) the one file ever read out of
/// the accounts/ directory. Used both to allow it into `by_path` (the
/// accounts/ allowlist above) and to REJECT it from every OTHER
/// component category's generic lookup (codex P1, PR #2379 round 6) —
/// components.instructions/mcpServers must never be able to pull its raw
/// JSON content in as if it were ordinary bundle content (e.g. straight
/// into `instructions/AGENTS.md` on a later export, or into an mcpServer
/// entry), since only the dedicated accounts/requirements.json parser is
/// meant to ever read it.
fn is_requirements_json(path: &str) -> bool {
    path.eq_ignore_ascii_case("accounts/requirements.json")
}

/// Bounds a manifest component array's length BEFORE any per-entry
/// processing happens, so one cap protects every warning a per-entry
/// loop can produce (non-string entries, unsafe paths, not-found
/// lookups, malformed content, ...) at once — codex P1, PR #2379 round
/// 7: round 6 bounded only the "duplicate reference" warning
/// specifically; a manifest filled with non-string junk values hit an
/// entirely different (still unbounded) warning path for the same
/// amplification effect. Reuses [`MAX_ENTRY_COUNT`] — a manifest
/// component array legitimately needs the same "more entries than any
/// real bundle uses" ceiling a zip archive's entry count does.
fn capped_component_array<'a>(arr: Option<&'a [Value]>, key: &str, warnings: &mut WarningSink) -> &'a [Value] {
    let Some(arr) = arr else { return &[] };
    let len = arr.len();
    if len > MAX_ENTRY_COUNT {
        warnings.push(format!(
            "components.{key}: {len} entries exceeds the limit ({MAX_ENTRY_COUNT}); only the first {MAX_ENTRY_COUNT} are used"
        ));
        &arr[..MAX_ENTRY_COUNT]
    } else {
        arr
    }
}

/// Parse a SKILL.md file (`render_skill_md`'s exact output shape, and
/// nothing more general — this is not a YAML parser, just the inverse of
/// the one writer this codebase has). Expects exactly:
/// `---\nname: "<json-string>"\ndescription: "<json-string>"\n---\n\n<body>`.
/// Returns `None` for anything that doesn't match — a real Agent Skills
/// SKILL.md from elsewhere in the ecosystem may use additional frontmatter
/// fields or different quoting; broadening this to a general YAML parser
/// is a follow-up, not silently guessed at here (mirrors the exporter's
/// own documented `server.json` scope decision).
fn parse_skill_md(content: &str) -> Option<ParsedSkill> {
    let rest = content.strip_prefix("---\n")?;
    let (frontmatter, body) = rest.split_once("\n---\n\n")?;
    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    for line in frontmatter.lines() {
        let (key, value) = line.split_once(": ")?;
        let parsed: String = serde_json::from_str(value).ok()?;
        match key {
            "name" => name = Some(parsed),
            "description" => description = Some(parsed),
            _ => {} // unrecognized frontmatter field — ignore, don't fail
        }
    }
    Some(ParsedSkill {
        // Filled in by the caller (parse_bundle_import), which alone knows
        // the components.skills directory reference that led here.
        source_dir: String::new(),
        slug: name?,
        // Required, not defaulted: render_skill_md always writes both
        // fields (falling back to a placeholder string, never omitting
        // the key), so a real exported SKILL.md always has both present.
        // A file missing `description` entirely isn't this writer's
        // output shape — reject it the same as a missing `name`.
        description: description?,
        content: body.to_string(),
    })
}

/// Per-entry decompressed-size cap. Generous for any legitimate
/// instructions/SKILL.md/context file (10 MB of text is already an
/// absurdly large single bundle component) while bounding how much a
/// single malicious entry can force into memory.
const MAX_ENTRY_UNCOMPRESSED_BYTES: u64 = 10 * 1024 * 1024;

/// Aggregate decompressed-size cap across the whole archive. Bounds a
/// zip-bomb-style archive built from many medium-sized entries that would
/// each individually pass [`MAX_ENTRY_UNCOMPRESSED_BYTES`].
const MAX_TOTAL_UNCOMPRESSED_BYTES: u64 = 50 * 1024 * 1024;

/// Maximum number of entries an archive may contain. A legitimate bundle
/// (instructions, a handful of context files, a handful of skills, one
/// manifest) never comes close to this; it exists purely to bound the CPU
/// cost of iterating + path-sanitizing + hashing every entry, independent
/// of the per-entry/aggregate byte caps below (reagent P2, PR #2379 round
/// 2) — a crafted archive of many tiny/valid entries passes both size caps
/// while still forcing unbounded iteration work.
const MAX_ENTRY_COUNT: usize = 10_000;

/// Maximum number of account requirements accepted from a single bundle's
/// `accounts/requirements.json`. A real bundle needs a handful at most —
/// one per distinct external account it depends on. Exists to bound the
/// RPC handler's downstream per-provider account-lookup work (codex P1,
/// PR #2379 round 4): an untrusted archive's requirements array,
/// unbounded, could otherwise drive a synchronous store query for every
/// row.
const MAX_ACCOUNT_REQUIREMENTS: usize = 1_000;

/// Maximum number of skills actually imported (i.e. written to the Store)
/// from a single bundle. `MAX_ENTRY_COUNT`/`capped_component_array` bound
/// how many `components.skills` entries are even LOOKED AT, but codex P1
/// (PR #2379 round 7) points out that's still far too high a ceiling for
/// the RPC handler's write side: importing up to that many skills means
/// up to that many separate synchronous Store transactions, each creating
/// a permanent, globally-visible skill row — a compact malicious archive
/// (well within the size caps) could otherwise monopolize the handler and
/// pollute the installation's skill catalog. A real bundle needs a
/// handful to a few dozen skills at most.
///
/// Also reused by `bundle.import.commit` (Phase 3 spec §3.2, round 3) as
/// the cap on `include_skills`'s length — the same reasoning applies to a
/// client-supplied selection array as to the parser's own component list:
/// neither may drive more Store-write attempts than a real bundle could
/// ever need.
pub const MAX_IMPORTED_SKILLS: usize = 200;

/// Maximum character length of a bundle's manifest `name`, enforced at
/// parse time (Phase 3 spec §3.1, round 13) rather than at a later display
/// boundary — `name` is re-submitted verbatim as `bundle_name` when a user
/// doesn't edit the preview's suggested value, so the canonical, bounded
/// value must be what both `preview` and `commit` converge on from the
/// moment parsing completes. Generous for any real bundle name.
pub const MAX_BUNDLE_NAME_CHARS: usize = 200;

/// Bounds a bundle name to [`MAX_BUNDLE_NAME_CHARS`] via plain truncation
/// (no ellipsis, unlike [`truncate_display`]) — this IS the canonical
/// value, not a display abbreviation of a longer "real" one. Shared by
/// `parse_bundle_import`'s own manifest-`name` bounding and the Phase 3
/// `bundle.import.commit` RPC's `bundle_name` override (round 3), so a
/// client-supplied override can't bypass the same bound `parsed.name` is
/// already held to.
pub fn bound_bundle_name(name: &str) -> String {
    if name.chars().count() > MAX_BUNDLE_NAME_CHARS {
        name.chars().take(MAX_BUNDLE_NAME_CHARS).collect()
    } else {
        name.to_string()
    }
}

/// Single choke point for the per-entry/aggregate size caps, shared by
/// BOTH intake paths (zip decompression and the raw `files` RPC list) —
/// reagent P1 / codex P1, PR #2379 round 5. Two separate gaps motivated
/// unifying this instead of patching each path independently:
///
/// - The raw `files` RPC branch previously ran NO size/count checks at
///   all, even though the spec explicitly treats it as an equally
///   untrusted alternate ingestion path to the zip — a hostile bundle
///   submitted via `files` instead of `zip_base64` bypassed every
///   zip-bomb/DoS defense this module had built for the zip path alone.
/// - Even on the zip path, an entry whose ACTUAL decompressed size
///   exceeds the per-entry cap was decompressed (paying the full
///   MAX_ENTRY_UNCOMPRESSED_BYTES-sized read) and then skipped WITHOUT
///   ever counting that work toward the aggregate budget — so up to
///   MAX_ENTRY_COUNT such entries could force on the order of
///   MAX_ENTRY_COUNT * MAX_ENTRY_UNCOMPRESSED_BYTES of real decompression
///   work while the aggregate cap never tripped.
///
/// This function accounts `content_len` toward the aggregate budget
/// UNCONDITIONALLY, before deciding whether the entry itself is kept —
/// closing the second gap structurally (every byte actually processed
/// counts, whether or not the entry is ultimately retained) — and both
/// intake paths call it, closing the first gap by construction (there is
/// no path left that skips this check).
///
/// Returns `Ok(true)` to keep the entry, `Ok(false)` to skip it (a
/// warning has already been pushed), or `Err` to reject the whole import
/// (aggregate cap exceeded).
fn check_entry_size(
    safe_name: &str,
    content_len: u64,
    total_uncompressed: &mut u64,
    warnings: &mut WarningSink,
) -> Result<bool, String> {
    *total_uncompressed = total_uncompressed.saturating_add(content_len);
    if *total_uncompressed > MAX_TOTAL_UNCOMPRESSED_BYTES {
        return Err(format!(
            "aggregate decompressed size exceeds the total limit ({MAX_TOTAL_UNCOMPRESSED_BYTES} bytes) — rejecting the whole import"
        ));
    }
    if content_len > MAX_ENTRY_UNCOMPRESSED_BYTES {
        warnings.push(format!(
            "{safe_name}: size ({content_len} bytes) exceeds the per-entry limit ({MAX_ENTRY_UNCOMPRESSED_BYTES} bytes); skipped"
        ));
        return Ok(false);
    }
    Ok(true)
}

/// Applies [`check_entry_size`]'s caps (plus [`MAX_ENTRY_COUNT`]) to the
/// raw `files` RPC intake path — the path `unzip_bundle_import` never
/// covers (reagent P1, PR #2379 round 5). Unlike zip entries, a raw
/// file's `content` is already fully materialized (no separate
/// declared-vs-actual distinction, no incremental decompression to
/// bound), so this only needs the shared per-entry/aggregate check, no
/// backstop-read machinery.
pub fn enforce_raw_files_caps(files: Vec<BundleImportFile>) -> Result<(Vec<BundleImportFile>, Vec<String>), String> {
    enforce_raw_files_caps_with_budget(files, WarningBudget::unbounded())
}

/// Same as [`enforce_raw_files_caps`], but with an explicit [`WarningBudget`]
/// (Phase 3 spec §3.1, round 11).
pub fn enforce_raw_files_caps_with_budget(
    files: Vec<BundleImportFile>,
    budget: WarningBudget,
) -> Result<(Vec<BundleImportFile>, Vec<String>), String> {
    if files.len() > MAX_ENTRY_COUNT {
        return Err(format!(
            "{} entries exceeds the limit ({MAX_ENTRY_COUNT}) — rejecting the whole import",
            files.len()
        ));
    }
    let mut warnings = WarningSink::new(budget);
    let mut total_uncompressed: u64 = 0;
    let mut out = Vec::new();
    for f in files {
        let keep = check_entry_size(&f.path, f.content.len() as u64, &mut total_uncompressed, &mut warnings)?;
        if keep {
            out.push(f);
        }
    }
    Ok((out, warnings.into_vec()))
}

/// Unpack a `.abf` zip archive into a flat file list, applying the same
/// path-safety check as everywhere else in this module to every entry
/// name before it's trusted (§4.3.4) — a zip's own internal paths are
/// exactly as untrusted as any other part of the archive's content.
/// Directory entries are skipped; anything that fails the safety check is
/// dropped with a warning rather than surfaced as a file.
///
/// Codex P1, PR #2379: also bounds decompressed size, per-entry and in
/// aggregate — `read_to_string` alone has no size limit, so an untrusted
/// `.abf` containing a highly compressed entry (a classic zip-bomb shape;
/// DEFLATE alone permits ratios past 1000:1) could otherwise exhaust the
/// server process's memory through `bundle.import`. An oversized single
/// entry is skipped with a warning (matches this function's existing
/// per-entry-problem philosophy); an oversized AGGREGATE fails the whole
/// import — letting a partially-capped import through silently would be
/// more confusing than useful, and hitting the aggregate cap at all
/// already indicates a genuinely abusive archive rather than one bad file.
pub fn unzip_bundle_import(zip_bytes: &[u8]) -> Result<(Vec<BundleImportFile>, Vec<String>), String> {
    unzip_bundle_import_with_budget(zip_bytes, WarningBudget::unbounded())
}

/// Same as [`unzip_bundle_import`], but with an explicit [`WarningBudget`]
/// (Phase 3 spec §3.1, round 11).
pub fn unzip_bundle_import_with_budget(
    zip_bytes: &[u8],
    budget: WarningBudget,
) -> Result<(Vec<BundleImportFile>, Vec<String>), String> {
    use std::io::Read;
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| format!("not a valid zip archive: {e}"))?;

    if archive.len() > MAX_ENTRY_COUNT {
        return Err(format!(
            "zip archive: {} entries exceeds the limit ({MAX_ENTRY_COUNT}) — rejecting the whole import",
            archive.len()
        ));
    }

    // First pass: collect every non-dir entry's raw name, to DETECT
    // whether every single one shares one common wrapping directory
    // (`zip_bundle_export`'s convention: `<root_slug>/bundle.json`, etc.)
    // before deciding whether to strip a path component at all.
    //
    // reagent P1, PR #2379: the previous version unconditionally stripped
    // the first `/`-delimited segment of EVERY entry, on the assumption a
    // wrapper always exists. A hand-built `.abf` zipped directly from the
    // loose directory tree (no wrapping folder) — which this spec's §2
    // explicitly requires an importer to also accept — has entries like
    // `instructions/AGENTS.md` already bundle-relative; blindly stripping
    // silently mangled it to `AGENTS.md`, breaking every `components.*`
    // path lookup in `parse_bundle_import` (each degrades to a "not
    // found" warning and the content is dropped) with no error surfaced.
    // Only strip when EVERY entry agrees on the same first segment.
    let mut raw_names: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("zip archive: failed to read entry {i}: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        raw_names.push(entry.name().to_string());
    }
    let common_wrapper: Option<&str> = raw_names.first().and_then(|first| {
        let candidate = first.split_once('/').map(|(root, _)| root)?;
        raw_names
            .iter()
            .all(|n| n.split_once('/').map(|(root, _)| root == candidate).unwrap_or(false))
            .then_some(candidate)
    });
    let strip_len: Option<usize> = common_wrapper.map(|w| w.len() + 1); // +1 for the '/'

    let mut out = Vec::new();
    let mut warnings = WarningSink::new(budget);
    let mut seen: HashSet<String> = HashSet::new();
    let mut total_uncompressed: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("zip archive: failed to read entry {i}: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        let raw_name = entry.name().to_string();
        let relative_name = match strip_len {
            Some(n) if raw_name.len() > n => &raw_name[n..],
            _ => raw_name.as_str(),
        };
        let Some(safe_name) = sanitize_context_relative_path(relative_name) else {
            warnings.push(format!("{raw_name}: not a safe path; skipped"));
            continue;
        };
        if !seen.insert(safe_name.clone()) {
            warnings.push(format!("{safe_name}: duplicate entry in archive; first occurrence kept"));
            continue;
        }

        // Declared size check first — cheap, and avoids decompressing at
        // all for an entry that's already too large per its own metadata.
        // This is ONLY a cheap pre-filter: `entry.size()` is the zip's own
        // declared/attacker-controlled uncompressed size, not a verified
        // value, so it must never feed the aggregate cap below (reagent
        // P1, PR #2379 round 2) — an archive of many entries that each lie
        // with a tiny declared size while actually containing content up
        // to the per-entry cap would otherwise pass both checks here while
        // still exhausting memory once decompressed.
        let declared_size = entry.size();
        if declared_size > MAX_ENTRY_UNCOMPRESSED_BYTES {
            warnings.push(format!(
                "{safe_name}: declared uncompressed size ({declared_size} bytes) exceeds the per-entry limit ({MAX_ENTRY_UNCOMPRESSED_BYTES} bytes); skipped"
            ));
            continue;
        }

        // Hard backstop on the actual read, independent of the declared
        // size above: read at most one byte past the cap so an entry
        // whose real decompressed content exceeds what it declared is
        // still caught (rather than trusting zip metadata alone).
        let mut limited = entry.by_ref().take(MAX_ENTRY_UNCOMPRESSED_BYTES + 1);
        let mut buf: Vec<u8> = Vec::new();
        let read_err = limited.read_to_end(&mut buf).err();

        // codex P1, PR #2379 round 5/6: `check_entry_size` accounts these
        // bytes toward the aggregate budget BEFORE deciding whether to
        // keep the entry, unlike the previous inline checks here (which
        // accumulated only KEPT entries' bytes). Called unconditionally
        // here — even when the read above ultimately errored (e.g. a
        // forged CRC on an otherwise-decompressing entry) — since
        // `read_to_end` can leave real decompressed bytes in `buf` before
        // returning an error; that decompression work already happened
        // regardless of the read's outcome, and up to MAX_ENTRY_COUNT
        // such corrupt entries must not be able to force ~10MB of work
        // each while the aggregate counter stays untouched.
        let keep = check_entry_size(&safe_name, buf.len() as u64, &mut total_uncompressed, &mut warnings)?;

        if let Some(e) = read_err {
            warnings.push(format!("{safe_name}: failed to read entry: {e}"));
            continue;
        }
        if !keep {
            continue;
        }
        let content = match String::from_utf8(buf) {
            Ok(s) => s,
            Err(e) => {
                warnings.push(format!("{safe_name}: not valid UTF-8 text; skipped ({e})"));
                continue;
            }
        };
        out.push(BundleImportFile { path: safe_name, content });
    }
    Ok((out, warnings.into_vec()))
}

#[cfg(test)]
#[path = "bundle_import_tests.rs"]
mod tests;
