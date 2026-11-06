// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Settings › Widgets, pinned as a whole ahead of splitting it into a state object and view parts: each scenario mounts
 * the real section over the real package store (the RPCs answer from mocks) with stand-ins for the widget bar, the
 * catalog and the approval details, and logs the section as an indented tree and, in order, every call it makes (the
 * RPCs with their arguments, the host's dialog, folder reveal and approval). The logs are snapshots recorded on the
 * unsplit component; the split must leave them byte for byte. (widgets-section.test.tsx asserts what the person is
 * told; this pins the markup and the calls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { WidgetPackageInfo } from "@/app/store/rpc-api/widgets";

const h = vi.hoisted(() => ({
    log: [] as string[],
    loadErrors: {} as Record<string, string>,
    home: "C:/Users/me/.agentmux" as string | Error,
    hasForget: true,
    catalogIds: new Set<string>(),
    impl: {
        list: (async () => ({ packages: [] as unknown[], publishers: [] as unknown[] })) as (...a: unknown[]) => Promise<any>,
        install: (async () => ({ id: "acme.notes", packages: [] as unknown[] })) as (...a: unknown[]) => Promise<any>,
        uninstall: (async () => ({ packages: [] as unknown[] })) as (...a: unknown[]) => Promise<any>,
        setEnabled: (async () => ({ packages: [] as unknown[] })) as (...a: unknown[]) => Promise<any>,
        rescan: (async () => ({ packages: [] as unknown[] })) as (...a: unknown[]) => Promise<any>,
        decide: (async () => {}) as (...a: unknown[]) => Promise<void>,
        forget: (async () => {}) as (...a: unknown[]) => Promise<void>,
        pick: (async () => "C:/src/acme.notes/widget.json") as () => Promise<string | null>,
        reveal: (async () => {}) as (...a: unknown[]) => Promise<void>,
    },
}));

const call = <T,>(name: string, args: unknown, f: (...a: any[]) => Promise<T>): Promise<T> => {
    h.log.push(`${name} ${JSON.stringify(args)}`);
    return f(args);
};
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        WidgetsListCommand: (_c: unknown, d: unknown) => call("WidgetsList", d, h.impl.list),
        WidgetsInstallCommand: (_c: unknown, d: unknown) => call("WidgetsInstall", d, h.impl.install),
        WidgetsUninstallCommand: (_c: unknown, d: unknown) => call("WidgetsUninstall", d, h.impl.uninstall),
        WidgetsSetEnabledCommand: (_c: unknown, d: unknown) => call("WidgetsSetEnabled", d, h.impl.setEnabled),
        WidgetsRescanCommand: () => call("WidgetsRescan", null, h.impl.rescan),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
vi.mock("@/app/block/widget-loader", () => ({ widgetLoadErrors: () => h.loadErrors }));
vi.mock("@/app/view/agent/agent-launch-env", () => ({
    agentmuxHome: () => {
        if (h.home instanceof Error) throw h.home;
        return h.home;
    },
}));
vi.mock("@/util/app-api", () => ({
    getApi: () => ({
        approvals: h.hasForget
            ? {
                  decideWidget: (...a: unknown[]) => call("decideWidget", a, h.impl.decide),
                  forgetWidgetKey: (...a: unknown[]) => call("forgetWidgetKey", a, h.impl.forget),
              }
            : { decideWidget: (...a: unknown[]) => call("decideWidget", a, h.impl.decide) },
        showOpenFileDialog: () => call("showOpenFileDialog", null, h.impl.pick),
        revealInFileExplorer: (p: string) => call("revealInFileExplorer", p, h.impl.reveal),
    }),
}));
vi.mock("./widget-bar-settings", () => ({ WidgetBarSettings: () => <div data-stand="WidgetBarSettings" /> }));
vi.mock("./widget-catalog", () => ({
    CATALOG_LINE: "From the catalog.",
    inCatalog: (p: { id: string }) => h.catalogIds.has(p.id),
    WidgetCatalog: (props: { onInstalled: (id: string, packages: unknown[]) => void }) => (
        <div data-stand="WidgetCatalog">
            <span data-cb="onInstalled" onClick={() => props.onInstalled("catalog.thing", [{ ...MIN, id: "catalog.thing", name: "From catalog", state: "needs_approval" }])}>
                installed
            </span>
        </div>
    ),
}));
vi.mock("./widget-approval-details", () => ({
    installLabel: (p: { id: string }, label = "Install") => `${label} ${p.id}`,
    WidgetApprovalDetails: (props: { pkg: { id: string } }) => <div data-stand="WidgetApprovalDetails">{props.pkg.id}</div>,
}));

import { refreshWidgetPackages, setWidgetPackages } from "@/app/store/widget-packages-store";
import { WidgetsSection } from "./widgets-section";

const T = h.log;
const log = (line: string) => T.push(line);
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(`./__snapshots__/widgets-section.behavior.${name}.txt`);

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type|href|id)$/;
/** The section as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim().replace(/\s+/g, " ");
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    if (el.tagName === "svg" || el.tagName === "STYLE") return out;
    const attrs = [...el.attributes].filter((a) => ATTRS.test(a.name)).map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let host: HTMLElement;
const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n"));
};
const flush = async () => {
    for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
};
const gate = <V,>() => {
    let open!: (v: V) => void;
    let fail!: (e: unknown) => void;
    const promise = new Promise<V>((res, rej) => ((open = res), (fail = rej)));
    return { promise, open, fail };
};

const MIN = {
    id: "acme.notes",
    name: "Notes",
    version: "1.0.0",
    description: "A scratchpad",
    author: "Acme",
    homepage: null,
    icon: "note-sticky",
    default_hue: null,
    kind: "sandboxed",
    permissions: ["storage", "net:https://api.github.com", "agents:send"],
    granted: [],
    state: "needs_approval",
    error: null,
    hash: "abc123",
    panes: [],
    commands: [],
    status_items: [],
    signature: { state: "unsigned", publisher: "acme", fingerprint: null, pinned: null },
    files_url: null,
    implied: false,
    folder: "C:/Users/me/.agentmux/widgets/acme.notes",
};
const pkg = (over: Record<string, unknown> = {}): WidgetPackageInfo => ({ ...MIN, ...over }) as unknown as WidgetPackageInfo;

/** The store's list, and what srv answers when the section asks for it. */
async function given(packages: WidgetPackageInfo[], publishers: { publisher: string; fingerprint: string }[] = []): Promise<void> {
    h.impl.list = async () => ({ packages, publishers });
    await refreshWidgetPackages();
    T.length = 0;
}

function mount() {
    const view = render(() => <WidgetsSection />);
    host = view.container;
}
const button = (text: string, within: ParentNode = host) =>
    [...within.querySelectorAll("button")].find((b) => b.textContent?.trim() === text) as HTMLElement;
const press = async (text: string, within: ParentNode = host) => {
    fireEvent.click(button(text, within));
    await flush();
};
const rowOf = (id: string) => host.querySelector(`[data-widget-id="${id}"]`) as HTMLElement;

beforeEach(() => {
    T.length = 0;
    h.loadErrors = {};
    h.home = "C:/Users/me/.agentmux";
    h.hasForget = true;
    h.catalogIds = new Set();
    h.impl.install = async () => ({ id: "acme.notes", packages: [pkg()] });
    h.impl.uninstall = async () => ({ packages: [] });
    h.impl.setEnabled = async () => ({ packages: [pkg({ state: "approved" })] });
    h.impl.rescan = async () => ({ packages: [pkg()] });
    h.impl.decide = async () => {};
    h.impl.forget = async () => {};
    h.impl.pick = async () => "C:/src/acme.notes/widget.json";
    h.impl.reveal = async () => {};
});
afterEach(() => {
    cleanup();
});

describe("WidgetsSection, pinned", () => {
    // Runs first: the package store is a module singleton, and "Loading…" is only shown until its first list arrives.
    it("says it is loading until the first list arrives", async () => {
        h.impl.list = () => new Promise(() => {});
        mount();
        await flush();
        look("loading");
        setWidgetPackages([]);
        await flush();
        look("loaded, none installed");
        await snapshot("loading");
    });

    it("lists every kind of package with its state, signature, permissions and errors", async () => {
        h.loadErrors = { "acme.broken": "Could not start" };
        await given([
            pkg(),
            pkg({ id: "acme.on", name: "On one", state: "approved", signature: { state: "signed", publisher: "acme", fingerprint: "ab:cd", pinned: true } }),
            pkg({ id: "acme.off", name: "Off one", state: "disabled", description: "", signature: { state: "signed_new", publisher: "acme", fingerprint: "ef:01", pinned: false } }),
            pkg({ id: "acme.changed", name: "Changed one", state: "changed", signature: { state: "key_changed", publisher: "acme", fingerprint: "99:99", pinned: true } }),
            pkg({ id: "acme.unsigned-other", name: "Unsigned unlike", state: "approved", signature: { state: "key_changed", publisher: "acme", fingerprint: null, pinned: true } }),
            pkg({ id: "acme.broken", name: "Broken one", state: "invalid", error: null, permissions: [] }),
            pkg({ id: "acme.errored", name: "Errored one", state: "invalid", error: "Bad manifest" }),
            pkg({ id: "acme.trusted", name: "Trusted one", kind: "trusted", state: "approved", implied: true }),
            pkg({ id: "acme.noperm", name: "No permissions", state: "approved", permissions: [] }),
        ]);
        mount();
        await flush();
        look("every kind");
        await snapshot("list");
    });

    it("shows the section without a widgets folder when the home can't be read", async () => {
        h.home = new Error("no home");
        await given([]);
        mount();
        await flush();
        look("no home");
        h.home = "C:/Users/me/.agentmux";
        await snapshot("no-home");
    });

    it("asks for approval, and approves through the host", async () => {
        await given([pkg(), pkg({ id: "acme.changed", name: "Changed one", state: "changed" }), pkg({ id: "acme.on", name: "On one", state: "approved" })]);
        h.catalogIds = new Set(["acme.notes"]);
        mount();
        await flush();
        await press("Approve");
        look("asking about notes");
        await press("Install acme.notes", rowOf("acme.notes"));
        look("approved");
        await press("Review and approve");
        look("asking about the changed one");
        await press("Approve acme.changed", rowOf("acme.changed"));
        await snapshot("approve");
    });

    it("cancels an approval, and shows why a decision failed, and holds the buttons while it is out", async () => {
        const g = gate<void>();
        await given([pkg({ signature: { state: "key_changed", publisher: "acme", fingerprint: "99:99", pinned: true } })]);
        mount();
        await flush();
        await press("Approve");
        h.impl.decide = () => g.promise;
        fireEvent.click(button("Install acme.notes", rowOf("acme.notes")));
        await flush();
        look("decision in flight");
        g.fail(new Error("srv refused"));
        await flush();
        look("decision failed");
        h.impl.decide = async () => {};
        await press("Cancel", rowOf("acme.notes"));
        look("cancelled");
        h.impl.decide = async () => {
            throw "plain refusal";
        };
        await press("Approve");
        await press("Cancel", rowOf("acme.notes"));
        look("cancel that failed");
        await snapshot("approval-failures");
    });

    it("turns a widget off and on, and shows what an RPC failure says", async () => {
        await given([pkg({ state: "approved" }), pkg({ id: "acme.off", name: "Off one", state: "disabled" })]);
        mount();
        await flush();
        const g = gate<{ packages: WidgetPackageInfo[] }>();
        h.impl.setEnabled = () => g.promise;
        fireEvent.click(button("Turn off", rowOf("acme.notes")));
        await flush();
        look("turning off");
        g.open({ packages: [pkg({ state: "disabled" }), pkg({ id: "acme.off", name: "Off one", state: "disabled" })] });
        await flush();
        look("turned off");
        h.impl.setEnabled = async () => ({ packages: [pkg({ state: "approved" }), pkg({ id: "acme.off", name: "Off one", state: "approved" })] });
        await press("Turn on", rowOf("acme.off"));
        look("turned on");
        h.impl.setEnabled = async () => {
            throw new Error("nope");
        };
        await press("Turn off", rowOf("acme.off"));
        look("failed");
        await snapshot("enable");
    });

    it("asks before uninstalling, takes it back, and removes the widget", async () => {
        await given([pkg({ state: "approved" }), pkg({ id: "acme.implied", name: "Built in", state: "approved", implied: true })]);
        mount();
        await flush();
        await press("Uninstall");
        look("confirm");
        await press("Keep");
        look("kept");
        await press("Uninstall");
        await press("Delete");
        look("deleted");
        await snapshot("uninstall");
    });

    it("installs a package from a file, and offers to replace one that is already installed", async () => {
        await given([]);
        mount();
        await flush();
        h.impl.pick = async () => null;
        await press("Install…");
        look("dialog cancelled");
        h.impl.pick = async () => "C:/src/acme.notes/widget.json";
        const g = gate<{ id: string; packages: WidgetPackageInfo[] }>();
        h.impl.install = () => g.promise;
        fireEvent.click(button("Install…"));
        await flush();
        look("install in flight");
        g.open({ id: "acme.notes", packages: [pkg()] });
        await flush();
        look("installed, asking");
        h.impl.install = async () => {
            throw new Error("widget acme.notes is already installed");
        };
        await press("Install…");
        look("already installed");
        h.impl.install = async () => ({ id: "acme.notes", packages: [pkg({ version: "2.0.0" })] });
        await press("Replace");
        look("replaced");
        h.impl.install = async () => {
            throw new Error("widget acme.notes is already installed");
        };
        await press("Install…");
        await press("Cancel", host.querySelector(".widget-approval:not([role])")!);
        look("replace dropped");
        h.impl.install = async () => {
            throw new Error("not a widget package");
        };
        await press("Install…");
        look("install failed");
        h.impl.install = async () => {
            throw new Error("widget acme.notes is already installed");
        };
        await press("Install…");
        h.impl.install = async () => {
            throw new Error("disk full");
        };
        await press("Replace");
        look("replace failed");
        await snapshot("install");
    });

    it("rescans, and reveals folders, and says when a folder can't be opened", async () => {
        await given([pkg({ state: "approved" })]);
        mount();
        await flush();
        await press("Rescan");
        look("rescanned");
        await press("Open widgets folder");
        await press("Show folder");
        h.impl.reveal = async () => {
            throw new Error("no explorer");
        };
        await press("Show folder");
        look("reveal failed");
        h.impl.reveal = async () => {};
        await press("Open widgets folder");
        look("reveal worked again");
        h.impl.rescan = async () => {
            throw new Error("srv busy");
        };
        await press("Rescan");
        look("rescan failed");
        await snapshot("rescan-reveal");
    });

    it("adds a widget the catalog installed, and asks about it", async () => {
        await given([]);
        mount();
        await flush();
        fireEvent.click(host.querySelector('[data-stand="WidgetCatalog"] [data-cb]')!);
        await flush();
        look("from the catalog");
        await snapshot("catalog");
    });

    it("lists the publishers' keys and forgets one, when the host can", async () => {
        await given([pkg({ state: "approved" })], [{ publisher: "acme", fingerprint: "ab:cd" }, { publisher: "zeta", fingerprint: "12:34" }]);
        mount();
        await flush();
        look("publishers");
        await press("Forget key", host.querySelector('[data-publisher="acme"]')!);
        look("forgot one");
        h.impl.forget = async () => {
            throw new Error("host said no");
        };
        await press("Forget key", host.querySelector('[data-publisher="zeta"]')!);
        look("forget failed");
        h.hasForget = false;
        cleanup();
        mount();
        await flush();
        look("a host that can't forget keys");
        await snapshot("publishers");
    });
});
