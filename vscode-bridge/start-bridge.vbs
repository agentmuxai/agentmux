Set WshShell = CreateObject("WScript.Shell")
Set fso = CreateObject("Scripting.FileSystemObject")

' Get the directory where this script is located
scriptDir = fso.GetParentFolderName(WScript.ScriptFullName)

' Change to the vscode-bridge directory
WshShell.CurrentDirectory = scriptDir

' Start node index.js in the background (hidden window)
WshShell.Run "node index.js", 0, False
