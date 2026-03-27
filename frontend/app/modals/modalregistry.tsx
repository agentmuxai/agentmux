// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { MessageModal } from "@/app/modals/messagemodal";
import type { JSX } from "solid-js";
import { AboutModal } from "./about";
import { UserInputModal } from "./userinputmodal";

// Onboarding modals removed for lightweight build
const modalRegistry: { [key: string]: (props: any) => JSX.Element } = {
    [UserInputModal.displayName || "UserInputModal"]: UserInputModal,
    [AboutModal.displayName || "AboutModal"]: AboutModal,
    [MessageModal.displayName || "MessageModal"]: MessageModal,
};

export const getModalComponent = (key: string): ((props: any) => JSX.Element) | undefined => {
    return modalRegistry[key];
};
