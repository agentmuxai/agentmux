// Copyright 2023-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { useLongClick } from "@/app/hook/useLongClick";
import { makeIconClass } from "@/util/util";
import clsx from "clsx";
import { createMemo, JSX } from "solid-js";
import "./iconbutton.scss";

type IconButtonProps = { decl: IconButtonDecl; className?: string };

export function IconButton(props: IconButtonProps): JSX.Element {
    let btnRef!: HTMLButtonElement;
    useLongClick(
        () => btnRef,
        (e) => props.decl.click?.(e),
        // Read longClick once at mount to preserve the null-guard inside
        // useLongClick. A lambda wrapper would always be truthy, arming
        // the 300ms timer on every mousedown and swallowing clicks on
        // buttons that have no long-click handler.
        props.decl.longClick ? (e) => props.decl.longClick!(e) : undefined,
        props.decl.disabled ?? false
    );
    return (
        <button
            ref={btnRef}
            class={clsx("wave-iconbutton", props.className, props.decl.className, {
                disabled: props.decl.disabled ?? false,
                "no-action": props.decl.noAction,
            })}
            title={props.decl.title}
            aria-label={props.decl.title}
            style={{ color: props.decl.iconColor ?? "inherit" }}
            disabled={props.decl.disabled ?? false}
        >
            {typeof props.decl.icon === "string"
                ? <i class={makeIconClass(props.decl.icon, true, { spin: props.decl.iconSpin ?? false })} />
                : props.decl.icon}
        </button>
    );
}

type ToggleIconButtonProps = { decl: ToggleIconButtonDecl; className?: string };

export function ToggleIconButton({ decl, className }: ToggleIconButtonProps): JSX.Element {
    let btnRef!: HTMLButtonElement;
    const spin = decl.iconSpin ?? false;
    const active = createMemo(() => decl.active?.() ?? false);
    const title = createMemo(() => `${decl.title}${active() ? " (Active)" : ""}`);
    const disabled = decl.disabled ?? false;
    return (
        <button
            ref={btnRef}
            class={clsx("wave-iconbutton", "toggle", className, decl.className, {
                "no-action": decl.noAction,
            })}
            classList={{ active: active(), disabled }}
            title={title()}
            aria-label={title()}
            style={{ color: decl.iconColor ?? "inherit" }}
            onClick={() => decl.active?._set(!active())}
            disabled={disabled}
        >
            {typeof decl.icon === "string" ? <i class={makeIconClass(decl.icon, true, { spin })} /> : decl.icon}
        </button>
    );
}
