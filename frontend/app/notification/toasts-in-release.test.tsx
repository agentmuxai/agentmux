// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Toasts in every build (SPEC_RECOVERY_NOTICES_AND_TOASTS_2026_10_16.md §4.5):
// how many are kept and shown, and what closing one does.

import { capNotifications, MAX_KEPT_NOTIFICATIONS, setNotifications } from "@/app/store/flash-notifications";
import { createRoot } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import { MAX_VISIBLE_NOTIFICATIONS, visibleNotifications } from "./notificationbubbles";
import { useNotification } from "./usenotification";

function toast(id: string, extra: Partial<NotificationType> = {}): NotificationType {
    return { id, icon: "bell", title: id, message: "", timestamp: "", ...extra };
}

describe("the toast list", () => {
    it("keeps at most twenty, dropping the oldest ordinary toast before a persistent one", () => {
        const list = [toast("sticky", { persistent: true }), ...Array.from({ length: 24 }, (_, i) => toast(`t${i}`))];
        const kept = capNotifications(list);
        expect(kept).toHaveLength(MAX_KEPT_NOTIFICATIONS);
        expect(kept[0].id).toBe("sticky");
        expect(kept.map((n) => n.id)).not.toContain("t4");
        expect(kept.at(-1)?.id).toBe("t23");
    });

    it("shows the newest five and every persistent toast, the rest behind +N more", () => {
        const plain = Array.from({ length: 8 }, (_, i) => toast(`t${i}`));
        expect(visibleNotifications(plain, false).map((n) => n.id)).toEqual(["t3", "t4", "t5", "t6", "t7"]);
        expect(visibleNotifications(plain, true)).toHaveLength(8);

        const mixed = [toast("s1", { persistent: true }), toast("s2", { persistent: true }), ...plain];
        const shown = visibleNotifications(mixed, false).map((n) => n.id);
        expect(shown).toEqual(["s1", "s2", "t5", "t6", "t7"]);
        expect(shown).toHaveLength(MAX_VISIBLE_NOTIFICATIONS);
        expect(visibleNotifications([toast("h", { hidden: true }), toast("v")], false).map((n) => n.id)).toEqual(["v"]);
    });
});

describe("closing a toast", () => {
    it("removes it and tells its sender, which an expiry doesn't", () => {
        const onClose = vi.fn();
        createRoot((dispose) => {
            setNotifications([toast("a", { onClose }), toast("b")]);
            const { closeNotification, notifications } = useNotification();
            closeNotification("a");
            expect(notifications().map((n) => n.id)).toEqual(["b"]);
            expect(onClose).toHaveBeenCalledTimes(1);
            closeNotification("missing");
            expect(onClose).toHaveBeenCalledTimes(1);
            setNotifications([]);
            dispose();
        });
    });
});
