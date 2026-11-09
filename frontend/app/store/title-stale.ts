// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { makeORef } from "@/app/store/mos";
import { META_TITLE_AT } from "@/app/store/meta-keys";
import { ObjectService } from "@/app/store/services";
import { fireAndForget } from "@/util/util";

/**
 * The work moved on (a compaction, a catch-up turn): judge the session title afresh at the next
 * human message instead of waiting for it to age. A zero stamp reads as aged
 * (`isTitleAged`, title-schedule.ts); the backend replaces it when it judges.
 */
export function markTitleStale(blockId: string): void {
    fireAndForget(() => ObjectService.UpdateObjectMeta(makeORef("block", blockId), { [META_TITLE_AT]: 0 } as any));
}
