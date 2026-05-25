// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Warden widget — shell + section stubs. Phase 1 (this PR) registers the
// view and renders the three-layer scaffold; Host data, LAN data, and
// enforcement actions land in follow-up PRs.
//
// Spec: specs/SPEC_WARDEN_WIDGET_2026-05-25.md

import { For, type JSX } from "solid-js";

import "./warden.scss";

class WardenViewModel implements ViewModel {
    viewType: string;
    blockId: string;

    constructor(blockId: string) {
        this.viewType = "warden";
        this.blockId = blockId;
    }

    get viewComponent(): ViewComponent {
        return WardenView as unknown as ViewComponent;
    }
}

interface LayerSection {
    key: "host" | "lan" | "internet";
    title: string;
    summary: string;
    status: "live" | "stub" | "disabled";
    body: JSX.Element;
}

const SECTIONS: LayerSection[] = [
    {
        key: "host",
        title: "Host",
        summary: "This AgentMux process · jekt tiers 1–2 · < 1 ms",
        status: "stub",
        body: (
            <div class="warden-section-stub">
                Agent list, identity provenance, capability set, jekt/min, and audit
                stream land in the next PR. See spec § Phase 2.
            </div>
        ),
    },
    {
        key: "lan",
        title: "LAN",
        summary: "mDNS-discovered peers · jekt tier 3 · 1–10 ms",
        status: "stub",
        body: (
            <div class="warden-section-stub">
                Peer list reads through to <code>lan_discovery</code>. Enrollment +
                policy push wait on lan-awareness Phase 3 (LAN jekt forwarding).
            </div>
        ),
    },
    {
        key: "internet",
        title: "Internet",
        summary: "AgentBus cloud relay · jekt tier 4 · opt-in",
        status: "disabled",
        body: (
            <div class="warden-section-stub">
                Closed by default. Cross-network governance ships behind
                lan-awareness Phase 4 (cloud fallback).
            </div>
        ),
    },
];

function WardenView({ model: _model }: { model: WardenViewModel }): JSX.Element {
    return (
        <div class="warden-pane">
            <header class="warden-header">
                <span class="warden-title">Warden</span>
                <span class="warden-subtitle">3-layer operator surface</span>
            </header>
            <div class="warden-sections">
                <For each={SECTIONS}>
                    {(section) => (
                        <section
                            class="warden-section"
                            data-status={section.status}
                            data-layer={section.key}
                        >
                            <header class="warden-section-header">
                                <span class="warden-section-title">{section.title}</span>
                                <span class="warden-section-summary">{section.summary}</span>
                                <span class="warden-section-status">{section.status}</span>
                            </header>
                            <div class="warden-section-body">{section.body}</div>
                        </section>
                    )}
                </For>
            </div>
        </div>
    );
}

WardenView.displayName = "WardenView";

export { WardenViewModel };
