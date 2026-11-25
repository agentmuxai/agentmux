// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Who added, changed and bound an MCP server, in words. The srv records an actor label for each
 * (`rpc::actor::Actor::label`): `agent:<id>`, `operator`, `person:<id>`, `unproven`, `agentmux`, or `unknown` for
 * a server or bind made before this was kept.
 */

import { t } from "@/app/i18n/i18n";
import { formatExactTimeWithDate } from "@/util/format-time";

/** An agent's display name by the id or slug the label carries; undefined when none matches. */
export type AgentNameOf = (agentId: string) => string | undefined;

export function actorName(actor: string, agentNameOf: AgentNameOf): string {
    if (actor === "operator") return t("mcp.actor.operator");
    if (actor === "agentmux") return t("mcp.actor.agentmux");
    if (actor === "unproven") return t("mcp.actor.unproven");
    if (actor.startsWith("person:")) return t("mcp.actor.person");
    if (actor.startsWith("agent:")) {
        const id = actor.slice("agent:".length);
        return agentNameOf(id) ?? id;
    }
    return t("mcp.actor.unknown");
}

/** "Added by you, Nov 8, 3:46:12 PM"; without the time when none was recorded (0). */
export function originLine(kind: "added" | "changed" | "bound", actor: string, at: number, agentNameOf: AgentNameOf): string {
    const who = actorName(actor, agentNameOf);
    const when = at > 0 ? formatExactTimeWithDate(at) : "";
    switch (kind) {
        case "added":
            return at > 0 ? t("mcp.origin.addedAt", { who, when }) : t("mcp.origin.added", { who });
        case "changed":
            return at > 0 ? t("mcp.origin.changedAt", { who, when }) : t("mcp.origin.changed", { who });
        case "bound":
            return at > 0 ? t("mcp.origin.boundAt", { who, when }) : t("mcp.origin.bound", { who });
    }
}
