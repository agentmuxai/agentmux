// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a `UI` window capture of this window blacks out (docs/specs/SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md §7,
 * G5): the rectangle of each pane showing the Vault, the Audit, Memory or a browser in the person's own profile, and
 * of each panel elsewhere that shows their content (`PRIVATE_PANELS`), and of each tooltip, popover or menu opened
 * from one of them, wherever it is drawn (`overlay-anchors.ts`).
 * Agent panes and terminals stay capturable.
 *
 * srv asks just before the frame (`begin`) and just after (`end`). From `begin` the window measures every frame
 * until `end`, and `end` says whether anything changed, so the capture is refused rather than blacked out from
 * rectangles of another moment. While a pane is being dragged the window refuses.
 */

import { parseIdentity, IDENTITY_META_KEY } from "@/app/util/browser-identity";
import type { CaptureRect } from "@/types/rpc/CaptureRect";
import type { WindowCaptureMask } from "@/types/rpc/WindowCaptureMask";

/**
 * Every panel outside the Vault, Audit, Memory and personal-browser panes that shows their content, by the selector
 * it is drawn with and the file that draws it (the coverage test checks the file still uses it). Its rectangle is
 * blacked out wherever it is drawn: in an agent pane, in Settings, in a popover over a browser pane, in the status bar.
 */
export const PRIVATE_PANELS: { selector: string; file: string; shows: string }[] = [
    { selector: ".vault-request-card", file: "app/view/agent/components/vault-request-card/vault-request-cards.tsx", shows: "a Vault request in an agent pane" },
    { selector: '.pane-request-more-item[data-request-source^="vault"]', file: "app/view/agent/pane-requests/PaneRequestDock.tsx", shows: "a waiting Vault request's label, collapsed under the open one" },
    { selector: ".vault-form", file: "app/view/vault/vault-entry-form.tsx", shows: "the Vault's entry form, where a value is entered or shown, wherever it opens" },
    { selector: ".browser-signin-list", file: "app/view/browser/browser-signin-list.tsx", shows: "the saved sign-ins under a login field" },
    { selector: ".status-last-sound", file: "app/statusbar/LastSoundStatus.tsx", shows: "the last sound's words, from the Audit" },
    { selector: ".setting-vault-key", file: "app/view/settings/vault-key-field.tsx", shows: "a Settings key's choice of Vault entries" },
    { selector: ".job-vault-entry", file: "app/view/warden-jobs/job-params-form.tsx", shows: "a job's choice of Vault entry" },
    { selector: ".job-vault-secrets", file: "app/view/warden-jobs/job-dry-run.tsx", shows: "the Vault entries a job's dry run reads" },
];

/** Why the window can't give its rectangles while `active` (the focused element) is a list inside a private panel:
 *  an open list of a native select is drawn by the system, outside the page. */
export function openListRefusal(active: Element | null): string | undefined {
    return active instanceof HTMLSelectElement && active.closest(PRIVATE_SELECTOR) ? "a list of Vault entries may be open" : undefined;
}

export const PRIVATE_SELECTOR = PRIVATE_PANELS.map((p) => p.selector).join(", ");

/** The views whose whole pane is blacked out. */
const BLACKED_OUT_VIEWS = new Set(["vault", "audit", "memory"]);

export function blackedOutPane(meta: Record<string, unknown> | undefined): boolean {
    const view = String(meta?.view ?? "");
    if (BLACKED_OUT_VIEWS.has(view)) return true;
    return view === "browser" && parseIdentity(meta?.[IDENTITY_META_KEY]).kind === "personal";
}

/** What `measure` reads of the window, so it can be tested without one. */
export interface MaskSource {
    width: number;
    height: number;
    scale: number;
    /** Each pane: its block id and its rectangle. */
    panes(): { blockId: string; rect: CaptureRect }[];
    /** Each panel of `PRIVATE_PANELS` that is drawn. */
    panels(): CaptureRect[];
    /** Each floating overlay (a tooltip, a popover, a menu): its rectangle, and where it was opened from (the pane,
     *  and whether inside a panel of `PRIVATE_PANELS`). */
    overlays(): { rect: CaptureRect; blockId: string | null; inPanel: boolean }[];
    meta(blockId: string): Record<string, unknown> | undefined;
}

export interface Measured {
    width: number;
    height: number;
    scale: number;
    rects: CaptureRect[];
}

function clip(r: CaptureRect, width: number, height: number): CaptureRect | null {
    const x0 = Math.max(0, r.x);
    const y0 = Math.max(0, r.y);
    const x1 = Math.min(width, r.x + r.w);
    const y1 = Math.min(height, r.y + r.h);
    return x1 > x0 && y1 > y0 ? { x: x0, y: y0, w: x1 - x0, h: y1 - y0 } : null;
}

export function measure(src: MaskSource): Measured {
    const privateOverlays = src
        .overlays()
        .filter((o) => o.inPanel || (o.blockId != null && blackedOutPane(src.meta(o.blockId))))
        .map((o) => o.rect);
    const raw = [...src.panes().filter((p) => blackedOutPane(src.meta(p.blockId))).map((p) => p.rect), ...src.panels(), ...privateOverlays];
    const rects = raw
        .map((r) => clip(r, src.width, src.height))
        .filter((r): r is CaptureRect => r != null)
        .sort((a, b) => a.y - b.y || a.x - b.x || a.w - b.w || a.h - b.h);
    return { width: src.width, height: src.height, scale: src.scale, rects };
}

export function sameMeasure(a: Measured, b: Measured): boolean {
    return JSON.stringify(a) === JSON.stringify(b);
}

/** One capture's watch: from `begin` it measures each frame until `end`, and remembers whether anything changed and
 *  the first reason, at any frame, that the window could not give its rectangles. */
export class CaptureWatch {
    private first: Measured;
    private changed = false;
    private refusedWhy: string | undefined;
    private done = false;

    constructor(
        private readonly read: () => Measured,
        private readonly nextFrame: (cb: () => void) => void,
        private readonly giveUpAt: number,
        private readonly now: () => number,
        private readonly refusal: () => string | undefined = () => undefined
    ) {
        this.first = read();
        this.tick();
    }

    private tick(): void {
        this.nextFrame(() => {
            if (this.done || this.now() > this.giveUpAt) return;
            if (!sameMeasure(this.first, this.read())) this.changed = true;
            this.refusedWhy ??= this.refusal();
            this.tick();
        });
    }

    get begin(): Measured {
        return this.first;
    }

    /** Stop watching: what the window shows now, and whether it changed at any frame since `begin`. */
    end(): { measured: Measured; changed: boolean; refused?: string } {
        this.done = true;
        const last = this.read();
        return { measured: last, changed: this.changed || !sameMeasure(this.first, last), refused: this.refusedWhy };
    }
}

export function toMask(windowId: string, title: string, m: Measured, extra: { moving?: boolean; changed?: boolean; refuse?: string }): WindowCaptureMask {
    return { window_id: windowId, title, width: m.width, height: m.height, scale: m.scale, rects: m.rects, moving: extra.moving ?? false, changed: extra.changed ?? false, ...(extra.refuse ? { refuse: extra.refuse } : {}) };
}
