// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { TabBar } from "@/app/tab/tabbar";
import { WindowDrag } from "@/element/windowdrag";
import { useWindowDrag } from "@/app/hook/useWindowDrag.platform";
import { atoms } from "@/store/global";
import { createSignal, type JSX, Show } from "solid-js";
import { TitleBarContextMenu } from "@/app/window/titlebar-context-menu";
import { SystemStatus } from "@/app/window/system-status";
import { WindowControlsLeft } from "@/app/window/window-controls.platform";
import { HamburgerMenu } from "@/app/window/hamburger-menu";
import { isMacOS } from "@/util/platformutil";
import "./window-header.platform.scss";


interface WindowHeaderProps {
    workspace: Workspace;
}

const WindowHeader = (props: WindowHeaderProps): JSX.Element => {
    let windowHeaderRef!: HTMLDivElement;
    let draggerLeftRef!: HTMLDivElement;

    const fullConfig = atoms.fullConfigAtom;
    const { dragProps } = useWindowDrag();

    const [menuOpen, setMenuOpen] = createSignal(false);
    const [menuPos,  setMenuPos]  = createSignal({ x: 0, y: 0 });

    const handleContextMenu = (e: MouseEvent) => {
        e.preventDefault();
        e.stopPropagation(); // prevent bubbling to app-level onContextMenu (clipboard/URL menu)
        setMenuPos({ x: e.clientX, y: e.clientY });
        setMenuOpen(true);
    };

    return (
        <div
            ref={windowHeaderRef}
            class="window-header"
            data-testid="window-header"
            {...dragProps}
            onContextMenu={handleContextMenu}
        >
            <WindowControlsLeft />

            <WindowDrag ref={draggerLeftRef} class="left" />

            <TabBar workspace={props.workspace} />

            <SystemStatus />

            {/* macOS: the hamburger lives at the far right of the header,
                after the action widgets — the native traffic-light controls
                occupy the left, so a left-side hamburger crowds them. On
                Windows/Linux it stays in the tab strip (see TabBar). */}
            <Show when={isMacOS()}>
                <HamburgerMenu position="right" />
            </Show>

            {/* PopoverMenu (inside TitleBarContextMenu) already renders into
                a Portal — no extra Portal wrapper needed here. */}
            <Show when={menuOpen()}>
                <TitleBarContextMenu
                    pos={menuPos()}
                    fullConfig={fullConfig()}
                    onClose={() => setMenuOpen(false)}
                />
            </Show>
        </div>
    );
};

export { WindowHeader };
