// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Placeholder demo for the singleton-modal coordination layer — PR 3 of
 * docs/specs/SPEC_BUNDLE_MANAGEMENT_2026_05_22.md.
 *
 * This is NOT the real bundle manager (that is PR 4). It is a trivial
 * surface whose only job is to *exercise and demonstrate* the
 * coordination layer:
 *
 *   - `openSingletonPlaceholderDemo()` — the entry point a menu item /
 *     button calls. It gates on `acquireSingleton(kind)`:
 *       • acquired  → opens `SingletonPlaceholderModal` (a normal
 *         `<Modal scope="window">`); this window now holds the singleton.
 *       • not acquired → no modal; the caller surface instead shows the
 *         `SingletonElsewhereBanner` (open-in-<Window N>, click to focus).
 *   - `SingletonPlaceholderModal` releases the singleton on unmount.
 *   - `SingletonElsewhereBanner` is the persistent "open elsewhere"
 *     affordance — wired to `getApi().focusWindow(label)`.
 *
 * PR 4 swaps the placeholder for `BundleManagerModal` and the demo
 * button for the real hamburger "Identity & Memory" entry, keeping this
 * exact gating shape.
 */

import { createMemo, createSignal, Show, type Accessor, type JSX } from "solid-js";

import { getApi } from "@/store/global";
import { openWindowEntriesAtom } from "@/app/store/global";
import { resolveWindowName } from "@/util/window-title";
import { Modal } from "@/element/modal";
import type { ModalCloseProps } from "@/app/store/modalmodel";
import { openModal } from "@/app/store/modalmodel";
import {
    acquireSingleton,
    releaseSingleton,
    singletonHolder,
    SINGLETON_KIND_PLACEHOLDER,
} from "@/app/store/singleton-modal";
import "./singleton-placeholder-demo.scss";

/**
 * Resolve a window *label* to its human display name ("Window N", the
 * workspace name, or a user-set name). Mirrors InstancePanel's naming so
 * the banner and the panel agree. Falls back to the raw label if the
 * label is not (yet) in `openWindowEntriesAtom`.
 */
function windowNameForLabel(label: string): string {
    const entries = openWindowEntriesAtom();
    const idx = entries.findIndex((e) => e.label === label);
    if (idx === -1) return label;
    // The placeholder demo keeps naming minimal — positional "Window N".
    // PR 4's real manager can read the window's `meta.displayname` /
    // workspace name through `getObjectValue` if a richer name is wanted.
    return resolveWindowName({ indexInOpenWindows: idx });
}

// ── The placeholder modal ──────────────────────────────────────────────

/**
 * The window-scoped placeholder modal. Rendered ONLY in the window that
 * holds the singleton. Releases the claim when it closes so other
 * windows can take it.
 */
const SingletonPlaceholderModal = (props: ModalCloseProps): JSX.Element => {
    const handleClose = () => {
        releaseSingleton(SINGLETON_KIND_PLACEHOLDER);
        props.close();
    };

    return (
        <Modal open={true} onClose={handleClose} scope="window" size="md">
            <div class="modal-panel-header">
                <div class="modal-panel-title">Singleton placeholder</div>
            </div>
            <div class="modal-panel-body">
                <p>
                    This is the PR-3 placeholder for the app-wide singleton
                    modal. Exactly one of these can be open across every
                    AgentMux window.
                </p>
                <p>
                    Try the "Singleton demo" hamburger item in another window
                    while this is open — it will show a focus banner instead
                    of opening a second copy.
                </p>
            </div>
            <div class="modal-panel-footer">
                <button
                    type="button"
                    class="modal-btn modal-btn--confirm"
                    onClick={handleClose}
                >
                    Close
                </button>
            </div>
        </Modal>
    );
};

SingletonPlaceholderModal.displayName = "SingletonPlaceholderModal";

// ── The "open elsewhere" banner ────────────────────────────────────────

/**
 * Persistent banner shown in NON-holding windows: "Singleton placeholder
 * is open in <Window N> — click to focus". Clicking calls `focusWindow`
 * on the holding window.
 *
 * Reactive: `singletonHolder` is a tracked accessor, so this disappears
 * automatically when the holder releases (or crashes — crash-release
 * clears the holder via the launcher exit signal).
 */
export const SingletonElsewhereBanner = (props: {
    /** Reactive holder accessor — pass `singletonHolder(kind)`. */
    holder: Accessor<string | null>;
    /** Label of THIS window — banner only shows when holder !== this. */
    myLabel: string | null;
}): JSX.Element => {
    const showBanner = createMemo(() => {
        const h = props.holder();
        return h != null && h !== props.myLabel;
    });

    const holderName = createMemo(() => {
        const h = props.holder();
        return h ? windowNameForLabel(h) : "";
    });

    const focusHolder = () => {
        const h = props.holder();
        if (!h) return;
        getApi()
            .focusWindow(h)
            .catch((e) => console.error("[singleton-demo] focusWindow failed:", e));
    };

    return (
        <Show when={showBanner()}>
            <button
                type="button"
                class="singleton-elsewhere-banner"
                onClick={focusHolder}
                title="Bring the holding window to the front"
            >
                Singleton placeholder is open in {holderName()} — click to focus
            </button>
        </Show>
    );
};

SingletonElsewhereBanner.displayName = "SingletonElsewhereBanner";

/**
 * Self-contained banner for the placeholder demo, mountable directly in
 * the app shell. Resolves this window's label internally and renders the
 * "open in <Window N> — click to focus" banner whenever the placeholder
 * singleton is held by a *different* window. Renders nothing otherwise.
 *
 * PR 4 mounts the equivalent for the real bundle manager.
 */
export const SingletonDemoBanner = (): JSX.Element => {
    const [myLabel, setMyLabel] = createSignal<string | null>(null);
    getApi()
        .getWindowLabel()
        .then((l) => setMyLabel(l))
        .catch(() => setMyLabel("main"));

    return (
        <SingletonElsewhereBanner
            holder={singletonHolder(SINGLETON_KIND_PLACEHOLDER)}
            myLabel={myLabel()}
        />
    );
};

SingletonDemoBanner.displayName = "SingletonDemoBanner";

// ── Entry point ────────────────────────────────────────────────────────

/**
 * Entry point a menu item / button calls. Gates on the singleton:
 *  - acquired  → opens the placeholder modal in this window.
 *  - not acquired → focuses the holding window (the same affordance the
 *    banner gives) so a click is never a dead end.
 *
 * Returns `true` if the modal was opened here, `false` if the singleton
 * was held elsewhere (caller can rely on the banner for the persistent
 * affordance).
 */
export function openSingletonPlaceholderDemo(): boolean {
    if (acquireSingleton(SINGLETON_KIND_PLACEHOLDER)) {
        openModal(SingletonPlaceholderModal);
        return true;
    }
    // Held elsewhere — focus that window directly (banner stays the
    // persistent surface; this makes the menu click itself useful too).
    const holder = singletonHolder(SINGLETON_KIND_PLACEHOLDER)();
    if (holder) {
        getApi()
            .focusWindow(holder)
            .catch((e) => console.error("[singleton-demo] focusWindow failed:", e));
    }
    return false;
}
