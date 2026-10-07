# AgentMux Performance Benchmarking

Automated tools for measuring AgentMux performance metrics to validate the Tauri migration goals.

## Metrics Measured

### 1. Startup Time
- Time from process launch to main window ready
- Multiple runs for statistical accuracy
- Reports: average, median, min, max

### 2. Memory Usage
- Idle memory consumption (after 5s)
- After initialization memory (after 10s)
- RSS (Resident Set Size) measurement

### 3. Bundle Size
- Executable size
- Installer size
- Comparison with Electron (if available)

## Migration Goals (from TAURI_MIGRATION_STATUS.md)

| Metric | Electron Baseline | Tauri Target | Status |
|--------|------------------|--------------|--------|
| Installer Size | 120-150MB | < 25MB | ⏳ Measure |
| Idle Memory | 150-300MB | < 50MB | ⏳ Measure |
| Startup Time | 1-2s | < 0.5s | ⏳ Measure |
| Bundle Reduction | - | 10x | ⏳ Verify |

## Usage

### Windows (PowerShell)

```powershell
# Build release version first
task package

# Run benchmarks (5 runs by default)
.\scripts\benchmarks\measure-performance.ps1

# Custom number of runs
.\scripts\benchmarks\measure-performance.ps1 -Runs 10

# Output JSON results
.\scripts\benchmarks\measure-performance.ps1 -OutputJson

# Specify custom app path
.\scripts\benchmarks\measure-performance.ps1 -AppPath "path\to\AgentMux.exe"
```

### macOS / Linux (Bash)

```bash
# Build release version first
task package

# Run benchmarks (5 runs by default)
chmod +x scripts/benchmarks/measure-performance.sh
./scripts/benchmarks/measure-performance.sh

# Custom number of runs
./scripts/benchmarks/measure-performance.sh 10
```

## Output

### Console Output

```
========================================
  Startup Time Measurement
========================================

Running 5 iterations...
Run 1/5... 342ms
Run 2/5... 328ms
Run 3/5... 335ms
Run 4/5... 331ms
Run 5/5... 340ms

Results:
  Average: 335.2ms
  Median:  335ms
  Min:     328ms
  Max:     342ms

========================================
  Memory Usage Measurement
========================================

Idle Memory: 42.5 MB
After Init:  48.3 MB

========================================
  Bundle Size Measurement
========================================

AgentMux.exe:      8.2 MB
Installer (.msi): 15.4 MB

Electron Package: 142.3 MB
Size Reduction:   89.2%

========================================
  Summary
========================================

Startup Time (avg):  335.2ms
Memory Usage (idle): 42.5 MB
Executable Size:     8.2 MB
```

### JSON Output (PowerShell -OutputJson)

```json
{
  "Startup": {
    "Average": 335.2,
    "Median": 335,
    "Min": 328,
    "Max": 342,
    "Runs": [342, 328, 335, 331, 340]
  },
  "Memory": {
    "IdleMB": 42.5,
    "AfterInitMB": 48.3
  },
  "BundleSize": {
    "ExeMB": 8.2,
    "InstallerMB": 15.4
  }
}
```

## Interpreting Results

### Startup Time

**Target: < 500ms**

- ✅ < 500ms: Excellent - meets Tauri migration goal
- ⚠️ 500-1000ms: Good - better than Electron but not meeting target
- ❌ > 1000ms: Poor - investigate startup bottlenecks

**Common bottlenecks:**
- Backend (agentmuxsrv) spawn time
- WebSocket connection establishment
- Frontend bundle loading
- Window state restoration

### Memory Usage

**Target: < 50MB idle**

- ✅ < 50MB: Excellent - meets migration goal
- ⚠️ 50-100MB: Good - improvement over Electron
- ❌ > 100MB: Poor - check for memory leaks

**Common issues:**
- Too many cached tabs/workspaces
- Memory leaks in frontend
- Large Redux state
- Terminal buffers not cleaned up

### Bundle Size

**Target: < 25MB installer**

- ✅ < 25MB: Excellent - 10x reduction achieved
- ⚠️ 25-50MB: Good - 5x reduction
- ❌ > 50MB: Poor - review included dependencies

**Optimization tips:**
- Strip debug symbols: `strip AgentMux.exe`
- Use `--release` mode
- Review Cargo.toml dependencies
- Minimize frontend bundle with tree-shaking

## Comparison with Electron

If you have Electron builds in `dist/`, the script will automatically compare:

```
Electron Package: 142.3 MB
Tauri Installer:   15.4 MB
Size Reduction:    89.2% ✅
```

## Continuous Monitoring

Add benchmarking to CI/CD:

```yaml
# .github/workflows/benchmark.yml
- name: Run Performance Benchmarks
  run: |
    task package
    pwsh scripts/benchmarks/measure-performance.ps1 -OutputJson

- name: Upload Results
  uses: actions/upload-artifact@v4
  with:
    name: benchmark-results
    path: benchmark-results.json
```

## Troubleshooting

### App won't start

**Error:** Application not found
**Fix:** Run `task package` first to build the release version

### Startup time too high

**Error:** Startup time > 1s
**Possible causes:**
- Debug build instead of release
- Antivirus scanning
- Cold start (first run)
**Fix:** Run multiple iterations, use `--release` build

### Memory measurement fails

**Error:** Cannot measure memory usage
**macOS/Linux:** Ensure process has permission to read /proc or use ps
**Windows:** Run PowerShell as Administrator

## See Also

- [TAURI_MIGRATION_STATUS.md](../../docs/TAURI_MIGRATION_STATUS.md) - Migration progress
- [BUILD.md](../../BUILD.md) - Build instructions
- [Taskfile.yml](../../Taskfile.yml) - Build tasks

## Agent-open latency (`agent-open.mjs`)

Opens agents from My Agents in a running instance and checks each open's `[agent-open]` trace against a budget, so an open-latency regression fails a run instead of reaching users (`docs/reports/REPORT_AGENT_OPEN_STALL_RCA_2026_10_05.md`).

```bash
# A dev build (CDP on by default), or a release instance started with AGENTMUX_CDP_PORT
node scripts/benchmarks/agent-open.mjs --agents Maka,Lzop,Parko --port 9222
```

| Option | Default | Meaning |
|---|---|---|
| `--agents A,B,…` | (required) | Agents to open, by their name in My Agents |
| `--port N` | `$AGENTMUX_CDP_PORT` or 9222 | The instance's CDP port |
| `--read-budget MS` | 1000 | Transcript read (`history_read − history_start`) |
| `--reveal-budget MS` | 1500 | Loading cover lifted (`revealed`) |
| `--paint-budget MS` | none | First rows painted (`painted`); the spec's target is 300 |
| `--timeout MS` | 30000 | Wait per open before reporting it skipped |
| `--json` | off | Machine-readable results |

Prints one row per agent and p50/p95/max per phase. Exit code 0 when every measured open is within budget, 1 when any isn't, 2 on a setup error. An agent that's already open is focused rather than opened and is reported as skipped. Each open starts the agent in that instance, taking it over from any other instance, so run it against a dev build or a throwaway instance.
