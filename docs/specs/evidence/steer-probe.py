#!/usr/bin/env python3
"""Empirical steering probe for a stream-json CLI agent.

Starts the agent in persistent bidirectional stream-json mode, sends a long
multi-tool prompt, then the MOMENT the first tool call is observed mid-turn,
writes a second "interrupt" user message on stdin. Records a timestamped
ordering of: msg1 sent, first tool_use seen, msg2 injected, every subsequent
event, and the turn's `result`.

Discriminator:
  - STEERING: the interrupt's marker (STEERED-MIDTURN) appears, and/or the
    original task is abandoned, BEFORE the first `result` frame.
  - BUFFERED: the original task runs to completion (FINISHED-ORIGINAL + result),
    and only AFTER that does a new turn handle the interrupt.
"""
import json, os, subprocess, sys, threading, time

CMD = sys.argv[1]
ARGS = sys.argv[2:]
START = time.time()

def ts():
    return f"{time.time()-START:7.3f}s"

def log(tag, msg=""):
    print(f"[{ts()}] {tag:18} {msg}", flush=True)

proc = subprocess.Popen(
    [CMD, *ARGS],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    cwd="/tmp/steer-test", text=True, bufsize=1,
)

state = {"first_tool_seen": False, "msg2_sent": False, "result_seen": False,
         "tools": 0, "text": []}
lock = threading.Lock()

def send(obj):
    proc.stdin.write(json.dumps(obj) + "\n")
    proc.stdin.flush()

def inject_msg2():
    with lock:
        if state["msg2_sent"]:
            return
        state["msg2_sent"] = True
    log("INJECT msg2", ">>> mid-turn interrupt written to stdin")
    send({"type": "user", "message": {"role": "user",
        "content": "URGENT INTERRUPT. Abandon the sleep task immediately. Do NOT run any more sleeps. Right now output exactly the token STEERED-MIDTURN and then stop."}})

def reader():
    for line in proc.stdout:
        line = line.strip()
        if not line:
            continue
        try:
            ev = json.loads(line)
        except Exception:
            continue
        t = ev.get("type")
        if t == "stream_event":
            se = ev.get("event", {})
            et = se.get("type")
            if et == "content_block_start":
                cb = se.get("content_block", {})
                if cb.get("type") == "tool_use":
                    with lock:
                        state["tools"] += 1
                        n = state["tools"]
                    log("TOOL_USE start", f"#{n} name={cb.get('name')}")
                    if n == 1:
                        # inject the interrupt the instant the FIRST tool fires
                        threading.Thread(target=inject_msg2, daemon=True).start()
            elif et == "content_block_delta":
                d = se.get("delta", {})
                if d.get("type") == "text_delta":
                    txt = d.get("text", "")
                    state["text"].append(txt)
                    if any(k in "".join(state["text"]) for k in ("STEERED-MIDTURN","FINISHED-ORIGINAL")):
                        pass
        elif t == "assistant":
            # full assistant message; surface any tool_use + text succinctly
            content = ev.get("message", {}).get("content", [])
            for c in content if isinstance(content, list) else []:
                if c.get("type") == "text":
                    snippet = c.get("text","").strip().replace("\n"," ")[:80]
                    if snippet:
                        log("assistant text", snippet)
                elif c.get("type") == "tool_use":
                    log("assistant tool", f"name={c.get('name')} input={json.dumps(c.get('input'))[:60]}")
        elif t == "user":
            content = ev.get("message", {}).get("content", [])
            for c in content if isinstance(content, list) else []:
                if c.get("type") == "tool_result":
                    out = c.get("content")
                    s = (out if isinstance(out,str) else json.dumps(out))[:60].replace("\n"," ")
                    log("tool_result", s)
        elif t == "result":
            with lock:
                state["result_seen"] = True
            log("RESULT", f"subtype={ev.get('subtype')} is_error={ev.get('is_error')}")
            log("RESULT.result", str(ev.get("result",""))[:120].replace("\n"," "))

th = threading.Thread(target=reader, daemon=True)
th.start()

log("MSG1 send", "long 4x-sleep multi-tool task")
send({"type": "user", "message": {"role": "user",
    "content": "You are in a timing test harness. Do EXACTLY this and nothing else: call the Bash tool 4 separate times, sequentially (not batched). Each call runs: sleep 4 && echo done-K  (K = 1 then 2 then 3 then 4). After all four complete, output the token FINISHED-ORIGINAL."}})

# Hard cap so we never hang.
deadline = time.time() + 90
while time.time() < deadline:
    if proc.poll() is not None:
        break
    # If we've seen a result AFTER injecting msg2, give a short grace for the
    # follow-up turn, then stop.
    time.sleep(0.5)

try:
    proc.stdin.close()
except Exception:
    pass
time.sleep(1.0)
proc.terminate()
try:
    proc.wait(timeout=5)
except Exception:
    proc.kill()

full = "".join(state["text"])
log("VERDICT-DATA", f"tools={state['tools']} STEERED-MIDTURN in text={'STEERED-MIDTURN' in full} FINISHED-ORIGINAL in text={'FINISHED-ORIGINAL' in full}")
err = proc.stderr.read() if proc.stderr else ""
if err.strip():
    log("STDERR", err.strip()[:300])
