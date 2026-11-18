// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * Zoom parts: a pane whose parts zoom apart (the Editor's file tree and its document, Files' Places and
 * its list). One mechanism for every such pane: the pane declares its parts in its `paneZoom`
 * capability (`ZoomPartSpec`: id, meta key, label) and marks each part's element `data-zoom-part`
 * (`zoomPartAttrs`); this module and the zoom store do the rest: which part a wheel or a key zooms, the
 * held level, the anchor, the indicator's name, all-panes zoom and the first-start seed.
 *
 * A *subject* names what a level belongs to: the block id for the pane's main part (`term:zoom`, as
 * every pane has always had), `<blockId>#<partId>` for another part. Everything the store keys by
 * block id (held level, anchor, step ratio, the raised PDF maximum) is keyed by subject, so a part
 * never reads or overwrites another's.
 * docs/specs/SPEC_EDITOR_TREE_AND_EDITOR_SEPARATE_ZOOM_2026_11_16.md.
 */

import { getBlockComponentModel } from "./block-component-registry";
import { paneTabCapability, type PaneZoomCapability, type ZoomPartSpec } from "./pane-tab-registry";
import { PANE_ZOOM_KEY, readZoom } from "./zoom-factor";

export const ZOOM_PART_ATTR = "data-zoom-part";
/** The part that keeps the pane's own `term:zoom`; every element outside a declared part belongs to it. */
export const MAIN_PART = "main";
const SEP = "#";

/** The attribute that marks an element as a part of its pane, for JSX spread: `<div {...zoomPartAttrs("tree")}>`. */
export const zoomPartAttrs = (partId: string): Record<string, string> => ({ [ZOOM_PART_ATTR]: partId });

export const subjectId = (blockId: string, partId?: string | null): string =>
    partId && partId !== MAIN_PART ? `${blockId}${SEP}${partId}` : blockId;

export function splitSubject(subject: string): { blockId: string; partId: string | undefined } {
    const at = subject.indexOf(SEP);
    return at < 0 ? { blockId: subject, partId: undefined } : { blockId: subject.slice(0, at), partId: subject.slice(at + 1) };
}

function capabilityOf(blockId: string): PaneZoomCapability | undefined {
    return paneTabCapability(getBlockComponentModel(blockId)?.viewModel?.viewType, "paneZoom");
}

/** The parts a pane declares besides its main one; empty for a pane with none (every pane today but two). */
export function zoomPartsOf(blockId: string): readonly ZoomPartSpec[] {
    return capabilityOf(blockId)?.parts ?? [];
}

export function zoomPartSpec(subject: string): ZoomPartSpec | undefined {
    const { blockId, partId } = splitSubject(subject);
    return partId ? zoomPartsOf(blockId).find((p) => p.id === partId) : undefined;
}

/** The meta key a subject's level is stored under. */
export const zoomMetaKey = (subject: string): string => zoomPartSpec(subject)?.metaKey ?? PANE_ZOOM_KEY;

/** Every level of a pane: its main part, then its declared parts. */
export const zoomSubjectsOf = (blockId: string): string[] => [blockId, ...zoomPartsOf(blockId).map((p) => subjectId(blockId, p.id))];

/** The i18n key of the name a subject's amount shows, or null when it is the pane's only part on screen. */
export function zoomLabelKey(subject: string): string | null {
    const { blockId, partId } = splitSubject(subject);
    const cap = capabilityOf(blockId);
    if (!cap?.parts?.length || !cap.parts.some((p) => partIsShown(blockId, p.id))) return null;
    return (partId ? zoomPartSpec(subject)?.labelKey : cap.mainLabelKey) ?? null;
}

/** `CSS.escape`, which a test DOM may not have: ids are made of letters, digits and dashes, so quotes and backslashes are all that need it. */
const cssString = (v: string): string => (globalThis.CSS?.escape ? CSS.escape(v) : v.replace(/["\\]/g, "\\$&"));

const partSelector = (blockId: string, partId: string): string =>
    `[data-blockid="${cssString(blockId)}"] [${ZOOM_PART_ATTR}="${cssString(partId)}"]`;

/** The element of a pane's part (the first one drawn), or null when it is not on screen (a hidden tree). */
export function zoomPartElement(blockId: string, partId: string): Element | null {
    return document.querySelector(partSelector(blockId, partId));
}
const partIsShown = (blockId: string, partId: string): boolean => zoomPartElement(blockId, partId) != null;

/** Is the keyboard inside this pane's part? (A pane that takes focus when it is clicked leaves it there.) */
export function focusIsInPart(blockId: string, partId: string): boolean {
    const active = document.activeElement;
    return !!active && !!zoomPartElement(blockId, partId)?.contains(active);
}

/**
 * Which level a pointer over `target` zooms: the declared part whose element holds it, else the pane's
 * main level. A wheel over the pane header or a dialog outside every part is the main part's.
 */
export function zoomSubjectAt(target: EventTarget | null, blockId: string): string {
    if (zoomPartsOf(blockId).length === 0) return blockId;
    // The caller found `blockId` from this target (or is the pane the event is in), so a part found on
    // the way up is this pane's.
    const partId = target instanceof Element ? target.closest(`[${ZOOM_PART_ATTR}]`)?.getAttribute(ZOOM_PART_ATTR) : null;
    if (!partId) return blockId;
    return zoomPartsOf(blockId).some((p) => p.id === partId) ? subjectId(blockId, partId) : blockId;
}

// ── The part that has the keyboard ──────────────────────────────────

// The last part each pane's focus or pointer was in. Kept when focus moves out of the pane (to its
// header, a modal, the palette, a native menu), so a zoom key there still lands on the part the person
// was using. A pane that never had either is its main part.
const lastPart = new Map<string, string>();

function noteLastPart(e: Event): void {
    const el = e.target instanceof Element ? e.target.closest(`[${ZOOM_PART_ATTR}]`) : null;
    const partId = el?.getAttribute(ZOOM_PART_ATTR);
    const blockId = el?.closest("[data-blockid]")?.getAttribute("data-blockid");
    if (partId && blockId) lastPart.set(blockId, partId);
}
if (typeof window !== "undefined") {
    window.addEventListener("focusin", noteLastPart, { capture: true, passive: true });
    window.addEventListener("pointerdown", noteLastPart, { capture: true, passive: true });
}

/** Which level the zoom keys and commands change for `blockId`: the part that last had the keyboard, while it is on screen. */
export function zoomSubjectForKeys(blockId: string): string {
    const partId = lastPart.get(blockId);
    if (!partId || partId === MAIN_PART || !zoomPartsOf(blockId).some((p) => p.id === partId)) return blockId;
    return partIsShown(blockId, partId) ? subjectId(blockId, partId) : blockId;
}

/** For tests. */
export function resetZoomPartFocusForTests(): void {
    lastPart.clear();
}

// ── The first start ─────────────────────────────────────────────────

/**
 * The meta to write once so a pane that gains parts keeps looking as it did: each part with no level
 * yet takes the pane's `term:zoom`, or 100%. Writing 100% too is deliberate: a part left unset would,
 * once the main part is zoomed later, be seeded from that later level after a reload. Empty when every
 * part has a level; a pane that has the keys never reseeds.
 */
export function zoomPartsSeed(parts: readonly ZoomPartSpec[], meta: object | null | undefined): Record<string, number> {
    const seed: Record<string, number> = {};
    const m = meta as Record<string, unknown> | null | undefined;
    const level = readZoom(m);
    for (const part of parts) if (typeof m?.[part.metaKey] !== "number") seed[part.metaKey] = level;
    return seed;
}
