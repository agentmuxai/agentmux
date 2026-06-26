// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export type TrustSection = "accounts" | "identities" | "brain" | "memories";

export class TrustViewModel implements ViewModel {
    viewType = "trust";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon = () => "id-card";
    viewName = () => "Trust Center";
    // wired in trust.tsx to avoid circular import
    declare viewComponent: ViewComponent<TrustViewModel>;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
    }
}
