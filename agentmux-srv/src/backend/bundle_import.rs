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

/// Parse and validate a bundle's files into [`ParsedBundleImport`].
/// Structural failures (missing/malformed `armory.json`) reject the whole
/// import — `Err` — since there is nothing safe to partially write.
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

    let manifest_raw = by_path
        .get("armory.json")
        .ok_or_else(|| "armory.json: missing from bundle".to_string())?;
    let manifest: Value = serde_json::from_str(manifest_raw)
        .map_err(|e| format!("armory.json: malformed JSON ({e})"))?;

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
            "armory.json: name exceeds {MAX_BUNDLE_NAME_CHARS} characters; truncated"
        ));
    }
    let name = bound_bundle_name(raw_name);
    let description = manifest
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // §4.3.2: schema/version are recorded (via warnings, since ParsedBundleImport
    // has no dedicated field for them yet — no schema registry exists to
    // validate against) but never block the import.
    if manifest.get("$schema").is_none() {
        warnings.push("armory.json: no $schema field present".to_string());
    }
    match manifest.get("version").and_then(|v| v.as_str()) {
        // codex P2, PR #2379 round 5: a bare prefix match treated "0.10.0"
        // and "0.1garbage" as recognized v0.1.x versions too — this
        // warning is the only signal that potentially incompatible
        // semantics are being accepted for an importer that deliberately
        // proceeds with unknown versions anyway, so it must actually
        // distinguish "0.1", "0.1.x" from anything merely starting with
        // the same three characters.
        Some(v) if v == "0.1" || v.starts_with("0.1.") => {}
        Some(other) => warnings.push(format!(
            "armory.json: version \"{other}\" is not a recognized ABF v0.1.x version; importing anyway"
        )),
        None => warnings.push("armory.json: no version field present".to_string()),
    }

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
    // bundle.import.preview, bundle.import.commit) — none of them has an
    // agent to write memory into. A components.memory key is explicitly
    // skipped here, with a warning rather than silently dropped, so a
    // caller inspecting the response can tell the memory component was
    // present but requires bundle.import_for_agent, not that it was
    // missing from the source bundle. Fixing this once here, rather than
    // in each of the three RPC handlers, means it applies uniformly by
    // construction — no handler can forget the check.
    if components.and_then(|c| c.get("memory")).is_some() {
        warnings.push(
            "components.memory: present but ignored — memory requires an agent-scoped import (bundle.import_for_agent), not bundle.import".to_string(),
        );
    }

    Ok(ParsedBundleImport {
        name,
        description,
        instructions,
        instructions_by_provider,
        context_files,
        mcp_servers,
        skills,
        skipped_skills,
        requirements,
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
    // (`zip_bundle_export`'s convention: `<root_slug>/armory.json`, etc.)
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
mod tests {
    use super::*;

    fn file(path: &str, content: &str) -> BundleImportFile {
        BundleImportFile { path: path.to_string(), content: content.to_string() }
    }

    fn minimal_manifest(components: Value) -> String {
        serde_json::to_string(&serde_json::json!({
            "$schema": "https://docs.agentmux.ai/schemas/armory-bundle/v0.1/bundle.schema.json",
            "name": "test-bundle",
            "version": "0.1.0",
            "description": "A test bundle",
            "components": components,
            "metadata": {},
        }))
        .unwrap()
    }

    #[test]
    fn rejects_missing_armory_json() {
        let err = parse_bundle_import(&[]).unwrap_err();
        assert!(err.contains("armory.json"));
    }

    #[test]
    fn rejects_malformed_armory_json() {
        let err = parse_bundle_import(&[file("armory.json", "not json")]).unwrap_err();
        assert!(err.contains("malformed JSON"));
    }

    #[test]
    fn deduplicates_repeated_instruction_references_instead_of_cloning_content_per_reference() {
        // Codex P1, PR #2379 round 3: components.instructions is manifest-
        // controlled and its length isn't bounded by the decompression
        // caps -- those cap file CONTENT, not how many times a manifest
        // can reference the same path. An untrusted manifest repeating one
        // path thousands of times must not clone that content once per
        // repetition.
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "instructions": ["instructions/AGENTS.md", "instructions/AGENTS.md", "instructions/AGENTS.md"],
            }))),
            file("instructions/AGENTS.md", "Be concise."),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert_eq!(result.instructions, "Be concise.");
        // codex P1, PR #2379 round 6: one bounded summary warning instead
        // of one warning string per duplicate (its own amplification
        // vector -- see the round-6 tests below).
        assert!(result.warnings.iter().any(|w| w.contains("2 duplicate reference(s) skipped")));
    }

    #[test]
    fn deduplicates_repeated_skill_and_mcp_references() {
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "skills": ["skills/deploy", "skills/deploy"],
                "mcpServers": ["mcp/server.json", "mcp/server.json"],
            }))),
            file("skills/deploy/SKILL.md", "---\nname: \"deploy\"\ndescription: \"d\"\n---\n\nbody"),
            file("mcp/server.json", r#"{"command":"npx","args":["-y","thing"]}"#),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.mcp_servers.len(), 1);
        assert!(result.warnings.iter().any(|w| w.contains("components.skills: 1 duplicate reference(s) skipped")));
        assert!(result.warnings.iter().any(|w| w.contains("components.mcpServers: 1 duplicate reference(s) skipped")));
    }

    #[test]
    fn imports_instructions_from_agents_md() {
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "instructions": ["instructions/AGENTS.md"],
            }))),
            file("instructions/AGENTS.md", "Be concise."),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert_eq!(result.instructions, "Be concise.");
        assert_eq!(result.name, "test-bundle");
        assert_eq!(result.description, "A test bundle");
        assert!(result.instructions_by_provider.is_empty());
    }

    #[test]
    fn imports_v02_keyed_object_shape_with_default_and_provider_variants() {
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "instructions": {
                    "default": ["instructions/AGENTS.md"],
                    "claude": ["instructions/claude/AGENTS.md"],
                    "codex": ["instructions/codex/AGENTS.md"],
                },
            }))),
            file("instructions/AGENTS.md", "Be concise."),
            file("instructions/claude/AGENTS.md", "Claude-specific."),
            file("instructions/codex/AGENTS.md", "Codex-specific."),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert_eq!(result.instructions, "Be concise.");
        assert_eq!(result.instructions_by_provider.get("claude"), Some(&"Claude-specific.".to_string()));
        assert_eq!(result.instructions_by_provider.get("codex"), Some(&"Codex-specific.".to_string()));
        assert_eq!(result.instructions_by_provider.len(), 2);
    }

    #[test]
    fn imports_v02_keyed_object_shape_with_no_default_key() {
        // A bundle that only ever defines provider-specific content, no
        // shared default — must not crash or silently invent a "default".
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "instructions": { "claude": ["instructions/claude/AGENTS.md"] },
            }))),
            file("instructions/claude/AGENTS.md", "Claude-only."),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert_eq!(result.instructions, "");
        assert_eq!(result.instructions_by_provider.get("claude"), Some(&"Claude-only.".to_string()));
    }

    #[test]
    fn a_context_file_referenced_from_default_and_a_provider_variant_is_deduped_across_both() {
        // reagent P1, PR #2523: dedup used to be local to each
        // components.instructions array, so the same context-file path
        // referenced from both "default" and a provider variant was
        // pushed into context_files TWICE. Must dedupe across the whole
        // components.instructions parse, not per-array.
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "instructions": {
                    "default": ["instructions/AGENTS.md", "instructions/context/shared.md"],
                    "claude": ["instructions/claude/AGENTS.md", "instructions/context/shared.md"],
                },
            }))),
            file("instructions/AGENTS.md", "Default."),
            file("instructions/claude/AGENTS.md", "Claude."),
            file("instructions/context/shared.md", "Shared context."),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert_eq!(result.context_files.len(), 1, "the shared context file must appear exactly once, not once per referencing variant");
        assert_eq!(result.context_files[0].content, "Shared context.");
        assert!(result.warnings.iter().any(|w| w.contains("duplicate reference")));
    }

    #[test]
    fn a_provider_named_context_is_not_misrouted_into_context_files() {
        // reagent P1, PR #2523: the instructions/context/ prefix check
        // applied to every path uniformly regardless of which provider
        // array was being parsed, so a provider literally named "context"
        // — whose exported path is instructions/context/AGENTS.md, per
        // bundle_export.rs's instructions/<provider>/AGENTS.md convention
        // — had its content silently misrouted into context_files instead
        // of instructions_by_provider["context"].
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "instructions": {
                    "default": ["instructions/AGENTS.md"],
                    "context": ["instructions/context/AGENTS.md"],
                },
            }))),
            file("instructions/AGENTS.md", "Default."),
            file("instructions/context/AGENTS.md", "For the 'context' provider."),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert_eq!(
            result.instructions_by_provider.get("context"),
            Some(&"For the 'context' provider.".to_string()),
            "a provider literally named 'context' must land in instructions_by_provider, not context_files"
        );
        assert!(result.context_files.is_empty());
    }

    #[test]
    fn a_non_array_provider_variant_warns_and_is_skipped() {
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "instructions": { "default": ["instructions/AGENTS.md"], "claude": "not-an-array" },
            }))),
            file("instructions/AGENTS.md", "Be concise."),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert_eq!(result.instructions, "Be concise.");
        assert!(result.instructions_by_provider.is_empty());
        assert!(result.warnings.iter().any(|w| w.contains("instructions.claude") && w.contains("expected an array")));
    }

    #[test]
    fn caps_the_number_of_instruction_provider_variants() {
        let mut instructions = serde_json::Map::new();
        for i in 0..MAX_INSTRUCTION_PROVIDER_VARIANTS + 5 {
            instructions.insert(format!("provider-{i}"), serde_json::json!([]));
        }
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({ "instructions": instructions }))),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("provider variants exceeds the limit")));
    }

    #[test]
    fn a_memory_component_is_warned_about_and_ignored_by_the_agent_less_parser() {
        // ABF v0.2 §2.3: parse_bundle_import (shared by bundle.import and
        // bundle.import.preview/.commit, all agent-less) must not silently
        // drop a components.memory key — it needs bundle.import_for_agent
        // instead, and the response should say so.
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "instructions": ["instructions/AGENTS.md"],
                "memory": ["memory/MEMORY.md"],
            }))),
            file("instructions/AGENTS.md", "Be concise."),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("components.memory") && w.contains("bundle.import_for_agent")));
    }

    #[test]
    fn no_memory_warning_when_the_component_is_absent() {
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "instructions": ["instructions/AGENTS.md"],
            }))),
            file("instructions/AGENTS.md", "Be concise."),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert!(!result.warnings.iter().any(|w| w.contains("components.memory")));
    }

    #[test]
    fn imports_context_files_separately_from_instructions() {
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "instructions": ["instructions/AGENTS.md", "instructions/context/notes.md"],
            }))),
            file("instructions/AGENTS.md", "Main instructions."),
            file("instructions/context/notes.md", "Extra context."),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert_eq!(result.instructions, "Main instructions.");
        assert_eq!(result.context_files, vec![ImportedContextFile {
            id: 0,
            path: "notes.md".to_string(),
            content: "Extra context.".to_string(),
        }]);
    }

    #[test]
    fn warns_and_skips_a_missing_referenced_file_rather_than_failing() {
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "instructions": ["instructions/AGENTS.md"],
            }))),
            // instructions/AGENTS.md deliberately absent
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert_eq!(result.instructions, "");
        assert!(result.warnings.iter().any(|w| w.contains("not found")));
    }

    #[test]
    fn imports_a_skill_round_tripped_through_render_skill_md() {
        let rendered = super::super::agent_config::render_skill_md(
            "deploy-checklist",
            "Runs the checklist",
            "1. Test\n2. Deploy",
        );
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "skills": ["skills/deploy-checklist"],
            }))),
            file("skills/deploy-checklist/SKILL.md", &rendered),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].source_dir, "skills/deploy-checklist");
        assert_eq!(result.skills[0].slug, "deploy-checklist");
        assert_eq!(result.skills[0].description, "Runs the checklist");
        assert_eq!(result.skills[0].content, "1. Test\n2. Deploy");
        assert!(result.skipped_skills.is_empty());
    }

    #[test]
    fn skips_a_skill_whose_skill_md_is_missing() {
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "skills": ["skills/ghost"],
            }))),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert!(result.skills.is_empty());
        assert_eq!(result.skipped_skills, vec!["skills/ghost".to_string()]);
    }

    #[test]
    fn skips_a_skill_with_malformed_frontmatter() {
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "skills": ["skills/broken"],
            }))),
            file("skills/broken/SKILL.md", "not frontmatter at all"),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert!(result.skills.is_empty());
        assert_eq!(result.skipped_skills, vec!["skills/broken".to_string()]);
        assert!(result.warnings.iter().any(|w| w.contains("malformed SKILL.md")));
    }

    #[test]
    fn imports_mcp_servers_still_containing_placeholders() {
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "mcpServers": ["mcp/github.server.json"],
            }))),
            file("mcp/github.server.json", r#"{"type":"stdio","command":"gh-mcp","env":{"GITHUB_TOKEN":"${GITHUB_TOKEN}"}}"#),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert_eq!(result.mcp_servers.len(), 1);
        assert_eq!(result.mcp_servers[0].source_path, "mcp/github.server.json");
        assert_eq!(result.mcp_servers[0].config["env"]["GITHUB_TOKEN"], "${GITHUB_TOKEN}");
    }

    #[test]
    fn parses_account_requirements_read_only() {
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "accounts": "accounts/requirements.json",
            }))),
            file("accounts/requirements.json", r#"{"requirements":[{"id":"gh-main","credentialProvider":"github","kind":"api-key","env":"GITHUB_TOKEN","optional":false}]}"#),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert_eq!(result.requirements, vec![AccountRequirement {
            id: "gh-main".to_string(),
            provider: "github".to_string(),
            kind: "api-key".to_string(),
            env: "GITHUB_TOKEN".to_string(),
            optional: false,
        }]);
    }

    #[test]
    fn a_v01_bundle_still_carrying_the_old_provider_key_still_imports() {
        // ABF v0.2, §2.1: "provider" was renamed to "credentialProvider" on
        // export, but a bundle exported by a v0.1 build (or hand-authored
        // against the old spec) still uses the old key — must not break.
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "accounts": "accounts/requirements.json",
            }))),
            file("accounts/requirements.json", r#"{"requirements":[{"id":"gh-main","provider":"github","kind":"api-key","env":"GITHUB_TOKEN","optional":false}]}"#),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert_eq!(result.requirements, vec![AccountRequirement {
            id: "gh-main".to_string(),
            provider: "github".to_string(),
            kind: "api-key".to_string(),
            env: "GITHUB_TOKEN".to_string(),
            optional: false,
        }]);
    }

    #[test]
    fn rejects_any_accounts_file_other_than_requirements_json() {
        // §4.3.5 / the report's original invariant: never read anything
        // else under accounts/, even if present and well-formed.
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({}))),
            file("accounts/secrets.json", r#"{"github_token":"ghp_should_never_be_read"}"#),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("accounts/secrets.json") && w.contains("rejected")));
    }

    #[test]
    fn a_rejected_accounts_file_referenced_by_a_component_is_never_surfaced_in_the_result() {
        // Codex P1, PR #2379: a rejected accounts/ file must be excluded
        // from lookups entirely, not merely warned about — otherwise a
        // malicious bundle whose components.instructions REFERENCES
        // accounts/secrets.json would still get that content folded into
        // the imported bundle's instructions, defeating the whole
        // accounts/ allowlist. This is the exact attack shape: point a
        // legitimate-looking component path at the rejected file.
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "instructions": ["accounts/secrets.json"],
            }))),
            file("accounts/secrets.json", "GITHUB_TOKEN=ghp_should_never_leak_into_a_bundle"),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert!(
            !result.instructions.contains("ghp_should_never_leak_into_a_bundle"),
            "rejected accounts/ content leaked into instructions: {:?}",
            result.instructions
        );
        assert!(result.warnings.iter().any(|w| w.contains("accounts/secrets.json") && w.contains("rejected")));
        // The reference itself now resolves to "not found" (the file was
        // excluded from lookups), which is the correct, safe outcome —
        // not a crash, not a silent success with leaked content.
        assert!(result.warnings.iter().any(|w| w.contains("accounts/secrets.json") && w.contains("not found")));
    }

    #[test]
    fn rejects_an_other_case_accounts_path_the_same_as_the_canonical_lowercase_one() {
        // reagent P1, PR #2379 round 4: sanitize_context_relative_path
        // never case-folds, so a literal lowercase `starts_with("accounts/")`
        // check let `ACCOUNTS/secrets.json` sail through unrejected.
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "instructions": ["ACCOUNTS/secrets.json"],
            }))),
            file("ACCOUNTS/secrets.json", "GITHUB_TOKEN=ghp_should_never_leak_case_variant"),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert!(!result.instructions.contains("ghp_should_never_leak_case_variant"));
        assert!(result.warnings.iter().any(|w| w.contains("ACCOUNTS/secrets.json") && w.contains("rejected")));
    }

    #[test]
    fn resolves_a_non_canonical_manifest_reference_against_the_normalized_file_path() {
        // codex P2, PR #2379 round 4: by_path's keys are normalized (round
        // 4's earlier accounts/ fix), so a manifest reference using a
        // valid but non-canonical spelling of the SAME path must be
        // normalized identically before the lookup, or a genuinely present
        // file is reported "not found".
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "instructions": ["./instructions/AGENTS.md"],
            }))),
            file("instructions/AGENTS.md", "Be concise."),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert_eq!(result.instructions, "Be concise.");
        assert!(result.warnings.iter().all(|w| !w.contains("not found")), "unexpected warnings: {:?}", result.warnings);
    }

    #[test]
    fn caps_the_number_of_account_requirements_accepted_from_a_single_bundle() {
        // codex P1, PR #2379 round 4: an unbounded requirements array lets
        // a small (well within the per-entry size cap) accounts/requirements.json
        // drive an unbounded number of downstream per-provider account
        // lookups in the RPC handler.
        let many: Vec<_> = (0..MAX_ACCOUNT_REQUIREMENTS + 50)
            .map(|i| serde_json::json!({
                "id": format!("req-{i}"), "provider": "github", "kind": "api-key",
                "env": "GITHUB_TOKEN", "optional": false,
            }))
            .collect();
        let requirements_doc = serde_json::json!({ "requirements": many }).to_string();
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "accounts": "accounts/requirements.json",
            }))),
            file("accounts/requirements.json", &requirements_doc),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert_eq!(result.requirements.len(), MAX_ACCOUNT_REQUIREMENTS);
        assert!(result.warnings.iter().any(|w| w.contains("exceeds the limit")));
    }

    #[test]
    fn resolves_a_non_canonical_components_accounts_reference() {
        // reagent P2, PR #2379 round 6: components.accounts was the one
        // remaining manifest-reference lookup still comparing the raw,
        // un-normalized string -- the same bug class round 4 fixed for
        // instructions/skills/mcpServers.
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "accounts": "./accounts/requirements.json",
            }))),
            file("accounts/requirements.json", r#"{"requirements":[{"id":"gh-main","provider":"github","kind":"api-key","env":"GITHUB_TOKEN","optional":false}]}"#),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert_eq!(result.requirements.len(), 1);
        assert!(result.warnings.iter().all(|w| !w.contains("is not accounts/requirements.json")));
    }

    #[test]
    fn requirements_json_is_not_readable_through_components_instructions_or_mcp_servers() {
        // codex P1, PR #2379 round 6: accounts/requirements.json is
        // intentionally in by_path for the dedicated parser above, but
        // must never be reachable through the generic component lookups
        // -- otherwise its raw content leaks into instructions (and later
        // an export's AGENTS.md) or an mcpServers entry.
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "instructions": ["accounts/requirements.json"],
                "mcpServers": ["accounts/requirements.json"],
            }))),
            file("accounts/requirements.json", r#"{"requirements":[{"id":"gh-main","provider":"github","kind":"api-key","env":"GITHUB_TOKEN_SECRET_LEAK","optional":false}]}"#),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert!(!result.instructions.contains("GITHUB_TOKEN_SECRET_LEAK"));
        assert!(result.mcp_servers.is_empty());
        assert!(result.warnings.iter().any(|w| w.contains("components.instructions") && w.contains("not readable as instructions")));
        assert!(result.warnings.iter().any(|w| w.contains("components.mcpServers") && w.contains("not readable as an MCP server config")));
    }

    #[test]
    fn duplicate_reference_warnings_are_bounded_to_one_summary_per_component_category() {
        // codex P1, PR #2379 round 6: dedup already avoids cloning
        // CONTENT per duplicate (round 3), but pushing one warning STRING
        // per duplicate was itself unbounded -- a permitted 10 MB
        // manifest repeating one short path hundreds of thousands of
        // times could allocate hundreds of megabytes of warning text.
        let many_dupes: Vec<Value> = (0..1_000).map(|_| serde_json::json!("instructions/AGENTS.md")).collect();
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({ "instructions": many_dupes }))),
            file("instructions/AGENTS.md", "Be concise."),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert_eq!(result.instructions, "Be concise.");
        assert_eq!(
            result.warnings.iter().filter(|w| w.contains("duplicate reference(s) skipped")).count(),
            1,
            "expected exactly one summary warning, got: {:?}",
            result.warnings
        );
        assert!(result.warnings.iter().any(|w| w.contains("999 duplicate reference(s) skipped")));
    }

    #[test]
    fn unzip_counts_bytes_read_before_a_decompression_error_toward_the_aggregate() {
        // codex P1, PR #2379 round 6: read_to_end can leave real
        // decompressed bytes in `buf` even when it ultimately returns an
        // error (e.g. a forged CRC on an otherwise-valid entry) -- that
        // decompression work happened regardless, and must count.
        // Directly exercises the ordering fix (check_entry_size runs
        // before the read-error branch) since forging a CRC mismatch
        // through the public ZipWriter API isn't practical.
        let mut total: u64 = 0;
        let mut warnings = WarningSink::new(WarningBudget::unbounded());
        // Simulates: read_to_end returned Err, but buf still holds
        // MAX_ENTRY_UNCOMPRESSED_BYTES bytes of real decompressed data.
        let keep = check_entry_size("corrupt.md", MAX_ENTRY_UNCOMPRESSED_BYTES, &mut total, &mut warnings);
        assert_eq!(keep, Ok(true), "bytes read before the error must still count");
        assert_eq!(total, MAX_ENTRY_UNCOMPRESSED_BYTES);
    }

    #[test]
    fn components_accounts_non_string_value_gets_an_explicit_warning() {
        // reagent P2, PR #2379 round 7: every other component category
        // warns on a malformed value; this one used to silently drop it.
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "accounts": ["not", "a", "string"],
            }))),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert!(result.requirements.is_empty());
        assert!(result.warnings.iter().any(|w| w.contains("components.accounts") && w.contains("non-string value")));
    }

    #[test]
    fn caps_malformed_component_entries_before_they_can_amplify_into_unbounded_warnings() {
        // codex P1, PR #2379 round 7: round 6 bounded the "duplicate
        // reference" warning specifically; a manifest filled with
        // non-string junk hits a DIFFERENT (still unbounded, until this
        // fix) warning path for the same amplification effect.
        // capped_component_array truncates the array itself before any
        // per-entry processing, so this must produce at most one
        // "exceeds the limit" warning plus MAX_ENTRY_COUNT "non-string
        // entry" warnings -- not one per array element.
        let many_junk: Vec<Value> = (0..MAX_ENTRY_COUNT + 500).map(|i| serde_json::json!(i)).collect();
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({ "instructions": many_junk }))),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("components.instructions") && w.contains("exceeds the limit")));
        let non_string_warnings = result.warnings.iter().filter(|w| w.contains("non-string entry skipped")).count();
        assert_eq!(non_string_warnings, MAX_ENTRY_COUNT, "expected exactly the capped count, got {non_string_warnings}");
    }

    #[test]
    fn caps_the_number_of_skills_actually_imported_from_a_single_bundle() {
        // codex P1, PR #2379 round 7: MAX_ENTRY_COUNT bounds how many
        // components.skills entries are even looked at, but that's still
        // far too high a ceiling for the RPC handler's write side --
        // each imported skill becomes a separate synchronous Store
        // transaction creating a permanent global row.
        let n = MAX_IMPORTED_SKILLS + 50;
        let mut manifest_skills: Vec<Value> = Vec::new();
        let mut skill_files: Vec<BundleImportFile> = Vec::new();
        for i in 0..n {
            let dir = format!("skills/s{i}");
            manifest_skills.push(serde_json::json!(dir));
            skill_files.push(file(
                &format!("{dir}/SKILL.md"),
                &format!("---\nname: \"s{i}\"\ndescription: \"d\"\n---\n\nbody"),
            ));
        }
        let mut files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({ "skills": manifest_skills }))),
        ];
        files.extend(skill_files);
        let result = parse_bundle_import(&files).unwrap();
        assert_eq!(result.skills.len(), MAX_IMPORTED_SKILLS);
        assert_eq!(result.skipped_skills.len(), 50);
        assert!(result.warnings.iter().any(|w| w.contains("exceeds the import limit")));
    }

    #[test]
    fn treats_a_version_that_merely_starts_with_0_1_as_unrecognized() {
        // codex P2, PR #2379 round 5: "0.10.0" and "0.1garbage" both start
        // with "0.1" but are not the 0.1.x family the importer actually
        // recognizes.
        for bad_version in ["0.10.0", "0.1garbage"] {
            let manifest = serde_json::to_string(&serde_json::json!({
                "name": "test-bundle",
                "version": bad_version,
                "components": {},
            })).unwrap();
            let files = vec![file("armory.json", &manifest)];
            let result = parse_bundle_import(&files).unwrap();
            assert!(
                result.warnings.iter().any(|w| w.contains("not a recognized")),
                "expected a warning for version {bad_version:?}, got: {:?}",
                result.warnings
            );
        }
    }

    #[test]
    fn recognizes_bare_0_1_with_no_patch_component() {
        let manifest = serde_json::to_string(&serde_json::json!({
            "name": "test-bundle",
            "version": "0.1",
            "components": {},
        })).unwrap();
        let files = vec![file("armory.json", &manifest)];
        let result = parse_bundle_import(&files).unwrap();
        assert!(result.warnings.iter().all(|w| !w.contains("not a recognized")));
    }

    #[test]
    fn a_raw_files_duplicate_path_resolves_to_the_first_occurrence_with_a_warning() {
        // reagent P2, PR #2379 round 5: unzip_bundle_import already warns
        // on a duplicate zip entry; the raw files list reaches by_path
        // construction directly, bypassing that pass, so it previously
        // silently resolved last-write-wins with no warning at all.
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({
                "instructions": ["instructions/AGENTS.md"],
            }))),
            file("instructions/AGENTS.md", "first"),
            file("instructions/AGENTS.md", "second -- should never be used"),
        ];
        let result = parse_bundle_import(&files).unwrap();
        assert_eq!(result.instructions, "first");
        assert!(result.warnings.iter().any(|w| w.contains("instructions/AGENTS.md") && w.contains("duplicate path")));
    }

    // ── raw `files` intake path now shares the zip path's size/count caps
    // (reagent P1, PR #2379 round 5) ──────────────────────────────────

    #[test]
    fn enforce_raw_files_caps_rejects_more_entries_than_the_count_cap() {
        let files: Vec<BundleImportFile> = (0..=MAX_ENTRY_COUNT)
            .map(|i| file(&format!("f{i}.md"), "x"))
            .collect();
        let err = enforce_raw_files_caps(files).unwrap_err();
        assert!(err.contains("entries exceeds the limit"));
    }

    #[test]
    fn enforce_raw_files_caps_skips_a_single_oversized_entry_with_a_warning() {
        let oversized = "a".repeat(MAX_ENTRY_UNCOMPRESSED_BYTES as usize + 1);
        let files = vec![file("armory.json", "{}"), file("big.md", &oversized)];
        let (kept, warnings) = enforce_raw_files_caps(files).unwrap();
        assert!(kept.iter().any(|f| f.path == "armory.json"));
        assert!(!kept.iter().any(|f| f.path == "big.md"));
        assert!(warnings.iter().any(|w| w.contains("big.md") && w.contains("exceeds the per-entry limit")));
    }

    #[test]
    fn enforce_raw_files_caps_rejects_the_whole_import_over_the_aggregate_cap() {
        let each = "a".repeat((MAX_ENTRY_UNCOMPRESSED_BYTES as f64 * 0.9) as usize);
        let files = vec![
            file("a.md", &each), file("b.md", &each), file("c.md", &each),
            file("d.md", &each), file("e.md", &each), file("f.md", &each),
        ];
        let err = enforce_raw_files_caps(files).unwrap_err();
        assert!(err.contains("aggregate decompressed size exceeds"));
    }

    #[test]
    fn enforce_raw_files_caps_counts_a_discarded_oversized_entrys_bytes_toward_the_aggregate() {
        // codex P1, PR #2379 round 5: an entry too large to KEEP must
        // still count toward the aggregate budget -- otherwise many such
        // entries can each force per-entry-cap-sized work while never
        // tripping the aggregate limit. Five entries just over the
        // per-entry cap (~10.5MB each) sum to ~52.5MB > the 50MB
        // aggregate cap, even though every single one is discarded rather
        // than kept.
        let oversized = "a".repeat((MAX_ENTRY_UNCOMPRESSED_BYTES as f64 * 1.05) as usize);
        let files = vec![
            file("a.md", &oversized), file("b.md", &oversized), file("c.md", &oversized),
            file("d.md", &oversized), file("e.md", &oversized),
        ];
        let err = enforce_raw_files_caps(files).unwrap_err();
        assert!(err.contains("aggregate decompressed size exceeds"));
    }

    #[test]
    fn check_entry_size_accounts_bytes_toward_the_aggregate_even_when_the_entry_is_discarded() {
        // codex P1, PR #2379 round 5: directly exercises the shared
        // choke point both unzip_bundle_import (after its backstop read)
        // and enforce_raw_files_caps call -- an entry too large to KEEP
        // must still count toward the aggregate budget first, or many
        // discarded oversized entries could each force per-entry-cap-
        // sized work while the aggregate cap never trips. (A zip-level
        // reproduction would need to forge a declared size that
        // UNDERSTATES real content while still reaching this function --
        // the `zip` crate reads declared size from the central directory,
        // not the local file header this module can cheaply patch, so
        // the invariant is verified directly at its actual implementation
        // site instead.)
        let mut total: u64 = 0;
        let mut warnings = WarningSink::new(WarningBudget::unbounded());
        let oversized = MAX_ENTRY_UNCOMPRESSED_BYTES + 1;
        for i in 0..5 {
            let name = format!("f{i}.md");
            let result = check_entry_size(&name, oversized, &mut total, &mut warnings);
            if i < 4 {
                assert_eq!(result, Ok(false), "entry {i} should be discarded (not an error) with total={total}");
            } else {
                assert!(result.is_err(), "5th entry should trip the aggregate cap now that all 5 discarded entries' bytes were counted (total={total})");
            }
        }
        assert!(warnings.into_vec().iter().filter(|w| w.contains("exceeds the per-entry limit")).count() >= 4);
    }

    #[test]
    fn bounds_an_oversized_manifest_name_at_parse_time() {
        // Phase 3 spec §3.1, round 13: name must be bounded at the parse
        // source (not at a later response boundary) so preview and commit
        // -- which independently re-parse -- always converge on the exact
        // same canonical value.
        let oversized_name = "n".repeat(MAX_BUNDLE_NAME_CHARS + 50);
        let manifest = serde_json::to_string(&serde_json::json!({
            "name": oversized_name,
            "version": "0.1.0",
            "components": {},
        })).unwrap();
        let files = vec![file("armory.json", &manifest)];
        let result_a = parse_bundle_import(&files).unwrap();
        let result_b = parse_bundle_import(&files).unwrap();
        assert_eq!(result_a.name.chars().count(), MAX_BUNDLE_NAME_CHARS);
        assert_eq!(result_a.name, result_b.name, "two independent parses of the same bytes must converge on the identical truncated name");
        assert!(result_a.warnings.iter().any(|w| w.contains("name exceeds") && w.contains("truncated")));
    }

    #[test]
    fn warns_on_unrecognized_version_but_does_not_fail() {
        let manifest = serde_json::to_string(&serde_json::json!({
            "name": "test-bundle",
            "version": "2.0.0",
            "components": {},
        })).unwrap();
        let files = vec![file("armory.json", &manifest)];
        let result = parse_bundle_import(&files).unwrap();
        assert!(result.warnings.iter().any(|w| w.contains("version")));
    }

    #[test]
    fn parse_skill_md_round_trips_special_characters() {
        let rendered = super::super::agent_config::render_skill_md(
            "weird-name",
            "Has a colon: and \"quotes\"",
            "body\nwith\nnewlines",
        );
        let parsed = parse_skill_md(&rendered).expect("parses");
        assert_eq!(parsed.slug, "weird-name");
        assert_eq!(parsed.description, "Has a colon: and \"quotes\"");
        assert_eq!(parsed.content, "body\nwith\nnewlines");
    }

    #[test]
    fn parse_skill_md_rejects_non_matching_shape() {
        assert!(parse_skill_md("not frontmatter at all").is_none());
        assert!(parse_skill_md("---\nname: \"x\"\n---\n\nbody").is_none()); // missing description
        assert!(parse_skill_md("").is_none());
    }

    // ── zip round-trip ──────────────────────────────────────────────

    #[test]
    fn unzips_a_bundle_exported_as_zip() {
        use crate::backend::storage::store::Memory;

        let bundle = Memory {
            id: "b1".to_string(),
            name: "Zip Roundtrip".to_string(),
            description: "desc".to_string(),
            is_blank: false,
            is_global: false,
            provider: String::new(),
            model: String::new(),
            instructions: "Be helpful.".to_string(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
        };
        let export = super::super::bundle_export::export_bundle(&bundle, &[]);
        let zip_bytes = super::super::bundle_export::zip_bundle_export(&export).unwrap();

        let (files, warnings) = unzip_bundle_import(&zip_bytes).unwrap();
        assert!(warnings.is_empty());
        assert!(files.iter().any(|f| f.path == "armory.json"));
        assert!(files.iter().any(|f| f.path == "instructions/AGENTS.md" && f.content == "Be helpful."));

        let parsed = parse_bundle_import(&files).unwrap();
        assert_eq!(parsed.instructions, "Be helpful.");
    }

    #[test]
    fn unzip_rejects_a_non_zip_input() {
        let err = unzip_bundle_import(b"not a zip file").unwrap_err();
        assert!(err.contains("not a valid zip archive"));
    }

    /// Build a zip archive directly from `(path, content)` pairs — no
    /// `bundle_export` involvement, so these tests cover a hand-built
    /// `.abf` independent of the exporter's own wrapping convention.
    fn build_zip(entries: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut buf);
        let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (path, content) in entries {
            writer.start_file(*path, options).unwrap();
            writer.write_all(content.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn unzips_a_flat_bundle_with_no_wrapping_directory() {
        // reagent P1, PR #2379: the spec's §2 explicitly requires an
        // unwrapped directory tree (zipped directly, no bundle-slug
        // folder) to round-trip just like the exporter's own wrapped
        // form. Nested paths must survive UNCHANGED — no stripping.
        let zip_bytes = build_zip(&[
            ("armory.json", "{}"),
            ("instructions/AGENTS.md", "Be concise."),
            ("skills/deploy/SKILL.md", "---\nname: \"deploy\"\ndescription: \"d\"\n---\n\nbody"),
        ]);
        let (files, warnings) = unzip_bundle_import(&zip_bytes).unwrap();
        assert!(warnings.is_empty());
        assert!(files.iter().any(|f| f.path == "armory.json"));
        assert!(files.iter().any(|f| f.path == "instructions/AGENTS.md" && f.content == "Be concise."));
        assert!(files.iter().any(|f| f.path == "skills/deploy/SKILL.md"));
    }

    #[test]
    fn unzips_a_wrapped_bundle_with_a_directory_other_than_the_exporters_own_slug() {
        // The wrapper-detection must work for ANY consistent single root
        // name, not just names zip_bundle_export happens to produce.
        let zip_bytes = build_zip(&[
            ("my-hand-built-bundle/armory.json", "{}"),
            ("my-hand-built-bundle/instructions/AGENTS.md", "Be concise."),
        ]);
        let (files, warnings) = unzip_bundle_import(&zip_bytes).unwrap();
        assert!(warnings.is_empty());
        assert!(files.iter().any(|f| f.path == "armory.json"));
        assert!(files.iter().any(|f| f.path == "instructions/AGENTS.md"));
    }

    #[test]
    fn does_not_strip_when_entries_disagree_on_a_common_wrapper() {
        // An inconsistent archive (some entries wrapped, some not) must
        // not have a wrapper GUESSED at — every entry keeps its raw path,
        // which then fails components.* lookups with a clear "not found"
        // warning downstream rather than silently importing wrong content.
        let zip_bytes = build_zip(&[
            ("root-a/armory.json", "{}"),
            ("root-b/instructions/AGENTS.md", "Be concise."),
        ]);
        let (files, _warnings) = unzip_bundle_import(&zip_bytes).unwrap();
        assert!(files.iter().any(|f| f.path == "root-a/armory.json"));
        assert!(files.iter().any(|f| f.path == "root-b/instructions/AGENTS.md"));
    }

    #[test]
    fn full_import_of_a_flat_hand_built_abf_finds_every_component() {
        // End-to-end: unzip -> parse, for the unwrapped case specifically
        // (the exact scenario reagent's finding said silently broke).
        let manifest = minimal_manifest(serde_json::json!({
            "instructions": ["instructions/AGENTS.md"],
            "skills": ["skills/deploy"],
        }));
        let zip_bytes = build_zip(&[
            ("armory.json", &manifest),
            ("instructions/AGENTS.md", "Be concise."),
            ("skills/deploy/SKILL.md", "---\nname: \"deploy\"\ndescription: \"d\"\n---\n\nbody"),
        ]);
        let (files, warnings) = unzip_bundle_import(&zip_bytes).unwrap();
        assert!(warnings.is_empty());
        let parsed = parse_bundle_import(&files).unwrap();
        assert_eq!(parsed.instructions, "Be concise.");
        assert_eq!(parsed.skills.len(), 1);
        assert!(parsed.warnings.is_empty(), "unexpected warnings: {:?}", parsed.warnings);
    }

    // ── decompression size bounds (Codex P1, PR #2379) ────────────────
    //
    // Content is a repeated single character so it compresses to a few KB
    // in the actual test zip regardless of its declared/logical size —
    // these tests exercise the real cap logic without moving tens of MB
    // through the test binary.

    #[test]
    fn skips_a_single_entry_exceeding_the_per_entry_cap() {
        let oversized = "a".repeat(MAX_ENTRY_UNCOMPRESSED_BYTES as usize + 1);
        let zip_bytes = build_zip(&[
            ("armory.json", "{}"),
            ("instructions/AGENTS.md", &oversized),
        ]);
        let (files, warnings) = unzip_bundle_import(&zip_bytes).unwrap();
        assert!(files.iter().any(|f| f.path == "armory.json"));
        assert!(!files.iter().any(|f| f.path == "instructions/AGENTS.md"));
        assert!(warnings.iter().any(|w| w.contains("instructions/AGENTS.md") && w.contains("exceeds the per-entry limit")));
    }

    #[test]
    fn accepts_an_entry_right_at_the_per_entry_cap() {
        let exactly_at_cap = "a".repeat(MAX_ENTRY_UNCOMPRESSED_BYTES as usize);
        let zip_bytes = build_zip(&[
            ("armory.json", "{}"),
            ("instructions/AGENTS.md", &exactly_at_cap),
        ]);
        let (files, warnings) = unzip_bundle_import(&zip_bytes).unwrap();
        assert!(warnings.is_empty());
        assert!(files.iter().any(|f| f.path == "instructions/AGENTS.md"));
    }

    #[test]
    fn rejects_the_whole_import_when_aggregate_size_exceeds_the_total_cap() {
        // Six entries at ~90% of the per-entry cap each -- individually
        // well under MAX_ENTRY_UNCOMPRESSED_BYTES, but summing to ~5.4x
        // MAX_TOTAL_UNCOMPRESSED_BYTES's actual ratio (6 * 0.9*10MB = 54MB
        // > the 50MB aggregate cap). Real content, so this exercises the
        // aggregate check regardless of whether it's keyed on declared or
        // actual bytes -- see the dedicated "lying declared size" test
        // below for the distinction that matters (reagent P1, round 2).
        let each = "a".repeat((MAX_ENTRY_UNCOMPRESSED_BYTES as f64 * 0.9) as usize);
        let zip_bytes = build_zip(&[
            ("armory.json", "{}"),
            ("instructions/context/a.md", &each),
            ("instructions/context/b.md", &each),
            ("instructions/context/c.md", &each),
            ("instructions/context/d.md", &each),
            ("instructions/context/e.md", &each),
            ("instructions/context/f.md", &each),
        ]);
        let err = unzip_bundle_import(&zip_bytes).unwrap_err();
        assert!(err.contains("aggregate decompressed size exceeds"));
    }

    #[test]
    fn aggregate_cap_is_enforced_against_actual_bytes_even_when_declared_size_understates_them() {
        // reagent P1, PR #2379 round 2: the aggregate check must accumulate
        // ACTUAL decompressed bytes, not the zip's own declared/attacker-
        // controlled `entry.size()` -- otherwise many entries that each lie
        // with a tiny declared size while really containing content up to
        // the per-entry cap would pass both checks and still exhaust
        // memory. `ZipWriter`'s public API always writes a correct
        // declared size for real content, so this is verified by patching
        // the archive's declared-size fields post-write to a tiny value
        // while leaving the real (large) compressed/uncompressed payload
        // bytes untouched -- exactly the "lying" shape the finding
        // describes. The local file header's uncompressed-size field sits
        // at a fixed offset (22) from the start of each entry's header.
        let each = "a".repeat((MAX_ENTRY_UNCOMPRESSED_BYTES as f64 * 0.9) as usize);
        let mut zip_bytes = build_zip(&[
            ("armory.json", "{}"),
            ("instructions/context/a.md", &each),
            ("instructions/context/b.md", &each),
            ("instructions/context/c.md", &each),
            ("instructions/context/d.md", &each),
            ("instructions/context/e.md", &each),
            ("instructions/context/f.md", &each),
        ]);
        // Local file header signature: 0x04034b50, little-endian bytes
        // 50 4B 03 04. Uncompressed size is the u32 at header offset 22.
        let sig = [0x50u8, 0x4b, 0x03, 0x04];
        let mut i = 0usize;
        let mut patched = 0;
        while i + 26 <= zip_bytes.len() {
            if zip_bytes[i..i + 4] == sig {
                zip_bytes[i + 22..i + 26].copy_from_slice(&1u32.to_le_bytes());
                patched += 1;
            }
            i += 1;
        }
        assert!(patched >= 6, "expected to patch every local file header, patched {patched}");
        let err = unzip_bundle_import(&zip_bytes).unwrap_err();
        assert!(
            err.contains("aggregate decompressed size exceeds"),
            "declared-size lie should not bypass the aggregate cap: {err:?}"
        );
    }

    #[test]
    fn rejects_an_archive_with_more_entries_than_the_count_cap() {
        // reagent P2, PR #2379 round 2: per-entry/aggregate byte caps alone
        // don't bound the CPU cost of iterating + sanitizing + hashing an
        // archive with an enormous number of small, individually-valid
        // entries.
        let mut entries: Vec<(String, String)> = (0..=MAX_ENTRY_COUNT)
            .map(|i| (format!("instructions/context/f{i}.md"), "x".to_string()))
            .collect();
        entries.push(("armory.json".to_string(), "{}".to_string()));
        let refs: Vec<(&str, &str)> = entries.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect();
        let zip_bytes = build_zip(&refs);
        let err = unzip_bundle_import(&zip_bytes).unwrap_err();
        assert!(err.contains("entries exceeds the limit"));
    }

    // ── Phase 3 shared helpers (SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md) ──

    #[test]
    fn warning_budget_bounds_accumulation_during_parsing_itself() {
        // codex P2, PR #2381, round 11: the cap must live at the parser's
        // own warning-push sites, not just at a later response-boundary
        // projection -- called directly here (not through an RPC response
        // serializer) to prove the returned Vec is bounded regardless of
        // any downstream projection.
        let many_junk: Vec<Value> = (0..5_000).map(|i| serde_json::json!(i)).collect();
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({ "instructions": many_junk }))),
        ];
        let result = parse_bundle_import_with_budget(&files, WarningBudget::bounded(50, 40)).unwrap();
        assert!(result.warnings.len() <= 51, "expected at most 50 warnings plus one summary, got {}", result.warnings.len());
        assert!(result.warnings.iter().any(|w| w.contains("more warning(s) not shown")));
        for w in &result.warnings {
            // truncate_display appends "..." after taking max_chars, so the
            // hard ceiling is max_len + 3, not max_len exactly.
            assert!(w.chars().count() <= 43, "warning exceeded the budget's max length: {w:?}");
        }
    }

    #[test]
    fn warning_budget_unbounded_matches_todays_existing_route_behavior() {
        // parse_bundle_import (no budget arg) must behave identically to
        // parse_bundle_import_with_budget(files, WarningBudget::unbounded())
        // -- the existing bundle.import route's real, already-shipped
        // behavior must not change.
        let many_junk: Vec<Value> = (0..500).map(|i| serde_json::json!(i)).collect();
        let files = vec![
            file("armory.json", &minimal_manifest(serde_json::json!({ "instructions": many_junk.clone() }))),
        ];
        let a = parse_bundle_import(&files).unwrap();
        let b = parse_bundle_import_with_budget(&files, WarningBudget::unbounded()).unwrap();
        assert_eq!(a.warnings, b.warnings);
        assert_eq!(a.warnings.len(), 500);
    }

    #[test]
    fn duplicate_in_bundle_slugs_flags_only_slugs_appearing_more_than_once() {
        let skills = vec![
            ParsedSkill { source_dir: "skills/a".to_string(), slug: "code-review".to_string(), description: String::new(), content: String::new() },
            ParsedSkill { source_dir: "skills/b".to_string(), slug: "code-review".to_string(), description: String::new(), content: String::new() },
            ParsedSkill { source_dir: "skills/c".to_string(), slug: "unique".to_string(), description: String::new(), content: String::new() },
        ];
        let dupes = duplicate_in_bundle_slugs(&skills);
        assert!(dupes.contains("code-review"));
        assert!(!dupes.contains("unique"));
        assert_eq!(dupes.len(), 1);
    }

    #[test]
    fn classify_skill_collision_prioritizes_global_catalog_over_intra_bundle_duplicate() {
        // Phase 3 spec §3.1: pass 1 (global catalog) takes priority over
        // pass 2 (intra-bundle duplicate) when both apply.
        let mut global: HashSet<String> = HashSet::new();
        global.insert("taken".to_string());
        let mut dupes: HashSet<String> = HashSet::new();
        dupes.insert("taken".to_string());
        dupes.insert("dupe-only".to_string());
        assert_eq!(classify_skill_collision("taken", &global, &dupes), "name_conflict");
        assert_eq!(classify_skill_collision("dupe-only", &global, &dupes), "duplicate_in_bundle");
        assert_eq!(classify_skill_collision("free", &global, &dupes), "none");
    }

    #[test]
    fn truncate_display_truncates_only_when_over_the_cap() {
        assert_eq!(truncate_display("short", 100), "short");
        let long = "a".repeat(150);
        let truncated = truncate_display(&long, 100);
        assert_eq!(truncated.chars().count(), 103); // 100 chars + "..."
        assert!(truncated.ends_with("..."));
    }

    #[test]
    fn bounded_instructions_preview_reports_truncation_and_true_total_length() {
        let long = "x".repeat(MAX_INSTRUCTIONS_PREVIEW_CHARS + 500);
        let (preview, truncated, total) = bounded_instructions_preview(&long);
        assert!(truncated);
        assert_eq!(total, MAX_INSTRUCTIONS_PREVIEW_CHARS + 500);
        assert_eq!(preview.chars().count(), MAX_INSTRUCTIONS_PREVIEW_CHARS);

        let short = "short instructions";
        let (preview2, truncated2, total2) = bounded_instructions_preview(short);
        assert!(!truncated2);
        assert_eq!(total2, short.chars().count());
        assert_eq!(preview2, short);
    }

    #[test]
    fn mcp_server_display_never_returns_the_full_config() {
        let config = serde_json::json!({
            "name": "github",
            "command": "npx",
            "args": ["-y", "gh-mcp"],
            "env": { "GITHUB_TOKEN": "${GITHUB_TOKEN}" },
        });
        let display = mcp_server_display(&config);
        assert_eq!(display["name"], "github");
        assert_eq!(display["command"], "npx");
        assert!(display.get("args").is_none());
        assert!(display.get("env").is_none());
    }

    #[test]
    fn mcp_server_display_bounds_an_oversized_name_or_command() {
        let oversized = "n".repeat(MAX_MCP_DISPLAY_FIELD_CHARS + 50);
        let config = serde_json::json!({ "name": oversized, "command": "npx" });
        let display = mcp_server_display(&config);
        let name = display["name"].as_str().unwrap();
        assert!(name.chars().count() <= MAX_MCP_DISPLAY_FIELD_CHARS + 3);
    }

    #[test]
    fn mcp_server_display_falls_back_to_null_when_fields_absent_or_wrong_type() {
        let config = serde_json::json!({ "command": 123 });
        let display = mcp_server_display(&config);
        assert!(display["name"].is_null());
        assert!(display["command"].is_null());
    }

    #[test]
    fn content_digest_raw_bytes_differs_by_mode_for_identical_bytes() {
        // Phase 3 spec §3.0.5, round 7: file_path and zip_base64 both
        // canonicalize to the same raw zip bytes -- without a mode tag
        // mixed into the hash domain, a file_path preview would be
        // satisfiable by a zip_base64 commit of the same underlying
        // archive, defeating the round-6 same-mode-required fix.
        let bytes = b"identical zip bytes";
        let a = content_digest_raw_bytes(ImportInputMode::FilePath, bytes);
        let b = content_digest_raw_bytes(ImportInputMode::ZipBase64, bytes);
        assert_ne!(a, b);
        // Same mode, same bytes -> identical digest, deterministically.
        assert_eq!(a, content_digest_raw_bytes(ImportInputMode::FilePath, bytes));
    }

    #[test]
    fn content_digest_files_is_order_independent_for_genuinely_equivalent_inputs() {
        let a = vec![file("armory.json", "{}"), file("instructions/AGENTS.md", "Be concise.")];
        let b = vec![file("instructions/AGENTS.md", "Be concise."), file("armory.json", "{}")];
        assert_eq!(content_digest_files(&a), content_digest_files(&b));
    }

    #[test]
    fn content_digest_files_changes_when_reordering_changes_the_first_wins_outcome() {
        // codex P1, PR #2381, round 6: a naive raw-input sort would make
        // two differently-ordered request bodies hash identically even
        // when the parser's own first-wins rule would actually import
        // DIFFERENT content from each (whichever happened to be first).
        let a = vec![file("instructions/AGENTS.md", "first wins"), file("instructions/AGENTS.md", "second, discarded")];
        let b = vec![file("instructions/AGENTS.md", "second, discarded"), file("instructions/AGENTS.md", "first wins")];
        assert_ne!(
            content_digest_files(&a),
            content_digest_files(&b),
            "reordering which entry wins the normalize-then-first-wins reduction must change the digest"
        );
    }
}
