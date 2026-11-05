// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BundleImportPreviewModalPanel, pinned as a whole before it was split into a state object and view parts: each
 * scenario drives the panel (the name and its suggested alternative, the instructions, context files, skills with
 * their collisions and rename conflicts, widgets, the dismissable warnings, Cancel and Next) and logs, in order,
 * the whole rendered panel as a tree, every RPC it made and every selection it handed on. The logs are snapshots
 * recorded on the unsplit component; the split must leave them byte for byte.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { BundleImportPreviewResponse } from "@/app/store/rpc-api";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        SkillCatalogListCommand: vi.fn(),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { RpcApi } from "@/app/store/rpc-api";
import { BundleImportPreviewModalPanel } from "./BundleImportPreviewModal";

const T: string[] = [];
const log = (line: string) => T.push(line);
const SNAP = (name: string) => `./__snapshots__/BundleImportPreviewModal.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const widget = (over: Record<string, unknown> = {}) => ({
    id: "acme.board",
    name: "Board",
    version: "1.2.0",
    description: null,
    author: "Acme",
    kind: "sandboxed" as const,
    permissions: ["storage", "net:https://api.acme.test"],
    hash: "h",
    signature: { state: "signed_new" as const, publisher: "acme", fingerprint: "K7Q2-MZ4D-PX3A-9TWE", pinned: null },
    installed_version: null,
    installed_state: null,
    same_as_installed: false,
    error: null,
    ...over,
});

function makePreview(over: Partial<BundleImportPreviewResponse> = {}): BundleImportPreviewResponse {
    return {
        project_instructions: [],
        name: "Release kit",
        description: "",
        instructions_preview: "Always run the checks.",
        instructions_truncated: true,
        instructions_total_chars: 9000,
        context_files: [
            { id: 1, display_path: "docs/RULES.md", size_bytes: 2048 },
            { id: 2, display_path: "docs/NOTES.md", size_bytes: 12 },
        ],
        skills: [
            {
                source_dir: "skills/plain",
                slug: "plain",
                description: "A plain skill",
                collision: "none",
                content: "Plain text.",
                content_truncated: false,
                content_total_chars: 11,
            },
            {
                source_dir: "skills/exists",
                slug: "exists",
                description: "",
                collision: "name_conflict",
                content: "Exists text.",
                content_truncated: false,
                content_total_chars: 12,
            },
            {
                source_dir: "skills/dup",
                slug: "dup",
                description: "Duplicate",
                collision: "duplicate_in_bundle",
                content: "Cut off…",
                content_truncated: true,
                content_total_chars: 5000,
            },
            {
                source_dir: "skills/empty",
                slug: "empty",
                description: "",
                collision: "none",
                content: "",
                content_truncated: false,
                content_total_chars: 0,
            },
        ],
        widgets: [
            widget(),
            widget({ id: "acme.tool", name: "Tool", error: "bundles can carry sandboxed widgets only" }),
            widget({ id: "acme.same", name: "Same", same_as_installed: true, installed_version: "1.2.0", author: null, permissions: [] }),
            widget({ id: "acme.up", name: "Up", installed_version: "1.0.0", signature: null }),
            widget({
                id: "acme.key",
                name: "Key",
                signature: { state: "key_changed" as const, publisher: "acme", fingerprint: "AAAA", pinned: "BBBB" },
            }),
        ],
        mcp_servers: [
            { source_path: "mcp/github.server.json", display: { name: "github", command: "gh-mcp" } },
            { source_path: "mcp/other/x.server.json", display: { name: null, command: null } },
        ],
        requirements: [
            { provider: "github", resolved: true },
            { provider: "slack", resolved: false },
        ],
        warnings: ["first warning", "second warning"],
        warnings_truncated: true,
        name_collision: true,
        content_digest: "abc123",
        ...over,
    } as BundleImportPreviewResponse;
}

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type|name|placeholder|for|style|href)$/;

/** The panel as an indented tree: tags, classes, the attributes that carry state, form values and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim();
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    if (el.tagName === "svg" || el.tagName === "STYLE") return out;
    // Generated ids count up across the whole file: they are not the panel's, and would tie a snapshot to the
    // order the scenarios run in.
    const attrs = [...el.attributes]
        .filter((a) => ATTRS.test(a.name) && !(el === document.body && a.name === "style"))
        .map((a) => `${a.name}=${JSON.stringify(a.value.replace(/(-cl)-\d+/g, "$1-N"))}`);
    if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) {
        attrs.push(`value=${JSON.stringify(el.value)}`);
        if (el instanceof HTMLInputElement && (el.type === "checkbox" || el.type === "radio")) {
            attrs.push(`checked=${el.checked}`);
        }
    }
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

const seen = { rpc: 0, next: 0, cancel: 0 };
const onNext = vi.fn();
const onCancel = vi.fn();

/** The RPCs and the hand-offs made since the last look. */
function calls() {
    const rpc = vi.mocked(RpcApi.SkillCatalogListCommand).mock.calls;
    for (; seen.rpc < rpc.length; seen.rpc++) log(`  rpc SkillCatalogListCommand ${JSON.stringify(rpc[seen.rpc].slice(1))}`);
    for (; seen.next < onNext.mock.calls.length; seen.next++) {
        log(`  onNext ${JSON.stringify(onNext.mock.calls[seen.next][0])}`);
    }
    for (; seen.cancel < onCancel.mock.calls.length; seen.cancel++) log("  onCancel");
}

function panel(label: string, root: HTMLElement) {
    log(`== ${label}`);
    dump(root).forEach((l) => log(l));
    calls();
}

const flush = async () => {
    for (let i = 0; i < 8; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
    for (let i = 0; i < 8; i++) await Promise.resolve();
};

async function mount(preview: BundleImportPreviewResponse = makePreview()) {
    const result = render(() => <BundleImportPreviewModalPanel preview={preview} onNext={onNext} onCancel={onCancel} />);
    await flush();
    return result;
}

const click = (el: Element) => fireEvent.click(el);
const typeInto = (el: Element, value: string) => fireEvent.input(el, { target: { value } });
/** The checkbox in the row that holds this text. */
const boxOf = (text: string) => screen.getByText(text).closest("label")!.querySelector("input")!;
const skillRow = (slug: string) => screen.getByText(slug).closest(".bundle-import-skill-row") as HTMLElement;

beforeEach(() => {
    T.length = 0;
    seen.rpc = seen.next = seen.cancel = 0;
    onNext.mockReset();
    onCancel.mockReset();
    vi.mocked(RpcApi.SkillCatalogListCommand).mockReset().mockResolvedValue([{ name: "taken" }, { name: "other" }] as never);
});
afterEach(() => cleanup());

describe("BundleImportPreviewModalPanel behavior", () => {
    it("shows everything the preview carries, then hands the default selection on", async () => {
        const { container } = await mount();
        panel("mounted", container);
        click(screen.getByText("Next"));
        calls();
        click(screen.getByText("Cancel"));
        calls();
        await snapshot("default");
    });

    it("renames the bundle, suggests the next free name, and falls back to the original when blank", async () => {
        const { container } = await mount();
        const name = container.querySelector(".bundle-import-input") as HTMLInputElement;
        click(screen.getByText(/Use "Release kit \(2\)" instead/));
        panel("suggestion taken", container);
        click(screen.getByText(/Use "Release kit \(3\)" instead/));
        panel("suggestion taken again", container);
        typeInto(name, "  Mine  ");
        click(screen.getByText("Next"));
        calls();
        typeInto(name, "   ");
        click(screen.getByText("Next"));
        calls();
        typeInto(name, "Plain name");
        panel("typed over, hint still offered", container);
        await snapshot("name");
    });

    it("has no name hint when nothing collides", async () => {
        const { container } = await mount(makePreview({ name_collision: false }));
        panel("no collision", container);
        await snapshot("name-free");
    });

    it("includes or leaves out the instructions, and folds their preview", async () => {
        const { container } = await mount();
        click(screen.getByText("Show preview"));
        panel("preview shown", container);
        click(screen.getByText("Hide preview"));
        click(screen.getByText("Show preview"));
        click(boxOf("Include instructions"));
        panel("instructions left out: no preview", container);
        click(screen.getByText("Next"));
        calls();
        click(boxOf("Include instructions"));
        panel("included again: preview still open", container);
        await snapshot("instructions");
    });

    it("shows no preview section for empty instructions, and no truncation note for a whole one", async () => {
        const { container } = await mount(makePreview({ instructions_preview: "" }));
        panel("empty instructions", container);
        cleanup();
        const second = await mount(makePreview({ instructions_truncated: false }));
        click(screen.getByText("Show preview"));
        panel("whole instructions", second.container);
        await snapshot("instructions-edges");
    });

    it("toggles the context files and hands the ticked ones on", async () => {
        const { container } = await mount();
        click(boxOf("docs/RULES.md"));
        panel("one left out", container);
        click(screen.getByText("Next"));
        calls();
        click(boxOf("docs/RULES.md"));
        click(boxOf("docs/NOTES.md"));
        click(screen.getByText("Next"));
        calls();
        await snapshot("context");
    });

    it("starts a truncated skill unticked, and flags renames that clash with the library or another pick", async () => {
        const { container } = await mount();
        panel("start", container);
        const exists = skillRow("exists");
        typeInto(exists.querySelector(".bundle-import-rename-input")!, "taken");
        panel("renamed to a library name", container);
        typeInto(exists.querySelector(".bundle-import-rename-input")!, "plain");
        panel("renamed to another picked skill's slug", container);
        typeInto(exists.querySelector(".bundle-import-rename-input")!, "  fresh  ");
        panel("renamed to a free name", container);
        click(boxOf("plain"));
        typeInto(exists.querySelector(".bundle-import-rename-input")!, "plain");
        panel("the other skill unticked: no clash", container);
        click(skillRow("dup").querySelector("input[type=checkbox]")!);
        typeInto(skillRow("dup").querySelector(".bundle-import-rename-input")!, "exists");
        panel("dup ticked, renamed to exists's slug", container);
        click(exists.querySelector("input[type=checkbox]")!);
        panel("exists unticked: its rename input is off", container);
        click(screen.getByText("Next"));
        calls();
        await snapshot("skills");
    });

    it("folds a skill's text away and back, per skill", async () => {
        const { container } = await mount();
        const buttons = screen.getAllByText("Hide skill text");
        click(buttons[0]);
        panel("first skill's text folded", container);
        click(screen.getByText("Show skill text"));
        panel("shown again", container);
        click(screen.getAllByText("Hide skill text")[2]);
        panel("third skill's text folded", container);
        await snapshot("skill-text");
    });

    it("does not need the library list: a failed fetch is advisory", async () => {
        vi.mocked(RpcApi.SkillCatalogListCommand).mockReset().mockRejectedValue(new Error("srv down"));
        const { container } = await mount();
        typeInto(skillRow("exists").querySelector(".bundle-import-rename-input")!, "taken");
        panel("rename with no library list", container);
        await snapshot("skills-no-catalog");
    });

    it("offers widgets that can be imported, and hands the ticked ones on", async () => {
        const { container } = await mount();
        panel("start", container);
        click(screen.getByText("Next"));
        calls();
        click(boxOf("Board"));
        click(boxOf("Up"));
        click(boxOf("Tool"));
        click(boxOf("Same"));
        panel("toggled, the disabled rows untouched", container);
        click(screen.getByText("Next"));
        calls();
        click(boxOf("Key"));
        click(screen.getByText("Next"));
        calls();
        await snapshot("widgets");
    });

    it("leaves out sections the preview has nothing for", async () => {
        const { container } = await mount(
            makePreview({
                context_files: [],
                skills: [],
                widgets: undefined as never,
                mcp_servers: [],
                requirements: [],
                warnings: [],
                warnings_truncated: false,
                instructions_preview: "",
            }),
        );
        panel("bare", container);
        click(screen.getByText("Next"));
        calls();
        await snapshot("bare");
    });

    it("dismisses the warnings, and shows a lone warning without the truncation line", async () => {
        const { container } = await mount();
        click(screen.getByLabelText("Dismiss warnings"));
        panel("dismissed", container);
        cleanup();
        const second = await mount(makePreview({ warnings: ["only one"], warnings_truncated: false }));
        panel("one warning", second.container);
        await snapshot("warnings");
    });
});
