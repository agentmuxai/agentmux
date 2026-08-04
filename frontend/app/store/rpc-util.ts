// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { wpsReconnectHandler } from "@/app/store/wps";
import { TabClient } from "@/app/store/tabrpcclient";
import { makeTabRouteId, RpcRouter } from "@/app/store/rpc-router";
import { getWSServerEndpoint } from "@/util/endpoints";
import { addWSReconnectHandler, globalWS, initGlobalWS, WSControl } from "./ws";
import { DefaultRouter, setDefaultRouter } from "./rpc-util-base";
import { getApi } from "./global";

let TabRpcClient: TabClient;

function initWshrpc(tabId: string): WSControl {
    const router = new RpcRouter(new UpstreamRpcProxy());
    setDefaultRouter(router);
    const handleFn = (event: WSEventType) => {
        if (event.data == null) return;
        DefaultRouter.recvRpcMessage(event.data);
    };

    // For Tauri: Get auth key from API and pass it to WebSocket
    // Browser WebSocket doesn't support custom headers, so wsutil.ts will
    // append it as a query parameter: ws://endpoint?authkey=xxx
    const authKey = getApi().getAuthKey();
    const authOpts = authKey ? { authKey } : undefined;

    initGlobalWS(getWSServerEndpoint(), tabId, handleFn, authOpts);
    globalWS.connectNow("connectWshrpc");
    TabRpcClient = new TabClient(makeTabRouteId(tabId));
    DefaultRouter.registerRoute(TabRpcClient.routeId, TabRpcClient);
    addWSReconnectHandler(() => {
        DefaultRouter.reannounceRoutes();
    });
    addWSReconnectHandler(wpsReconnectHandler);
    return globalWS;
}

class UpstreamRpcProxy implements AbstractRpcClient {
    recvRpcMessage(msg: RpcMessage): void {
        const wsMsg: WSRpcCommand = { wscommand: "rpc", message: msg };
        globalWS?.pushMessage(wsMsg);
    }
}

export { DefaultRouter, initWshrpc, TabRpcClient };
