// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { TermViewModel } from "@/app/view/term/termViewModel";
import { computeTheme } from "@/app/view/term/termutil";
import { TermWrap } from "@/app/view/term/termwrap";
import { atoms } from "@/app/store/global";
import { appliedWindowTheme } from "@/app/store/window-appearance";
import { createEffect, createMemo } from "solid-js";
import type { JSX } from "solid-js";

interface TermThemeProps {
    blockId: string;
    termRef: { current: TermWrap | null };
    model: TermViewModel;
}

function TermThemeUpdater(props: TermThemeProps): JSX.Element {
    const theme = createMemo(() => {
        const fullConfig = atoms.fullConfigAtom();
        // A terminal with no theme of its own takes the window theme's
        // CSS colours, so recompute once this window's theme is applied.
        appliedWindowTheme();
        const blockTermTheme = props.model.termThemeNameAtom();
        const transparency = props.model.termTransparencyAtom();
        const [t] = computeTheme(fullConfig, blockTermTheme, transparency);
        return t;
    });

    createEffect(() => {
        const t = theme();
        if (props.termRef.current?.terminal) {
            props.termRef.current.terminal.options.theme = t;
        }
    });

    return null;
}

export { TermThemeUpdater };
