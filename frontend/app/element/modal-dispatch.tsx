// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Modal dispatch helpers — label/render lookup by request kind.
// The panels themselves are registered from app/modals/modal-layer-panels.tsx, which app-init.ts
// loads at startup; this file names no view, so the element layer stays below the views.
// To add a new modal kind: add a variant to ModalLayerRequest in modal-layer.ts, register it in
// modal-layer-panels.tsx, and implement the panel component. ModalLayer.tsx never needs to change.

import type { JSX } from "solid-js";
import type { ModalLayerApi, ModalLayerRequest } from "./modal-layer";

type Kind = ModalLayerRequest["kind"];
type RequestOf<K extends Kind> = Extract<ModalLayerRequest, { kind: K }>;

export interface ModalKind<K extends Kind> {
    label: (req: RequestOf<K>) => string;
    panel: (req: RequestOf<K>, api: ModalLayerApi, setSubmitting: (v: boolean) => void) => JSX.Element;
}

const kinds = new Map<Kind, ModalKind<any>>();

/** Register how a request kind is labelled and rendered. */
export function registerModalKind<K extends Kind>(kind: K, entry: ModalKind<K>): void {
    kinds.set(kind, entry);
}

export function requestLabel(req: ModalLayerRequest): string {
    return kinds.get(req.kind)?.label(req) ?? req.kind;
}

export function renderRequest(
    req: ModalLayerRequest,
    api: ModalLayerApi,
    setSubmitting: (v: boolean) => void,
): { label: string; panel: JSX.Element } {
    const entry = kinds.get(req.kind);
    if (!entry) console.error(`[modal-dispatch] no panel registered for modal kind "${req.kind}"`);
    return { label: requestLabel(req), panel: entry?.panel(req, api, setSubmitting) ?? null };
}
