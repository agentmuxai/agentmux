// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * LowDiskNotice: the panel that says free space on the data volume is low. AgentMux Web has no host to push the
 * desktop's low-disk signal, so the page works it out (low-disk.ts) and opens this panel on the Disk readout once for
 * each crossing: warn, then critical, then again after a recovery. It is a panel on the element it is about, not a
 * window modal, and closing it (the button, Esc, a click outside) dismisses it until the next crossing; the numbers in
 * it stay current while it is open.
 */

import { t } from "@/app/i18n/i18n";
import { AnchoredPopover, type PopoverAnchor } from "@/app/element/anchored-popover";
import { Button } from "@/app/element/ui";
import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import { type JSX } from "solid-js";
import { lowDiskText, type DiskReading } from "./low-disk";

export interface LowDiskNoticeData {
    level: "warn" | "critical";
    reading: DiskReading;
    /** The watched volume as the Disk readout names it ("C:", "/home"). */
    label?: string;
}

interface LowDiskNoticeProps {
    /** The status bar's Disk readout. */
    anchor: PopoverAnchor;
    notice: LowDiskNoticeData;
    onDismiss: () => void;
}

export const LowDiskNotice = (props: LowDiskNoticeProps): JSX.Element => {
    const text = () => lowDiskText(props.notice.level, props.notice.reading, props.notice.label);
    return (
        <AnchoredPopover
            anchor={props.anchor}
            placement="top-start"
            fitHeight
            onDismiss={() => props.onDismiss()}
            class="low-disk-notice"
            role="alert"
            aria-label={t("lowDiskNotice.aria")}
            style={{ width: "340px" }}
        >
            <div class={`low-disk-notice-card low-disk-notice-card--${props.notice.level}`} data-level={props.notice.level}>
                <div class="low-disk-notice-title">
                    <span aria-hidden="true">{props.notice.level === "critical" ? "⚠" : "▲"}</span>
                    <span>{text().title}</span>
                </div>
                <div class="low-disk-notice-free">{text().free}</div>
                <div class="low-disk-notice-line">{text().why}</div>
                <div class="low-disk-notice-line">{text().how}</div>
                <div class="low-disk-notice-line">{text().space}</div>
                <div class="low-disk-notice-actions">
                    <Button
                        tone="quiet"
                        density="compact"
                        onClick={() => {
                            props.onDismiss();
                            void openOrFocusPaneByView("history");
                        }}
                    >
                        {t("lowDiskNotice.openHistory")}
                    </Button>
                    <Button density="compact" onClick={() => props.onDismiss()}>
                        {t("lowDiskNotice.dismiss")}
                    </Button>
                </div>
            </div>
        </AnchoredPopover>
    );
};

LowDiskNotice.displayName = "LowDiskNotice";
