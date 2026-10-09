param(
    [string]$ResearchWorkspace = 'C:\Users\ADMIN\Documents\Codex\2026-09-19\referenced-chatgpt-conversation-this-is-an\outputs\Research-Team'
)

$monitorExe = Join-Path $PSScriptRoot 'src-tauri\target\release\agent-control-center.exe'
if (-not (Test-Path -LiteralPath $monitorExe)) {
    throw "Agent Control Center is not built. Run: npm run tauri build"
}
if (-not (Test-Path -LiteralPath $ResearchWorkspace)) {
    throw "Research workspace not found: $ResearchWorkspace"
}

$env:AGENT_CONTROL_RESEARCH_WORKSPACE = (Resolve-Path -LiteralPath $ResearchWorkspace).Path
Start-Process -FilePath $monitorExe
