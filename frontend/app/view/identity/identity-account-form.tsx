// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, Show, type Accessor, type JSX, type Setter } from "solid-js";
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
import { GITHUB_APP_KIND_LABEL, GITHUB_TOKEN_KIND_LABEL } from "@/app/view/accounts/github-account-copy";
import { supportsOAuth } from "@/app/view/accounts/oauth-catalog";
import { isBrokeredGithub, isServiceOAuthAccount, reconnectScopes } from "@/app/view/accounts/oauth-account-state";
import "./identity-view.scss";
import "@/app/view/accounts/oauth-connect.scss";
import { Button, CloseButton, TextInput } from "@/app/element/ui";
import { focusOnOpen } from "@/util/focusutil";

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

type AddPreset = ReturnType<IdentityViewModel["addPresetAtom"]>;

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

/** The form's field signals, one accessor and setter per field. */
interface FormFields {
    name: Accessor<string>;
    setName: Setter<string>;
    provider: Accessor<AccountProvider>;
    setProvider: Setter<AccountProvider>;
    kind: Accessor<AccountKind>;
    setKind: Setter<AccountKind>;
    displayName: Accessor<string>;
    setDisplayName: Setter<string>;
    secretBackend: Accessor<SecretRef["backend"]>;
    setSecretBackend: Setter<SecretRef["backend"]>;
    secretEnvVar: Accessor<string>;
    setSecretEnvVar: Setter<string>;
    secretSmPath: Accessor<string>;
    setSecretSmPath: Setter<string>;
    secretSmJsonPath: Accessor<string>;
    setSecretSmJsonPath: Setter<string>;
    secretValue: Accessor<string>;
    setSecretValue: Setter<string>;
    ghUsername: Accessor<string>;
    setGhUsername: Setter<string>;
    ghScopes: Accessor<string>;
    setGhScopes: Setter<string>;
    awsProfile: Accessor<string>;
    setAwsProfile: Setter<string>;
    awsRoleArn: Accessor<string>;
    setAwsRoleArn: Setter<string>;
    awsRegion: Accessor<string>;
    setAwsRegion: Setter<string>;
    anthropicModel: Accessor<string>;
    setAnthropicModel: Setter<string>;
    description: Accessor<string>;
    setDescription: Setter<string>;
}

/** The secure-key lifecycle's signals (keychain backend). */
interface KeyFlow {
    keychainKey: Accessor<string>;
    setKeychainKey: Setter<string>;
    keyBusy: Accessor<boolean>;
    setKeyBusy: Setter<boolean>;
    keyError: Accessor<string | null>;
    setKeyError: Setter<string | null>;
    keyReplacing: Accessor<boolean>;
    setKeyReplacing: Setter<boolean>;
}

/** What every part of the form reads: the model, the edited account, the fields and the key flow. */
interface FormCtx {
    model: IdentityViewModel;
    editing: Accessor<Account | null>;
    isEdit: () => boolean;
    f: FormFields;
    key: KeyFlow;
}

function createFormFields(editing: Accessor<Account | null>, preset: AddPreset): FormFields {
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
    return {
        name, setName, provider, setProvider, kind, setKind, displayName, setDisplayName,
        secretBackend, setSecretBackend, secretEnvVar, setSecretEnvVar, secretSmPath, setSecretSmPath,
        secretSmJsonPath, setSecretSmJsonPath, secretValue, setSecretValue,
        ghUsername, setGhUsername, ghScopes, setGhScopes, awsProfile, setAwsProfile,
        awsRoleArn, setAwsRoleArn, awsRegion, setAwsRegion, anthropicModel, setAnthropicModel,
        description, setDescription,
    };
}

// ── Account secure-key lifecycle (SPEC_TRUST_CENTER §5) ──
// Active when secretBackend === "keychain": the user pastes a key and
// clicks Validate (single user-initiated egress) or "Save without
// validating". The plaintext is sent once to the backend, which stores it
// in the OS keychain and returns only a masked tail + metadata — it is
// never read back into the UI. Existing keychain accounts render locked
// (masked) and require Replace to re-enter.
function createKeyFlow(editing: Accessor<Account | null>): KeyFlow {
    const editingIsKeychain = editing()?.secret_ref?.backend === "keychain";
    const [keychainKey, setKeychainKey] = createSignal("");
    const [keyBusy, setKeyBusy] = createSignal(false);
    const [keyError, setKeyError] = createSignal<string | null>(null);
    // For an existing keychain account, start locked (masked) until Replace.
    const [keyReplacing, setKeyReplacing] = createSignal(!editingIsKeychain);
    return { keychainKey, setKeychainKey, keyBusy, setKeyBusy, keyError, setKeyError, keyReplacing, setKeyReplacing };
}

// Build the non-secret context fresh from the form fields, then re-apply
// only the non-rendered keys (masked_tail + validation metadata) so a
// metadata-only edit doesn't wipe them.
function buildContext({ f, editing }: FormCtx): AccountContext {
    const context: AccountContext = {};
    if (f.provider() === "github") {
        if (f.ghUsername()) context.github_username = f.ghUsername().trim();
        if (f.ghScopes()) context.github_scopes = f.ghScopes().split(",").map((s) => s.trim()).filter(Boolean);
    }
    if (f.provider() === "aws") {
        if (f.awsProfile()) context.aws_profile = f.awsProfile().trim();
        if (f.awsRoleArn()) context.aws_role_arn = f.awsRoleArn().trim();
        if (f.awsRegion()) context.aws_region = f.awsRegion().trim();
    }
    if (f.provider() === "anthropic") {
        if (f.anthropicModel()) context.anthropic_model = f.anthropicModel().trim();
    }
    if (f.description()) context.description = f.description().trim();
    const prior = editing()?.context;
    if (prior) {
        for (const k of PRESERVED_CONTEXT_KEYS) {
            if (prior[k] !== undefined) (context[k] as unknown) = prior[k];
        }
    }
    return context;
}

function buildAccount(ctx: FormCtx): Omit<Account, "id" | "status" | "created_at" | "updated_at"> | null {
    const { model, editing, f } = ctx;
    const n = f.name().trim();
    if (!n) {
        model["setFormError"]("Name is required");
        return null;
    }

    const secretRef: SecretRef = { backend: f.secretBackend() };
    if (f.secretBackend() === "env") secretRef.env_var = f.secretEnvVar().trim();
    if (f.secretBackend() === "secrets_manager") {
        secretRef.sm_path = f.secretSmPath().trim();
        secretRef.sm_json_path = f.secretSmJsonPath().trim() || undefined;
    }
    if (f.secretBackend() === "plaintext_dev") secretRef.value = f.secretValue();
    if (f.secretBackend() === "keychain") {
        // Preserve the existing keychain pointer (service/account) so a
        // metadata-only Save on a locked account doesn't drop it. New
        // keychain accounts go through the key flow, not this path.
        secretRef.service = editing()?.secret_ref?.service;
        secretRef.account = editing()?.secret_ref?.account;
    }

    const context = buildContext(ctx);

    return {
        name: n,
        provider: f.provider(),
        kind: f.kind(),
        display_name: f.displayName().trim() || undefined,
        secret_ref: secretRef,
        context,
        // assigned_agents is deprecated and never read by the backend —
        // see identity-model.ts's Account.assigned_agents doc comment.
        // Real assignment happens via the agent-side reverse index.
        assigned_agents: [],
    };
}

function handleSubmit(ctx: FormCtx): void {
    const { model, editing, isEdit } = ctx;
    const data = buildAccount(ctx);
    if (!data) return;
    if (isEdit()) {
        model.updateAccount(editing()!.id, data);
    } else {
        model.createAccount(data);
    }
}

// Secure-key submit. `validate` true → the backend runs one live probe
// against the service (user-initiated egress) before storing; false →
// store with status "unknown" (the air-gapped escape hatch). On success
// the key lives only in the OS keychain; we refresh the cache and close.
async function submitKey(ctx: FormCtx, validate: boolean): Promise<void> {
    const { model, editing, isEdit, f, key } = ctx;
    const n = f.name().trim();
    if (!n) {
        key.setKeyError("Name is required");
        return;
    }
    if (!key.keychainKey().trim()) {
        key.setKeyError("Enter a key first");
        return;
    }
    key.setKeyBusy(true);
    key.setKeyError(null);
    try {
        const res = await RpcApi.AccountKeyVerifyCommand(TabRpcClient, {
            provider: f.provider(),
            name: n,
            displayName: f.displayName().trim() || undefined,
            kind: f.kind(),
            apiKey: key.keychainKey(),
            validate,
            accountId: isEdit() ? editing()!.id : undefined,
            // Carry the user-entered context so the backend merges (not
            // drops) github_username/scopes/notes on a key change.
            context: buildContext(ctx) as unknown as Record<string, unknown>,
        });
        // Drop the plaintext from the form immediately.
        key.setKeychainKey("");
        if (!res.valid && validate) {
            key.setKeyError(res.error ?? "Validation failed");
            return;
        }
        await refreshAccountCache();
        model.cancelForm();
    } catch (err) {
        key.setKeyError((err as Error)?.message ?? String(err));
    } finally {
        key.setKeyBusy(false);
    }
}

/** `inline`: the form fills the section it is in (Connectors' add section), where the pane's own
 *  overlay would be a second dialog over the pane. */
export function AccountForm({ model, inline }: { model: IdentityViewModel; inline?: boolean }): JSX.Element {
    const editing = () => model.editingAccountAtom();
    const isEdit = () => editing() !== null;

    // Preset from a brand tile (Accounts gallery → openAddFormFor). Only used
    // for a fresh Add (editing wins). Read once at setup; the form remounts on
    // each open so this is always current.
    const preset = model.addPresetAtom();

    // Form field signals
    const f = createFormFields(editing, preset);
    const key = createKeyFlow(editing);
    const ctx: FormCtx = { model, editing, isEdit, f, key };

    const form = (
        <div class={inline ? "identity-form identity-form--inline" : "identity-form"}>
            <div class="identity-form-header">
                <span>{isEdit() ? "Edit Account" : "Add Account"}</span>
                <CloseButton tooltip={false} onClick={() => model.cancelForm()} />
            </div>

            <div class="identity-form-body">
                <Show when={model.formErrorAtom()}>
                    <div class="identity-form-error">{model.formErrorAtom()}</div>
                </Show>

                <NameField f={f} />
                <ProviderField f={f} />
                <KindField f={f} />

                <FormField label="Display name">
                    <TextInput
                        class="identity-input"
                        type="text"
                        value={f.displayName()}
                        onInput={(e) => f.setDisplayName(e.currentTarget.value)}
                        placeholder={displayNamePlaceholder(f.provider())}
                    />
                </FormField>

                <OAuthSection ctx={ctx} />
                <SecretAndContextSections ctx={ctx} />
            </div>

            <FormFooter ctx={ctx} />
        </div>
    );
    if (inline) return form;
    return (
        <div class="identity-form-overlay" onClick={(e) => e.target === e.currentTarget && model.cancelForm()}>
            {form}
        </div>
    );
}

/** What the Name field suggests: the provider's own name, which follows the Provider select. */
export function namePlaceholder(provider: AccountProvider): string {
    return provider === "custom" ? "Name this account" : PROVIDER_LABELS[provider];
}

/** What the Display name field suggests: a GitHub login for GitHub, otherwise how the account is shown. */
export function displayNamePlaceholder(provider: AccountProvider): string {
    return provider === "github" ? "your GitHub login" : "optional";
}

function NameField({ f }: { f: FormFields }): JSX.Element {
    return (
        <FormField label="Name *">
            <TextInput
                class="identity-input"
                type="text"
                value={f.name()}
                onInput={(e) => f.setName(e.currentTarget.value)}
                placeholder={namePlaceholder(f.provider())}
                ref={(el) => focusOnOpen(el, { select: true })}
            />
        </FormField>
    );
}

function ProviderField({ f }: { f: FormFields }): JSX.Element {
    return (
        <FormField label="Provider">
            <select
                class="identity-select"
                value={f.provider()}
                onChange={(e) => {
                    const p = e.currentTarget.value as AccountProvider;
                    f.setProvider(p);
                    // Kinds are provider-specific; reset to a valid
                    // default for the new provider so the Kind select
                    // never shows a stale/invalid value.
                    f.setKind(defaultKindFor(p));
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
    );
}

function KindField({ f }: { f: FormFields }): JSX.Element {
    return (
        <FormField label="Kind">
            <select
                class="identity-select"
                value={f.provider() === "github" && f.kind() === "api_key" ? "pat" : f.kind()}
                onChange={(e) => f.setKind(e.currentTarget.value as AccountKind)}
            >
                {/* Browser-login path for OAuth-capable providers. */}
                <Show when={supportsOAuth(f.provider())}>
                    <option value="oauth">
                        {f.provider() === "github" ? GITHUB_APP_KIND_LABEL : "Connect with OAuth (browser login)"}
                    </option>
                </Show>
                {/* GitHub has no API key apart from a token: an account saved as one shows as the token. */}
                <Show when={f.provider() === "github"}>
                    <option value="pat">{GITHUB_TOKEN_KIND_LABEL}</option>
                </Show>
                <Show when={f.provider() === "aws"}>
                    <option value="role">IAM Role</option>
                    <option value="env_ref">Env Reference</option>
                </Show>
                <Show when={f.provider() === "openai"}>
                    <option value="api_key">API Key</option>
                </Show>
                <Show when={f.provider() === "anthropic"}>
                    <option value="api_key">API Key</option>
                </Show>
                <Show when={f.provider() === "custom"}>
                    <option value="api_key">API Key</option>
                    <option value="env_ref">Env Reference</option>
                    <option value="pat">Token</option>
                </Show>
                <Show when={f.provider() === "agentmux"}>
                    <option value="api_key">API Key</option>
                </Show>
            </select>
        </FormField>
    );
}

function OAuthSection({ ctx }: { ctx: FormCtx }): JSX.Element {
    const { model, editing, f } = ctx;
    // The account being edited, when it was connected with service OAuth
    // here: its OAuth path re-authorizes it.
    const reauthTarget = (): Account | null => {
        const e = editing();
        return e && e.provider === f.provider() && isServiceOAuthAccount(e) ? e : null;
    };
    return (
        // OAuth (browser login) replaces the secret/key sections —
        // on success the backend creates the keychain-backed account
        // itself, so this form's secret-ref + Save path is skipped.
        // Editing an account connected that way authorizes it
        // again rather than making a second one.
        <Show when={f.kind() === "oauth"}>
            <OAuthConnectPanel
                provider={f.provider()}
                name={f.name}
                accountId={reauthTarget()?.id}
                scopes={reauthTarget() ? reconnectScopes(reauthTarget()!) : undefined}
                clientIdHint={reauthTarget()?.context.oauth_client?.client_id}
                // "Use AgentMux's GitHub App" connects the App (a new account, or one the App made), whatever
                // client the person saved; an account made with their own client reconnects with it.
                useBuiltinClient={f.provider() === "github" && (!reauthTarget() || isBrokeredGithub(reauthTarget()!))}
                onConnected={() => model.cancelForm()}
            />
        </Show>
    );
}

function SecretAndContextSections({ ctx }: { ctx: FormCtx }): JSX.Element {
    const { f } = ctx;
    return (
        <Show when={f.kind() !== "oauth"}>
            {/* Secret storage */}
            <FormField label="Secret backend">
                <select class="identity-select" value={f.secretBackend()} onChange={(e) => f.setSecretBackend(e.currentTarget.value as SecretRef["backend"])}>
                    <option value="keychain">OS Keychain (validated) — recommended</option>
                    <option value="env">Environment variable</option>
                    <option value="secrets_manager">AWS Secrets Manager</option>
                    <option value="plaintext_dev">Plaintext (dev only ⚠)</option>
                </select>
            </FormField>

            {/* ── Secure key lifecycle (keychain backend) ── */}
            <Show when={f.secretBackend() === "keychain"}>
                <KeychainKeySection ctx={ctx} />
            </Show>

            <SecretBackendFields f={f} />
            <ProviderContextFields f={f} />

            <FormField label="Notes">
                <TextInput class="identity-input" type="text" value={f.description()} onInput={(e) => f.setDescription(e.currentTarget.value)} placeholder="Optional description" />
            </FormField>
        </Show>
    );
}

function KeychainKeySection({ ctx }: { ctx: FormCtx }): JSX.Element {
    const { editing, key } = ctx;
    return (
        <Show
            when={key.keyReplacing()}
            fallback={
                <div class="identity-key-locked">
                    <span class="identity-key-masked">
                        {editing()?.context.masked_tail ?? "••••••••"}
                    </span>
                    <span class="identity-key-locked-note">
                        Stored in the OS keychain · not recoverable
                    </span>
                    <Button
                        class="identity-btn identity-btn-secondary"
                        onClick={() => key.setKeyReplacing(true)}
                    >
                        Replace key
                    </Button>
                </div>
            }
        >
            <KeyEntry ctx={ctx} />
        </Show>
    );
}

function KeyEntry({ ctx }: { ctx: FormCtx }): JSX.Element {
    const { f, key } = ctx;
    const validationEndpoint = (): string | undefined => KEY_VALIDATION_ENDPOINT[f.provider()];
    return (
        <>
            <FormField label="Key / token">
                <TextInput
                    class="identity-input"
                    type="password"
                    autocomplete="off"
                    spellcheck={false}
                    value={key.keychainKey()}
                    onInput={(e) => key.setKeychainKey(e.currentTarget.value)}
                    placeholder="paste key — never stored in plaintext"
                />
            </FormField>
            {/* §5.1 egress transparency — placed at the point of action */}
            <Show
                when={validationEndpoint()}
                fallback={
                    <div class="identity-key-egress-note">
                        ⓘ No validator for {PROVIDER_LABELS[f.provider()]} yet — use
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
            <Show when={key.keyError()}>
                <div class="identity-form-error">{key.keyError()}</div>
            </Show>
            <div class="identity-key-actions">
                <Show when={validationEndpoint()}>
                    <Button
                        tone="accent"
                        class="identity-btn identity-btn-primary"
                        disabled={key.keyBusy()}
                        onClick={() => void submitKey(ctx, true)}
                    >
                        {key.keyBusy() ? "Validating…" : "Validate & Save"}
                    </Button>
                </Show>
                <Button
                    class="identity-btn identity-btn-secondary"
                    disabled={key.keyBusy()}
                    onClick={() => void submitKey(ctx, false)}
                    title="Store the key without contacting the service"
                >
                    Save without validating
                </Button>
            </div>
        </>
    );
}

function SecretBackendFields({ f }: { f: FormFields }): JSX.Element {
    return (
        <>
            <Show when={f.secretBackend() === "env"}>
                <FormField label="Env var name">
                    <TextInput
                        class="identity-input"
                        type="text"
                        value={f.secretEnvVar()}
                        onInput={(e) => f.setSecretEnvVar(e.currentTarget.value)}
                        placeholder="GH_TOKEN"
                    />
                </FormField>
            </Show>

            <Show when={f.secretBackend() === "secrets_manager"}>
                <FormField label="Secret path">
                    <TextInput
                        class="identity-input"
                        type="text"
                        value={f.secretSmPath()}
                        onInput={(e) => f.setSecretSmPath(e.currentTarget.value)}
                        placeholder="my-app/credentials"
                    />
                </FormField>
                <FormField label="JSON path (optional)">
                    <TextInput
                        class="identity-input"
                        type="text"
                        value={f.secretSmJsonPath()}
                        onInput={(e) => f.setSecretSmJsonPath(e.currentTarget.value)}
                        placeholder=".gh-token"
                    />
                </FormField>
            </Show>

            <Show when={f.secretBackend() === "plaintext_dev"}>
                <div class="identity-form-warning">⚠ Stored in localStorage — for dev/testing only</div>
                <FormField label="Value">
                    <TextInput
                        class="identity-input"
                        type="password"
                        value={f.secretValue()}
                        onInput={(e) => f.setSecretValue(e.currentTarget.value)}
                        placeholder="secret value"
                    />
                </FormField>
            </Show>
        </>
    );
}

/** Provider-specific context fields. */
function ProviderContextFields({ f }: { f: FormFields }): JSX.Element {
    return (
        <>
            <Show when={f.provider() === "github"}>
                <FormField label="GitHub username">
                    <TextInput class="identity-input" type="text" value={f.ghUsername()} onInput={(e) => f.setGhUsername(e.currentTarget.value)} placeholder="your GitHub login" />
                </FormField>
                <FormField label="Scopes (comma-separated)">
                    <TextInput class="identity-input" type="text" value={f.ghScopes()} onInput={(e) => f.setGhScopes(e.currentTarget.value)} placeholder="repo, workflow, read:org" />
                </FormField>
            </Show>

            <Show when={f.provider() === "aws"}>
                <FormField label="AWS profile">
                    <TextInput class="identity-input" type="text" value={f.awsProfile()} onInput={(e) => f.setAwsProfile(e.currentTarget.value)} placeholder="dev" />
                </FormField>
                <FormField label="Role ARN (optional)">
                    <TextInput class="identity-input" type="text" value={f.awsRoleArn()} onInput={(e) => f.setAwsRoleArn(e.currentTarget.value)} placeholder="arn:aws:iam::123:role/dev-role" />
                </FormField>
                <FormField label="Region">
                    <TextInput class="identity-input" type="text" value={f.awsRegion()} onInput={(e) => f.setAwsRegion(e.currentTarget.value)} placeholder="us-east-1" />
                </FormField>
            </Show>

            <Show when={f.provider() === "anthropic"}>
                <FormField label="Default model">
                    <TextInput class="identity-input" type="text" value={f.anthropicModel()} onInput={(e) => f.setAnthropicModel(e.currentTarget.value)} placeholder="claude-sonnet-4-6" />
                </FormField>
            </Show>
        </>
    );
}

function FormFooter({ ctx }: { ctx: FormCtx }): JSX.Element {
    const { model, isEdit, f, key } = ctx;
    return (
        <div class="identity-form-footer">
            <Button class="identity-btn identity-btn-secondary" onClick={() => model.cancelForm()}>
                Cancel
            </Button>
            {/* OAuth connects via its own panel; the keychain *entry* path
                has its own Validate/Save buttons — hide the generic upsert
                in both. A locked keychain account (editing, not replacing
                the key) still uses this button for metadata-only saves;
                buildAccount preserves the keychain pointer + context. */}
            <Show when={f.kind() !== "oauth" && (f.secretBackend() !== "keychain" || (isEdit() && !key.keyReplacing()))}>
                <Button tone="accent" class="identity-btn identity-btn-primary" onClick={() => handleSubmit(ctx)}>
                    {isEdit() ? "Save" : "Add Account"}
                </Button>
            </Show>
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
