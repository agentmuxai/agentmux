// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SessionDigestBanner — the AI-generated session summary rendered as a Pane
 * Accessory row (`<PaneRow>`), in the `top-fixed` region.
 *
 * The digest is derived from a single source of truth in `useSessionDigest`
 * (block meta + transient signals → `computeDigestAccessory`); this component
 * is presentational only — it maps the resolved `DigestAccessory` onto the
 * shared row chrome. A ≤10-word summary is the row title (no markdown body);
 * the status accent surfaces fresh / stale / generating / failed, so a drifted
 * summary visibly invites a refresh (↻).
 *
 * Spec: docs/specs/SPEC_SESSION_DIGEST_AS_PANE_ACCESSORY_2026_06_15.md.
 */

import { Show, type Accessor, type JSX } from "solid-js";
import { PaneRow, type PaneRowAccent } from "./PaneRow";
import type { DigestAccessory, DigestStatus } from "../digest/digest-accessory";

interface SessionDigestBannerProps {
    accessory: Accessor<DigestAccessory | null>;
    onDismiss: () => void;
    onRegenerate: () => void;
}

/** Map digest lifecycle → the shared PaneRow status accent. */
function accentFor(status: DigestStatus): PaneRowAccent {
    switch (status) {
        case "generating": return "running";
        case "stale": return "idle"; // amber — drifted from the conversation
        case "failed": return "error";
        case "fresh": return "neutral";
    }
}

function formatAge(ms: number): string {
    const diff = Date.now() - ms;
    const hours = Math.floor(diff / 3600000);
    if (hours < 1) return "just now";
    if (hours < 24) return `${hours}h ago`;
    return `${Math.floor(hours / 24)}d ago`;
}

/** Right-of-title meta: age, plus a "+N new" hint when the digest is stale. */
function metaFor(row: DigestAccessory): string | undefined {
    const parts: string[] = [];
    if (row.generatedAt) parts.push(formatAge(row.generatedAt));
    if (row.stale && row.linesSinceDigest > 0) parts.push(`+${row.linesSinceDigest} new`);
    return parts.length ? parts.join(" · ") : undefined;
}

export const SessionDigestBanner = (props: SessionDigestBannerProps): JSX.Element => {
    return (
        <Show when={props.accessory()}>
            {(row) => (
                <div class="agent-session-digest">
                    <PaneRow
                        sigil={row().status === "generating" ? "↻" : "✦"}
                        title={row().title}
                        meta={metaFor(row())}
                        accent={accentFor(row().status)}
                        actions={[
                            { glyph: "↻", title: "Regenerate digest", onClick: () => props.onRegenerate() },
                            { glyph: "×", title: "Dismiss", onClick: () => props.onDismiss() },
                        ]}
                    />
                </div>
            )}
        </Show>
    );
};

SessionDigestBanner.displayName = "SessionDigestBanner";
