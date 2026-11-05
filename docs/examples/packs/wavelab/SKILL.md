---
name: wavelab
description: Use when you need to edit, process or master audio in Steinberg WaveLab (WaveLab Pro or Elements on Windows): open files, batch-process, render. Covers WaveLab's ActiveX/COM scripting first, then its shortcuts and Batch Processor, then Desktop clicks with zoom, with a check after each step.
---

# WaveLab (Steinberg WaveLab Pro / Elements)

Pack: `wavelab` v0.1, unverified. Written from Steinberg's WaveLab help and forum reports; no step has been run by an agent yet, and WaveLab's own pages could not be read in full while writing (the help site refused automated fetches), so **read the installed documentation before you trust a name below** (see Pitfalls).

## The ladder: use the first route that works

1. **ActiveX / COM scripting (Windows).** WaveLab is a COM server. A script (VBScript or JScript) or any COM client creates the application object, then calls its few classes and methods.
   - Start of a script: `Set app = CreateObject("WaveLab10_0.Application")`. **The name changes per version and bitness** (WaveLab 8 used `WaveLab8.Application` and `WaveLab8_64.Application`): take the exact name from the documentation in the install folder, `Steinberg\WaveLab Pro <n>\Tools\Windows Scripting\`, or from the registry (`HKEY_CLASSES_ROOT`, keys starting `WaveLab`). "Automation server can't create object" means a wrong name, a wrong bitness, or WaveLab not registered.
   - Run it from the shell with `cscript script.vbs` (VBScript) or `wscript`; keep scripts in your workspace, not WaveLab's folders.
   - Reported breakage: the `Record` command stopped working from ActiveX in WaveLab Pro 11.1.20 (Steinberg forum). Do not build on recording through COM without testing the installed version.
2. **WaveLab's own script language** (ECMAScript, run from inside WaveLab) for tasks done in the program. Scripts are registered in WaveLab's script manager and can be bound to a shortcut.
3. **The Batch Processor and its plug-in chains**, started from the UI or a shortcut, for the same operation over many files. Set it up by hand once with the person watching; reuse the saved batch.
4. **Shortcuts and `Desktop` clicks**, last. **Do not assume a default shortcut**: WaveLab lets the person remap them (File > Preferences > Shortcuts). Read the current assignments there, or from the menu text, before pressing anything. Use `zoom` on meters and small values; WaveLab's windows are dense.

## Pitfalls

- **No macro recorder.** What cannot be scripted is done in the UI, with a screenshot after each action.
- **Long operations** (render, batch) take time: wait, look, check the progress and the output file; do not click again.
- **Check bitness.** A 64-bit script host against a 32-bit install (or the reverse) is the usual cause of the "can't create object" error.
- **Never overwrite the source file.** Save a copy first (a new name), process the copy, compare.
- **Playback is audible.** Say so before you play anything.
- Never type a licence code or an account password; ask the person.
- `Desktop` needs the person's per-app grant for WaveLab like any other app, and the grant prompt comes first. A pack never grants it.

## Verify after each step

| Step | Cue that it worked |
|---|---|
| COM connected | the script prints the application's name or version (from the documented property) |
| File opened | the file appears in WaveLab's window title or its file list; the sample rate and length match `ffprobe` or Python's `wave` |
| Processed | the output file exists with a new modified time and size, and the source file is unchanged |
| Level result | read the output with Python's `wave` module and compute the peak; compare with what you asked for |
| Closed | no WaveLab process you started remains |

## Proof

`test.json` is the pack's test. Verified on: none yet.
