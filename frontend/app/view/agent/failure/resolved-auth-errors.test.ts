// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { createSignInNotice } from "../sign-in-notice";
import type { DocumentNode } from "../types";
import { markResolvedAuthErrors } from "./resolved-auth-errors";

const error = (id: string, code = 401): DocumentNode => ({ type: "agent_error", id, code, message: "Failed to authenticate" });
const said = (id: string, content: string): DocumentNode => ({ type: "markdown", id, content });
const user = (id: string): DocumentNode => ({ type: "user_message", id, message: "hi" });
const resolvedIds = (nodes: DocumentNode[]) =>
    markResolvedAuthErrors(nodes).filter((n) => n.type === "agent_error" && n.resolved).map((n) => n.id);

describe("markResolvedAuthErrors", () => {
    it("answers an error that a later sign-in follows", () => {
        expect(resolvedIds([user("u"), error("e"), said("system_notification_sign_in", "✓ Login successful")])).toEqual(["e"]);
    });

    it("answers a failed turn's error that lands after the sign-in", () => {
        expect(resolvedIds([user("u"), said("system_notification_signed_in_1", "✓ Login successful"), error("e")])).toEqual(["e"]);
    });

    it("leaves an error from a turn that started after the sign-in", () => {
        const nodes = [said("system_notification_signed_in_1", "✓ Login successful"), user("retry"), error("e")];
        expect(resolvedIds(nodes)).toEqual([]);
        expect(markResolvedAuthErrors(nodes)).toBe(nodes);
    });

    it("leaves a sign-in line that still asks, and non-auth errors", () => {
        expect(resolvedIds([error("e"), said("system_notification_sign_in", "Sign in to continue")])).toEqual([]);
        expect(resolvedIds([error("e", 429), said("system_notification_signed_in_1", "✓ Login successful")])).toEqual([]);
    });

    it("reads the lines the sign-in notice itself posts", () => {
        const lines: DocumentNode[] = [];
        const say = (text: string, style: string, id: string, replaces: boolean) => {
            const node = said(id, `${style === "success" ? "✓ " : ""}${text}`);
            const at = lines.findIndex((n) => n.id === id);
            if (replaces && at >= 0) lines[at] = node;
            else lines.push(node);
        };
        const notice = createSignInNotice(say as never);
        notice.answered("Login successful");
        expect(resolvedIds([error("e"), ...lines])).toEqual(["e"]);
        lines.length = 0;
        notice.needed("Sign in");
        notice.answered("Logged in as **a@b.c**");
        expect(resolvedIds([error("e"), ...lines])).toEqual(["e"]);
    });
});
