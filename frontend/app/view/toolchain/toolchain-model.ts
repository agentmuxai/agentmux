// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export class ToolchainViewModel implements ViewModel {
    viewType = "toolchain";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon = () => "wrench";
    viewName = () => "Toolchain";
    // wired in toolchain.tsx to avoid circular import
    declare viewComponent: ViewComponent<ToolchainViewModel>;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
    }
}
