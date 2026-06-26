// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BundleManagerModal — PR 4 (Feature 2) of
 * docs/specs/SPEC_BUNDLE_MANAGEMENT_2026_05_22.md (§3).
 *
 * The canonical, app-wide home for managing Identity & Memory bundles.
 * It is a window-scoped `<Modal>` (inerts its own window) AND an
 * app-wide singleton (PR 3's coordination layer) — only one bundle
 * manager ever exists across every AgentMux window at a time.
 *
 * Layout (§5 decision 2 — left-rail toggle): a left rail toggles
 * between the **Identities** and **Memories** sections; the selected
 * section renders the context-free `<IdentityManager/>` / `<MemoryManager/>`
 * components extracted in PR 2. Each owns its own block-free model and
 * drives off the `bundle_*` RPCs + `*:changed` WPS events, so any other
 * surface that displays a bundle stays consistent for free.
 *
 * Singleton gating (§3 "Singleton"):
 *  - `openBundleManager()` is the entry point the hamburger calls. It
 *    gates on `acquireSingleton("bundle-manager")`:
 *      • acquired  → opens this modal here; this window now holds it.
 *      • not acquired → no modal; focuses the holding window instead
 *        (the persistent banner stays the durable affordance).
 *  - The modal calls `releaseSingleton` on close so another window can
 *    take it. Crash-release (PR 3) covers a holder that dies without
 *    closing cleanly.
 *
 * `BundleManagerElsewhereBanner` is the persistent "open elsewhere"
 * affordance (§5 decision 6) shown in non-holding windows — mounted in
 * the app shell, reactive to `singletonHolder`.
 */

import { createMemo, createSignal, For, onCleanup, Show, type Accessor, type JSX } from "solid-js";

import { getApi, openOrFocusPaneByView } from "@/store/global";
import { openWindowEntriesAtom } from "@/app/store/global";
import { resolveWindowName } from "@/util/window-title";
import { Modal } from "@/element/modal";
import type { ModalCloseProps } from "@/app/store/modalmodel";
import { openModal } from "@/app/store/modalmodel";
import {
    acquireSingleton,
    releaseSingleton,
    singletonHolder,
    type SingletonKind,
} from "@/app/store/singleton-modal";
import { IdentityManager } from "@/app/view/identity/identity-manager";
import { MemoryManager } from "@/app/view/memory/memory-manager";
import { AccountsManager } from "@/app/view/accounts/accounts-manager";
import { GlobalBrainManager } from "@/app/view/brain/global-brain-manager";
import "./bundle-manager-modal.scss";

/**
 * Singleton kind for the bundle manager. Distinct scope string so its
 * claims never collide with any other (future) singleton modal.
 */
export const SINGLETON_KIND_BUNDLE_MANAGER: SingletonKind = "bundle-manager";

/** Which section the left rail currently shows. */
type BundleSection = "accounts" | "identities" | "brain" | "memories";

/**
 * Resolve a window *label* to its human display name ("Window N", the
 * workspace name, or a user-set name). Mirrors InstancePanel's naming so
 * the banner agrees with the instance panel. Falls back to the raw label
 * when it is not (yet) in `openWindowEntriesAtom`.
 */
function windowNameForLabel(label: string): string {
    const entries = openWindowEntriesAtom();
    const idx = entries.findIndex((e) => e.label === label);
    if (idx === -1) return label;
    return resolveWindowName({ indexInOpenWindows: idx });
}

// ── The bundle manager modal ───────────────────────────────────────────

/**
 * The window-scoped bundle manager. Rendered ONLY in the window that
 * holds the singleton. Releases the claim when it closes so other
 * windows can take it.
 */
export const BundleManagerModal = (props: ModalCloseProps): JSX.Element => {
    const [section, setSection] = createSignal<BundleSection>("accounts");

    // Release the singleton on unmount — guaranteed regardless of HOW the
    // modal closes. The Close button and ESC route through `onClose`, but
    // the global Escape handler (keymodel.ts) closes via
    // `modalsModel.closeTopModal()`, which bypasses `onClose` entirely.
    // An onCleanup release covers every path. (releaseSingleton is
    // idempotent — a no-op when this window isn't the holder.)
    onCleanup(() => releaseSingleton(SINGLETON_KIND_BUNDLE_MANAGER));

    const handleClose = () => props.close();

    // "Brain" (global brain — the brain icon) is distinct from "Memories"
    // (the full bundle library, now layer-group) so the brain icon means one
    // thing only. See SPEC_TRUST_CENTER_GLOBAL_BRAIN_2026_06_19.md.
    const rail: { id: BundleSection; label: string; icon: string }[] = [
        { id: "accounts", label: "Accounts", icon: "key" },
        { id: "identities", label: "Identities", icon: "id-card" },
        { id: "brain", label: "Brain", icon: "brain" },
        { id: "memories", label: "Presets", icon: "sliders" },
    ];

    return (
        <Modal open={true} onClose={handleClose} scope="window" size="xl">
            <div class="modal-panel-header">
                <div class="modal-panel-title">Trust Center</div>
            </div>
            <div class="modal-panel-body bundle-manager-body">
                <nav class="bundle-manager-rail" aria-label="Bundle section">
                    <For each={rail}>
                        {(item) => (
                            <button
                                type="button"
                                class="bundle-manager-rail-item"
                                classList={{ "is-active": section() === item.id }}
                                aria-pressed={section() === item.id}
                                onClick={() => setSection(item.id)}
                            >
                                <i
                                    class={`fa-sharp fa-solid fa-${item.icon}`}
                                    aria-hidden="true"
                                />
                                <span>{item.label}</span>
                            </button>
                        )}
                    </For>
                </nav>
                <div class="bundle-manager-section">
                    {/*
                     * Each section keeps its own live manager mounted —
                     * <Show> with no `fallback` unmounts the hidden one,
                     * which would dispose its model and re-fetch on every
                     * toggle. Two cheap models is the simpler, snappier
                     * choice; both stay consistent via the `*:changed`
                     * WPS events regardless.
                     */}
                    <div
                        class="bundle-manager-pane"
                        classList={{ "is-hidden": section() !== "accounts" }}
                    >
                        <AccountsManager />
                    </div>
                    <div
                        class="bundle-manager-pane"
                        classList={{ "is-hidden": section() !== "identities" }}
                    >
                        <IdentityManager />
                    </div>
                    <div
                        class="bundle-manager-pane"
                        classList={{ "is-hidden": section() !== "brain" }}
                    >
                        <GlobalBrainManager />
                    </div>
                    <div
                        class="bundle-manager-pane"
                        classList={{ "is-hidden": section() !== "memories" }}
                    >
                        <MemoryManager />
                    </div>
                </div>
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

BundleManagerModal.displayName = "BundleManagerModal";

// ── The "open elsewhere" banner ────────────────────────────────────────

/**
 * Persistent banner shown in NON-holding windows: "Identity & Memory is
 * open in <Window N> — click to focus" (§5 decision 6). Clicking calls
 * `focusWindow` on the holding window.
 *
 * Reactive: `singletonHolder` is a tracked accessor, so this disappears
 * automatically when the holder releases — or crashes, since PR 3's
 * crash-release clears the holder on the launcher window-exit signal.
 */
const BundleManagerElsewhereBannerInner = (props: {
    /** Reactive holder accessor — `singletonHolder(kind)`. */
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
            .catch((e) => console.error("[bundle-manager] focusWindow failed:", e));
    };

    return (
        <Show when={showBanner()}>
            <button
                type="button"
                class="bundle-manager-elsewhere-banner"
                onClick={focusHolder}
                title="Bring the holding window to the front"
            >
                Trust Center is open in {holderName()} — click to focus
            </button>
        </Show>
    );
};

/**
 * Self-contained banner mountable directly in the app shell. Resolves
 * this window's label internally and renders the "open in <Window N> —
 * click to focus" banner whenever the bundle manager is held by a
 * *different* window. Renders nothing otherwise.
 */
export const BundleManagerElsewhereBanner = (): JSX.Element => {
    const [myLabel, setMyLabel] = createSignal<string | null>(null);
    getApi()
        .getWindowLabel()
        .then((l) => setMyLabel(l))
        .catch(() => setMyLabel("main"));

    return (
        <BundleManagerElsewhereBannerInner
            holder={singletonHolder(SINGLETON_KIND_BUNDLE_MANAGER)}
            myLabel={myLabel()}
        />
    );
};

BundleManagerElsewhereBanner.displayName = "BundleManagerElsewhereBanner";

// ── Entry point ────────────────────────────────────────────────────────

/**
 * Entry point the hamburger "Identity & Memory" item calls. Gates on the
 * singleton:
 *  - acquired  → opens the bundle manager modal in this window.
 *  - not acquired → focuses the holding window (same affordance the
 *    banner gives) so the menu click is never a dead end.
 *
 * Returns `true` if the modal was opened here, `false` if the singleton
 * was held elsewhere (the banner stays the persistent affordance).
 */
/** Shim — Trust Center is now a widget pane. Kept for call-site compatibility. */
export function openBundleManager(): boolean {
    void openOrFocusPaneByView("trust");
    return true;
}
