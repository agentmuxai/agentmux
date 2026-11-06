// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * An agent's request to drive a native app, as a card in its own pane, docked above the composer
 * (docs/specs/SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md row 4, D4). Drawn from srv's own record of
 * the request (the agent, the app and its window), never from the agent's words, and operator-only: the
 * dock marks it so an agent's UI query, click and keys never reach it. Closing the pane with it open is a No
 * (the caller's `onGone`). The tone, badges and notification come from srv's attention event for the same
 * pane, so the card adds no wait of its own.
 */

import { createSignal, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import type { PaneRequestProps } from "@/app/store/pane-requests";
import type { DesktopRequest } from "@/types/rpc/DesktopRequest";
import "./desktop-request-card.scss";

export type DesktopChoice = "timed" | "session" | "deny";

export function DesktopRequestCard(
    props: { request: DesktopRequest; decide: (choice: DesktopChoice) => Promise<void> } & PaneRequestProps
): JSX.Element {
    const r = props.request;
    const [error, setError] = createSignal<string | null>(null);
    const answer = async (choice: DesktopChoice) => {
        try {
            await props.decide(choice);
            props.close();
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        }
    };
    return (
        <section class="desktop-request-card" aria-label={`${r.agent} wants to control ${r.app}`}>
            <h3 class="desktop-request-card-title">
                {r.agent} wants to control {r.app}
            </h3>
            <div class="desktop-request-card-body">
                {r.title ? <p class="desktop-request-window">{`Window: ${r.title}`}</p> : null}
                <p class="desktop-request-note">
                    It will see this window and be able to click and type in it. You can stop it at any time with Stop in
                    its pane. Everything it does is logged.
                </p>
                {error() && (
                    <div class="settings-config-error" role="alert">
                        {error()}
                    </div>
                )}
            </div>
            <div class="desktop-request-card-actions">
                <Button onClick={() => void answer("deny")}>Don't allow</Button>
                <Button onClick={() => void answer("session")}>Allow for this session</Button>
                <Button tone="attention" onClick={() => void answer("timed")}>
                    {`Allow for ${r.grant_minutes} min`}
                </Button>
            </div>
        </section>
    );
}
