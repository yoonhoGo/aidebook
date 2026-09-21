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
  M1 adds an explicitly selected, read-only Obsidian vault adapter, and M2
  adds a selected-scope GitHub read-only adapter with credential-store
  boundaries, and M4 adds an authenticated local Core IPC owner plus CLI/MCP
  transports. G1 adds a separately rebuilt SQLite document graph with
  provenance, snapshot/link freshness digests, and bounded traversal; it does
  not alter canonical user relations. G2 adds persistent observations and a
  human-review candidate queue; only an explicit Tauri review command can
  promote a candidate into canonical memory. G3 adds the bounded
  `context.query.v1` packet and review-safe observation/candidate IPC, CLI, and
  MCP methods. G4 adds deterministic text-only Markdown exchange: imports are
  proposed candidates and never write an external vault.

The first UI slice mirrors the planned focus: work contexts on the left,
memory and activity in the centre, and source evidence on the right. Work
context navigation, note creation/editing, revision restore, search, source
scope dialogs, explicit native vault/repository selection and refresh, settings,
and keyboard shortcuts are functional. Existing notes, activities, settings,
and session state remain in local webview storage; in the native runtime,
memory writes are committed to Core first and mirrored into that UI state.

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
M5 now provides a local SQLite/FTS5 core, deterministic connector fixtures,
explicitly selected read-only Obsidian/GitHub source adapters, and a Core-backed
memory UI path with explicit localStorage import. The React UI keeps existing
localStorage data unless the user chooses import; browser fallback saves are
labelled as demos. Markdown exchange is available from the native settings
review panel; browser copy/paste remains a fixture/demo boundary. External
writes, remote webhook infrastructure, real account
smoke, packaged distribution, and native app verification remain staged
boundaries. CLI/MCP use the authenticated Core owner and never open SQLite.
Backups, restore integrity checks, cache-only deletion (including derived graph
builds), accessibility behavior,
and a local arm64 package helper are implemented; release signing and public
Cask publication are intentionally not performed.
See
[`docs/CORE_CONTRACT.md`](docs/CORE_CONTRACT.md) and
[`docs/IMPLEMENTATION_STATUS.md`](docs/IMPLEMENTATION_STATUS.md) for the
contract and verification boundary.
