// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An Audit pane's instance, loaded with the first one (audit.tsx): a row of four sections (audit-sections.ts), the
// selected one in block meta under `audit:section` (store/pane-sections.ts). A section's code loads when it is first
// shown and then stays mounted, hidden, so it keeps its scroll and a half-typed filter; what it reads and polls runs only
// while it is the one on screen.

import { createComputed, createEffect, createSignal, on, Show, type Component, type JSX } from "solid-js";
import { Dynamic } from "solid-js/web";
import type { PaneTabHostContext, PaneTabInstance } from "@/app/store/pane-tab-registry";
import { SectionPaneModel, SectionPaneView, sectionPaneSpec } from "@/app/view/section-pane/section-pane";
import type { AuditSectionProps } from "./audit-section-props";
import { AUDIT_SECTIONS, AUDIT_VIEW, type AuditSection } from "./audit-sections";
import { AUDIT_AGENT_META_KEY, AUDIT_MESSAGE_META_KEY, AUDIT_SOUND_META_KEY } from "./audit-shared";
import "./audit-manager.scss";

type Load = () => Promise<{ default: Component<AuditSectionProps> }>;

/** The section `id`, imported when it is first selected (not a `lazy` read: that would hide the whole pane while it loads). */
function sectionOf(ctx: PaneTabHostContext, id: AuditSection, load: Load) {
    return (pane: SectionPaneModel<AuditSection>): JSX.Element => {
        const current = () => pane.sectionAtom() === id;
        const [Section, setSection] = createSignal<Component<AuditSectionProps>>();
        let asked = false;
        createComputed(() => {
            if (!current() || asked) return;
            asked = true;
            void load().then((m) => setSection(() => m.default));
        });
        return (
            <Show when={Section()}>
                {(Comp) => <Dynamic component={Comp()} paneBlockId={ctx.blockId} visible={ctx.visibility() === "active" && current()} meta={ctx.meta()} />}
            </Show>
        );
    };
}

/** The events the pane's meta points at (an agent's audit, a message, a sound), as one string that changes when any does. */
const eventRefs = (meta: Record<string, unknown> | undefined) =>
    [AUDIT_AGENT_META_KEY, AUDIT_SOUND_META_KEY, AUDIT_MESSAGE_META_KEY].map((k) => String(meta?.[k] ?? "")).join("|");

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    const spec = sectionPaneSpec<AuditSection>(AUDIT_VIEW, "Audit section", AUDIT_SECTIONS, {
        events: sectionOf(ctx, "events", () => import("./audit-events-section")),
        conversations: sectionOf(ctx, "conversations", () => import("./conversations/conversations-section")),
        typed: sectionOf(ctx, "typed", () => import("./typed/typed-section")),
        storage: sectionOf(ctx, "storage", () => import("./storage/storage-section")),
    });
    const model = new SectionPaneModel(ctx, spec);
    // A link to an agent's audit, a message or a sound is a link to an event: it shows Events whatever section was open.
    createEffect(
        on(
            () => eventRefs(ctx.meta()),
            (refs, prev) => {
                if (prev !== undefined && refs !== prev && refs !== "||" && model.sectionAtom() !== "events") model.selectSection("events");
            }
        )
    );
    return { component: () => <SectionPaneView model={model} /> };
}
