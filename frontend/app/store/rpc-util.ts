// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { mpsReconnectHandler } from "@/app/store/mps";
import { TabClient } from "@/app/store/tabrpcclient";
import { makeTabRouteId, RpcRouter } from "@/app/store/rpc-router";
import { getWSServerEndpoint } from "@/util/endpoints";
import { addWSReconnectHandler, globalWS, initGlobalWS, WSControl } from "./ws";
import { DefaultRouter, setDefaultRouter } from "./rpc-util-base";
import { getApi } from "./global";
import { noteSrvConnectionOpened, noteSrvInfoMessage } from "./srv-info";
import { noteEventsDroppedMessage } from "./resync";
import { expectOperatorProof, proveOperatorConnection } from "./operator-proof";
import { RpcApi } from "./rpc-api";

let TabRpcClient: TabClient;

function initWshrpc(tabId: string): WSControl {
    const router = new RpcRouter(new UpstreamRpcProxy());
    setDefaultRouter(router);
    const handleFn = (event: WSEventType) => {
        if (event.data == null) return;
        noteSrvInfoMessage(event.data);
        noteEventsDroppedMessage(event.data);
        DefaultRouter.recvRpcMessage(event.data);
    };

    // Host app: get the auth key from the API and pass it to WebSocket
    // Browser WebSocket doesn't support custom headers, so wsutil.ts will
    // append it as a query parameter: ws://endpoint?authkey=xxx
    const authKey = getApi().getAuthKey();
    const authOpts = authKey ? { authKey } : undefined;

    initGlobalWS(getWSServerEndpoint(), tabId, handleFn, authOpts);
    // Each connection reports its own srvinfo (a reconnect can reach another srv).
    addWSReconnectHandler(noteSrvConnectionOpened);
    // Each connection proves, through the host, that it is the operator's
    // window: operator-only RPCs are refused until it has.
    if (getApi().approvals?.attestOperator) expectOperatorProof();
    addWSReconnectHandler(() => {
        const attest = getApi().approvals?.attestOperator;
        void proveOperatorConnection({
            challenge: () => RpcApi.OperatorChallengeCommand(TabRpcClient),
            attest: attest ? (nonce) => attest.call(getApi().approvals, nonce) : undefined,
        });
    });
    globalWS.connectNow("connectWshrpc");
    TabRpcClient = new TabClient(makeTabRouteId(tabId));
    DefaultRouter.registerRoute(TabRpcClient.routeId, TabRpcClient);
    addWSReconnectHandler(() => {
        DefaultRouter.reannounceRoutes();
    });
    addWSReconnectHandler(mpsReconnectHandler);
    return globalWS;
}

class UpstreamRpcProxy implements AbstractRpcClient {
    recvRpcMessage(msg: RpcMessage): void {
        const wsMsg: WSRpcCommand = { wscommand: "rpc", message: msg };
        globalWS?.pushMessage(wsMsg);
    }
}

export { DefaultRouter, initWshrpc, TabRpcClient };
