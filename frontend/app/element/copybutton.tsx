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
    let timeoutRef: ReturnType<typeof setTimeout> | null = null;

    const handleOnClick = async (e: MouseEvent) => {
        if (isCopied()) {
            return;
        }
        if (timeoutRef) {
            clearTimeout(timeoutRef);
            timeoutRef = null;
        }

        try {
            await onClick?.(e);
            setIsError(false);
            setIsCopied(true);
        } catch (err) {
            console.error("copy failed:", err);
            setIsCopied(false);
            setIsError(true);
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
                click: handleOnClick,
            }}
            className={className}
        />
    );
};

export { CopyButton };
