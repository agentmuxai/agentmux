// Copyright 2023, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { useLongClick } from "@/app/hook/useLongClick";
import { makeIconClass } from "@/util/util";
import clsx from "clsx";
import { createMemo, JSX } from "solid-js";
import "./iconbutton.scss";

type IconButtonProps = { decl: IconButtonDecl; className?: string };

export function IconButton({ decl, className }: IconButtonProps): JSX.Element {
    let btnRef!: HTMLButtonElement;
    const spin = decl.iconSpin ?? false;
    useLongClick(
        () => btnRef,
        decl.click,
        decl.longClick,
        decl.disabled
    );
    const disabled = decl.disabled ?? false;
    return (
        <button
            ref={btnRef}
            class={clsx("wave-iconbutton", className, decl.className, {
                disabled,
                "no-action": decl.noAction,
            })}
            title={decl.title}
            aria-label={decl.title}
            style={{ color: decl.iconColor ?? "inherit" }}
            disabled={disabled}
        >
            {typeof decl.icon === "string" ? <i class={makeIconClass(decl.icon, true, { spin })} /> : decl.icon}
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
