# Local Core IPC

M4는 Tauri desktop 프로세스를 단일 Core owner로 사용한다. Tauri setup에서
`Core::open`을 한 번 수행하고, 같은 프로세스의 `CoreServer`가 Unix socket을
소유한다. CLI와 MCP stdio는 SQLite 파일·Keychain·provider를 직접 열지 않고
이 socket의 인증된 request만 보낸다.

```text
Tauri window ───────────────┐
CLI JSON stdout/stderr ──────┼─ authenticated Unix socket ── Core owner ── SQLite/FTS5
MCP JSON-RPC stdio ──────────┘
```

## Endpoint and authentication

The owner creates these exact files in the app data directory:

- `aidebook-core.sock` — Unix domain socket, mode `0600`
- `aidebook-core.token` — random per-owner bearer credential, mode `0600`
- `aidebook-core.lock` — `create_new` owner lock, mode `0600`

An existing live socket is never replaced. The lock records the owner PID so a
dead owner can be recovered after an unclean exit; a live PID still causes an
`owner_exists` error. A stale socket can only be removed after acquiring the
exclusive lock. The token is compared without logging or returning it. Invalid
credentials return a structured `unauthenticated` error.

## Request/response

Each connection sends one UTF-8 JSON line:

```json
{
  "id": "request-id",
  "token": "owner-token",
  "method": "context.search",
  "params": {"query":"release","limit":20}
}
```

The response has exactly one of `result` or `error`:

```json
{"id":"request-id","result":{"results":[]}}
{"id":"request-id","error":{"code":"invalid_input","message":"...","details":{}}}
```

The six legacy methods are `context.search`, `context.get`, `memory.upsert`,
`memory.retract`, `sources.refresh`, and `connections.status`; their request and
response semantics remain unchanged. The versioned `context.query.v1` read
method combines bounded lexical, memory, and graph evidence. The review-safe
observation/candidate methods are `observation.capture`, `observation.get`,
`candidate.distill`, `candidate.propose`, `candidate.get`, and `candidate.list`.
They let a local agent capture and propose evidence-backed candidates for a
human review queue. `candidate.accept` and `candidate.reject` are deliberately
absent from generic IPC/MCP and are available only through trusted Tauri review
commands. `sources.refresh` requires an explicit fixture path when called
through generic IPC; the Tauri command supplies the selected read-only adapter
directly. No method accepts a database path or arbitrary SQL.

## CLI and MCP

`aidebook-cli` reads `AIDEBOOK_IPC_SOCKET` and `AIDEBOOK_IPC_TOKEN_FILE`, or
accepts `--socket` and `--token-file` together. Successful command data is JSON
on stdout; structured errors and diagnostics are JSON on stderr. `memory` calls
use `--params` so evidence, idempotency, and expected-version fields stay
explicit.

`aidebook-cli mcp serve --stdio` implements MCP JSON-RPC initialize,
`tools/list`, and `tools/call`; every listed tool maps one-to-one to the same
Core method. Context queries use `context query --query ...` or a JSON
`--params` object. Candidate mutations use JSON `--params` so evidence,
idempotency, and expected-version fields stay explicit. MCP tool errors are
returned as `isError: true` content and never include the token. Candidate
acceptance and rejection do not appear in `tools/list`.

The local protocol and fixture client smoke are tested. Actual installed CLI
launch from a packaged app, a third-party MCP host, native window-close/restart,
and macOS permission prompts remain native verification items.
