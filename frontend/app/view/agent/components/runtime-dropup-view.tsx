// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What AgentRuntimeDropup draws: the trigger button, the drift status line and the panel's rows. */

import { AnchoredPopover } from "@/app/element/anchored-popover";
import { tip } from "@/app/element/tip";
import { Button, CloseButton } from "@/app/element/ui";
import clsx from "clsx";
import { createEffect, For, Show, type JSX } from "solid-js";
import type { DropupAgreement } from "./runtime-dropup-agreement";
import { PERMISSION_COLORS, type Row } from "./runtime-dropup-options";
import type { DropupActions } from "./runtime-dropup-rows";
import type { DropupState } from "./runtime-dropup-state";

interface DropupViewProps {
    s: DropupState;
    a: DropupAgreement;
    act: DropupActions;
}

function triggerTitle(s: DropupState, a: DropupAgreement): string {
    const kind = s.shownAgreement().kind;
    return s.applyError() != null
        ? `Couldn't apply that change — the agent may still be running the previous settings. ${s.applyError()}`
        : kind === "differs"
          ? `The agent is not running what is selected. ${a.driftSummary()}. Open this menu to restart it with your selection.`
          : kind === "pending"
            ? `Applies after the current turn. ${a.driftSummary()}.`
            : "Mode / Model / Effort — applies on the next turn";
}

function triggerAriaLabel(s: DropupState): string {
    const kind = s.shownAgreement().kind;
    return `Runtime settings: ${s.compactSummary()}${s.applyError() != null ? " — last change failed to apply" : ""}${
        kind === "differs"
            ? " — the agent is not running this selection"
            : kind === "pending"
              ? " — applies after the current turn"
              : ""
    }`;
}

function DropupTrigger(props: DropupViewProps): JSX.Element {
    const s = props.s;
    return (
        <Button
            density="compact"
            ref={(el: HTMLButtonElement) => (s.referenceEl = el)}
            class={clsx("agent-runtime-dropup-trigger", {
                "agent-runtime-dropup-trigger--error": s.applyError() != null,
                "agent-runtime-dropup-trigger--differs": s.shownAgreement().kind === "differs",
                "agent-runtime-dropup-trigger--pending": s.shownAgreement().kind === "pending",
            })}
            style={{
                "border-left": `3px solid ${PERMISSION_COLORS[s.shown().permissionMode] ?? PERMISSION_COLORS.default}`,
                ...(s.heldWidth() != null ? { width: `${s.heldWidth()}px`, "flex-shrink": 0 } : {}),
            }}
            title={triggerTitle(s, props.a)}
            aria-haspopup="listbox"
            aria-expanded={s.open()}
            aria-label={triggerAriaLabel(s)}
            onClick={() => props.act.toggleOpen()}
        >
            <span class="agent-runtime-dropup-trigger-label">{s.compactSummary()}</span>
            <Show when={s.applyError() != null || s.shownAgreement().kind === "differs"}>
                <span class="agent-runtime-dropup-trigger-warn" aria-hidden="true">
                    {" ⚠"}
                </span>
            </Show>
            <Show when={s.applyError() == null && s.shownAgreement().kind === "pending"}>
                <span class="agent-runtime-dropup-trigger-warn" aria-hidden="true">
                    {" …"}
                </span>
            </Show>
        </Button>
    );
}

/** Always there, two lines high, whatever the agent is doing: a notice that came and went would resize the panel. */
function DropupStatus(props: DropupViewProps): JSX.Element {
    const { s, a } = props;
    return (
        <div
            class="agent-runtime-dropup-drift"
            role="status"
            data-kind={a.driftOf().length > 0 ? s.shownAgreement().kind : "agrees"}
        >
            <div class="agent-runtime-dropup-drift-text">
                <div class="agent-runtime-dropup-drift-line">{a.statusHeadline()}</div>
                <div class="agent-runtime-dropup-drift-line" {...tip(a.driftSummary() || undefined)}>
                    {a.driftSummary()}
                </div>
            </div>
            {/* Keeps its place when not needed, so the text beside it wraps the same. */}
            <Button
                density="compact"
                class={clsx("agent-runtime-dropup-drift-btn", !a.differs() && "is-hidden")}
                aria-hidden={a.differs() ? undefined : "true"}
                tabIndex={a.differs() ? undefined : -1}
                disabled={!a.differs()}
                onClick={a.restartToApply}
            >
                Restart to apply
            </Button>
        </div>
    );
}

function OptionRowView(props: DropupViewProps & { row: Extract<Row, { kind: "option" }> }): JSX.Element {
    const { s, act, row } = props;
    const optIndex = () =>
        act.build().options.findIndex((o) => o.section === row.section && o.value === row.value);
    let el: HTMLDivElement | undefined;
    // The panel scrolls when its rows outnumber the room: keep the row the keys are on in view.
    createEffect(() => {
        if (!row.disabled && optIndex() === s.selectedOptIndex()) el?.scrollIntoView?.({ block: "nearest" });
    });
    return (
        <div
            ref={el}
            class="menu-item agent-runtime-dropup-row"
            classList={{
                active: !row.disabled && optIndex() === s.selectedOptIndex(),
                "is-disabled": !!row.disabled,
            }}
            role="option"
            aria-selected={row.current}
            aria-disabled={row.disabled ? "true" : undefined}
            {...tip(row.detail)}
            onMouseEnter={() => {
                if (!row.disabled) s.setSelectedOptIndex(optIndex());
            }}
            // These rows are plain non-focusable divs — without this, a
            // mousedown here blurs the trigger and shifts
            // document.activeElement to <body> (outside the panel), which
            // handleFocusChange reads as "focus left" and closes on. That
            // made every selection close the panel despite §9.2's
            // stays-open decision already being implemented correctly —
            // this was a second, independent close path.
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => {
                if (row.disabled) return;
                const idx = optIndex();
                s.setSelectedOptIndex(idx);
                void act.applySelection(idx);
            }}
        >
            <i
                class={`fa-solid fa-fw menu-item-icon menu-item-check${row.current ? " fa-check" : ""}`}
                style={row.color ? { color: row.color } : undefined}
            />
            {/* Label, then its description on a line of its own, so the
                panel can cap its width and wrap instead of growing past the
                pane. */}
            <span class="agent-runtime-dropup-text">
                <span class="label">{row.label}</span>
                <Show when={row.description}>
                    <span class="agent-runtime-dropup-description">{row.description}</span>
                </Show>
            </span>
        </div>
    );
}

function DropupRow(props: DropupViewProps & { row: Row }): JSX.Element {
    const row = props.row;
    if (row.kind === "header") {
        return (
            <div class="agent-runtime-dropup-section">
                {row.section}
                <Show when={row.note}>
                    <span class="agent-runtime-dropup-section-note" {...tip(row.note)}>
                        {row.note}
                    </span>
                </Show>
            </div>
        );
    }
    if (row.kind === "group") {
        return (
            <div class="agent-runtime-dropup-group">
                {row.text}
                <Show when={row.note}>
                    <span class="agent-runtime-dropup-section-note">{row.note}</span>
                </Show>
            </div>
        );
    }
    if (row.kind === "note") {
        return (
            <div
                class="agent-runtime-dropup-note"
                classList={{ "is-placeholder": !!row.placeholder }}
                {...tip(row.placeholder ? undefined : row.text)}
            >
                {row.text}
            </div>
        );
    }
    return <OptionRowView s={props.s} a={props.a} act={props.act} row={row} />;
}

function DropupPanel(props: DropupViewProps): JSX.Element {
    const s = props.s;
    return (
        <AnchoredPopover
            anchor={s.referenceEl}
            placement="top-start"
            fitHeight
            shellClass="anchored-popover--menu"
            class="menu agent-runtime-dropup-panel"
            ref={(el) => {
                s.floatingEl = el;
            }}
        >
            {/* Sibling of the listbox below, not a child of it — a
                role="listbox" should only contain role="option"
                rows (plus an optional label); an interactive
                button dropped inside it would be an invalid
                listbox structure for assistive tech. Placed first
                (matching Modal's showCloseButton convention) so
                it's the first Tab stop after the trigger. */}
            <CloseButton
                size="sm"
                tooltip={false}
                class="agent-runtime-dropup-close-btn"
                onClick={() => s.setOpen(false)}
            />
            <DropupStatus {...props} />
            <div role="listbox" aria-label="Runtime settings">
                <For each={props.act.build().rows}>{(row) => <DropupRow {...props} row={row} />}</For>
            </div>
        </AnchoredPopover>
    );
}

/** The trigger, and the panel while it is open. */
export function DropupView(props: DropupViewProps): JSX.Element {
    return (
        <>
            <DropupTrigger {...props} />
            <Show when={props.s.open()}>
                <DropupPanel {...props} />
            </Show>
        </>
    );
}
