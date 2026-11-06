// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Maintenance section, pinned as a whole ahead of splitting it into a state object and view parts: each
 * scenario feeds it an update state (the host updater's own, srv's release with every step of a download and
 * a restart, a Microsoft Store install, an install older than the upgrade floor) and logs the section as a
 * tree and, in order, every outward call its buttons make. The logs are snapshots recorded on the unsplit
 * component; the split must leave them byte for byte. (UpdateStatus.test.tsx asserts what the person is
 * told; this pins the markup and what each button does.)
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    updaterStatus: "up-to-date" as string,
    updaterVersion: null as string | null,
    status: null as any,
    check: null as any,
    storeOffer: { state: "available", message: "" } as any,
    storeResult: undefined as any,
    hostError: null as string | null,
    revealError: null as string | null,
    log: [] as string[],
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AppUpdateStatusCommand: async () => {
            h.log.push("rpc AppUpdateStatus");
            return h.status;
        },
        AppUpdateCheckCommand: async () => {
            h.log.push("rpc AppUpdateCheck");
            if (h.check instanceof Error) throw h.check;
            return h.check ?? h.status;
        },
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/util/app-api", () => ({
    getApi: () => ({
        approvals: {
            appUpdate: async (action: string, now?: boolean) => {
                h.log.push(`host appUpdate ${action}${now === undefined ? "" : ` now=${now}`}`);
                if (h.hostError) throw new Error(h.hostError);
            },
            storeUpdateCheck: async () => {
                h.log.push("host storeUpdateCheck");
                return h.storeOffer;
            },
            storeUpdateCameBack: async () => ({ state: "none", from: "", target: "" }),
        },
    }),
}));
vi.mock("@/store/global", () => ({
    atoms: { updaterStatusAtom: () => h.updaterStatus, updaterVersionAtom: () => h.updaterVersion },
    getApi: () => ({
        installAppUpdate: () => void h.log.push("host installAppUpdate"),
        revealInFileExplorer: async (path: string) => {
            h.log.push(`host revealInFileExplorer ${path}`);
            if (h.revealError) throw new Error(h.revealError);
        },
        approvals: {
            storeUpdate: (version: string) => {
                h.log.push(`host storeUpdate ${version}`);
                return h.storeResult instanceof Promise ? h.storeResult : h.storeResult === "reject" ? Promise.reject(new Error("no")) : Promise.resolve(h.storeResult);
            },
        },
        listen: () => Promise.resolve(() => {}),
    }),
    openLink: (url: string) => void h.log.push(`openLink ${url}`),
}));
vi.mock("@/app/store/app-update", async (orig) => ({
    ...(await orig<typeof import("@/app/store/app-update")>()),
    checkedAt: () => "<time>",
}));

import { acceptAppUpdate, forgetStoreOffer, setStoreStayed } from "@/app/store/app-update";
import { MaintenanceSection } from "./MaintenanceSection";

const T = h.log;
const log = (line: string) => T.push(line);
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(`./__snapshots__/MaintenanceSection.behavior.${name}.txt`);

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type|href)$/;
/** The section as an indented tree: tags, classes, the attributes that carry state, and text. */
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
    for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 5));
};
async function mount() {
    const view = render(() => <MaintenanceSection />);
    host = view.container;
    await flush();
    return view;
}
const click = async (name: string | RegExp) => {
    fireEvent.click(screen.getByRole("button", { name }));
    await flush();
};
/** srv's status, and the same on the next ask. */
const feed = (s: Record<string, unknown>) => {
    h.status = s;
    acceptAppUpdate(s);
};

const AVAILABLE = {
    state: "available",
    current: "0.59.26",
    latest: "0.59.27",
    url: "https://github.com/agentmuxai/agentmux/releases/tag/v0.59.27",
    notes: "",
    checked_ms: 1,
    install: "windows-setup",
    can_download: false,
};
const PORTABLE = { ...AVAILABLE, install: "windows-portable", can_download: true };
const downloading = (state: string, extra: Record<string, unknown> = {}, base: Record<string, unknown> = PORTABLE) => ({
    ...base,
    download: { version: "0.59.27", state, received: 50, total: 200, waiting_for: 0, stopping: [], ...extra },
});
const STORE = { ...AVAILABLE, install: "windows-store" };

beforeEach(() => {
    h.log.length = 0;
    h.updaterStatus = "up-to-date";
    h.updaterVersion = null;
    h.check = null;
    h.storeOffer = { state: "available", message: "" };
    h.storeResult = undefined;
    h.hostError = null;
    h.revealError = null;
    setStoreStayed(null);
    forgetStoreOffer();
    feed({ state: "unknown", current: "0.59.26", checked_ms: 0, install: "unknown", can_download: false });
});
afterEach(cleanup);

describe("the host updater's own states", () => {
    it("shows each state the host reports, with nothing from srv", async () => {
        for (const [status, version] of [
            ["up-to-date", null],
            ["checking", null],
            ["available", "1.2.3"],
            ["available", null],
            ["downloading", "1.2.3"],
            ["ready", "1.2.3"],
            ["ready", null],
            ["error", null],
        ] as const) {
            h.updaterStatus = status;
            h.updaterVersion = version;
            const view = await mount();
            look(`host updater ${status} ${version ?? "no version"}`);
            view.unmount();
        }
        await snapshot("host-states");
    });

    it("the host's Download asks the host to install, and Restart does the same when it is ready", async () => {
        h.updaterStatus = "available";
        h.updaterVersion = "1.2.3";
        await mount();
        await click("What's new");
        await click("Download");
        cleanup();
        h.updaterStatus = "ready";
        await mount();
        await click("Restart");
        await snapshot("host-buttons");
    });
});

describe("a release srv found", () => {
    it("offers a download that opens the release page when AgentMux cannot download it", async () => {
        feed(AVAILABLE);
        await mount();
        look("available, setup install");
        await click("What's new");
        await click("Download");
        await snapshot("release-setup");
    });

    it("starts srv's download for a portable, and shows a refusal", async () => {
        feed(PORTABLE);
        await mount();
        look("available, portable");
        await click("Download");
        h.hostError = "disk is full";
        await click("Download");
        look("after a refusal");
        await snapshot("release-portable-download");
    });

    it("follows a download: downloading, unpacking, and a status with no download yet", async () => {
        for (const [label, s] of [
            ["downloading", downloading("downloading")],
            ["downloading, size unknown", downloading("downloading", { total: 0 })],
            ["unpacking", downloading("unpacking")],
        ] as const) {
            feed(s);
            const view = await mount();
            look(label);
            view.unmount();
        }
        await snapshot("release-downloading");
    });

    it("a staged release: show the folder or the installer, upgrade and restart, and a failed reveal", async () => {
        feed(downloading("ready", { folder: "C:/AgentMux/next", installer: "C:/Downloads/setup.exe" }));
        const view = await mount();
        look("ready: folder and installer");
        await click("Show folder");
        await click("Show installer");
        h.revealError = "no such folder";
        await click("Show folder");
        look("after a failed reveal");
        await click(/Upgrade/);
        view.unmount();
        feed(downloading("ready", { installer: "/home/u/AgentMux.AppImage" }, { ...PORTABLE, install: "linux-appimage" }));
        await mount();
        look("ready: an AppImage");
        feed(downloading("ready", { folder: "C:/AgentMux/next", error: "could not verify one file" }));
        await flush();
        look("ready with an error");
        await snapshot("release-ready");
    });

    it("waits for agents, with what the restart stops, and Restart now and Cancel", async () => {
        feed(downloading("waiting", { waiting_for: 2, stopping: ["a dev server (pane 3)", "a watcher"] }));
        const view = await mount();
        look("waiting for 2 agents");
        await click("Restart now");
        h.hostError = "the launcher is busy";
        await click("Cancel");
        look("cancel refused");
        view.unmount();
        for (const [label, n] of [["waiting for 1", 1], ["waiting for none", 0]] as const) {
            feed(downloading("waiting", { waiting_for: n }));
            const again = await mount();
            look(label);
            again.unmount();
        }
        feed(downloading("restarting", { stopping: ["a watcher"] }));
        await mount();
        look("restarting");
        await snapshot("release-waiting-restarting");
    });

    it("a failed download says why, and offers Try again and the release page", async () => {
        feed(downloading("failed", { error: "checksum mismatch" }));
        await mount();
        look("failed");
        await click("Try again");
        await click("Release page");
        feed(downloading("failed"));
        await flush();
        look("failed, no reason");
        await snapshot("release-failed");
    });

    it("an install older than the upgrade floor sends the person to the version to install first", async () => {
        feed({ ...AVAILABLE, install_first: "0.58.0" });
        await mount();
        look("below the floor");
        await click("Get v0.58.0");
        await snapshot("release-install-first");
    });
});

describe("a Microsoft Store install", () => {
    it("asks the Store, and says what it answered", async () => {
        for (const [label, offer] of [
            ["available", { state: "available", message: "" }],
            ["none", { state: "none", message: "" }],
            ["no answer", { state: "no_answer", message: "" }],
            ["unknown", { state: "unknown", message: "offline" }],
        ] as const) {
            h.storeOffer = offer;
            forgetStoreOffer();
            feed(STORE);
            const view = await mount();
            look(`store ${label}`);
            view.unmount();
        }
        await snapshot("store-answers");
    });

    it("Update in the Store asks the Store to install: a fallback opens its page, an answer is shown, none is remembered", async () => {
        feed(STORE);
        await mount();
        h.storeResult = { outcome: "fallback", message: "Couldn't start." };
        await click("Update in the Store");
        look("fallback");
        h.storeResult = { outcome: "started", message: "The Store is installing it." };
        await click("Update in the Store");
        look("answered");
        h.storeResult = { outcome: "none", message: "Nothing to install." };
        await click("Update in the Store");
        look("none: the Store has nothing yet");
        await snapshot("store-update");
    });

    it("shows that it is asking while the Store works, and opens its page when the call fails", async () => {
        feed(STORE);
        await mount();
        let finish!: (v: unknown) => void;
        h.storeResult = new Promise((r) => (finish = r));
        fireEvent.click(screen.getByRole("button", { name: "Update in the Store" }));
        await flush();
        look("busy");
        fireEvent.click(screen.getByRole("button", { name: "Update in the Store" }));
        finish({ outcome: "started", message: "Installing." });
        await flush();
        look("done");
        h.storeResult = "reject";
        await click("Update in the Store");
        look("call failed");
        await snapshot("store-busy");
    });

    it("a Store install below the upgrade floor is not offered the update, and is told how to move off", async () => {
        feed({ ...STORE, install_first: "0.58.0" });
        await mount();
        look("store below the floor");
        await click("How to move off the Store build");
        await snapshot("store-below-floor");
    });

    it("says what the last Store install left when the app came back on the old version", async () => {
        setStoreStayed({ state: "stayed", from: "0.59.25", target: "0.59.26" });
        feed(STORE);
        await mount();
        look("stayed");
        await snapshot("store-stayed");
    });
});

describe("Check for update", () => {
    it("says where this AgentMux stands, and checks, with the button busy meanwhile", async () => {
        feed({ state: "up-to-date", current: "0.59.27", latest: "0.59.27", checked_ms: 1, install: "windows-setup", can_download: false });
        await mount();
        look("up to date");
        h.check = AVAILABLE;
        fireEvent.click(screen.getByRole("button", { name: "Check for update" }));
        look("checking");
        await flush();
        look("a release found");
        await snapshot("check-ok");
    });

    it("shows why a check failed, and a development build links to the latest release", async () => {
        h.check = new Error("GitHub can't be reached");
        feed({ state: "local-build", current: "0.59.27-dev", latest: "0.59.28", url: "https://github.com/agentmuxai/agentmux/releases/tag/v0.59.28", checked_ms: 1, install: "unknown", can_download: false });
        await mount();
        look("development build");
        await click("View");
        await click("Check for update");
        look("check failed");
        await snapshot("check-failed");
    });

    it("cannot check while a download or restart is under way, and says nothing was checked yet at first", async () => {
        await mount();
        look("never checked");
        cleanup();
        feed(downloading("downloading"));
        await mount();
        look("download under way");
        expect((screen.getByRole("button", { name: "Check again" }) as HTMLButtonElement).disabled).toBe(true);
        await snapshot("check-disabled");
    });
});
