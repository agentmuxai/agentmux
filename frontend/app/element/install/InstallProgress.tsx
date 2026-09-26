// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * InstallProgress — both layers of an install, with no chrome
 * (SPEC_UNIVERSAL_INSTALL_DIALOG_2026_09_23.md §4, §5.1): Layer 1's step
 * list, and Layer 2's "Details" console, collapsed by default. Used inline
 * (the Toolchain pane, the prereq modal) and inside `<InstallDialog>`.
 *
 * Details remembers its open/closed choice per install kind for the
 * session, opens by itself on failure, and then scrolls to the first
 * error line.
 */

import { createEffect, createSignal, on, Show, type JSX } from "solid-js";

import { ErrorBanner } from "@/app/errors/ErrorBanner";
import { redactSecrets } from "@/app/errors/redact";
import { writeText as clipboardWriteText } from "@/util/clipboard";

import type { InstallSession } from "./install-session";
import { InstallSteps } from "./InstallSteps";
import { LogView, type LogViewApi } from "./LogView";
import "./install-progress-view.scss";

// Details open/closed per install kind, for the session (spec §4.2).
const detailsOpenByKind = new Map<string, boolean>();

interface InstallProgressProps {
    session: InstallSession;
    /** Install kind, e.g. "provider-cli" or "system-tool"; keys the remembered Details choice. */
    kind: string;
}

export const InstallProgress = (props: InstallProgressProps): JSX.Element => {
    const [open, setOpen] = createSignal(detailsOpenByKind.get(props.kind) ?? false);
    let details: HTMLDetailsElement | undefined;
    let log: LogViewApi | undefined;

    // Typed backend errors (e.g. disk full creating the install directory)
    // carry a fuller explanation than the category line.
    const typedError = () => {
        const e = props.session.error();
        return e != null && typeof e === "object" ? e : null;
    };

    // Spec §7: Details opens by itself on failure. A failure doesn't change
    // the remembered preference.
    createEffect(
        on(props.session.state, (state) => {
            if (state === "failed") setOpen(true);
        }),
    );

    // Whenever Details is (or stays) open across a state change, bring the
    // log into view: the first error after a failure, the newest line
    // otherwise. Tracks state too, so a failure while Details is already
    // open still scrolls (codex P2 on #3661).
    createEffect(() => {
        const failed = props.session.state() === "failed";
        if (!open()) return;
        requestAnimationFrame(() => {
            if (!details?.open || !log) return;
            const index = props.session.failure()?.firstErrorLine;
            if (failed && index != null) log.scrollToLine(index - props.session.trimmedLines());
            else log.refresh();
        });
    });

    return (
        <div class="install-view">
            <InstallSteps steps={props.session.steps()} />
            <Show when={typedError()}>
                <ErrorBanner error={typedError()} />
            </Show>
            <details
                class="install-details"
                ref={details}
                open={open()}
                onToggle={(e) => {
                    const isOpen = e.currentTarget.open;
                    if (isOpen === open()) return;
                    setOpen(isOpen);
                    detailsOpenByKind.set(props.kind, isOpen);
                }}
            >
                <summary>Details</summary>
                <div class="install-details-toolbar">
                    <button
                        type="button"
                        class="install-details-copy"
                        disabled={props.session.lines().length === 0}
                        onClick={() =>
                            // SPEC_ERROR_COPY_EVERYWHERE_2026_09_24.md §5 — moved to the redacted path.
                            void clipboardWriteText(redactSecrets(props.session.logText())).catch((err) =>
                                console.log("clipboard write failed", err),
                            )
                        }
                    >
                        Copy all
                    </button>
                </div>
                <LogView
                    lines={props.session.lines}
                    trimmedLines={props.session.trimmedLines}
                    copyAllText={props.session.logText}
                    apiRef={(api) => (log = api)}
                />
            </details>
        </div>
    );
};

/** Test hook: forget remembered Details choices. */
export function resetInstallDetailsPrefs(): void {
    detailsOpenByKind.clear();
}
