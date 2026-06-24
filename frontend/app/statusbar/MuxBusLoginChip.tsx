// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { useMuxBusStatus } from "@/app/view/accounts/AgentMuxConnectPanel";
import { usePaneOverlay } from "@/app/platform/pane-overlay";

/**
 * Status-bar chip that surfaces MuxBus Cloud account state.
 *
 * Three visible states:
 *   - Hidden: build has no VITE_MUXBUS_CLIENT_ID (dev/internal).
 *   - "Sign in to MuxBus Cloud" button: not signed in or token expired.
 *   - Email chip + popover: signed in and valid.
 *
 * Uses the same `useMuxBusStatus` controller as the Trust Center panel —
 * a connect/disconnect here is immediately reflected there and vice-versa
 * because each controller independently polls `muxbus.status` on mount.
 */
export const MuxBusLoginChip = (): JSX.Element => {
    const muxbus = useMuxBusStatus();
    onMount(() => void muxbus.refresh());

    const [popoverOpen, setPopoverOpen] = createSignal(false);
    const [anchorRect, setAnchorRect] = createSignal<DOMRect | null>(null);
    let chipRef!: HTMLButtonElement;
    let popoverRef: HTMLDivElement | undefined;

    usePaneOverlay(() => popoverRef);

    const handleChipClick = () => {
        if (popoverOpen()) {
            setPopoverOpen(false);
            return;
        }
        setAnchorRect(chipRef?.getBoundingClientRect() ?? null);
        setPopoverOpen(true);
    };

    createEffect(() => {
        if (!popoverOpen()) return;
        const onKey = (e: KeyboardEvent) => {
            if (e.key === "Escape") setPopoverOpen(false);
        };
        const onDown = (e: MouseEvent) => {
            const t = e.target as Node;
            if (chipRef?.contains(t)) return;
            if (popoverRef?.contains(t)) return;
            setPopoverOpen(false);
        };
        document.addEventListener("keydown", onKey);
        document.addEventListener("mousedown", onDown);
        onCleanup(() => {
            document.removeEventListener("keydown", onKey);
            document.removeEventListener("mousedown", onDown);
        });
    });

    // Not configured in this build — show nothing.
    if (!muxbus.isConfigured()) return <></>;

    const isSignedIn = () => muxbus.status()?.connected && muxbus.status()?.valid;
    const isExpired = () => muxbus.status()?.connected && !muxbus.status()?.valid;

    return (
        <>
            <Show
                when={isSignedIn()}
                fallback={
                    <button
                        type="button"
                        class="muxbus-login-chip muxbus-login-chip-signin"
                        disabled={muxbus.loading()}
                        data-tip={isExpired() ? "MuxBus Cloud session expired — click to re-login" : "Sign in to MuxBus Cloud"}
                        aria-label="Sign in to MuxBus Cloud"
                        onClick={() => void muxbus.connect()}
                    >
                        {muxbus.loading() ? "●···" : isExpired() ? "MuxBus (expired)" : "Sign in"}
                    </button>
                }
            >
                <button
                    ref={chipRef!}
                    type="button"
                    class="muxbus-login-chip muxbus-login-chip-connected clickable"
                    data-tip="MuxBus Cloud account"
                    aria-label="MuxBus Cloud account — open options"
                    aria-haspopup="menu"
                    aria-expanded={popoverOpen()}
                    onClick={handleChipClick}
                >
                    <span class="muxbus-login-chip-dot" aria-hidden="true" />
                    <span class="muxbus-login-chip-email">{muxbus.status()?.email}</span>
                </button>
            </Show>

            <Show when={popoverOpen()}>
                <MuxBusPopover
                    ref={(el) => (popoverRef = el)}
                    anchorRect={anchorRect()}
                    email={muxbus.status()?.email ?? ""}
                    loading={muxbus.loading()}
                    onDisconnect={async () => {
                        await muxbus.disconnect();
                        setPopoverOpen(false);
                    }}
                />
            </Show>

            <Show when={muxbus.error()}>
                <span class="muxbus-login-chip-error" title={muxbus.error()!}>!</span>
            </Show>
        </>
    );
};

MuxBusLoginChip.displayName = "MuxBusLoginChip";

interface MuxBusPopoverProps {
    ref: (el: HTMLDivElement) => void;
    anchorRect: DOMRect | null;
    email: string;
    loading: boolean;
    onDisconnect: () => void;
}

const MuxBusPopover = (props: MuxBusPopoverProps): JSX.Element => {
    const style = (): string => {
        const r = props.anchorRect;
        if (!r) return "";
        const right = window.innerWidth - r.right;
        const bottom = window.innerHeight - r.top;
        return `position: fixed; right: ${right}px; bottom: ${bottom}px;`;
    };

    return (
        <div
            ref={props.ref}
            class="status-bar-popover host-popover muxbus-login-popover"
            role="menu"
            style={style()}
        >
            <div class="status-bar-popover-row">
                <span class="status-bar-popover-label">Signed in as</span>
                <span class="status-bar-popover-mono muxbus-popover-email">{props.email}</span>
            </div>
            <div class="status-bar-popover-divider" />
            <button
                type="button"
                class="muxbus-popover-disconnect-btn"
                disabled={props.loading}
                onClick={() => props.onDisconnect()}
            >
                {props.loading ? "Disconnecting…" : "Disconnect"}
            </button>
        </div>
    );
};
