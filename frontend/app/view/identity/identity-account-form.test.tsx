// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Characterisation tests for `AccountForm` (the Add/Edit account dialog):
 * what it shows for each provider, kind and secret backend, what the generic
 * Save builds and hands to the model, and how the keychain key flow talks to
 * `account.key.verify`. They pin today's behaviour so the component can be
 * split into named functions without changing it.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Account, AccountContext, AccountKind, AccountProvider, IdentityViewModel } from "./identity-model";

type VerifyArgs = Record<string, unknown>;
type VerifyResult = { valid: boolean; error?: string };

const h = vi.hoisted(() => ({
    client: { rpc: "tab" },
    verify: vi.fn<(client: unknown, args: Record<string, unknown>) => Promise<{ valid: boolean; error?: string }>>(),
    refresh: vi.fn<() => Promise<unknown[]>>(),
    panelProps: null as Record<string, unknown> | null,
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AccountKeyVerifyCommand: (client: unknown, args: Record<string, unknown>) => h.verify(client, args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: h.client }));
vi.mock("./identity-model", () => ({
    PROVIDER_LABELS: {
        github: "GitHub",
        openai: "OpenAI",
        aws: "AWS",
        anthropic: "Anthropic",
        google: "Google",
        slack: "Slack",
        custom: "Custom",
        agentmux: "AgentMux",
    },
    refreshAccountCache: () => h.refresh(),
}));
vi.mock("@/app/view/accounts/OAuthConnectPanel", () => ({
    OAuthConnectPanel: (props: Record<string, unknown> & { name: () => string; onConnected: () => void }) => {
        h.panelProps = props;
        return (
            <div data-testid="oauth-panel">
                <span data-testid="oauth-panel-name">{props.name()}</span>
                <button onClick={() => props.onConnected()}>panel-connected</button>
            </div>
        );
    },
}));

import { reconnectScopes } from "@/app/view/accounts/oauth-account-state";
import { AccountForm } from "./identity-account-form";

type Preset = { provider: AccountProvider; kind: AccountKind } | null;

function makeAccount(over: Partial<Account> = {}): Account {
    return {
        id: "acc-1",
        name: "work",
        provider: "github",
        kind: "pat",
        secret_ref: { backend: "keychain", service: "agentmux", account: "acc-1" },
        context: {},
        assigned_agents: [],
        status: "valid",
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
        ...over,
    };
}

function renderForm(opts: { editing?: Account | null; preset?: Preset } = {}) {
    const editing = opts.editing ?? null;
    const preset = opts.preset ?? null;
    const [formError, setFormErrorSignal] = createRoot(() => createSignal<string | null>(null));
    const spies = {
        setFormError: vi.fn((v: string | null) => setFormErrorSignal(v)),
        createAccount: vi.fn(),
        updateAccount: vi.fn(),
        cancelForm: vi.fn(),
    };
    const model = {
        editingAccountAtom: () => editing,
        addPresetAtom: () => preset,
        formErrorAtom: formError,
        ...spies,
    } as unknown as IdentityViewModel;
    const view = render(() => <AccountForm model={model} />);
    return { ...view, ...spies };
}

function fieldOf(label: string): HTMLElement {
    const lab = [...document.querySelectorAll("label.identity-form-label")].find((l) => l.textContent === label);
    if (!lab) throw new Error(`no field labelled ${label}`);
    return lab.parentElement as HTMLElement;
}
const hasField = (label: string) =>
    [...document.querySelectorAll("label.identity-form-label")].some((l) => l.textContent === label);
const inputOf = (label: string) => fieldOf(label).querySelector("input") as HTMLInputElement;
const selectOf = (label: string) => fieldOf(label).querySelector("select") as HTMLSelectElement;
const typeInto = (el: HTMLInputElement, value: string) => fireEvent.input(el, { target: { value } });
const choose = (el: HTMLSelectElement, value: string) => fireEvent.change(el, { target: { value } });
const button = (name: string | RegExp) => screen.getByRole("button", { name });
const maybeButton = (name: string | RegExp) => screen.queryByRole("button", { name });
const optionValues = (label: string) => [...selectOf(label).querySelectorAll("option")].map((o) => o.value);
const text = () => document.body.textContent ?? "";

/** Switch to a non-keychain backend so the generic footer button shows. */
function useBackend(value: string) {
    choose(selectOf("Secret backend"), value);
}

beforeEach(() => {
    h.verify.mockReset();
    h.verify.mockResolvedValue({ valid: true });
    h.refresh.mockReset();
    h.refresh.mockResolvedValue([]);
    h.panelProps = null;
});

afterEach(() => {
    cleanup();
});

describe("AccountForm: opening", () => {
    it("opens an empty Add form on GitHub / Personal Access Token / OS keychain", () => {
        renderForm();
        expect(text()).toContain("Add Account");
        expect(inputOf("Name *").value).toBe("");
        expect(selectOf("Provider").value).toBe("github");
        expect(selectOf("Kind").value).toBe("pat");
        expect(selectOf("Secret backend").value).toBe("keychain");
        expect(inputOf("Display name").value).toBe("");
        expect(inputOf("Notes").value).toBe("");
    });

    it("lists every provider in the Provider select", () => {
        renderForm();
        expect(optionValues("Provider")).toEqual([
            "github",
            "google",
            "aws",
            "openai",
            "anthropic",
            "slack",
            "custom",
            "agentmux",
        ]);
    });

    it("takes provider and kind from an Add preset", () => {
        renderForm({ preset: { provider: "openai", kind: "api_key" } });
        expect(selectOf("Provider").value).toBe("openai");
        expect(selectOf("Kind").value).toBe("api_key");
    });

    it("prefers the account being edited over a preset", () => {
        renderForm({
            editing: makeAccount({ provider: "aws", kind: "role", secret_ref: { backend: "env", env_var: "AWS_X" } }),
            preset: { provider: "openai", kind: "api_key" },
        });
        expect(selectOf("Provider").value).toBe("aws");
        expect(selectOf("Kind").value).toBe("role");
    });

    it("fills every field from the account being edited and titles the form Edit Account", () => {
        renderForm({
            editing: makeAccount({
                name: "work-gh",
                display_name: "Work GitHub",
                secret_ref: { backend: "env", env_var: "GH_TOKEN" },
                context: { github_username: "octo", github_scopes: ["repo", "workflow"], description: "for CI" },
            }),
        });
        expect(text()).toContain("Edit Account");
        expect(inputOf("Name *").value).toBe("work-gh");
        expect(inputOf("Display name").value).toBe("Work GitHub");
        expect(selectOf("Secret backend").value).toBe("env");
        expect(inputOf("Env var name").value).toBe("GH_TOKEN");
        expect(inputOf("GitHub username").value).toBe("octo");
        expect(inputOf("Scopes (comma-separated)").value).toBe("repo, workflow");
        expect(inputOf("Notes").value).toBe("for CI");
        expect(button("Save")).toBeTruthy();
    });

    it("gives the Name field the caret and selects its text", async () => {
        renderForm({ editing: makeAccount({ name: "work-gh", secret_ref: { backend: "env" } }) });
        const name = inputOf("Name *");
        await waitFor(() => expect(document.activeElement).toBe(name));
        expect(name.selectionStart).toBe(0);
        expect(name.selectionEnd).toBe("work-gh".length);
    });

    it("shows the model's form error above the fields", () => {
        const { setFormError } = renderForm();
        expect(document.querySelector(".identity-form-error")).toBeNull();
        // The form error comes from buildAccount, not the key flow.
        useBackend("env");
        fireEvent.click(button("Add Account"));
        expect(setFormError).toHaveBeenCalledWith("Name is required");
        expect(document.querySelector(".identity-form-body > .identity-form-error")?.textContent).toBe(
            "Name is required"
        );
    });
});

describe("AccountForm: closing", () => {
    it("closes from the Close button", () => {
        const { cancelForm } = renderForm();
        fireEvent.click(button("Close"));
        expect(cancelForm).toHaveBeenCalledTimes(1);
    });

    it("closes from Cancel", () => {
        const { cancelForm } = renderForm();
        fireEvent.click(button("Cancel"));
        expect(cancelForm).toHaveBeenCalledTimes(1);
    });

    it("closes on a click on the backdrop but not on a click inside the dialog", () => {
        const { cancelForm } = renderForm();
        fireEvent.click(document.querySelector(".identity-form") as HTMLElement);
        fireEvent.click(inputOf("Name *"));
        expect(cancelForm).not.toHaveBeenCalled();
        fireEvent.click(document.querySelector(".identity-form-overlay") as HTMLElement);
        expect(cancelForm).toHaveBeenCalledTimes(1);
    });
});

describe("AccountForm: provider and kind", () => {
    const defaults: [AccountProvider, AccountKind][] = [
        ["google", "oauth"],
        ["slack", "oauth"],
        ["github", "pat"],
        ["aws", "role"],
        ["openai", "api_key"],
        ["anthropic", "api_key"],
        ["custom", "api_key"],
        ["agentmux", "api_key"],
    ];

    it.each(defaults)("resets Kind to the default for %s when the provider is switched to it", (provider, kind) => {
        renderForm({ preset: { provider: provider === "openai" ? "github" : "openai", kind: "api_key" } });
        choose(selectOf("Provider"), provider);
        expect(selectOf("Provider").value).toBe(provider);
        expect(selectOf("Kind").value).toBe(kind);
    });

    const kindOptions: [AccountProvider, string[]][] = [
        ["github", ["oauth", "pat", "api_key"]],
        ["google", ["oauth"]],
        ["slack", ["oauth"]],
        ["aws", ["role", "env_ref"]],
        ["openai", ["api_key"]],
        ["anthropic", ["api_key"]],
        ["custom", ["api_key", "env_ref", "pat"]],
        ["agentmux", ["api_key"]],
    ];

    it.each(kindOptions)("offers the right kinds for %s", (provider, expected) => {
        renderForm({ preset: { provider, kind: expected[0] as AccountKind } });
        expect(optionValues("Kind")).toEqual(expected);
    });

    it("keeps a hand-picked kind until the provider changes", () => {
        renderForm();
        choose(selectOf("Kind"), "api_key");
        expect(selectOf("Kind").value).toBe("api_key");
        choose(selectOf("Provider"), "aws");
        expect(selectOf("Kind").value).toBe("role");
    });

    it("shows the provider-specific fields only for their provider", () => {
        renderForm();
        expect(hasField("GitHub username")).toBe(true);
        expect(hasField("Scopes (comma-separated)")).toBe(true);
        expect(hasField("AWS profile")).toBe(false);
        expect(hasField("Default model")).toBe(false);

        choose(selectOf("Provider"), "aws");
        expect(hasField("GitHub username")).toBe(false);
        expect(hasField("AWS profile")).toBe(true);
        expect(hasField("Role ARN (optional)")).toBe(true);
        expect(hasField("Region")).toBe(true);

        choose(selectOf("Provider"), "anthropic");
        expect(hasField("AWS profile")).toBe(false);
        expect(hasField("Default model")).toBe(true);

        choose(selectOf("Provider"), "openai");
        expect(hasField("Default model")).toBe(false);
        expect(hasField("Notes")).toBe(true);
    });
});

describe("AccountForm: secret backends", () => {
    it("shows the field for each backend and hides the others", () => {
        renderForm();
        expect(optionValues("Secret backend")).toEqual(["keychain", "env", "secrets_manager", "plaintext_dev"]);
        expect(hasField("Key / token")).toBe(true);

        useBackend("env");
        expect(hasField("Key / token")).toBe(false);
        expect(hasField("Env var name")).toBe(true);
        expect(hasField("Secret path")).toBe(false);

        useBackend("secrets_manager");
        expect(hasField("Env var name")).toBe(false);
        expect(hasField("Secret path")).toBe(true);
        expect(hasField("JSON path (optional)")).toBe(true);

        useBackend("plaintext_dev");
        expect(hasField("Secret path")).toBe(false);
        expect(hasField("Value")).toBe(true);
        expect(inputOf("Value").type).toBe("password");
        expect(document.querySelector(".identity-form-warning")?.textContent).toContain("dev/testing only");
    });

    it("shows the generic Add button for every backend except a new keychain account", () => {
        renderForm();
        expect(maybeButton("Add Account")).toBeNull();
        useBackend("env");
        expect(maybeButton("Add Account")).not.toBeNull();
        useBackend("keychain");
        expect(maybeButton("Add Account")).toBeNull();
    });
});

describe("AccountForm: generic Add and Save", () => {
    it("refuses a blank name and reports it through the model", () => {
        const { createAccount, updateAccount, setFormError } = renderForm();
        useBackend("env");
        typeInto(inputOf("Name *"), "   ");
        fireEvent.click(button("Add Account"));
        expect(setFormError).toHaveBeenCalledWith("Name is required");
        expect(createAccount).not.toHaveBeenCalled();
        expect(updateAccount).not.toHaveBeenCalled();
    });

    it("creates an env account with trimmed values and no stale provider keys", () => {
        const { createAccount } = renderForm();
        useBackend("env");
        typeInto(inputOf("Name *"), "  gh-ci  ");
        typeInto(inputOf("Display name"), "  CI token ");
        typeInto(inputOf("Env var name"), " GH_TOKEN ");
        typeInto(inputOf("GitHub username"), " octo ");
        typeInto(inputOf("Scopes (comma-separated)"), "repo, ,workflow ,");
        typeInto(inputOf("Notes"), " for CI ");
        fireEvent.click(button("Add Account"));

        expect(createAccount).toHaveBeenCalledTimes(1);
        expect(createAccount).toHaveBeenCalledWith({
            name: "gh-ci",
            provider: "github",
            kind: "pat",
            display_name: "CI token",
            secret_ref: { backend: "env", env_var: "GH_TOKEN" },
            context: { github_username: "octo", github_scopes: ["repo", "workflow"], description: "for CI" },
            assigned_agents: [],
        });
    });

    it("leaves display_name undefined when it is blank", () => {
        const { createAccount } = renderForm();
        useBackend("env");
        typeInto(inputOf("Name *"), "gh");
        typeInto(inputOf("Display name"), "   ");
        fireEvent.click(button("Add Account"));
        expect(createAccount.mock.calls[0][0].display_name).toBeUndefined();
    });

    it("keeps whitespace-only username and scopes as an empty string and an empty list", () => {
        const { createAccount } = renderForm();
        useBackend("env");
        typeInto(inputOf("Name *"), "gh");
        typeInto(inputOf("GitHub username"), "  ");
        typeInto(inputOf("Scopes (comma-separated)"), " ");
        fireEvent.click(button("Add Account"));
        expect(createAccount.mock.calls[0][0].context).toEqual({ github_username: "", github_scopes: [] });
    });

    it("omits context keys for blank fields", () => {
        const { createAccount } = renderForm();
        useBackend("env");
        typeInto(inputOf("Name *"), "gh");
        fireEvent.click(button("Add Account"));
        expect(createAccount.mock.calls[0][0].context).toEqual({});
    });

    it("builds a Secrets Manager reference with an optional JSON path", () => {
        const { createAccount } = renderForm();
        useBackend("secrets_manager");
        typeInto(inputOf("Name *"), "sm");
        typeInto(inputOf("Secret path"), " my-app/credentials ");
        fireEvent.click(button("Add Account"));
        expect(createAccount.mock.calls[0][0].secret_ref).toEqual({
            backend: "secrets_manager",
            sm_path: "my-app/credentials",
            sm_json_path: undefined,
        });

        typeInto(inputOf("JSON path (optional)"), " .gh-token ");
        fireEvent.click(button("Add Account"));
        expect(createAccount.mock.calls[1][0].secret_ref).toEqual({
            backend: "secrets_manager",
            sm_path: "my-app/credentials",
            sm_json_path: ".gh-token",
        });
    });

    it("passes a plaintext dev value through untrimmed", () => {
        const { createAccount } = renderForm();
        useBackend("plaintext_dev");
        typeInto(inputOf("Name *"), "dev");
        typeInto(inputOf("Value"), " s3cret ");
        fireEvent.click(button("Add Account"));
        expect(createAccount.mock.calls[0][0].secret_ref).toEqual({ backend: "plaintext_dev", value: " s3cret " });
    });

    it("builds AWS and Anthropic context from their own fields", () => {
        const { createAccount } = renderForm();
        useBackend("env");
        choose(selectOf("Provider"), "aws");
        typeInto(inputOf("Name *"), "aws-dev");
        typeInto(inputOf("AWS profile"), " dev ");
        typeInto(inputOf("Role ARN (optional)"), " arn:aws:iam::123:role/dev-role ");
        typeInto(inputOf("Region"), " us-east-1 ");
        fireEvent.click(button("Add Account"));
        expect(createAccount.mock.calls[0][0]).toMatchObject({
            provider: "aws",
            kind: "role",
            context: { aws_profile: "dev", aws_role_arn: "arn:aws:iam::123:role/dev-role", aws_region: "us-east-1" },
        });

        choose(selectOf("Provider"), "anthropic");
        typeInto(inputOf("Default model"), " claude-sonnet-4-6 ");
        fireEvent.click(button("Add Account"));
        expect(createAccount.mock.calls[1][0]).toMatchObject({
            provider: "anthropic",
            kind: "api_key",
            context: { anthropic_model: "claude-sonnet-4-6" },
        });
    });

    it("drops fields of the provider that was switched away from", () => {
        const { createAccount } = renderForm();
        useBackend("env");
        typeInto(inputOf("Name *"), "x");
        typeInto(inputOf("GitHub username"), "octo");
        typeInto(inputOf("Notes"), "keep me");
        choose(selectOf("Provider"), "openai");
        fireEvent.click(button("Add Account"));
        expect(createAccount.mock.calls[0][0].context).toEqual({ description: "keep me" });
    });

    it("updates the edited account with its id", () => {
        const { createAccount, updateAccount } = renderForm({
            editing: makeAccount({ id: "acc-9", name: "old", secret_ref: { backend: "env", env_var: "OLD" } }),
        });
        typeInto(inputOf("Name *"), "renamed");
        fireEvent.click(button("Save"));
        expect(createAccount).not.toHaveBeenCalled();
        expect(updateAccount).toHaveBeenCalledTimes(1);
        expect(updateAccount.mock.calls[0][0]).toBe("acc-9");
        expect(updateAccount.mock.calls[0][1]).toMatchObject({
            name: "renamed",
            secret_ref: { backend: "env", env_var: "OLD" },
        });
    });

    it("carries the non-rendered context keys across an edit and removes cleared fields", () => {
        const context: AccountContext = {
            masked_tail: "••ab12",
            openai_model_count: 3,
            anthropic_model_count: 2,
            slack_team: "T1",
            slack_user: "U1",
            github_username: "octo",
            description: "old note",
        };
        const { updateAccount } = renderForm({
            editing: makeAccount({
                provider: "github",
                secret_ref: { backend: "env", env_var: "GH" },
                context,
            }),
        });
        typeInto(inputOf("GitHub username"), "");
        typeInto(inputOf("Notes"), "");
        fireEvent.click(button("Save"));
        expect(updateAccount.mock.calls[0][1].context).toEqual({
            masked_tail: "••ab12",
            openai_model_count: 3,
            anthropic_model_count: 2,
            slack_team: "T1",
            slack_user: "U1",
        });
    });

    it("does not carry the previous provider's own keys after a provider switch", () => {
        const { updateAccount } = renderForm({
            editing: makeAccount({
                secret_ref: { backend: "env", env_var: "GH" },
                context: { github_username: "octo", github_scopes: ["repo"] },
            }),
        });
        choose(selectOf("Provider"), "openai");
        fireEvent.click(button("Save"));
        expect(updateAccount.mock.calls[0][1].context).toEqual({});
    });

    it("keeps the keychain pointer on a metadata-only Save of a locked keychain account", () => {
        const { updateAccount } = renderForm({
            editing: makeAccount({
                secret_ref: { backend: "keychain", service: "agentmux", account: "acc-1" },
                context: { masked_tail: "••ab12", github_username: "octo" },
            }),
        });
        typeInto(inputOf("Notes"), "renamed note");
        fireEvent.click(button("Save"));
        expect(updateAccount.mock.calls[0][1]).toMatchObject({
            secret_ref: { backend: "keychain", service: "agentmux", account: "acc-1" },
            context: { masked_tail: "••ab12", github_username: "octo", description: "renamed note" },
        });
    });
});

describe("AccountForm: locked keychain account", () => {
    const locked = () =>
        makeAccount({ context: { masked_tail: "••ab12" }, secret_ref: { backend: "keychain", service: "s", account: "a" } });

    it("shows the masked tail instead of a key field, with Save available", () => {
        renderForm({ editing: locked() });
        expect(document.querySelector(".identity-key-masked")?.textContent).toBe("••ab12");
        expect(text()).toContain("Stored in the OS keychain · not recoverable");
        expect(hasField("Key / token")).toBe(false);
        expect(maybeButton("Save")).not.toBeNull();
        expect(maybeButton("Save without validating")).toBeNull();
    });

    it("falls back to bullets when the account has no masked tail", () => {
        renderForm({ editing: makeAccount({ context: {} }) });
        expect(document.querySelector(".identity-key-masked")?.textContent).toBe("••••••••");
    });

    it("unlocks on Replace key: the key field appears and the generic Save goes away", () => {
        renderForm({ editing: locked() });
        fireEvent.click(button("Replace key"));
        expect(document.querySelector(".identity-key-locked")).toBeNull();
        expect(hasField("Key / token")).toBe(true);
        expect(maybeButton("Save")).toBeNull();
        expect(maybeButton("Save without validating")).not.toBeNull();
    });

    it("starts unlocked for a keychain account that is being created", () => {
        renderForm();
        expect(document.querySelector(".identity-key-locked")).toBeNull();
        expect(hasField("Key / token")).toBe(true);
    });

    it("starts unlocked when an account that is not in the keychain is switched to the keychain", () => {
        renderForm({ editing: makeAccount({ secret_ref: { backend: "env", env_var: "GH" } }) });
        useBackend("keychain");
        expect(hasField("Key / token")).toBe(true);
        expect(maybeButton("Save")).toBeNull();
    });
});

describe("AccountForm: key egress note", () => {
    it("names the validation endpoint for providers that have a validator", () => {
        renderForm();
        const note = document.querySelector(".identity-key-egress-note") as HTMLElement;
        expect(note.querySelector("code")?.textContent).toBe("api.github.com/user");
        expect(maybeButton("Validate & Save")).not.toBeNull();

        choose(selectOf("Provider"), "openai");
        expect(document.querySelector(".identity-key-egress-note code")?.textContent).toBe("api.openai.com/v1/models");
        choose(selectOf("Provider"), "anthropic");
        expect(document.querySelector(".identity-key-egress-note code")?.textContent).toBe(
            "api.anthropic.com/v1/models"
        );
    });

    it.each([
        ["aws", "AWS"],
        ["custom", "Custom"],
        ["agentmux", "AgentMux"],
    ] as const)("says there is no validator for %s and hides Validate & Save", (provider, label) => {
        renderForm();
        choose(selectOf("Provider"), provider);
        expect(document.querySelector(".identity-key-egress-note code")).toBeNull();
        expect(document.querySelector(".identity-key-egress-note")?.textContent).toContain(
            `No validator for ${label} yet`
        );
        expect(maybeButton("Validate & Save")).toBeNull();
        expect(maybeButton("Save without validating")).not.toBeNull();
    });

    it("masks the key field and turns off autocomplete", () => {
        renderForm();
        const key = inputOf("Key / token");
        expect(key.type).toBe("password");
        expect(key.getAttribute("autocomplete")).toBe("off");
        expect(key.getAttribute("spellcheck")).toBe("false");
    });
});

describe("AccountForm: key submit", () => {
    it("asks for a name first, then for a key, without calling the backend", () => {
        renderForm();
        fireEvent.click(button("Validate & Save"));
        expect(document.querySelector(".identity-key-actions")?.previousElementSibling?.textContent).toBe(
            "Name is required"
        );

        typeInto(inputOf("Name *"), "   ");
        fireEvent.click(button("Save without validating"));
        expect(document.querySelector(".identity-key-actions")?.previousElementSibling?.textContent).toBe(
            "Name is required"
        );

        typeInto(inputOf("Name *"), "gh");
        fireEvent.click(button("Validate & Save"));
        expect(document.querySelector(".identity-key-actions")?.previousElementSibling?.textContent).toBe(
            "Enter a key first"
        );
        typeInto(inputOf("Key / token"), "   ");
        fireEvent.click(button("Validate & Save"));
        expect(document.querySelector(".identity-key-actions")?.previousElementSibling?.textContent).toBe(
            "Enter a key first"
        );
        expect(h.verify).not.toHaveBeenCalled();
    });

    it("sends the key once with validate on, then refreshes the cache and closes", async () => {
        const { cancelForm } = renderForm();
        typeInto(inputOf("Name *"), "  gh  ");
        typeInto(inputOf("Display name"), " Work ");
        typeInto(inputOf("GitHub username"), " octo ");
        typeInto(inputOf("Key / token"), "ghp_secret");
        fireEvent.click(button("Validate & Save"));

        await waitFor(() => expect(cancelForm).toHaveBeenCalledTimes(1));
        expect(h.verify).toHaveBeenCalledTimes(1);
        const [client, args] = h.verify.mock.calls[0];
        expect(client).toBe(h.client);
        expect(args).toEqual({
            provider: "github",
            name: "gh",
            displayName: "Work",
            kind: "pat",
            apiKey: "ghp_secret",
            validate: true,
            accountId: undefined,
            context: { github_username: "octo" },
        });
        expect(h.refresh).toHaveBeenCalledTimes(1);
        expect(inputOf("Key / token").value).toBe("");
    });

    it("sends validate off from Save without validating, and a blank display name as undefined", async () => {
        const { cancelForm } = renderForm();
        typeInto(inputOf("Name *"), "gh");
        typeInto(inputOf("Key / token"), "k");
        fireEvent.click(button("Save without validating"));
        await waitFor(() => expect(cancelForm).toHaveBeenCalled());
        const args = h.verify.mock.calls[0][1] as VerifyArgs;
        expect(args.validate).toBe(false);
        expect(args.displayName).toBeUndefined();
    });

    it("keeps going when validation is off even if the backend says the key is not valid", async () => {
        h.verify.mockResolvedValue({ valid: false, error: "ignored" });
        const { cancelForm } = renderForm();
        typeInto(inputOf("Name *"), "gh");
        typeInto(inputOf("Key / token"), "k");
        fireEvent.click(button("Save without validating"));
        await waitFor(() => expect(cancelForm).toHaveBeenCalledTimes(1));
        expect(h.refresh).toHaveBeenCalledTimes(1);
    });

    it("shows the backend's reason when validation fails, clears the key and stays open", async () => {
        h.verify.mockResolvedValue({ valid: false, error: "401 bad credentials" });
        const { cancelForm } = renderForm();
        typeInto(inputOf("Name *"), "gh");
        typeInto(inputOf("Key / token"), "bad");
        fireEvent.click(button("Validate & Save"));

        await waitFor(() => expect(text()).toContain("401 bad credentials"));
        expect(inputOf("Key / token").value).toBe("");
        expect(h.refresh).not.toHaveBeenCalled();
        expect(cancelForm).not.toHaveBeenCalled();
        expect((button("Validate & Save") as HTMLButtonElement).disabled).toBe(false);
    });

    it("falls back to a generic message when a failed validation has no reason", async () => {
        h.verify.mockResolvedValue({ valid: false });
        renderForm();
        typeInto(inputOf("Name *"), "gh");
        typeInto(inputOf("Key / token"), "bad");
        fireEvent.click(button("Validate & Save"));
        await waitFor(() => expect(text()).toContain("Validation failed"));
    });

    it("shows the error message when the call throws, and re-enables the buttons", async () => {
        h.verify.mockRejectedValue(new Error("network down"));
        const { cancelForm } = renderForm();
        typeInto(inputOf("Name *"), "gh");
        typeInto(inputOf("Key / token"), "k");
        fireEvent.click(button("Validate & Save"));

        await waitFor(() => expect(text()).toContain("network down"));
        expect(cancelForm).not.toHaveBeenCalled();
        expect((button("Validate & Save") as HTMLButtonElement).disabled).toBe(false);
        expect((button("Save without validating") as HTMLButtonElement).disabled).toBe(false);
        // The key is left in the field for a retry when the call itself failed.
        expect(inputOf("Key / token").value).toBe("k");
    });

    it("shows a thrown non-Error as text", async () => {
        h.verify.mockRejectedValue("boom");
        renderForm();
        typeInto(inputOf("Name *"), "gh");
        typeInto(inputOf("Key / token"), "k");
        fireEvent.click(button("Save without validating"));
        await waitFor(() => expect(text()).toContain("boom"));
    });

    it("disables both buttons and says Validating… while the call is in flight", async () => {
        let resolve!: (r: VerifyResult) => void;
        h.verify.mockReturnValue(new Promise<VerifyResult>((r) => (resolve = r)));
        const { cancelForm } = renderForm();
        typeInto(inputOf("Name *"), "gh");
        typeInto(inputOf("Key / token"), "k");
        fireEvent.click(button("Validate & Save"));

        await waitFor(() => expect(maybeButton("Validating…")).not.toBeNull());
        expect((button("Validating…") as HTMLButtonElement).disabled).toBe(true);
        expect((button("Save without validating") as HTMLButtonElement).disabled).toBe(true);
        expect(cancelForm).not.toHaveBeenCalled();

        resolve({ valid: true });
        await waitFor(() => expect(cancelForm).toHaveBeenCalledTimes(1));
        expect(maybeButton("Validating…")).toBeNull();
    });

    it("clears an earlier error when the next submit starts", async () => {
        h.verify.mockResolvedValueOnce({ valid: false, error: "first failure" });
        renderForm();
        typeInto(inputOf("Name *"), "gh");
        typeInto(inputOf("Key / token"), "bad");
        fireEvent.click(button("Validate & Save"));
        await waitFor(() => expect(text()).toContain("first failure"));

        typeInto(inputOf("Key / token"), "good");
        fireEvent.click(button("Validate & Save"));
        await waitFor(() => expect(h.verify).toHaveBeenCalledTimes(2));
        await waitFor(() => expect(text()).not.toContain("first failure"));
    });

    it("sends the edited account's id and the preserved context when the key is replaced", async () => {
        const { cancelForm } = renderForm({
            editing: makeAccount({
                id: "acc-7",
                provider: "openai",
                kind: "api_key",
                secret_ref: { backend: "keychain", service: "s", account: "a" },
                context: { masked_tail: "••ab12", openai_model_count: 4, description: "prod" },
            }),
        });
        fireEvent.click(button("Replace key"));
        typeInto(inputOf("Key / token"), "sk-new");
        fireEvent.click(button("Validate & Save"));
        await waitFor(() => expect(cancelForm).toHaveBeenCalledTimes(1));
        expect(h.verify.mock.calls[0][1]).toMatchObject({
            provider: "openai",
            kind: "api_key",
            apiKey: "sk-new",
            validate: true,
            accountId: "acc-7",
            context: { description: "prod", masked_tail: "••ab12", openai_model_count: 4 },
        });
    });
});

describe("AccountForm: OAuth", () => {
    it("shows the OAuth panel instead of the secret and context fields", () => {
        const { cancelForm } = renderForm({ preset: { provider: "google", kind: "oauth" } });
        expect(screen.getByTestId("oauth-panel")).toBeTruthy();
        expect(hasField("Secret backend")).toBe(false);
        expect(hasField("Key / token")).toBe(false);
        expect(hasField("Notes")).toBe(false);
        expect(maybeButton("Add Account")).toBeNull();
        expect(maybeButton("Save")).toBeNull();
        expect(maybeButton("Cancel")).not.toBeNull();

        expect(h.panelProps?.provider).toBe("google");
        expect(h.panelProps?.accountId).toBeUndefined();
        expect(h.panelProps?.scopes).toBeUndefined();
        expect(h.panelProps?.clientIdHint).toBeUndefined();

        fireEvent.click(button("panel-connected"));
        expect(cancelForm).toHaveBeenCalledTimes(1);
    });

    it("hands the panel the live name", () => {
        renderForm({ preset: { provider: "google", kind: "oauth" } });
        expect(screen.getByTestId("oauth-panel-name").textContent).toBe("");
        typeInto(inputOf("Name *"), "my-google");
        expect(screen.getByTestId("oauth-panel-name").textContent).toBe("my-google");
    });

    it("offers the OAuth option for GitHub, Google and Slack only", () => {
        renderForm();
        expect(optionValues("Kind")).toContain("oauth");
        choose(selectOf("Provider"), "aws");
        expect(optionValues("Kind")).not.toContain("oauth");
        choose(selectOf("Provider"), "slack");
        expect(optionValues("Kind")).toEqual(["oauth"]);
    });

    it("switches between the panel and the secret fields when the kind changes", () => {
        renderForm();
        choose(selectOf("Kind"), "oauth");
        expect(screen.queryByTestId("oauth-panel")).not.toBeNull();
        expect(hasField("Secret backend")).toBe(false);
        choose(selectOf("Kind"), "pat");
        expect(screen.queryByTestId("oauth-panel")).toBeNull();
        expect(hasField("Secret backend")).toBe(true);
    });

    it("re-authorizes the edited account when it was connected with service OAuth", () => {
        const account = makeAccount({
            id: "acc-g",
            provider: "google",
            kind: "oauth",
            secret_ref: { backend: "keychain", service: "s", account: "a" },
            context: {
                granted_scopes: ["openid", "https://www.googleapis.com/auth/gmail.readonly"],
                oauth_client: { mode: "byo", client_id: "client-123" },
            } as AccountContext,
        });
        renderForm({ editing: account });
        expect(h.panelProps?.accountId).toBe("acc-g");
        expect(h.panelProps?.scopes).toEqual(reconnectScopes(account));
        expect(h.panelProps?.scopes).toEqual(["https://www.googleapis.com/auth/gmail.readonly"]);
        expect(h.panelProps?.clientIdHint).toBe("client-123");
    });

    it("does not re-authorize an account that was not connected with service OAuth", () => {
        renderForm({
            editing: makeAccount({
                provider: "google",
                kind: "oauth",
                secret_ref: { backend: "env", env_var: "G" },
            }),
        });
        expect(h.panelProps?.accountId).toBeUndefined();
        expect(h.panelProps?.scopes).toBeUndefined();
    });

    it("stops re-authorizing the edited account once the provider is switched to another one", () => {
        renderForm({
            editing: makeAccount({
                id: "acc-g",
                provider: "google",
                kind: "oauth",
                secret_ref: { backend: "keychain", service: "s", account: "a" },
            }),
        });
        expect(h.panelProps?.accountId).toBe("acc-g");
        choose(selectOf("Provider"), "slack");
        expect(h.panelProps?.provider).toBe("slack");
        expect(h.panelProps?.accountId).toBeUndefined();
    });
});
