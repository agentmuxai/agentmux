// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * An agent's request to drive a native app, as a card in its own pane, docked above the composer
 * (docs/specs/SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md row 4, D4). Drawn from srv's own record of the
 * request, never from anything the agent prints, and operator-only: the dock marks it so an agent's UI query,
 * click and keys never reach it. It looks stronger than a routine card (a warning frame and a line saying
 * what it can do), because it controls a native app. The tone, badges and notification come from srv's
 * attention event for the same pane, so the card adds no wait of its own.
 */

import { createSignal, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import type { PaneRequestProps } from "@/app/store/pane-requests";
import type { DesktopRequest } from "@/types/rpc/DesktopRequest";
import type { DesktopChoice } from "./desktop-requests";
import "./desktop-request-card.scss";

export function DesktopRequestCard(props: { request: DesktopRequest; decide: (choice: DesktopChoice) => Promise<void> } & PaneRequestProps): JSX.Element {
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
        <section class="desktop-request-card" role="alertdialog" aria-label={`${r.agent} wants to control ${r.app}`}>
            <h3 class="desktop-request-card-title">
                <i class="fa-solid fa-computer-mouse" aria-hidden="true" /> {r.agent} wants to control {r.app}
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
                <span class="desktop-request-card-yes">
                    <Button onClick={() => void answer("session")}>Allow for this session</Button>
                    <Button tone="attention" onClick={() => void answer("timed")}>
                        {`Allow for ${r.grant_minutes} min`}
                    </Button>
                </span>
            </div>
        </section>
    );
}
