// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export type ArmorySection = "accounts" | "brain" | "skills" | "mcp" | "memories";

export class ArmoryViewModel implements ViewModel {
    viewType = "armory";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon = () => "vault";
    viewName = () => "Armory";
    // wired in armory.tsx to avoid circular import
    declare viewComponent: ViewComponent<ArmoryViewModel>;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
    }
}
