// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { createMemo, type JSX } from "solid-js";
import { makeIconClass } from "@/util/util";
import { Button } from "./Button";

export interface DeclIconButtonProps {
    decl: IconButtonDecl | ToggleIconButtonDecl;
    className?: string;
}

/**
 * A view model's icon button (an `IconButtonDecl`, or a `ToggleIconButtonDecl` for an
 * on/off one) as the kit's `chrome` button: the pane header's icons and the search bar's.
 * A toggle shows as `aria-pressed` and flips its `active` signal on click. The click is
 * a native listener on the button, as before, so a header's own listeners see it in the
 * same order.
 */
export function DeclIconButton(props: DeclIconButtonProps): JSX.Element {
    const toggle = () => (props.decl.elemtype === "toggleiconbutton" ? props.decl : undefined);
    const active = createMemo(() => toggle()?.active() ?? false);
    const title = createMemo(() => `${props.decl.title ?? ""}${toggle() && active() ? " (Active)" : ""}` || undefined);
    const onClick = (e: MouseEvent) => {
        if (props.decl.disabled) return;
        const t = toggle();
        if (t) t.active._set(!active());
        else if (props.decl.elemtype === "iconbutton") props.decl.click?.(e);
    };
    return (
        <Button
            variant="chrome"
            class={clsx(props.className, props.decl.className)}
            title={title()}
            aria-label={title()}
            // An inline colour beats any hover rule on the button, as it always has; a toggle sets
            // none unless asked, so the pressed colour in ui.scss applies.
            style={{
                color: props.decl.iconColor ?? (toggle() ? undefined : "inherit"),
                // Inline, as the header's own width rule for its buttons outranks any class.
                ...(props.decl.fitContent ? { width: "auto", "min-width": "26px" } : {}),
            }}
            disabled={props.decl.disabled ?? false}
            pressed={toggle() ? active() : undefined}
            on:click={onClick}
        >
            {typeof props.decl.icon === "string" ? (
                <i class={makeIconClass(props.decl.icon, true, { spin: props.decl.iconSpin ?? false })} />
            ) : (
                props.decl.icon
            )}
        </Button>
    );
}
