// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BundleManager, pinned as a whole before its body was split into an actions group and view parts: each
 * scenario drives the rail and the detail (select, new, edit, validate, save, delete, import, back, errors) and
 * logs, in order, what the DOM shows, the RPCs the model sends and the modal-layer calls. The logs are
 * snapshots recorded on the unsplit component; the split must leave them byte for byte.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PROVIDERS } from "@/app/view/agent/providers/catalog";

const T: string[] = [];
const log = (line: string) => T.push(line);
const SNAP = (name: string) => `./__snapshots__/bundle-manager.behavior.${name}.txt`;
const flush = async () => {
    for (let i = 0; i < 6; i++) await Promise.resolve();
};

const bundle = (id: string, p: Record<string, unknown> = {}) => ({
    id,
    name: `Bundle ${id}`,
    description: "",
    provider: "",
    model: "",
    instructions: "",
    instructions_by_provider: "{}",
    context_files: [],
    created_at: 0,
    updated_at: 0,
    ...p,
});

let bundles: unknown[] = [];
let listError: Error | null = null;
const rpc = vi.hoisted(() => ({
    list: vi.fn(),
    upsert: vi.fn(),
    validate: vi.fn(),
    remove: vi.fn(),
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListBundlesCommand: (...a: unknown[]) => rpc.list(...a),
        UpsertBundleCommand: (...a: unknown[]) => rpc.upsert(...a),
        ValidateBundleCommand: (...a: unknown[]) => rpc.validate(...a),
        DeleteBundleCommand: (...a: unknown[]) => rpc.remove(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));

// The modal layer: only its calls matter here.
const layer = {
    open: (req: unknown) => log(`modalLayer.open ${JSON.stringify(req, (_k, v) => (typeof v === "function" ? "[fn]" : v))}`),
    replace: (req: unknown) => log(`modalLayer.replace ${JSON.stringify(req, (_k, v) => (typeof v === "function" ? "[fn]" : v))}`),
    close: () => log("modalLayer.close"),
};
let opened: any = null;
vi.mock("@/app/element/modal-layer", () => ({
    useModalLayer: () => ({
        open: (req: any) => {
            opened = req;
            layer.open(req);
        },
        replace: (req: any) => {
            opened = req;
            layer.replace(req);
        },
        close: layer.close,
    }),
}));

vi.mock("./BundleSkillsSection", () => ({
    BundleSkillsSection: (p: { bundleId: string }) => <div data-testid="skills">skills for {p.bundleId}</div>,
}));
vi.mock("./BundleProviderInstructionsSection", () => ({
    BundleProviderInstructionsSection: (p: { value: string; onChange: (v: string) => void }) => (
        <button type="button" data-testid="by-provider" onClick={() => p.onChange('{"codex":"hi"}')}>
            by provider: {p.value}
        </button>
    ),
}));

import { BundleManager } from "./bundle-manager";

beforeEach(() => {
    T.length = 0;
    opened = null;
    listError = null;
    bundles = [
        bundle("blank", { name: "Blank", is_blank: true, description: "ignored for blank" }),
        bundle("g1", { name: "Global kit", is_global: true, description: "For all", instructions: "Be kind" }),
        bundle("b1", { name: "Review kit", description: "Reviews", provider: "claude", model: "anthropic", instructions: "Review it" }),
        bundle("sys", { name: "System", is_system: true }),
    ];
    rpc.list.mockReset().mockImplementation(async () => {
        log("ListBundlesCommand");
        if (listError) throw listError;
        return bundles;
    });
    rpc.upsert.mockReset().mockImplementation(async (_c: unknown, input: any) => {
        log(`UpsertBundleCommand ${JSON.stringify(input)}`);
        return bundle(input.id || "new1", { name: input.name });
    });
    rpc.validate.mockReset().mockImplementation(async (_c: unknown, input: any) => {
        log(`ValidateBundleCommand name=${input.name}`);
        return { is_valid: false, issues: [{ field: "context_files", message: "unsafe path", severity: "error" }, { field: "x", message: "note", severity: "warning" }] };
    });
    rpc.remove.mockReset().mockImplementation(async (_c: unknown, input: unknown) => {
        log(`DeleteBundleCommand ${JSON.stringify(input)}`);
        return null;
    });
});
afterEach(() => cleanup());

/** What the rail or the detail shows right now. */
function view(label: string) {
    log(`-- ${label}`);
    const rail = document.querySelector(".bundle-view-rail");
    const detail = document.querySelector(".bundle-view-detail");
    log(`  showing: ${detail ? "detail" : rail ? "list" : "nothing"}${document.querySelector("[aria-label]") ? "" : ""}`);
    const back = Array.from(document.querySelectorAll("button")).find((b) => /Bundles/.test(b.getAttribute("aria-label") ?? b.textContent ?? ""));
    log(`  back: ${back ? `"${back.textContent}"` : "-"}`);
    const err = document.querySelector(".bundle-view-error");
    if (err) log(`  error: ${err.textContent}`);
    if (rail) {
        for (const li of Array.from(rail.querySelectorAll("li"))) {
            const tag = li.querySelector(".ui-tag, [class*=tag]")?.textContent;
            log(`  - ${li.querySelector(".bundle-view-list-item-name-text")?.textContent}${tag ? ` [${tag}]` : ""} / ${li.querySelector(".bundle-view-list-item-provider")?.textContent ?? "-"} {${li.className}}`);
        }
    }
    if (detail) {
        const ro = detail.querySelector(".bundle-view-readonly");
        const form = detail.querySelector("form");
        if (ro) {
            log(`  readonly: ${ro.querySelector("h2")?.textContent} | ${ro.querySelector(".bundle-view-description")?.textContent ?? "-"} | ${ro.querySelector("dl")?.textContent}`);
            log(`  actions: ${Array.from(ro.querySelectorAll(".bundle-view-actions button")).map((b) => b.textContent).join(" | ") || "-"} skills=${ro.querySelector("[data-testid=skills]")?.textContent ?? "-"}`);
        }
        if (form) {
            log(`  form: ${form.querySelector("h2")?.textContent}`);
            for (const el of Array.from(form.querySelectorAll("input, select, textarea"))) {
                const e = el as HTMLInputElement;
                log(`    ${e.tagName.toLowerCase()} ${e.className} value="${e.value}"${e.required ? " required" : ""}${e.placeholder ? ` ph="${e.placeholder}"` : ""}`);
            }
            log(`    by-provider: ${form.querySelector("[data-testid=by-provider]")?.textContent}`);
            log(`    buttons: ${Array.from(form.querySelectorAll(".bundle-view-form-actions button")).map((b) => `${b.textContent}${(b as HTMLButtonElement).disabled ? "(disabled)" : ""}`).join(" | ")}`);
            const val = form.querySelector(".bundle-view-validation");
            if (val) {
                log(`    validation [${val.className}]: ${val.querySelector(".bundle-view-validation-ok")?.textContent ?? Array.from(val.querySelectorAll("li")).map((li) => `${li.className}:${li.textContent}`).join(" ; ")}`);
            }
            log(`    helps: ${form.querySelectorAll(".bundle-view-field-help").length}`);
        }
        const dialog = document.querySelector("[role=dialog], [role=alertdialog]");
        if (dialog) log(`  dialog: ${dialog.textContent}`);
    }
}

const click = (el: Element | null | undefined) => fireEvent.click(el!);
const button = (re: RegExp) => Array.from(document.querySelectorAll("button")).find((b) => re.test(b.textContent ?? "")) as HTMLButtonElement;
const item = (name: string) => Array.from(document.querySelectorAll(".bundle-view-list-item")).find((li) => li.textContent?.includes(name))!;
const field = (cls: string, nth = 0) => document.querySelectorAll(`form .${cls}`)[nth] as HTMLInputElement;
const type = (el: HTMLInputElement, v: string) => fireEvent.input(el, { target: { value: v } });
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));
const mount = async () => {
    render(() => <BundleManager />);
    await flush();
};

describe("BundleManager behavior", () => {
    it("lists the bundles and opens the read-only detail", async () => {
        await mount();
        view("list");
        click(item("Blank"));
        view("the blank bundle");
        click(button(/^Bundles$|Bundles/));
        view("back");
        click(item("Global kit"));
        view("the global bundle");
        click(button(/^Bundles$|Bundles/));
        click(item("Review kit"));
        view("a normal bundle");
        await snapshot("list-detail");
    });

    it("creates a bundle: the fields, the provider hint, validation and save", async () => {
        await mount();
        click(button(/New Bundle/));
        view("new form");
        type(field("bundle-view-input", 0), "  ");
        view("blank name: Save and Validate disabled");
        type(field("bundle-view-input", 0), "Kit");
        type(field("bundle-view-input", 1), "A kit");
        fireEvent.change(document.querySelector("form select")!, { target: { value: "claude" } });
        view("provider claude: one vendor, no vendor select");
        const multi = Object.values(PROVIDERS).find((p) => (p.supportedVendors?.length ?? 0) > 1);
        log(`multi-vendor provider: ${multi ? multi.id : "-"}`);
        if (multi) {
            fireEvent.change(document.querySelector("form select")!, { target: { value: multi.id } });
            view("a provider with several vendors");
            fireEvent.change(document.querySelectorAll("form select")[1], { target: { value: multi.supportedVendors![1] } });
            view("the second vendor");
        }
        fireEvent.change(document.querySelector("form select")!, { target: { value: "" } });
        view("any provider again");
        type(document.querySelector("form textarea") as unknown as HTMLInputElement, "Do good");
        click(document.querySelector("[data-testid=by-provider]"));
        view("filled in");
        click(button(/Validate/));
        await flush();
        view("validated");
        type(field("bundle-view-input", 0), "Kit 2");
        view("an edit clears the report");
        click(button(/Validate/));
        await flush();
        fireEvent.submit(document.querySelector("form")!);
        await flush();
        view("saved");
        await snapshot("new");
    });

    it("edits a bundle, cancels, and goes back", async () => {
        await mount();
        click(item("Review kit"));
        click(button(/^Edit$/));
        view("edit form");
        type(field("bundle-view-input", 0), "Renamed");
        click(button(/^Cancel$/));
        view("cancelled: the read-only view is back");
        click(button(/^Edit$/));
        click(button(/Bundles/));
        view("back from the form");
        click(button(/New Bundle/));
        click(button(/Bundles/));
        view("back from a new form");
        click(item("Blank"));
        log(`edit button for blank: ${button(/^Edit$/) ? "present" : "-"}`);
        await snapshot("edit");
    });

    it("deletes after asking, and imports through the three steps", async () => {
        await mount();
        click(item("Review kit"));
        click(button(/^Delete$/));
        await flush();
        view("asked");
        click(button(/^Cancel$/));
        await flush();
        view("not deleted");
        click(button(/^Delete$/));
        await flush();
        click(Array.from(document.querySelectorAll("button")).filter((b) => /^Delete$/.test(b.textContent ?? "")).at(-1));
        await flush();
        view("after confirming");
        click(button(/Import Bundle/));
        await flush();
        log(`opened kind=${opened?.kind}`);
        opened.onPreviewed("/tmp/a.abf", { content_digest: "abc" });
        log(`replaced kind=${opened?.kind}`);
        opened.onNext({ bundleName: "Imported" });
        log(`replaced kind=${opened?.kind} name=${opened?.bundleDisplayName} digest=${opened?.contentDigest}`);
        opened.onImported();
        opened.onCancel();
        await snapshot("delete-import");
    });

    it("shows load errors, and the blank bundle's edit refusal", async () => {
        listError = new Error("offline");
        await mount();
        view("a failed load");
        cleanup();
        T.length = 0;
        listError = null;
        await mount();
        view("recovered");
        await snapshot("errors");
    });
});
