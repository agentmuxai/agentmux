// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, Show, type JSX } from "solid-js";
import type {
    Account,
    AccountContext,
    AccountKind,
    AccountProvider,
    IdentityViewModel,
    SecretRef,
} from "./identity-model";
import { PROVIDER_LABELS, refreshAccountCache } from "./identity-model";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { OAuthConnectPanel } from "@/app/view/accounts/OAuthConnectPanel";
import { supportsOAuth } from "@/app/view/accounts/oauth-catalog";
import "./identity-view.scss";
import "@/app/view/accounts/oauth-connect.scss";

// Per-provider validation endpoint, surfaced in the egress help note next to
// the Validate button so the user sees exactly where their key is sent before
// they click. Mirrors the backend probes in key_validator.rs. Providers absent
// here have no validator yet → the key can only be saved without validating.
// See docs/specs/archive/SPEC_TRUST_CENTER_2026_06_15.md §5.1/§6.
const KEY_VALIDATION_ENDPOINT: Partial<Record<AccountProvider, string>> = {
    github: "api.github.com/user",
    openai: "api.openai.com/v1/models",
    anthropic: "api.anthropic.com/v1/models",
};

// ── Add/Edit form ─────────────────────────────────────────────────────────────

// Default account kind for a provider — used to reset the Kind select on a
// provider switch so it never holds a value invalid for the new provider.
// Google/Slack are OAuth-only; GitHub/AWS default to their primary key kind.
function defaultKindFor(provider: AccountProvider): AccountKind {
    if (provider === "google" || provider === "slack") return "oauth";
    if (provider === "github") return "pat";
    if (provider === "aws") return "role";
    return "api_key"; // openai, anthropic, custom
}

export function AccountForm({ model }: { model: IdentityViewModel }): JSX.Element {
    const editing = () => model.editingAccountAtom();
    const isEdit = () => editing() !== null;

    // Preset from a brand tile (Accounts gallery → openAddFormFor). Only used
    // for a fresh Add (editing wins). Read once at setup; the form remounts on
    // each open so this is always current.
    const preset = model.addPresetAtom();

    // Form field signals
    const [name, setName] = createSignal(editing()?.name ?? "");
    const [provider, setProvider] = createSignal<AccountProvider>(editing()?.provider ?? preset?.provider ?? "github");
    const [kind, setKind] = createSignal<AccountKind>(editing()?.kind ?? preset?.kind ?? "pat");
    const [displayName, setDisplayName] = createSignal(editing()?.display_name ?? "");
    const [secretBackend, setSecretBackend] = createSignal<SecretRef["backend"]>(editing()?.secret_ref?.backend ?? "keychain");
    const [secretEnvVar, setSecretEnvVar] = createSignal(editing()?.secret_ref?.env_var ?? "");
    const [secretSmPath, setSecretSmPath] = createSignal(editing()?.secret_ref?.sm_path ?? "");
    const [secretSmJsonPath, setSecretSmJsonPath] = createSignal(editing()?.secret_ref?.sm_json_path ?? "");
    const [secretValue, setSecretValue] = createSignal(editing()?.secret_ref?.value ?? "");
    // Context
    const [ghUsername, setGhUsername] = createSignal(editing()?.context.github_username ?? "");
    const [ghScopes, setGhScopes] = createSignal(editing()?.context.github_scopes?.join(", ") ?? "");
    const [awsProfile, setAwsProfile] = createSignal(editing()?.context.aws_profile ?? "");
    const [awsRoleArn, setAwsRoleArn] = createSignal(editing()?.context.aws_role_arn ?? "");
    const [awsRegion, setAwsRegion] = createSignal(editing()?.context.aws_region ?? "");
    const [anthropicModel, setAnthropicModel] = createSignal(editing()?.context.anthropic_model ?? "");
    const [description, setDescription] = createSignal(editing()?.context.description ?? "");

    // ── Armory secure-key lifecycle (SPEC_TRUST_CENTER §5) ──
    // Active when secretBackend === "keychain": the user pastes a key and
    // clicks Validate (single user-initiated egress) or "Save without
    // validating". The plaintext is sent once to the backend, which stores it
    // in the OS keychain and returns only a masked tail + metadata — it is
    // never read back into the UI. Existing keychain accounts render locked
    // (masked) and require Replace to re-enter.
    const editingIsKeychain = editing()?.secret_ref?.backend === "keychain";
    const [keychainKey, setKeychainKey] = createSignal("");
    const [keyBusy, setKeyBusy] = createSignal(false);
    const [keyError, setKeyError] = createSignal<string | null>(null);
    // For an existing keychain account, start locked (masked) until Replace.
    const [keyReplacing, setKeyReplacing] = createSignal(!editingIsKeychain);
    const validationEndpoint = (): string | undefined => KEY_VALIDATION_ENDPOINT[provider()];

    // Non-rendered context keys: not editable in the form, so they must be
    // carried across an edit rather than dropped. Everything else is rebuilt
    // fresh from the form so cleared fields are removed and a provider switch
    // doesn't leak stale provider-specific keys. (github_username/scopes are
    // form fields, so they intentionally follow the form, not this list.)
    const PRESERVED_CONTEXT_KEYS: (keyof AccountContext)[] = [
        "masked_tail",
        "openai_model_count",
        "anthropic_model_count",
        "slack_team",
        "slack_user",
    ];

    // Build the non-secret context fresh from the form fields, then re-apply
    // only the non-rendered keys (masked_tail + validation metadata) so a
    // metadata-only edit doesn't wipe them.
    const buildContext = (): AccountContext => {
        const context: AccountContext = {};
        if (provider() === "github") {
            if (ghUsername()) context.github_username = ghUsername().trim();
            if (ghScopes()) context.github_scopes = ghScopes().split(",").map((s) => s.trim()).filter(Boolean);
        }
        if (provider() === "aws") {
            if (awsProfile()) context.aws_profile = awsProfile().trim();
            if (awsRoleArn()) context.aws_role_arn = awsRoleArn().trim();
            if (awsRegion()) context.aws_region = awsRegion().trim();
        }
        if (provider() === "anthropic") {
            if (anthropicModel()) context.anthropic_model = anthropicModel().trim();
        }
        if (description()) context.description = description().trim();
        const prior = editing()?.context;
        if (prior) {
            for (const k of PRESERVED_CONTEXT_KEYS) {
                if (prior[k] !== undefined) (context[k] as unknown) = prior[k];
            }
        }
        return context;
    };

    const buildAccount = (): Omit<Account, "id" | "status" | "created_at" | "updated_at"> | null => {
        const n = name().trim();
        if (!n) {
            model["setFormError"]("Name is required");
            return null;
        }

        const secretRef: SecretRef = { backend: secretBackend() };
        if (secretBackend() === "env") secretRef.env_var = secretEnvVar().trim();
        if (secretBackend() === "secrets_manager") {
            secretRef.sm_path = secretSmPath().trim();
            secretRef.sm_json_path = secretSmJsonPath().trim() || undefined;
        }
        if (secretBackend() === "plaintext_dev") secretRef.value = secretValue();
        if (secretBackend() === "keychain") {
            // Preserve the existing keychain pointer (service/account) so a
            // metadata-only Save on a locked account doesn't drop it. New
            // keychain accounts go through the key flow, not this path.
            secretRef.service = editing()?.secret_ref?.service;
            secretRef.account = editing()?.secret_ref?.account;
        }

        const context = buildContext();

        return {
            name: n,
            provider: provider(),
            kind: kind(),
            display_name: displayName().trim() || undefined,
            secret_ref: secretRef,
            context,
            // assigned_agents is deprecated and never read by the backend —
            // see identity-model.ts's Account.assigned_agents doc comment.
            // Real assignment happens via the agent-side reverse index.
            assigned_agents: [],
        };
    };

    const handleSubmit = () => {
        const data = buildAccount();
        if (!data) return;
        if (isEdit()) {
            model.updateAccount(editing()!.id, data);
        } else {
            model.createAccount(data);
        }
    };

    // Secure-key submit. `validate` true → the backend runs one live probe
    // against the service (user-initiated egress) before storing; false →
    // store with status "unknown" (the air-gapped escape hatch). On success
    // the key lives only in the OS keychain; we refresh the cache and close.
    const submitKey = async (validate: boolean) => {
        const n = name().trim();
        if (!n) {
            setKeyError("Name is required");
            return;
        }
        if (!keychainKey().trim()) {
            setKeyError("Enter a key first");
            return;
        }
        setKeyBusy(true);
        setKeyError(null);
        try {
            const res = await RpcApi.AccountKeyVerifyCommand(TabRpcClient, {
                provider: provider(),
                name: n,
                displayName: displayName().trim() || undefined,
                kind: kind(),
                apiKey: keychainKey(),
                validate,
                accountId: isEdit() ? editing()!.id : undefined,
                // Carry the user-entered context so the backend merges (not
                // drops) github_username/scopes/notes on a key change.
                context: buildContext() as unknown as Record<string, unknown>,
            });
            // Drop the plaintext from the form immediately.
            setKeychainKey("");
            if (!res.valid && validate) {
                setKeyError(res.error ?? "Validation failed");
                return;
            }
            await refreshAccountCache();
            model.cancelForm();
        } catch (err) {
            setKeyError((err as Error)?.message ?? String(err));
        } finally {
            setKeyBusy(false);
        }
    };

    return (
        <div class="identity-form-overlay" onClick={(e) => e.target === e.currentTarget && model.cancelForm()}>
            <div class="identity-form">
                <div class="identity-form-header">
                    <span>{isEdit() ? "Edit Account" : "Add Account"}</span>
                    <button class="identity-form-close" onClick={() => model.cancelForm()}>✕</button>
                </div>

                <div class="identity-form-body">
                    <Show when={model.formErrorAtom()}>
                        <div class="identity-form-error">{model.formErrorAtom()}</div>
                    </Show>

                    <FormField label="Name *">
                        <input
                            class="identity-input"
                            type="text"
                            value={name()}
                            onInput={(e) => setName(e.currentTarget.value)}
                            placeholder="GitHub agent1-workflow"
                        />
                    </FormField>

                    <FormField label="Provider">
                        <select
                            class="identity-select"
                            value={provider()}
                            onChange={(e) => {
                                const p = e.currentTarget.value as AccountProvider;
                                setProvider(p);
                                // Kinds are provider-specific; reset to a valid
                                // default for the new provider so the Kind select
                                // never shows a stale/invalid value.
                                setKind(defaultKindFor(p));
                            }}
                        >
                            <option value="github">GitHub</option>
                            <option value="google">Google</option>
                            <option value="aws">AWS</option>
                            <option value="openai">OpenAI</option>
                            <option value="anthropic">Anthropic</option>
                            <option value="slack">Slack</option>
                            <option value="custom">Custom</option>
                            <option value="agentmux">AgentMux</option>
                        </select>
                    </FormField>

                    <FormField label="Kind">
                        <select class="identity-select" value={kind()} onChange={(e) => setKind(e.currentTarget.value as AccountKind)}>
                            {/* Browser-login path for OAuth-capable providers. */}
                            <Show when={supportsOAuth(provider())}>
                                <option value="oauth">Connect with OAuth (browser login)</option>
                            </Show>
                            <Show when={provider() === "github"}>
                                <option value="pat">Personal Access Token</option>
                                <option value="api_key">API Key</option>
                            </Show>
                            <Show when={provider() === "aws"}>
                                <option value="role">IAM Role</option>
                                <option value="env_ref">Env Reference</option>
                            </Show>
                            <Show when={provider() === "openai"}>
                                <option value="api_key">API Key</option>
                            </Show>
                            <Show when={provider() === "anthropic"}>
                                <option value="api_key">API Key</option>
                            </Show>
                            <Show when={provider() === "custom"}>
                                <option value="api_key">API Key</option>
                                <option value="env_ref">Env Reference</option>
                                <option value="pat">Token</option>
                            </Show>
                            <Show when={provider() === "agentmux"}>
                                <option value="api_key">API Key</option>
                            </Show>
                        </select>
                    </FormField>

                    <FormField label="Display name">
                        <input
                            class="identity-input"
                            type="text"
                            value={displayName()}
                            onInput={(e) => setDisplayName(e.currentTarget.value)}
                            placeholder="agent1-workflow (username / alias)"
                        />
                    </FormField>

                    {/* OAuth (browser login) replaces the secret/key sections —
                        on success the backend creates the keychain-backed account
                        itself, so this form's secret-ref + Save path is skipped. */}
                    <Show when={kind() === "oauth"}>
                        <OAuthConnectPanel
                            provider={provider()}
                            name={name}
                            onConnected={() => model.cancelForm()}
                        />
                    </Show>

                    <Show when={kind() !== "oauth"}>
                    {/* Secret storage */}
                    <FormField label="Secret backend">
                        <select class="identity-select" value={secretBackend()} onChange={(e) => setSecretBackend(e.currentTarget.value as SecretRef["backend"])}>
                            <option value="keychain">OS Keychain (validated) — recommended</option>
                            <option value="env">Environment variable</option>
                            <option value="secrets_manager">AWS Secrets Manager</option>
                            <option value="plaintext_dev">Plaintext (dev only ⚠)</option>
                        </select>
                    </FormField>

                    {/* ── Secure key lifecycle (keychain backend) ── */}
                    <Show when={secretBackend() === "keychain"}>
                        <Show
                            when={keyReplacing()}
                            fallback={
                                <div class="identity-key-locked">
                                    <span class="identity-key-masked">
                                        {editing()?.context.masked_tail ?? "••••••••"}
                                    </span>
                                    <span class="identity-key-locked-note">
                                        Stored in the OS keychain · not recoverable
                                    </span>
                                    <button
                                        class="identity-btn identity-btn-secondary"
                                        onClick={() => setKeyReplacing(true)}
                                    >
                                        Replace key
                                    </button>
                                </div>
                            }
                        >
                            <FormField label="Key / token">
                                <input
                                    class="identity-input"
                                    type="password"
                                    autocomplete="off"
                                    spellcheck={false}
                                    value={keychainKey()}
                                    onInput={(e) => setKeychainKey(e.currentTarget.value)}
                                    placeholder="paste key — never stored in plaintext"
                                />
                            </FormField>
                            {/* §5.1 egress transparency — placed at the point of action */}
                            <Show
                                when={validationEndpoint()}
                                fallback={
                                    <div class="identity-key-egress-note">
                                        ⓘ No validator for {PROVIDER_LABELS[provider()]} yet — use
                                        “Save without validating”. The key goes straight to your OS
                                        keychain; nothing is sent anywhere.
                                    </div>
                                }
                            >
                                <div class="identity-key-egress-note">
                                    ⓘ Clicking <strong>Validate &amp; Save</strong> sends this key once,
                                    over HTTPS, from the AgentMux backend on your machine directly to{" "}
                                    <code>{validationEndpoint()}</code> to confirm it works and read its
                                    details. The key is never stored in plaintext, never logged, and not
                                    sent anywhere else. After saving it lives in your OS keychain and
                                    can’t be viewed again.
                                </div>
                            </Show>
                            <Show when={keyError()}>
                                <div class="identity-form-error">{keyError()}</div>
                            </Show>
                            <div class="identity-key-actions">
                                <Show when={validationEndpoint()}>
                                    <button
                                        class="identity-btn identity-btn-primary"
                                        disabled={keyBusy()}
                                        onClick={() => void submitKey(true)}
                                    >
                                        {keyBusy() ? "Validating…" : "Validate & Save"}
                                    </button>
                                </Show>
                                <button
                                    class="identity-btn identity-btn-secondary"
                                    disabled={keyBusy()}
                                    onClick={() => void submitKey(false)}
                                    title="Store the key without contacting the service"
                                >
                                    Save without validating
                                </button>
                            </div>
                        </Show>
                    </Show>

                    <Show when={secretBackend() === "env"}>
                        <FormField label="Env var name">
                            <input
                                class="identity-input"
                                type="text"
                                value={secretEnvVar()}
                                onInput={(e) => setSecretEnvVar(e.currentTarget.value)}
                                placeholder="GH_TOKEN"
                            />
                        </FormField>
                    </Show>

                    <Show when={secretBackend() === "secrets_manager"}>
                        <FormField label="Secret path">
                            <input
                                class="identity-input"
                                type="text"
                                value={secretSmPath()}
                                onInput={(e) => setSecretSmPath(e.currentTarget.value)}
                                placeholder="services/infra"
                            />
                        </FormField>
                        <FormField label="JSON path (optional)">
                            <input
                                class="identity-input"
                                type="text"
                                value={secretSmJsonPath()}
                                onInput={(e) => setSecretSmJsonPath(e.currentTarget.value)}
                                placeholder=".gh-token"
                            />
                        </FormField>
                    </Show>

                    <Show when={secretBackend() === "plaintext_dev"}>
                        <div class="identity-form-warning">⚠ Stored in localStorage — for dev/testing only</div>
                        <FormField label="Value">
                            <input
                                class="identity-input"
                                type="password"
                                value={secretValue()}
                                onInput={(e) => setSecretValue(e.currentTarget.value)}
                                placeholder="secret value"
                            />
                        </FormField>
                    </Show>

                    {/* Provider-specific context */}
                    <Show when={provider() === "github"}>
                        <FormField label="GitHub username">
                            <input class="identity-input" type="text" value={ghUsername()} onInput={(e) => setGhUsername(e.currentTarget.value)} placeholder="agent1-workflow" />
                        </FormField>
                        <FormField label="Scopes (comma-separated)">
                            <input class="identity-input" type="text" value={ghScopes()} onInput={(e) => setGhScopes(e.currentTarget.value)} placeholder="repo, workflow, read:org" />
                        </FormField>
                    </Show>

                    <Show when={provider() === "aws"}>
                        <FormField label="AWS profile">
                            <input class="identity-input" type="text" value={awsProfile()} onInput={(e) => setAwsProfile(e.currentTarget.value)} placeholder="dev" />
                        </FormField>
                        <FormField label="Role ARN (optional)">
                            <input class="identity-input" type="text" value={awsRoleArn()} onInput={(e) => setAwsRoleArn(e.currentTarget.value)} placeholder="arn:aws:iam::123:role/dev-role" />
                        </FormField>
                        <FormField label="Region">
                            <input class="identity-input" type="text" value={awsRegion()} onInput={(e) => setAwsRegion(e.currentTarget.value)} placeholder="us-east-1" />
                        </FormField>
                    </Show>

                    <Show when={provider() === "anthropic"}>
                        <FormField label="Default model">
                            <input class="identity-input" type="text" value={anthropicModel()} onInput={(e) => setAnthropicModel(e.currentTarget.value)} placeholder="claude-sonnet-4-6" />
                        </FormField>
                    </Show>

                    <FormField label="Notes">
                        <input class="identity-input" type="text" value={description()} onInput={(e) => setDescription(e.currentTarget.value)} placeholder="Optional description" />
                    </FormField>
                    </Show>
                </div>

                <div class="identity-form-footer">
                    <button class="identity-btn identity-btn-secondary" onClick={() => model.cancelForm()}>
                        Cancel
                    </button>
                    {/* OAuth connects via its own panel; the keychain *entry* path
                        has its own Validate/Save buttons — hide the generic upsert
                        in both. A locked keychain account (editing, not replacing
                        the key) still uses this button for metadata-only saves;
                        buildAccount preserves the keychain pointer + context. */}
                    <Show when={kind() !== "oauth" && (secretBackend() !== "keychain" || (isEdit() && !keyReplacing()))}>
                        <button class="identity-btn identity-btn-primary" onClick={handleSubmit}>
                            {isEdit() ? "Save" : "Add Account"}
                        </button>
                    </Show>
                </div>
            </div>
        </div>
    );
}

function FormField({ label, children }: { label: string; children: JSX.Element }): JSX.Element {
    return (
        <div class="identity-form-field">
            <label class="identity-form-label">{label}</label>
            {children}
        </div>
    );
}
