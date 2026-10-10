// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Types for @agentmuxai/widget-sdk v1, the client of the AgentMux widget bridge,
 * protocol 1 (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §6, §7).
 */

export declare const SDK_VERSION: string;
export declare const PROTOCOL: 1;

export type ErrorName =
    | "parse_error"
    | "invalid_request"
    | "method_not_found"
    | "invalid_params"
    | "permission_denied"
    | "limit_exceeded"
    | "not_found"
    | "network_error"
    | "unavailable"
    | "cancelled"
    | "not_ready"
    | "internal"
    | "error";

/** An error answer from AgentMux. */
export declare class AgentMuxError extends Error {
    readonly code: number;
    readonly name: ErrorName;
    /** e.g. `{ permission: "storage" }` for `permission_denied`. */
    readonly data: Record<string, unknown> | null;
}

export type Visibility = "active" | "dormant" | "windowHidden";

export interface Theme {
    mode: "dark" | "light";
    /** CSS custom properties: --am-bg, --am-fg, --am-muted, --am-accent, --am-border,
     *  --am-error, --am-warning, --am-success, --am-font, --am-font-mono, --am-radius,
     *  --am-pane-hue. */
    vars: Record<string, string>;
}

export interface HelloInfo {
    protocol: 1;
    widget: { id: string; version: string; pane: string };
    agentmux: { version: string; host: string };
    /** The permissions the user granted. */
    permissions: string[];
    theme: Theme;
    /** This pane's widget meta. */
    meta: Record<string, unknown>;
    visibility: Visibility;
    focused: boolean;
}

export interface HeaderAction {
    id: string;
    /** A Font Awesome icon name, e.g. "rotate". */
    icon: string;
    title: string;
}

export type MenuItem = { id: string; label: string; disabled?: boolean } | { separator: true };

/** How a status bar item looks now; a field left out shows the manifest's. */
export interface StatusItemLook {
    /** 1–40 characters. */
    text?: string;
    /** A Font Awesome icon name. */
    icon?: string;
    /** At most 120 characters; AgentMux shows it after the widget's name. */
    tooltip?: string;
    tone?: "info" | "success" | "warning" | "error";
    hidden?: boolean;
}

export interface FetchInit {
    method?: string;
    headers?: Record<string, string>;
    body?: string | Uint8Array | ArrayBuffer;
    /** Default 30 000, at most 120 000. */
    timeoutMs?: number;
}

export interface FetchResult {
    status: number;
    statusText: string;
    headers: Record<string, string>;
    /** The final URL, after redirects. */
    url: string;
    ok: boolean;
    text(): string;
    json<T = unknown>(): T;
    bytes(): Uint8Array;
}

export interface PickedFile {
    name: string;
    type: string;
    size: number;
    dataBase64: string;
    bytes(): Uint8Array;
}

export interface AgentInfo {
    id: string;
    name: string;
    state: "working" | "idle" | "stopped";
}

export interface EventMap {
    visibility: { state: Visibility };
    focus: { focused: boolean };
    theme: Theme;
    meta: { meta: Record<string, unknown> };
    action: { id: string; source: "header" | "menu" };
    /** One of the widget's commands (`contributes.commands`) was run from the
     *  palette, or one of its status items was clicked. Sent to the pane it
     *  runs in, opened for it if none was. */
    command: { id: string; source: "palette" | "status" };
    /** The package's storage changed, from any of its panes in any window. */
    storage: { keys: string[] };
    dispose: Record<string, never>;
}

export interface AgentMux {
    readonly info: HelloInfo;
    on<E extends keyof EventMap>(event: E, cb: (params: EventMap[E]) => void): () => void;
    /** Any method, including ones this SDK has no wrapper for. */
    call<T = unknown>(method: string, params?: Record<string, unknown>): Promise<T>;
    meta: {
        get(): Promise<Record<string, unknown>>;
        /** A `null` value deletes the key. At most 64 KB per pane. */
        set(patch: Record<string, unknown>): Promise<void>;
    };
    ui: {
        setTitle(text: string, icon?: string): Promise<void>;
        /** Up to 4. A click comes back as the `action` event. */
        setHeaderActions(actions: HeaderAction[]): Promise<void>;
        /** Up to 20. A click comes back as the `action` event. */
        setContextMenu(items: MenuItem[]): Promise<void>;
        toast(text: string, kind?: "info" | "success" | "warning" | "error"): Promise<void>;
        /** Opens an http(s) link in a browser pane next to the widget. */
        openUrl(url: string): Promise<void>;
        /** Updates one of the widget's status bar items while this pane is
         *  open; with no look, the manifest's is shown again. */
        setStatusItem(id: string, look?: StatusItemLook): Promise<void>;
    };
    theme: { get(): Promise<Theme> };
    panes: {
        /** The package's own views need no permission; any other view needs `panes`. */
        open(view: string, meta?: Record<string, unknown>, split?: "right" | "down" | "tab"): Promise<{ pane: string }>;
    };
    /** Needs `storage`. Per package, at most 5 MB, values up to 1 MB. */
    storage: {
        get<T = unknown>(key: string): Promise<T | null>;
        set(key: string, value: unknown): Promise<void>;
        delete(key: string): Promise<void>;
        list(prefix?: string): Promise<string[]>;
    };
    /** Needs `net:<origin>` for the request's origin. */
    net: { fetch(url: string, init?: FetchInit): Promise<FetchResult> };
    /** Needs `files`. */
    files: {
        pick(opts?: { accept?: string[]; multiple?: boolean }): Promise<PickedFile[]>;
        save(name: string, data: string | Uint8Array, type?: string): Promise<boolean>;
    };
    /** Needs `clipboard:write`. */
    clipboard: { writeText(text: string): Promise<void> };
    agents: {
        /** Needs `agents:read`. */
        list(): Promise<AgentInfo[]>;
        /** Needs `agents:send`. Returns the message id. */
        send(agent: string, text: string): Promise<string>;
    };
}

export declare function connect(options?: { applyTheme?: boolean; timeoutMs?: number }): Promise<AgentMux>;

export declare function useVisibility(am: AgentMux, handlers?: { onActive?: () => void; onDormant?: () => void }): () => void;

export declare const base64: {
    encode(bytes: Uint8Array | ArrayBuffer): string;
    decode(text: string): Uint8Array;
};
