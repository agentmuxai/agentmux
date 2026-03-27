// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { createSignal, JSX, onCleanup } from "solid-js";
import "./copybutton.scss";
import { IconButton } from "./iconbutton";

type CopyButtonProps = {
    title: string;
    className?: string;
    onClick: (e: MouseEvent) => void;
};

const CopyButton = ({ title, className, onClick }: CopyButtonProps): JSX.Element => {
    const [isCopied, setIsCopied] = createSignal(false);
    let timeoutRef: ReturnType<typeof setTimeout> | null = null;

    const handleOnClick = (e: MouseEvent) => {
        if (isCopied()) {
            return;
        }
        setIsCopied(true);
        if (timeoutRef) {
            clearTimeout(timeoutRef);
        }
        timeoutRef = setTimeout(() => {
            setIsCopied(false);
            timeoutRef = null;
        }, 2000);

        if (onClick) {
            onClick(e);
        }
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
                icon: isCopied() ? "check" : "copy",
                title,
                className: clsx("copy-button", { copied: isCopied() }),
                click: handleOnClick,
            }}
            className={className}
        />
    );
};

export { CopyButton };
