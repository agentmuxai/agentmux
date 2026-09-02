// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// File and remote-file operations, plus the editor file-tree commands. Split
// from the original hand-maintained rpc-api.ts.

import { RpcClient } from "../rpc-client";

export const FileApi = {
    FileAppendCommand(client: RpcClient, data: FileData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("fileappend", data, opts);
    },

    FileAppendIJsonCommand(client: RpcClient, data: CommandAppendIJsonData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("fileappendijson", data, opts);
    },

    FileCopyCommand(client: RpcClient, data: CommandFileCopyData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("filecopy", data, opts);
    },

    FileCreateCommand(client: RpcClient, data: FileData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("filecreate", data, opts);
    },

    FileDeleteCommand(client: RpcClient, data: CommandDeleteFileData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("filedelete", data, opts);
    },

    FileInfoCommand(client: RpcClient, data: FileData, opts?: RpcOpts): Promise<FileInfo> {
        return client.rpcCall("fileinfo", data, opts);
    },

    FileJoinCommand(client: RpcClient, data: string[], opts?: RpcOpts): Promise<FileInfo> {
        return client.rpcCall("filejoin", data, opts);
    },

    FileListCommand(client: RpcClient, data: FileListData, opts?: RpcOpts): Promise<FileInfo[]> {
        return client.rpcCall("filelist", data, opts);
    },

    FileListStreamCommand(client: RpcClient, data: FileListData, opts?: RpcOpts): AsyncGenerator<CommandRemoteListEntriesRtnData, void, boolean> {
        return client.rpcStream("fileliststream", data, opts);
    },

    FileMkdirCommand(client: RpcClient, data: FileData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("filemkdir", data, opts);
    },

    FileMoveCommand(client: RpcClient, data: CommandFileCopyData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("filemove", data, opts);
    },

    FileReadCommand(client: RpcClient, data: FileData, opts?: RpcOpts): Promise<FileData> {
        return client.rpcCall("fileread", data, opts);
    },

    FileReadStreamCommand(client: RpcClient, data: FileData, opts?: RpcOpts): AsyncGenerator<FileData, void, boolean> {
        return client.rpcStream("filereadstream", data, opts);
    },

    FileShareCapabilityCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<FileShareCapability> {
        return client.rpcCall("filesharecapability", data, opts);
    },

    FileStreamTarCommand(client: RpcClient, data: CommandRemoteStreamTarData, opts?: RpcOpts): AsyncGenerator<Packet, void, boolean> {
        return client.rpcStream("filestreamtar", data, opts);
    },

    FileWriteCommand(client: RpcClient, data: FileData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("filewrite", data, opts);
    },

    RemoteFileCopyCommand(client: RpcClient, data: CommandFileCopyData, opts?: RpcOpts): Promise<boolean> {
        return client.rpcCall("remotefilecopy", data, opts);
    },

    RemoteFileDeleteCommand(client: RpcClient, data: CommandDeleteFileData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("remotefiledelete", data, opts);
    },

    RemoteFileInfoCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<FileInfo> {
        return client.rpcCall("remotefileinfo", data, opts);
    },

    RemoteFileJoinCommand(client: RpcClient, data: string[], opts?: RpcOpts): Promise<FileInfo> {
        return client.rpcCall("remotefilejoin", data, opts);
    },

    RemoteFileMoveCommand(client: RpcClient, data: CommandFileCopyData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("remotefilemove", data, opts);
    },

    RemoteFileTouchCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("remotefiletouch", data, opts);
    },

    RemoteGetInfoCommand(client: RpcClient, opts?: RpcOpts): Promise<RemoteInfo> {
        return client.rpcCall("remotegetinfo", null, opts);
    },

    RemoteInstallRcFilesCommand(client: RpcClient, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("remoteinstallrcfiles", null, opts);
    },

    RemoteListEntriesCommand(client: RpcClient, data: CommandRemoteListEntriesData, opts?: RpcOpts): AsyncGenerator<CommandRemoteListEntriesRtnData, void, boolean> {
        return client.rpcStream("remotelistentries", data, opts);
    },

    RemoteMkdirCommand(client: RpcClient, data: string, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("remotemkdir", data, opts);
    },

    RemoteStreamCpuDataCommand(client: RpcClient, opts?: RpcOpts): AsyncGenerator<TimeSeriesData, void, boolean> {
        return client.rpcStream("remotestreamcpudata", null, opts);
    },

    RemoteStreamFileCommand(client: RpcClient, data: CommandRemoteStreamFileData, opts?: RpcOpts): AsyncGenerator<FileData, void, boolean> {
        return client.rpcStream("remotestreamfile", data, opts);
    },

    RemoteTarStreamCommand(client: RpcClient, data: CommandRemoteStreamTarData, opts?: RpcOpts): AsyncGenerator<Packet, void, boolean> {
        return client.rpcStream("remotetarstream", data, opts);
    },

    RemoteWriteFileCommand(client: RpcClient, data: FileData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("remotewritefile", data, opts);
    },

    ReadEditorFileCommand(client: RpcClient, data: CommandReadEditorFileData, opts?: RpcOpts): Promise<CommandReadEditorFileResult> {
        return client.rpcCall("readeditorfile", data, opts);
    },

    WriteEditorFileCommand(client: RpcClient, data: CommandWriteEditorFileData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("writeeditorfile", data, opts);
    },

    // Start/stop live-reload watching for a (path, block_id) pair. The
    // backend publishes `editor:file_changed` (scoped to `block:<block_id>`)
    // when the path changes on disk. See
    // docs/specs/SPEC_EDITOR_LIVE_FILE_RELOAD_2026_07_18.md.
    WatchEditorFileCommand(
        client: RpcClient,
        data: { path: string; block_id: string },
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("watcheditorfile", data, opts);
    },

    UnwatchEditorFileCommand(
        client: RpcClient,
        data: { path: string; block_id: string },
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("unwatcheditorfile", data, opts);
    },

    // Spec: docs/specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md
    ListEditorDirCommand(
        client: RpcClient,
        data: { path: string },
        opts?: RpcOpts,
    ): Promise<{ path: string; entries: DirEntry[] }> {
        return client.rpcCall("listeditordir", data, opts);
    },

    // Media pane (SPEC_MEDIA_PANE_2026_07_26.md): watch a directory for
    // new/changed files matching `extensions` (lowercase, no dot). Backend
    // publishes `media:file_changed` (scoped to `block:<block_id>`) with
    // `{ path }` when a matching file is created/modified.
    WatchMediaDirCommand(
        client: RpcClient,
        data: { path: string; block_id: string; extensions: string[] },
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("watchmediadir", data, opts);
    },

    UnwatchMediaDirCommand(
        client: RpcClient,
        data: { path: string; block_id: string },
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("unwatchmediadir", data, opts);
    },

    GetEditorHomeCommand(
        client: RpcClient,
        data: Record<string, never> = {},
        opts?: RpcOpts,
    ): Promise<{ home: string }> {
        return client.rpcCall("geteditorhome", data, opts);
    },

    // Returns home + drives/mounts; the editor file-tree renders these as sibling top-level roots.
    GetEditorRootsCommand(
        client: RpcClient,
        data: Record<string, never> = {},
        opts?: RpcOpts,
    ): Promise<{ home: string; drives: { name: string; path: string }[] }> {
        return client.rpcCall("geteditorroots", data, opts);
    },

    // Spec: docs/specs/SPEC_FILE_TREE_CONTEXT_MENU_2026_06_14.md
    OpenInShellCommand(
        client: RpcClient,
        data: { path: string },
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("openinshell", data, opts);
    },

    RenameEditorFileCommand(
        client: RpcClient,
        data: { old_path: string; new_name: string },
        opts?: RpcOpts,
    ): Promise<{ new_path: string }> {
        return client.rpcCall("renameeditorfile", data, opts);
    },

    CreateEditorFileCommand(
        client: RpcClient,
        data: { parent_path: string; name: string },
        opts?: RpcOpts,
    ): Promise<{ file_path: string }> {
        return client.rpcCall("createeditorfile", data, opts);
    },

    CreateEditorDirCommand(
        client: RpcClient,
        data: { parent_path: string; name: string },
        opts?: RpcOpts,
    ): Promise<{ dir_path: string }> {
        return client.rpcCall("createeditordir", data, opts);
    },

    DeleteEditorFileCommand(
        client: RpcClient,
        data: { path: string; recursive: boolean },
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("deleteeditorfile", data, opts);
    },

    // Creates a scratch buffer file in ~/.agentmux/cache/scratch/. Returns the backing path + scratch_id.
    // Spec: docs/specs/SPEC_EDITOR_WIDGET_DEFAULT_UX_2026_06_14.md
    CreateScratchFileCommand(
        client: RpcClient,
        data: { display_name?: string; exclude_scratch_ids?: string[] } = {},
        opts?: RpcOpts,
    ): Promise<{ scratch_id: string; file_path: string; display_name: string }> {
        return client.rpcCall("createscratchfile", data, opts);
    },

    MoveScratchFileCommand(
        client: RpcClient,
        data: { scratch_id: string; destination_path: string },
        opts?: RpcOpts,
    ): Promise<{ file_path: string }> {
        return client.rpcCall("movescratchfile", data, opts);
    },

    // ── LSP — Phase 1 of SPEC_EDITOR_LSP_AND_THEMES_2026-05-26.md ──────
    // Backend is a dumb proxy: lspstart spawns (or attaches to) the
    // server for (workspace, language); lspsend forwards an arbitrary
    // LSP JSON-RPC message to its stdin; lspstop refcount-decrements.
    // Server-pushed notifications arrive via the `lsp:message` WS event.

    LspStartCommand(
        client: RpcClient,
        data: { language: string; file_path: string },
        opts?: RpcOpts,
    ): Promise<{ server_id: string; workspace_root: string }> {
        return client.rpcCall("lspstart", data, opts);
    },

    LspSendCommand(
        client: RpcClient,
        data: { server_id: string; message: unknown },
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("lspsend", data, opts);
    },

    LspStopCommand(
        client: RpcClient,
        data: { server_id: string },
        opts?: RpcOpts,
    ): Promise<void> {
        return client.rpcCall("lspstop", data, opts);
    },
};
