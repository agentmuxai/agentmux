// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SystemStats (the status bar's CPU, GPU, memory, page file, disk-space, network and disk I/O readouts), pinned as a
 * whole ahead of splitting it into a state object and smaller views: each scenario mounts the real readouts with
 * stand-ins for the system-info event, the two popovers and the colour and disk-volume helpers, feeds it readings,
 * clicks the CPU and Disk buttons, and logs the bar as an indented tree (inline styles included) and, in order, every
 * call it makes (the event it subscribes to, each reading parsed for volumes, each popover closing).
 * The logs are snapshots recorded on the unsplit component; the split must leave them byte for byte.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const h = vi.hoisted(() => ({ emit: null as any }));

vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (event: string, cb: (data: any) => void, scope: string) => {
        log(`onMuxEvent ${event} ${scope}`);
        h.emit = cb;
        return () => log("unsubscribe");
    },
}));
vi.mock("@/app/store/mps-events", () => ({ WpsEvent: { SysInfo: "sysinfo" } }));
vi.mock("./cpu-color", () => ({ cpuColor: (pct: number) => `var(--cpu-${Math.round(pct)})` }));
vi.mock("./disk-volumes", () => ({
    parseDiskVolumes: (vals: Record<string, number>) => {
        const n = Object.keys(vals).filter((k) => k.startsWith("disk:vol:")).length;
        log(`parseDiskVolumes ${n}`);
        return Array.from({ length: n }, (_, i) => ({ label: `v${i}` }));
    },
    diskTooltip: (volumes: unknown[]) => `${volumes.length} volumes`,
    watchVolumeColor: (pct: number | null, managed: boolean | null) => `var(--watch-${pct}-${managed})`,
}));
vi.mock("./CpuCoresPopover", () => ({
    CpuCoresPopover: (p: { anchor?: HTMLElement; onClose: () => void }) => (
        <div
            data-stand="CpuCoresPopover"
            data-anchor={p.anchor?.className}
            onClick={() => (log("CpuCoresPopover onClose"), p.onClose())}
        />
    ),
}));
vi.mock("./DiskVolumesPopover", () => ({
    DiskVolumesPopover: (p: { anchor?: HTMLElement; initialVolumes: { label: string }[]; onClose: () => void }) => (
        <div
            data-stand="DiskVolumesPopover"
            data-anchor={p.anchor?.className}
            data-volumes={p.initialVolumes.map((v) => v.label).join(",")}
            onClick={() => (log("DiskVolumesPopover onClose"), p.onClose())}
        />
    ),
}));

import { SystemStats } from "./SystemStats";

const SNAP = (name: string) => `./__snapshots__/SystemStats.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|id)$/;

/** The bar as an indented tree: tags, classes, the attributes that carry state (styles included), and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim();
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    const attrs = [...el.attributes]
        .filter((a) => ATTRS.test(a.name))
        .map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let host: HTMLElement;
let unmount: () => void;
const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
/** A system-info event: a quiet machine by default, with whatever a scenario sets. */
const send = (label: string, values: Record<string, number> = {}) => {
    log(`sysinfo ${label}`);
    h.emit({ values });
};
const click = (selector: string) => fireEvent.click(host.querySelector(selector)!);

function mount() {
    const view = render(() => <SystemStats />, { container: document.body.appendChild(document.createElement("div")) });
    host = view.container;
    unmount = view.unmount;
}

beforeEach(() => {
    T.length = 0;
    h.emit = null;
});
afterEach(() => {
    cleanup();
    document.body.innerHTML = "";
});

describe("SystemStats, pinned", () => {
    it("subscribes to the local system info on mount, shows nothing until the first reading, then every always-on readout (an empty reading is zeros), and unsubscribes on unmount", () => {
        mount();
        look("before any reading");
        send("empty");
        look("an empty reading");
        unmount();
        log("unmounted");
        return snapshot("lifecycle");
    });

    it("shows a full reading: CPU and GPU in the heat colour, memory and the page file with their totals, the watched disk's free share, and the network and disk I/O rates", () => {
        mount();
        send("busy", {
            cpu: 37.5,
            gpu: 12.4,
            "mem:used": 6.25,
            "mem:total": 16,
            "mem:commit:used": 9,
            "mem:commit:total": 24,
            "disk:read": 2.5,
            "disk:write": 0.5,
            "net:bytessent": 0.25,
            "net:bytesrecv": 1500,
            "disk:watch:free_gb": 120,
            "disk:watch:free_pct": 61.4,
            "disk:pagefile_system_managed": 1,
            "disk:vol:C:\\:free_gb": 120,
            "disk:vol:C:\\:total_gb": 200,
            "disk:vol:D:\\:free_gb": 10,
            "disk:vol:D:\\:total_gb": 20,
        });
        look("a busy machine");
        return snapshot("full");
    });

    it("warns on memory over 90% and on the page file over 85% and 95%, formats sizes in M below a gigabyte and rates in K, M and G, and mutes the network and disk I/O when both ways are zero", () => {
        mount();
        send("memory just under the line", { "mem:used": 14.4, "mem:total": 16, "mem:commit:used": 85, "mem:commit:total": 100 });
        look("at 90% and 85%");
        send("memory over the line", { "mem:used": 14.5, "mem:total": 16, "mem:commit:used": 85.5, "mem:commit:total": 100 });
        look("over 90% and 85%");
        send("page file over 95%", { "mem:used": 0.5, "mem:total": 0.75, "mem:commit:used": 96, "mem:commit:total": 100 });
        look("sizes in M, page file over 95%");
        send("rates", {
            "net:bytessent": 0.0005,
            "net:bytesrecv": 0.5,
            "disk:read": 1000,
            "disk:write": 2048,
        });
        look("rates in K, K, G, G");
        send("one way only", { "net:bytessent": 0, "net:bytesrecv": 1, "disk:read": 0, "disk:write": 0.0001 });
        look("one way only");
        send("no memory total", { "mem:used": 3, "mem:total": 0 });
        look("no memory total");
        return snapshot("thresholds");
    });

    it("leaves out the GPU without a GPU reading, the page file without a commit total and the Disk readout without a free share, and tells a missing page-file flag from a fixed-size one", () => {
        mount();
        send("nothing optional", { cpu: 5 });
        look("no optional readout");
        send("gpu at zero", { cpu: 5, gpu: 0 });
        look("a GPU at zero");
        send("fixed-size page file", { "disk:watch:free_pct": 40, "disk:pagefile_system_managed": 0 });
        look("page file fixed size");
        send("system-managed page file", { "disk:watch:free_pct": 40, "disk:pagefile_system_managed": 3 });
        look("page file system managed");
        send("free share, no flag", { "disk:watch:free_pct": 0 });
        look("a free share of zero, no flag (not Windows)");
        return snapshot("optional");
    });

    it("opens the per-core CPU panel from the CPU button and closes it from the button or the panel, and does the same for the per-drive Disk panel, which starts from the volumes of the latest reading", () => {
        mount();
        send("with a disk", {
            cpu: 20,
            "disk:watch:free_pct": 55,
            "disk:vol:C:\\:free_gb": 1,
            "disk:vol:C:\\:total_gb": 2,
        });
        click(".stat-cpu-button");
        look("CPU panel open");
        click(".stat-cpu-button");
        look("CPU panel closed by the button");
        click(".stat-cpu-button");
        click("[data-stand='CpuCoresPopover']");
        look("CPU panel closed by the panel");
        click(".stat-disk-button");
        look("Disk panel open");
        send("a second volume", {
            cpu: 20,
            "disk:watch:free_pct": 55,
            "disk:vol:C:\\:free_gb": 1,
            "disk:vol:C:\\:total_gb": 2,
            "disk:vol:D:\\:free_gb": 3,
            "disk:vol:D:\\:total_gb": 4,
        });
        look("Disk panel after another reading");
        click(".stat-cpu-button");
        look("both panels open");
        click("[data-stand='DiskVolumesPopover']");
        look("Disk panel closed by the panel");
        click(".stat-disk-button");
        click(".stat-disk-button");
        look("Disk panel toggled shut");
        send("the free share goes away", { cpu: 20 });
        look("the Disk readout gone");
        return snapshot("panels");
    });
});
