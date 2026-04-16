// Test fixture: reads all stdin, echoes it back as JSON on stdout, exits 0.
process.stdin.setEncoding("utf8");
let buf = "";
process.stdin.on("data", (d) => { buf += d; });
process.stdin.on("end", () => {
    process.stdout.write(JSON.stringify({ echo: buf.trim() }) + "\n");
    process.exit(0);
});
