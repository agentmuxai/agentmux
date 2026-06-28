// Test fixture: reads all stdin, echoes it back as JSON on stdout, exits 0.
process.stdin.setEncoding("utf8");
let buf = "";
process.stdin.on("data", (d) => { buf += d; });
process.stdin.on("end", () => {
    // Exit only after the write has fully drained to the pipe. Calling
    // process.exit(0) synchronously after a large write truncates output:
    // write() returns before a >pipe-buffer payload (e.g. 100KB) is flushed,
    // and exit() then kills the process mid-flush. The drain callback fires
    // once the data is handed off, so the reader sees the complete line.
    process.stdout.write(JSON.stringify({ echo: buf.trim() }) + "\n", () => {
        process.exit(0);
    });
});
