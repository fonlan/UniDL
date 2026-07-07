# aria2 Startup Error Reporting

## Goal

When UniDL launches a local aria2c process and that process exits before its RPC endpoint becomes ready, return aria2c's original stderr to the caller instead of the generic RPC-unreachable error.

## Design

- Start aria2c with stderr piped and stdout discarded as before.
- Pass the spawned child process into the existing RPC readiness loop.
- During each readiness attempt, check whether the child exited.
- If it exited, read stderr to completion, trim only surrounding whitespace, log it, and return it unchanged as the user-visible error.
- If stderr is empty, retain an exit-status error so startup never fails silently.
- If the process remains alive, preserve the current RPC polling and timeout behavior.

## Scope

Only `src-tauri/src/engine_adapters/aria2.rs` changes. No UI changes, dependencies, automatic port selection, or aria2 argument changes are included.

## Verification

Add one focused Rust test covering an aria2-like child process that exits with stderr, then run that test, `cargo fmt`, and `cargo check`.
