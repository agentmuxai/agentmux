---
'agentmux-srv': patch
---

feat(blockfile): output.idx byte-offset index for O(1) line seek

Adds a lazily-built, self-validating byte-offset index (`output.idx`) so
`blockfile:read_range` can seek directly to a requested line range instead of
loading the whole `output` file and slicing by line number.

The index is a pure cache of `output` with no incremental mutation: an 8-byte
header records the output size it was built for, and the read path rebuilds it
(one streaming scan) only when the output size changes. Because it is always
derived from the current output in one shot, it cannot desync, mishandle
chunk-split lines, or miscount blank lines. It indexes non-blank lines to match
the reader's addressing, is gated to non-circular files, and falls back to the
previous full-scan path on any error.
