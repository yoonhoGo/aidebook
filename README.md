# Aidebook

**A notebook for your AI assistant.**

Aidebook is a macOS-first desktop app that keeps an AI assistant's working
context locally: memories, source references, snapshots, and the relationships
between them. The product plan starts with read-only connector access and
user-reviewable in-app memories.

## Project shape

- `src/` — React and TypeScript desktop UI.
- `src-tauri/` — Tauri 2 host and Rust application core.
- `src-tauri/src/core/` — the shared Rust core boundary. M0 includes SQLite,
  FTS5, versioned local memories, sync state, and read-only fixture adapters;
  M1 adds an explicitly selected, read-only Obsidian vault adapter. GitHub,
  CLI, and MCP remain later staged boundaries.

The first UI slice mirrors the planned focus: work contexts on the left,
memory and activity in the centre, and source evidence on the right. Work
context navigation, note creation/editing, revision restore, search, source
scope dialogs, settings, and keyboard shortcuts are functional. Notes,
activities, settings, and the current session are persisted in the local
webview storage for now.

## Development

```sh
npm install
npm run dev
npm run tauri dev
```

Validation commands:

```sh
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
git diff --check
```

This repository uses [Jujutsu](https://martinvonz.github.io/jj/) colocated
with Git. Use `jj status`, `jj diff`, and `jj log` for change management.

## Scope notes

The product plan is kept in the Obsidian vault as `비서의 노트 기획서.md`.
M1 now provides a local SQLite/FTS5 core, deterministic connector fixtures, and
an explicitly selected read-only Obsidian vault scan. The React UI intentionally
keeps its existing `localStorage` persistence until its explicit migration and
save boundary is implemented. External writes, remote webhook infrastructure,
provider credentials, real account access, CLI/MCP processes, and native app
verification remain staged boundaries. See
[`docs/CORE_CONTRACT.md`](docs/CORE_CONTRACT.md) and
[`docs/IMPLEMENTATION_STATUS.md`](docs/IMPLEMENTATION_STATUS.md) for the
contract and verification boundary.
