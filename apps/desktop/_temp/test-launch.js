// Quick test to verify WebView2 debugging works
const { spawn } = require('child_process');
const path = require('path');

const debugPort = 9123;
const userDataDir = path.join(__dirname, '..', 'test-data', `webview2-test-${Date.now()}`);
const executablePath = path.join(__dirname, '..', 'src-tauri', 'target', 'release', 'agentmux.exe');

console.log(`[Test] Launching: ${executablePath}`);
console.log(`[Test] Debug port: ${debugPort}`);
console.log(`[Test] User data: ${userDataDir}`);

const proc = spawn(executablePath, [], {
  env: {
    ...process.env,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${debugPort}`,
    WEBVIEW2_USER_DATA_FOLDER: userDataDir,
    AGENTMUX_DISABLE_SINGLE_INSTANCE: '1',
    RUST_LOG: 'debug',
  },
  stdio: 'inherit',
});

proc.on('error', (err) => {
  console.error(`[Test] Failed to start:`, err);
});

// Wait 10 seconds then try to fetch CDP info
setTimeout(async () => {
  try {
    const response = await fetch(`http://localhost:${debugPort}/json/version`);
    const data = await response.json();
    console.log(`\n[Test] ✅ SUCCESS! WebView2 debugging is working:`);
    console.log(JSON.stringify(data, null, 2));
    proc.kill();
    process.exit(0);
  } catch (err) {
    console.error(`\n[Test] ❌ FAILED to connect to debugging port:`, err.message);
    proc.kill();
    process.exit(1);
  }
}, 10000);
