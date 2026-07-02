// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export type ArmorySection = "accounts" | "identities" | "brain" | "memories";

export class ArmoryViewModel implements ViewModel {
    viewType = "armory";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon = () => "shield-halved";
    viewName = () => "Armory";
    // wired in armory.tsx to avoid circular import
    declare viewComponent: ViewComponent<ArmoryViewModel>;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
    }
}
