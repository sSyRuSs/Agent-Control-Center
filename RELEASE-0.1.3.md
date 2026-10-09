# Agent Control Center 0.1.3

Built on Windows x64, 2026-10-02.

## Run

- `Agent-Control-Center-0.1.3-x64-setup.exe`: Windows installer.
- `Agent-Control-Center-0.1.3-x64.exe`: standalone executable; requires WebView2.
- Source: `Agent-Control-Center/` beside this report.

## Model settings

Open **Model settings** in the sidebar. Select a model and reasoning effort, then click **Save role** for Planner, Coder, Mobile Dev, Tester, or Reviewer. Selecting an option alone does not write configuration.

Choices are read from the real local `CODEX_HOME/models_cache.json` catalog. This cache does not verify account entitlement. Planner edits the global default in `config.toml`; other roles are matched by their `name` in `agents/*.toml`. Only model and reasoning effort fields are changed. Each changed save creates an exact backup beside the original file. External edits invalidate the displayed revision and require Reload. Reload discards drafts.

Defaults apply to newly created sessions; existing agents are not switched. Project configuration and explicit task overrides can take precedence. Planner's global default can affect sessions outside Software Team. Research Team settings are not edited.

## Real data and limits

Monitoring continues to read actual local Codex session JSONL events and persist activity metadata in SQLite. This release uses the local-log fallback, not a live App Server transport. Missing model/token/task metadata remains unavailable instead of fabricated. No pause, stop, or steer controls are provided. Model settings are an explicit configuration-writing feature separate from monitoring.

## Verification

- Independent Tester: Rust library tests 15 passed; TypeScript typecheck and Vite production build passed.
- Backend checks cover original backup bytes, inline TOML comments, unrelated root/second-role files unchanged, revision conflicts, invalid catalog model, role allowlist, and actual catalog default format.
- Independent Reviewer: approved after fixing cross-role revision refresh; saving one role preserves every other role's baseline/revision and draft until Reload.
- Native `npm run tauri build`: passed; release executable and NSIS installer produced.
- Windows process launch with isolated test `CODEX_HOME`: passed; process responded and exposed the application window.
- Interactive model-selection/save smoke test remains unverified: Windows was locked. No UI save was attempted.
- SHA-256 comparison confirms the real global config and four Software Team agent config files were unchanged during development and testing.

## Develop and build

From `Agent-Control-Center/` with Node, Rust, MSVC Build Tools and WebView2 installed:

```powershell
npm install
npm run tauri dev
npm run typecheck
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --lib --offline
npm run tauri build
```

Key files: `src/ModelSettings.tsx`, `src/model-settings.css`, `src/main.tsx`, `src-tauri/src/settings.rs`, `src-tauri/src/lib.rs`, and `src-tauri/tauri.conf.json`.

## SHA-256

Installer: `9CABC476C48B757AC09CC098492C7E4D374DBF3021DA1A730B604C276D578DF6`

Standalone: `1F7C82455CE7845EF49CAC40017914691B50E3918CAF3DA4A2524FDF5A7DDA00`
