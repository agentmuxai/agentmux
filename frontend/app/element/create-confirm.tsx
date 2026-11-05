// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An awaitable "are you sure?" that covers only the pane it is drawn in: the replacement for
// `window.confirm`, which freezes the whole renderer. A view makes one with `createConfirm()`, renders
// `<confirm.Dialog />` anywhere in its own tree, and awaits `confirm.ask({...})`.
// docs/specs/SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md row 11.

import { createSignal, Show } from "solid-js";
import type { JSX } from "solid-js";

import { ConfirmModal } from "@/app/element/confirm-modal";

export interface ConfirmQuestion {
    title: string;
    description?: string;
    confirmLabel?: string;
    cancelLabel?: string;
    /** Default true: these ask before something is lost, so focus starts on Cancel. */
    destructive?: boolean;
}

export interface PaneConfirm {
    /** Resolves true on the confirm button, false on Cancel, Escape or a click outside. A question asked while
     *  another is open gets that one's answer, rather than stacking a second dialog. */
    ask: (question: ConfirmQuestion) => Promise<boolean>;
    Dialog: () => JSX.Element;
}

export function createConfirm(): PaneConfirm {
    const [open, setOpen] = createSignal<{ question: ConfirmQuestion; answer: (ok: boolean) => void } | null>(null);
    let pending: Promise<boolean> | null = null;

    const ask = (question: ConfirmQuestion): Promise<boolean> => {
        if (pending) return pending;
        pending = new Promise<boolean>((resolve) => {
            setOpen({
                question,
                answer: (ok) => {
                    setOpen(null);
                    pending = null;
                    resolve(ok);
                },
            });
        });
        return pending;
    };

    // `keyed`: the dialog keeps the question it opened with while it closes, rather than reading a signal
    // that has just been cleared.
    const Dialog = (): JSX.Element => (
        <Show when={open()} keyed>
            {(o) => (
                <ConfirmModal
                    open={true}
                    scope="pane"
                    title={o.question.title}
                    description={o.question.description}
                    confirmLabel={o.question.confirmLabel}
                    cancelLabel={o.question.cancelLabel}
                    destructive={o.question.destructive ?? true}
                    onConfirm={() => o.answer(true)}
                    onCancel={() => o.answer(false)}
                />
            )}
        </Show>
    );

    return { ask, Dialog };
}
