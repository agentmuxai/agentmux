// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PlayedSound } from "@/app/notification/sound/sound-history";
import { openOrFocusPaneByView } from "@/app/store/block-component-registry";
import { encodeSoundRef } from "./audit-sound";
import { encodeMessageRef } from "./audit-message";
import { AUDIT_MESSAGE_META_KEY, AUDIT_SOUND_META_KEY } from "./audit-shared";

/** Opens the Audit, or switches the one already in this tab, to the row `sound` came from. */
export function openAuditAtSound(sound: PlayedSound): Promise<void> {
    const meta: Record<string, string> = { [AUDIT_SOUND_META_KEY]: encodeSoundRef(sound) };
    return openOrFocusPaneByView("audit", { meta: { view: "audit", ...meta } }, meta);
}

/** Opens the Audit, or switches the one already in this tab, to the row of the jekt whose marker carried `msgId`;
 *  `at` is when it reached its pane. */
export function openAuditAtMessage(msgId: string, at: number): Promise<void> {
    const meta: Record<string, string> = { [AUDIT_MESSAGE_META_KEY]: encodeMessageRef(msgId, at) };
    return openOrFocusPaneByView("audit", { meta: { view: "audit", ...meta } }, meta);
}
