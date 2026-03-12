// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { Button } from "@/element/button";
import { Popover, PopoverButton, PopoverContent } from "@/element/popover";
import { atoms, setNotificationPopoverMode } from "@/store/global";
import { makeIconClass } from "@/util/util";
import clsx from "clsx";
import { For, Show, type JSX } from "solid-js";
import { NotificationItem } from "./notificationitem";
import { useUpdateNotifier } from "./updatenotifier";
import { useNotification } from "./usenotification";

const NotificationPopover = (): JSX.Element => {
    useUpdateNotifier();
    const {
        notifications,
        removeNotification,
        removeAllNotifications,
        hideAllNotifications,
        copyNotification,
        handleActionClick,
        formatTimestamp,
        hoveredId,
        setHoveredId,
    } = useNotification();
    const notificationPopoverMode = atoms.notificationPopoverMode;

    const handleTogglePopover = () => {
        if (notificationPopoverMode()) {
            hideAllNotifications();
        }
        setNotificationPopoverMode(!notificationPopoverMode());
    };

    const hasErrors = () => notifications().some((n) => n.type === "error");
    const hasUpdate = () => notifications().some((n) => n.type === "update");

    const addOnClassNames = () => hasUpdate() ? "solid green" : hasErrors() ? "solid red" : "ghost grey";

    const getIcon = (): JSX.Element => {
        if (hasUpdate()) {
            return <i class={makeIconClass("arrows-rotate", false)}></i>;
        }
        return <i class={makeIconClass("bell", false)}></i>;
    };

    return (
        <Popover
            class="w-full pb-2 pt-1 pl-0 pr-0.5 flex items-center justify-center"
            placement="left-end"
            offset={{ mainAxis: 20, crossAxis: 2 }}
            onDismiss={handleTogglePopover}
        >
            <PopoverButton
                className={clsx(
                    "w-[27px] h-[26px] flex justify-center [&>i]:text-[17px] px-[6px] py-[4px]",
                    addOnClassNames()
                )}
                disabled={notifications().length === 0}
                onClick={handleTogglePopover}
            >
                {getIcon()}
            </PopoverButton>
            <Show when={notifications().length > 0}>
                <PopoverContent className="flex w-[380px] pt-2.5 pb-0 px-0 flex-col items-start gap-x-2 rounded-lg border-[0.5px] border-white/12 bg-[#232323] shadow-[0px_8px_32px_0px_rgba(0,0,0,0.25)]">
                    <div class="flex items-center justify-between w-full px-2.5 pb-2 border-b border-white/8">
                        <span class="text-foreground text-sm font-semibold leading-4">Notifications</span>
                        <Button
                            class="ghost grey text-[13px] font-normal leading-4 text-white/40 px-[3px] py-[3px]"
                            onClick={(e) => {
                                e.stopPropagation();
                                removeAllNotifications();
                            }}
                        >
                            Clear
                        </Button>
                    </div>
                    <div
                        class="scrollable"
                        style={{ "max-height": `${window.innerHeight / 2}px`, "overflow-y": "auto" }}
                    >
                        <For each={notifications()}>
                            {(notif, index) => (
                                <>
                                    <NotificationItem
                                        class={clsx({ hovered: hoveredId() === notif.id })}
                                        notification={notif}
                                        onRemove={removeNotification}
                                        onCopy={copyNotification}
                                        onActionClick={handleActionClick}
                                        formatTimestamp={formatTimestamp}
                                        isBubble={false}
                                        onMouseEnter={() => setHoveredId(notif.id)}
                                        onMouseLeave={() => setHoveredId(null)}
                                    />
                                    <Show when={index() !== notifications().length - 1}>
                                        <div class="bg-white/8 h-px w-full"></div>
                                    </Show>
                                </>
                            )}
                        </For>
                    </div>
                </PopoverContent>
            </Show>
        </Popover>
    );
};

export { NotificationPopover };
