// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane tab type whose code loads when its first pane is created, not at
 * startup (docs/specs/SPEC_FRONTEND_BUNDLE_SPLITTING_2026_10_21.md §6).
 *
 * The registry keeps the static part (view, label, icon, capabilities), which
 * the tab bar, menus and widget lists read without an instance. `create(ctx)`
 * stays synchronous, as the host requires: it returns a stand-in instance at
 * once and builds the real one, in the same reactive root (host rule 8), when
 * the module arrives.
 *
 * The host maps an instance's members onto the pane once, at creation
 * (`adaptPaneTabInstance`), and some mean something by being there at all: a
 * `rename` makes the title editable, a `selection` replaces the window's own.
 * So the stand-in has exactly the members the manifest declares, each
 * forwarding to the real instance once it exists. When the real instance
 * arrives with a different set, the mismatch is logged as an error: update
 * `members` to match.
 *
 * The stand-in exists only while the code is on its way. Until then it fills
 * the pane with the pane surface (`--main-bg-color`), not nothing, which let the
 * frame's dark backdrop show for a moment. Once a type's module has loaded, its
 * next panes are built at once, like a built-in pane's, with no stand-in at all.
 */

import { Button } from "@/app/element/ui";
import { focusManager } from "@/app/store/focusManager";
import { CenteredDiv } from "@/element/quickelems";
import { createSignal, getOwner, onCleanup, runWithOwner, Show, untrack } from "solid-js";
import type { PaneTabHostContext, PaneTabInstance, PaneTabManifest } from "./pane-tab-registry";

import "./lazy-pane-tab.scss";

/** An instance member besides `component` and `dispose`, which every stand-in has. */
export type PaneTabMember = Exclude<keyof PaneTabInstance, "component" | "dispose">;

export interface LazyPaneTab extends Omit<PaneTabManifest, "create"> {
    /** Exactly the members the real instance has. */
    members: readonly PaneTabMember[];
    /** The module with the real `create(ctx)`. */
    load: () => Promise<{ create(ctx: PaneTabHostContext): PaneTabInstance }>;
}

type PaneTabModule = Awaited<ReturnType<LazyPaneTab["load"]>>;

export function lazyPaneTab(spec: LazyPaneTab): PaneTabManifest {
    const { members, load, ...manifest } = spec;
    // One load per pane type, shared by the panes waiting for it; forgotten if
    // it fails, so Try again loads again.
    let loaded: PaneTabModule | undefined;
    let loading: Promise<PaneTabModule> | undefined;
    const loadOnce = () =>
        (loading ??= load().then(
            (module) => (loaded = module),
            (e) => {
                loading = undefined;
                throw e;
            }
        ));
    return {
        ...manifest,
        create: (ctx) =>
            loaded ? buildReal(manifest.view, members, loaded, ctx) : createStandIn(manifest, members, loadOnce, ctx),
    };
}

/** The real instance, with drift in its members logged. */
function buildReal(
    view: string,
    members: readonly PaneTabMember[],
    module: PaneTabModule,
    ctx: PaneTabHostContext
): PaneTabInstance {
    const instance = module.create(ctx);
    if (instance == null) throw new Error("create(ctx) returned nothing");
    const mismatch = memberMismatch(members, instance);
    if (mismatch.length > 0) {
        console.error(
            `[pane-tab] ${view}: its lazy manifest's members differ from its instance's (${mismatch.join(", ")}); update \`members\``
        );
    }
    return instance;
}

/** Members the real instance has but `members` doesn't list, and the reverse. */
export function memberMismatch(members: readonly PaneTabMember[], real: PaneTabInstance): string[] {
    const declared = new Set<string>(members);
    const actual = Object.keys(real).filter(
        (k) => k !== "component" && k !== "dispose" && (real as unknown as Record<string, unknown>)[k] !== undefined
    );
    return [
        ...actual.filter((k) => !declared.has(k)).map((k) => `+${k}`),
        ...[...declared].filter((k) => !actual.includes(k)).map((k) => `-${k}`),
    ];
}

function createStandIn(
    manifest: Omit<PaneTabManifest, "create">,
    members: readonly PaneTabMember[],
    load: LazyPaneTab["load"],
    ctx: PaneTabHostContext
): PaneTabInstance {
    const owner = getOwner();
    const [real, setReal] = createSignal<PaneTabInstance>();
    const [failure, setFailure] = createSignal<string>();
    let disposed = false;
    onCleanup(() => {
        disposed = true;
    });
    // The progress slot may be handed over before the real instance exists.
    let progressEl: HTMLDivElement | null = null;

    const start = () => {
        setFailure(undefined);
        load()
            .then((module) => {
                if (disposed) return;
                const instance = runWithOwner(owner, () =>
                    untrack(() => buildReal(manifest.view, members, module, ctx))
                )!;
                setReal(() => instance);
                if (progressEl) instance.progressMount?.(progressEl);
                // The host's `focus()` for a tab that showed in a focused pane
                // reached the stand-in, which had nothing to focus yet. Claimed
                // as a pane that just mounted does: only if it is still the
                // selection, with no modal or menu over it and the caret not
                // already placed by the user (focusManager.claimFocusOnMount).
                if (instance.focus && untrack(() => ctx.visibility?.() === "active")) {
                    queueMicrotask(() => {
                        if (!disposed) focusManager.claimFocusOnMount(ctx.blockId, () => instance.focus!());
                    });
                }
            })
            .catch((e) => {
                if (disposed) return;
                const message = e instanceof Error ? e.message : String(e);
                console.error(`[pane-tab] ${manifest.view} failed to load:`, e);
                setFailure(message);
            });
    };
    start();

    const forward: Required<Omit<PaneTabInstance, "component" | "dispose">> = {
        liveTitle: () => real()?.liveTitle?.() ?? { text: manifest.label, placeholder: true },
        rename: async (name) => real()?.rename?.(name),
        liveFavicon: () => real()?.liveFavicon?.() ?? "",
        headerIcon: () => real()?.headerIcon?.() ?? manifest.icon,
        headerText: () => real()?.headerText?.() ?? "",
        headerActions: () => real()?.headerActions?.() ?? [],
        contextMenu: (c) => real()?.contextMenu?.(c) ?? [],
        settingsMenu: () => real()?.settingsMenu?.() ?? [],
        background: () => real()?.background?.(),
        manageConnection: () => real()?.manageConnection?.() ?? true,
        voice: () => real()?.voice?.(),
        search: () => real()?.search?.(),
        selection: () => real()?.selection?.() ?? window.getSelection()?.toString() ?? "",
        paste: (text) => real()?.paste?.(text),
        progressMount: (el) => {
            progressEl = el;
            real()?.progressMount?.(el);
        },
        focus: () => real()?.focus?.() ?? false,
        onKeyDown: (e) => real()?.onKeyDown?.(e) ?? false,
        onActivate: () => real()?.onActivate?.(),
        onDeactivate: () => real()?.onDeactivate?.(),
        command: (id, source) => real()?.command?.(id, source),
    };

    const standIn: PaneTabInstance = {
        component: (props) => (
            <Show
                when={real()}
                keyed
                fallback={
                    <div class="lazy-pane-standin">
                        <Show when={failure()}>
                            <CenteredDiv>
                                <div role="alert">
                                    {manifest.label} failed to load: {failure()}
                                </div>
                                <Button density="compact" onClick={start}>
                                    Try again
                                </Button>
                            </CenteredDiv>
                        </Show>
                    </div>
                }
            >
                {(instance) => {
                    const Component = instance.component;
                    return <Component ctx={props.ctx} />;
                }}
            </Show>
        ),
        dispose: () => {
            disposed = true;
            real()?.dispose?.();
        },
    };
    for (const member of members) {
        (standIn as unknown as Record<string, unknown>)[member] = forward[member];
    }
    return standIn;
}
