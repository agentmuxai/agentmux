// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { createSignal, JSX, onCleanup } from "solid-js";
import "./copybutton.scss";
import { IconButton } from "./iconbutton";

type CopyButtonProps = {
    title: string;
    className?: string;
    onClick: (e: MouseEvent) => void | Promise<void>;
};

const CopyButton = ({ title, className, onClick }: CopyButtonProps): JSX.Element => {
    const [isCopied, setIsCopied] = createSignal(false);
    const [isError, setIsError] = createSignal(false);
    const [isPending, setIsPending] = createSignal(false);
    let timeoutRef: ReturnType<typeof setTimeout> | null = null;

    const handleOnClick = async (e: MouseEvent) => {
        if (isCopied() || isPending()) {
            return;
        }
        if (timeoutRef) {
            clearTimeout(timeoutRef);
            timeoutRef = null;
        }
        // Locks out re-entrant clicks for the duration of the (async) write —
        // without this, rapid clicks fire concurrent writes whose completion
        // order isn't guaranteed, letting an older attempt clobber a newer
        // one's success/error state.
        setIsPending(true);

        try {
            await onClick?.(e);
            setIsError(false);
            setIsCopied(true);
        } catch (err) {
            console.error("copy failed:", err);
            setIsCopied(false);
            setIsError(true);
        } finally {
            setIsPending(false);
        }

        timeoutRef = setTimeout(() => {
            setIsCopied(false);
            setIsError(false);
            timeoutRef = null;
        }, 2000);
    };

    onCleanup(() => {
        if (timeoutRef) {
            clearTimeout(timeoutRef);
        }
    });

    return (
        <IconButton
            decl={{
                elemtype: "iconbutton",
                icon: isCopied() ? "check" : isError() ? "triangle-exclamation" : "copy",
                title: isError() ? "Copy failed — see console" : title,
                className: clsx("copy-button", { copied: isCopied(), error: isError() }),
                disabled: isPending(),
                click: handleOnClick,
            }}
            className={className}
        />
    );
};

export { CopyButton };
