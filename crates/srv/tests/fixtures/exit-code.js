// Test fixture: exits with the code passed as first argument (default 1).
const code = parseInt(process.argv[2] || "1", 10);
process.stderr.write("intentional error output\n");
process.exit(code);
