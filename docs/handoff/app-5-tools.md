# HANDOFF: app-5-tools (phase 5 of the macOS Rust port: the ShareX tools)

Worktree `/Users/marcinasdfg/Desktop/modlight/_wt/app-5-tools`, branch `app-5-tools` (from
main `d665cb6`), everything under `app/`. Never pushed. Rules: `app/docs/PORT.md`.
Scratch: `/Users/marcinasdfg/Desktop/modlight/_scratch/app-5-tools/`.

## Layout

- `crates/modlight-tools`: the tools without windows (colours, formats, settings types).
- `crates/modlight-app/src/tools/`: the tool windows (`mod.rs` hosts them, `jobs.rs` is
  `TaskHelpers.ExecuteJob` for the tools, `kit.rs` the drawing pieces, `qa.rs` the QA
  screens).
- Wiring in shared files (kept small): `lib.rs` (`pub mod tools`), `state.rs` (field
  `tools`), `app.rs` (`tools::viewports`), `capture_flow.rs` (`execute_job` falls back to
  `run_tool_job`), `qa.rs` (screen list and the three hooks), core `tasks/settings.rs`
  (`TaskSettings.tools_settings`, the raw `ToolsSettings`), core `build.rs` (string tables
  of ShareX.Tools and ShareX.HistoryLib).

## State

In progress; see the commit log.
