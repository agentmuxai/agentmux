// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Provider classification: [`ProviderClass`] and [`provider_class`].
//!
//! Split out of the single ~2193-line `resolver.rs` (pure relocation, no
//! behavior change).

/// What kind of credential a provider uses, and how
/// `inject_identity_env` puts it into the agent's env at spawn time.
/// Per `specs/archive/SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md` §4.3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderClass {
    /// **API-key class.** The binding's `SecretRef` resolves to a
    /// single secret string, injected as the listed env vars. All
    /// listed vars receive the same value — multi-var emission
    /// covers "two CLIs want different var names for the same secret"
    /// (e.g. github writes both `GITHUB_TOKEN` and `GH_TOKEN`).
    ApiKey { env_vars: &'static [&'static str] },
    /// **OAuth class.** The binding's `SecretRef` is a
    /// `SecretRef::OAuthConfigDir` pointer; the resolver sets
    /// `config_dir_env_var = <dir>` at spawn so the CLI reads its
    /// OAuth tokens from the per-bundle directory.
    OAuth { config_dir_env_var: &'static str },
}

/// Classify a provider id. `None` for unknown providers — the
/// resolver logs and skips them.
///
/// reagent P1 on #2263: this used to match only canonical IDs directly, but
/// `backend/providers.rs` registers aliases (`gemini-cli`→`gemini`,
/// `copilot-cli`/`github-copilot`→`copilot`, `claude-code`→`claude`, etc.)
/// that `get_provider` already resolves — meaning `provider_class` and
/// `get_provider` could disagree on a definition/link still using an alias
/// ID, silently skipping both the spawn gate and config-dir injection for
/// it. Resolve to the canonical ID first so the two can never drift.
///
/// `resolve_provider_alias` only knows the CLI-tool registry (claude/codex/
/// gemini/etc.) — it returns `""` as a sentinel for anything outside that
/// registry, which includes the api-key-class service identifiers below
/// ("github"/"anthropic"/"openai"/"kimi"/"aws" — a completely different
/// namespace, not CLI tools). Only substitute the resolved value when it's
/// non-empty; otherwise keep matching on the original id so that namespace
/// is untouched.
pub fn provider_class(provider: &str) -> Option<ProviderClass> {
    let resolved = crate::backend::providers::resolve_provider_alias(provider);
    let provider = if resolved.is_empty() { provider } else { resolved };
    match provider {
        // ── API-key class ─────────────────────────────────────────
        // ApiKey.env_vars values match the legacy provider_env_vars
        // matrix exactly — the new dispatch is additive.
        "github" => Some(ProviderClass::ApiKey {
            env_vars: &["GITHUB_TOKEN", "GH_TOKEN"],
        }),
        "anthropic" => Some(ProviderClass::ApiKey {
            env_vars: &["ANTHROPIC_API_KEY"],
        }),
        "openai" => Some(ProviderClass::ApiKey {
            env_vars: &["OPENAI_API_KEY"],
        }),
        "kimi" => Some(ProviderClass::ApiKey {
            env_vars: &["MOONSHOT_API_KEY"],
        }),
        "aws" => Some(ProviderClass::ApiKey {
            env_vars: &["AWS_ACCESS_KEY_ID"],
        }),
        // ── OAuth class ───────────────────────────────────────────
        // Env-var names come from the CLI provider registry
        // (`agentmux-srv/src/backend/providers.rs` —
        // `ProviderConfig::auth_config_dir_env_var`) so the resolver
        // can never drift from the launcher spawn path: there is one
        // source of truth per CLI for which env var redirects its
        // config / auth directory. The match arm enumerates which
        // providers we currently treat as OAuth-class for identity
        // bundles. Originally just claude / codex / openclaw (spec
        // §4.3) — gemini and copilot were added later
        // (REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md
        // §2.5 / §6, Phase C) to close a drift gap: the frontend's
        // `ProviderDefinition` table already marked both
        // `authType: "oauth"`, but this match arm (the actual gate
        // for the spawn-time enforcement AND the per-account
        // isolation-dir minting in identity_handlers.rs, both of
        // which key off this single function) hadn't caught up —
        // meaning neither actually applied to them despite the UI
        // already presenting them as oauth-class. See
        // `oauth_class_matches_frontend_authtype_oauth_set` below,
        // which pins this set staying in sync with the frontend going
        // forward so this doesn't silently drift again.
        "claude" | "codex" | "openclaw" | "gemini" | "copilot" => {
            crate::backend::providers::get_provider(provider).map(|cfg| {
                ProviderClass::OAuth {
                    config_dir_env_var: cfg.auth_config_dir_env_var,
                }
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_class_oauth_providers() {
        // The known oauth providers must classify as OAuth with the SAME
        // config-dir env vars the CLI provider registry defines (single
        // source of truth). Pinning the expected strings here catches drift
        // in either direction — if the registry changes a value, this test
        // fails and the change becomes deliberate.
        assert_eq!(
            provider_class("claude"),
            Some(ProviderClass::OAuth { config_dir_env_var: "CLAUDE_CONFIG_DIR" }),
        );
        assert_eq!(
            provider_class("codex"),
            Some(ProviderClass::OAuth { config_dir_env_var: "CODEX_HOME" }),
        );
        assert_eq!(
            provider_class("openclaw"),
            Some(ProviderClass::OAuth { config_dir_env_var: "OPENCLAW_HOME" }),
        );
        assert_eq!(
            provider_class("gemini"),
            Some(ProviderClass::OAuth { config_dir_env_var: "GEMINI_CLI_HOME" }),
        );
        assert_eq!(
            provider_class("copilot"),
            Some(ProviderClass::OAuth { config_dir_env_var: "COPILOT_HOME" }),
        );
    }

    #[test]
    fn oauth_class_matches_frontend_authtype_oauth_set() {
        // REPORT_AUTH_ARCHITECTURE_STATE_AND_RETHINK_2026_07_21.md §2.5
        // found gemini/copilot marked `authType: "oauth"` in the frontend's
        // `ProviderDefinition` table (frontend/app/view/agent/providers/
        // index.ts) while this function — the actual gate for both the
        // spawn-time enforcement AND the per-account isolation-dir minting
        // — hadn't caught up, so neither mechanism applied to them despite
        // the UI already presenting them as oauth-class. This pins the two
        // sets staying equal going forward. There's no automated cross-
        // language check available, so this list is a manually-maintained
        // mirror of the frontend table — if you add a new `authType:
        // "oauth"` provider there, update FRONTEND_OAUTH_TYPED here too, in
        // the SAME change, not as a follow-up.
        const FRONTEND_OAUTH_TYPED: &[&str] = &["claude", "codex", "gemini", "openclaw", "copilot"];
        const ALL_KNOWN_PROVIDERS: &[&str] = &[
            "claude", "codex", "muxcode", "gemini", "qwen", "openclaw", "kimi", "copilot", "pi",
        ];
        for p in ALL_KNOWN_PROVIDERS {
            let is_oauth_class = matches!(provider_class(p), Some(ProviderClass::OAuth { .. }));
            let is_frontend_oauth_typed = FRONTEND_OAUTH_TYPED.contains(p);
            assert_eq!(
                is_oauth_class, is_frontend_oauth_typed,
                "provider '{p}': backend OAuth-class ({is_oauth_class}) must match \
                 frontend authType:\"oauth\" ({is_frontend_oauth_typed}) — see this \
                 test's doc comment",
            );
        }
    }

    #[test]
    fn provider_class_resolves_aliases_to_the_same_result_as_canonical() {
        // reagent P1 on #2263: provider_class used to match only canonical
        // IDs, silently disagreeing with get_provider (which already
        // resolves aliases) for any definition/link still using one.
        assert_eq!(provider_class("claude-code"), provider_class("claude"));
        assert_eq!(provider_class("claude_code"), provider_class("claude"));
        assert_eq!(provider_class("codex-cli"), provider_class("codex"));
        assert_eq!(provider_class("openclaw-cli"), provider_class("openclaw"));
        assert_eq!(provider_class("open-claw"), provider_class("openclaw"));
        // Api-key-class aliases must resolve identically too — this isn't
        // gated on oauth-class providers specifically.
        assert_eq!(provider_class("kimi-cli"), provider_class("kimi"));
        // A truly unknown id must still classify as None, not panic or
        // silently match something via an empty-string fallback.
        assert_eq!(provider_class("totally-unknown-provider-xyz"), None);
    }
}
