// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pane host for a sandboxed widget
 * (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §6).
 *
 * The widget's entry document runs in an `<iframe>` with an opaque origin
 * (`sandbox` without `allow-same-origin`), served by srv from the approved
 * package only (`/agentmux/widget-files/…`, under a CSP that allows no
 * network). It reaches AgentMux through one MessagePort, handed over after
 * its first `load`; a second `load` means it navigated away, and the port is
 * closed at once. Every request is checked here against the package's
 * granted permissions (and again by srv for what reaches srv).
 */

import { createEffect, createSignal, on, Show } from "solid-js";
import { Button } from "@/app/element/ui";
import { createBlockSplitHorizontally, createBlockSplitVertically } from "@/app/store/block-layout-actions";
import { pushNotification } from "@/app/store/flash-notifications";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { WidgetPackageInfo, WidgetPaneInfo } from "@/app/store/rpc-api/widgets";
import { getHostName } from "@/app/store/misc-utils";
import { UI_VERSION } from "@/app/store/srv-info";
import { addWidgetAsPaneTab } from "@/layout/index";
import { getLayoutModelForStaticTab } from "@/layout/lib/layoutModelHooks";
import { writeText } from "@/util/clipboard";
import { getWebServerEndpoint } from "@/util/endpoints";
import type { PaneTabHostContext, PaneTabInstance, PaneTabManifest } from "./pane-tab-registry";
import { BridgeError, bridgeErrorOf, bridgeTheme, ERR, handleBridgeRequest, type BridgeHost, type BridgeState } from "./widget-bridge";

import "./sandboxed-widget.scss";

export const PROTOCOL = 1;
const HELLO_TIMEOUT_MS = 10_000;
const MAX_IN_FLIGHT = 64;

export const IFRAME_SANDBOX = "allow-scripts allow-forms allow-modals allow-downloads";

export function makeSandboxedPaneManifest(pkg: WidgetPackageInfo, pane: WidgetPaneInfo): PaneTabManifest {
    return {
        apiVersion: 1,
        view: pane.view,
        label: pane.label,
        icon: pane.icon,
        defaultHue: pkg.default_hue ?? undefined,
        capabilities: { noPadding: true },
        create: (ctx) => createSandboxedInstance(pkg, pane, ctx),
    };
}

function createSandboxedInstance(pkg: WidgetPackageInfo, pane: WidgetPaneInfo, ctx: PaneTabHostContext): PaneTabInstance {
    const [title, setTitle] = createSignal<{ text: string; icon?: string } | null>(null);
    const [actions, setActions] = createSignal<{ id: string; icon: string; title: string }[]>([]);
    const [menu, setMenu] = createSignal<({ id: string; label: string; disabled?: boolean } | { separator: true })[]>([]);
    const [failure, setFailure] = createSignal<string | null>(null);
    // Bumped by Restart: a new frame, a new port, a fresh handshake.
    const [generation, setGeneration] = createSignal(0);

    let port: MessagePort | null = null;
    let iframe: HTMLIFrameElement | null = null;
    let loads = 0;
    let disposed = false;
    // One per session: a request still running when the widget restarts
    // finishes against its own session's state and port, never the new one's.
    let state: BridgeState = { ready: false, inFlight: 0 };

    const notify = (method: string, params: unknown) => port?.postMessage({ jsonrpc: "2.0", method, params });

    // The pane's session with srv, for the calls srv answers. Opened on the
    // first such call; srv checks every call against the package again.
    let session: Promise<string> | null = null;
    const sessionToken = (): Promise<string> =>
        (session ??= RpcApi.WidgetsSessionCommand(TabRpcClient, { id: pkg.id, hash: pkg.hash, blockid: ctx.blockId })
            .then((r) => r.token)
            .catch((e) => {
                session = null;
                throw bridgeErrorOf(e);
            }));

    const host: BridgeHost = {
        pkg,
        pane,
        ctx,
        setTitle,
        setActions,
        setMenu,
        toast: (text, kind) =>
            pushNotification({
                icon: kind === "error" ? "fa-circle-exclamation" : kind === "warning" ? "fa-triangle-exclamation" : "fa-puzzle-piece",
                title: pkg.name,
                message: text,
                timestamp: new Date().toISOString(),
                type: kind === "error" ? "error" : kind === "warning" ? "warning" : "info",
                expiration: Date.now() + 6000,
            }),
        openUrl: async (url) => {
            await createBlockSplitHorizontally({ meta: { view: "browser", url } }, ctx.blockId, "after");
        },
        openPane: async (view, meta, split) => {
            const blockDef = { meta: { ...meta, view } };
            if (split === "tab") {
                const model = getLayoutModelForStaticTab();
                const nodeId = model.getNodeByBlockId(ctx.blockId)?.id;
                if (!nodeId) throw new Error("this widget's pane isn't in the layout");
                await addWidgetAsPaneTab(model, nodeId, blockDef);
                return "";
            }
            return split === "down"
                ? createBlockSplitVertically(blockDef, ctx.blockId, "after")
                : createBlockSplitHorizontally(blockDef, ctx.blockId, "after");
        },
        agentmuxVersion: () => UI_VERSION,
        hostName: () => getHostName(),
        srv: async (method, params) => {
            const token = await sessionToken();
            try {
                return (await RpcApi.WidgetsCallCommand(TabRpcClient, { token, method, params })).result;
            } catch (e) {
                throw bridgeErrorOf(e);
            }
        },
        pickFiles: (accept, multiple) => pickFiles(accept, multiple),
        saveFile: (name, type, data) => saveFile(name, type, data),
        writeClipboard: (text) => writeText(text),
    };

    const closePort = () => {
        port?.close();
        port = null;
        state.ready = false;
        state = { ready: false, inFlight: 0 };
    };

    // Stopping removes the frame (it unmounts with the failure shown).
    const stop = (reason: string) => {
        closePort();
        setFailure(reason);
    };

    const restart = () => {
        closePort();
        loads = 0;
        setFailure(null);
        setGeneration((g) => g + 1);
    };

    const connect = () => {
        const started = generation();
        const channel = new MessageChannel();
        const own = channel.port1;
        const session = state;
        port = own;
        own.onmessage = async (ev) => {
            const msg = ev.data;
            if (!msg || msg.jsonrpc !== "2.0" || msg.id == null || typeof msg.method !== "string") return;
            if (session.inFlight >= MAX_IN_FLIGHT) {
                own.postMessage({ jsonrpc: "2.0", id: msg.id, error: { code: 1002, message: "too many requests in flight", data: { limit: "inFlight" } } });
                return;
            }
            session.inFlight++;
            try {
                let reply;
                try {
                    reply = await handleBridgeRequest(host, session, msg.method, msg.params ?? {});
                } catch (e) {
                    reply = { error: { code: 1099, message: e instanceof Error ? e.message : String(e) } };
                }
                if (port === own) own.postMessage({ jsonrpc: "2.0", id: msg.id, ...reply });
            } finally {
                session.inFlight--;
            }
        };
        own.start();
        iframe!.contentWindow?.postMessage({ type: "agentmux:connect", protocols: [PROTOCOL] }, "*", [channel.port2]);
        setTimeout(() => {
            if (!state.ready && port && !disposed && generation() === started) stop(`${pkg.name} didn't start (no hello within ${HELLO_TIMEOUT_MS / 1000} s).`);
        }, HELLO_TIMEOUT_MS);
    };

    // Events the widget follows.
    createEffect(on(() => ctx.visibility(), (v) => state.ready && notify("visibility", { state: v }), { defer: true }));
    createEffect(on(() => ctx.isFocused(), (f) => state.ready && notify("focus", { focused: f }), { defer: true }));
    createEffect(
        on(
            () => JSON.stringify(ownMeta(pkg.id, ctx.meta())),
            (json) => {
                if (!state.ready || json === state.lastMetaSent) return;
                state.lastMetaSent = json;
                notify("meta", { meta: JSON.parse(json) });
            },
            { defer: true }
        )
    );
    // Another pane of this package (or this one) changed its storage.
    const unsubscribeStorage = muxEventSubscribe({
        eventType: WpsEvent.WidgetStorage,
        handler: (event: { data?: { id?: string; keys?: string[] } }) => {
            if (state.ready && event?.data?.id === pkg.id && pkg.granted.includes("storage")) notify("storage", { keys: event.data.keys ?? [] });
        },
    });
    const themeObserver = new MutationObserver(() => state.ready && notify("theme", bridgeTheme(pkg)));
    themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ["class", "style", "data-theme"] });

    const src = (): string =>
        `${getWebServerEndpoint()}${pkg.files_url}${pane.entry}?pane=${encodeURIComponent(crypto.randomUUID())}`;

    return {
        liveTitle: () => title() ?? { text: pane.label },
        headerActions: () =>
            actions().map((a) => ({
                elemtype: "iconbutton" as const,
                icon: a.icon,
                title: a.title,
                click: () => notify("action", { id: a.id, source: "header" }),
            })),
        contextMenu: () =>
            menu().map((item) =>
                "separator" in item
                    ? { type: "separator" as const }
                    : { label: item.label, enabled: !item.disabled, click: () => notify("action", { id: item.id, source: "menu" }) }
            ),
        component: () => {
            const frame = () => {
                const el = (
                    <iframe
                        sandbox={IFRAME_SANDBOX}
                        allow="clipboard-write"
                        referrerpolicy="no-referrer"
                        title={pane.label}
                        class="sandboxed-widget-frame"
                        src={src()}
                    />
                ) as HTMLIFrameElement;
                el.addEventListener("load", () => {
                    loads++;
                    if (loads === 1) connect();
                    else stop(`${pkg.name} left its page and was stopped.`);
                });
                iframe = el;
                return el;
            };
            return (
                <div class="sandboxed-widget">
                    <Show when={failure() ? null : generation() + 1} keyed>
                        {(_generation: number) => frame()}
                    </Show>
                    <Show when={failure()}>
                        {(reason) => (
                            <div class="sandboxed-widget-failure" role="alert">
                                <i class="fa fa-solid fa-puzzle-piece" aria-hidden="true" />
                                <div class="sandboxed-widget-failure-text">{reason()}</div>
                                <Button icon="rotate-right" onClick={restart}>
                                    Restart
                                </Button>
                            </div>
                        )}
                    </Show>
                </div>
            );
        },
        dispose: () => {
            disposed = true;
            unsubscribeStorage();
            void session?.then((token) => RpcApi.WidgetsEndSessionCommand(TabRpcClient, { token })).catch(() => {});
            themeObserver.disconnect();
            notify("dispose", {});
            const p = port;
            setTimeout(() => p?.close(), 1000);
            port = null;
            iframe?.remove();
        },
    };
}

/**
 * The open dialog, as a file input in the app's own document: it works in a
 * browser as in the desktop app. A dialog opens only on a user's click; a
 * click in the widget's frame counts for the app too.
 */
function pickFiles(accept: string[], multiple: boolean): Promise<File[] | null> {
    if (navigator.userActivation && !navigator.userActivation.isActive) {
        return Promise.reject(new BridgeError(ERR.UNAVAILABLE, "files.pick opens a dialog only from a click"));
    }
    return new Promise((resolve) => {
        const input = document.createElement("input");
        input.type = "file";
        input.multiple = multiple;
        if (accept.length) input.accept = accept.join(",");
        input.style.display = "none";
        const done = (files: File[] | null) => {
            input.remove();
            resolve(files);
        };
        input.addEventListener("change", () => done(input.files ? [...input.files] : []));
        input.addEventListener("cancel", () => done(null));
        document.body.appendChild(input);
        input.click();
    });
}

/** The save dialog where the host has one; otherwise a download. Like
 *  the open dialog, only on a user's click. */
async function saveFile(name: string, type: string, data: Uint8Array): Promise<boolean> {
    if (navigator.userActivation && !navigator.userActivation.isActive) {
        throw new BridgeError(ERR.UNAVAILABLE, "files.save opens a dialog only from a click");
    }
    const picker = (window as { showSaveFilePicker?: (o: { suggestedName: string }) => Promise<FileSystemFileHandle> }).showSaveFilePicker;
    if (picker) {
        try {
            const handle = await picker({ suggestedName: name });
            const writable = await handle.createWritable();
            await writable.write(data as BlobPart);
            await writable.close();
            return true;
        } catch (e) {
            if (e instanceof DOMException && e.name === "AbortError") return false;
            throw new BridgeError(ERR.INTERNAL, e instanceof Error ? e.message : String(e));
        }
    }
    const url = URL.createObjectURL(new Blob([data as BlobPart], type ? { type } : {}));
    const a = document.createElement("a");
    a.href = url;
    a.download = name;
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 60_000);
    return true;
}

/** The widget's own meta keys (`widget:<id>:<key>`), without the prefix. */
export function ownMeta(id: string, meta: Record<string, unknown> | null | undefined): Record<string, unknown> {
    const prefix = `widget:${id}:`;
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(meta ?? {})) if (k.startsWith(prefix)) out[k.slice(prefix.length)] = v;
    return out;
}
