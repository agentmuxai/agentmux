// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { formatLocale } from "@/app/i18n/i18n";
import { formatRelativeTime } from "@/util/format-time";

/** How long a row of the attention list has waited, in the person's locale; "now" below a minute. */
export function waitedFor(since: number, now: number): string {
    return formatRelativeTime(since, now, { locale: formatLocale(), nowBelowSecs: 60 });
}
