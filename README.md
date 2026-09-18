# Aidebook

**A notebook for your AI assistant.**

Aidebook is a macOS-first desktop app that keeps an AI assistant's working
context locally: memories, source references, snapshots, and the relationships
between them. The product plan starts with read-only connector access and
user-reviewable in-app memories.

## Project shape

- `src/` — React and TypeScript desktop UI.
- `src-tauri/` — Tauri 2 host and Rust application core.
- `src-tauri/src/core/` — the first shared-core boundary; persistence,
  connector adapters, CLI, and MCP will build on this boundary.

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
cargo check --manifest-path src-tauri/Cargo.toml
```

This repository uses [Jujutsu](https://martinvonz.github.io/jj/) colocated
with Git. Use `jj status`, `jj diff`, and `jj log` for change management.

## Scope notes

The product plan is kept in the Obsidian vault as `비서의 노트 기획서.md`.
The current implementation intentionally leaves external writes, remote
webhook infrastructure, provider credentials, and native SQLite persistence
out of the project. GitHub and Obsidian content in the UI is clearly marked as
cached example data until read-only adapters are added.
