# Aidebook local core contract

이 문서는 `비서의 노트 MVP 설계.md`의 구체적인 MVP 규칙을 Rust 코어에
고정한다. M0의 코어는 UI·CLI·MCP가 함께 사용할 transport-neutral API와
SQLite/FTS5 저장소를 제공한다. 아직 실제 볼트 스캔, GitHub 계정 연결,
Keychain, CLI 프로세스, MCP stdio 서버를 시작하지 않는다.

## 소유권과 경계

```text
Tauri window ───────────────┐
CLI JSON / MCP stdio ────────┼─ authenticated local IPC ── Core owner ── SQLite + FTS5
                             └─ (no direct DB access)       ├─ snapshots (cache)
                                                           └─ memories (user-owned)
```

M4부터 Tauri desktop 프로세스가 하나의 `Core`와 IPC owner를 함께 소유한다.
CLI/MCP는 `docs/IPC_PROTOCOL.md`의 socket client이며 DB를 직접 열 수 없다.
`Core`만 DB를 열고 변경한다. UI는 현재 `localStorage`를 계속 사용하며,
M0에서 자동으로 DB로 가져오거나 기존 UI 데이터를 삭제하지 않는다. 외부
자료는 읽기 전용·비신뢰 데이터이고 외부 서비스에 쓰는 메서드나 토큰 필드는
계약에 없다. 사용자가 명시적으로 선택한 provider 범위만 adapter가 읽는다.
Tauri IPC, CLI, MCP는 이 API를 호출하는 얇은 transport이며 각각이 DB를 직접
열 수 없다.

## 공통 모델

### SourceRef와 Snapshot

`SourceRef`는 다음 다섯 값을 모두 요구한다.

- `provider`: `obsidian`, `github` 같은 공급자 namespace
- `account_id`: 볼트 또는 계정/조직 namespace
- `external_id`: 공급자에서 안정적인 ID (Obsidian은 vault-relative path)
- `url`: 정규 `http(s)`, `file`, `obsidian` URL
- `kind`: `note`, `issue`, `pull_request` 등 공급자 자료 유형

원본 identity는 `provider + account_id + external_id + kind`다. 따라서 같은
원본을 두 번 수집해도 한 `sources` 행만 남고, URL이 교정되면 기존 행의
URL만 갱신된다. 제목이 같다는 이유로 두 자료를 병합하지 않는다.

`Snapshot`은 `title`, `body`, `source_updated_at`, `fetched_at`, SHA-256
`content_hash`, `access_status`를 가진 캐시다. `accessible` snapshot만
`snapshot_fts`에 색인한다. `permission_denied`, `not_found`, `unavailable`
상태가 되면 FTS에서 제거하지만 기존 본문과 마지막 정상 `fetched_at`은
보존한다. 처음 실패한 자료도 실패 시각과 사유를 저장해 진단할 수 있다.

검색 결과는 항상 SourceRef, 원본 수정 시각, 마지막 수집 시각,
`freshness`를 포함한다. `max_age_seconds`를 지정하면 수집 시각이 기준을
넘은 결과는 `stale`이다. 접근 불가 자료는 검색 결과가 되지 않고
`context`의 `unavailable_sources`로 분리된다.

### Memory

메모에는 `body`, 저장 `reason`, 하나 이상의 `evidence` SourceRef,
`author`, `claim_type`, `version`, `idempotency_key`, 생성·수정 시각이
필수다. `claim_type`은 자유 문자열이지만 설계상 `decision`, `preference`,
`constraint`, `open_question`, `next_action`, `inferred`를 사용한다.

메모는 앱 DB 안에만 저장한다. `memory.upsert`는 새 메모를 version 1로
만들고, 기존 메모를 수정하려면 반드시 현재 `expected_version`을 보내야
한다. 모든 변경은 `memory_revisions`에 append-only로 남는다.

- 같은 `idempotency_key`와 같은 요청은 저장된 결과를 재생한다.
- 같은 키를 다른 내용으로 재사용하면 `idempotency_conflict`다.
- 잘못된 `expected_version`은 `version_conflict`이며 현재 내용을 덮지 않는다.
- 철회는 본문을 지우지 않고 새 revision과 `retracted_at`만 만든다.
- 복원은 과거 revision을 현재 값으로 복사해 새 version을 만들며 과거
  이력을 지우지 않는다.
- 근거가 없거나 저장 이유가 없으면 거부한다.
- 비밀번호·토큰으로 보이는 대표적인 credential marker는 메모에 저장하지
  않는다. 인증 정보는 이 계약과 DB에 들어오지 않는다.
- 다음 동기화가 사용자 메모의 본문을 덮어쓰는 경로는 없다. 근거의 접근
  상태만 바뀔 수 있다.

### Relation과 WorkContext

Relation은 명시적인 SourceRef 사이의 링크와 이유만 저장한다. M0의
`context`는 요청한 원본과 명시적 1-hop relation을 조회해 파생
`ContextResponse`를 만든다. 제목 유사도·벡터 추정·암묵적 병합은 하지
않는다. 접근 불가 근거는 `unavailable_sources`와 `missing_providers`에
남고, 메모 본문은 그대로 반환된다.

`relation_add`와 `relation_remove`는 URL·외부 ID·wikilink를 사용자가 명시한
경우에만 관계를 만들고 해제한다. Obsidian에서 발견한 link는 후보 metadata일
뿐 자동 관계가 아니다. UI 메모는 `ui_memories`가 title/work/kind를 Core
Memory와 함께 보존하며 Core commit 뒤에만 저장 성공으로 보고한다.

### SyncState

연결별 `last_success_at`, `last_attempt_at`, 실패 시각·코드·사유,
재시도 시각, cursor를 저장한다. 실패 기록은 이전 `last_success_at`과
cursor를 지우지 않는다. 이는 “실패했지만 이전 캐시를 최신으로 가장하지
않는다”는 규칙을 보장한다.

## 공통 API

`src-tauri/src/core/mod.rs`의 `Core`가 현재 구현한 호출은 다음과 같다.

| 계약 | Rust API | 동작 |
| --- | --- | --- |
| `context.search` | `Core::context_search(SearchRequest)` | FTS5 키워드·provider/kind·기간·freshness·limit |
| `context.get` | `Core::context_get(ContextRequest)` | 명시 SourceRef/ID와 relation, 근거 메모 |
| `memory.upsert` | `Core::memory_upsert(MemoryUpsertInput)` | 생성·수정·멱등성·근거 검증 |
| `memory.retract` | `Core::memory_retract(MemoryRetractInput)` | versioned 철회 |
| `memory.restore` | `Core::restore_memory(MemoryRestoreInput)` | revision 기반 복원 |
| `ui_memory.upsert` | `Core::ui_memory_upsert(UiMemoryUpsertInput)` | title/work/kind와 Core Memory commit |
| `sources.refresh` | `Core::sources_refresh(ReadOnlyConnector)` | 읽기 전용 snapshot ingest + SyncState |
| `connections.status` | `Core::connections_status` | 마지막 성공과 실패 상태 조회 |

검색 limit은 기본 20, 최대 50이다. 0 또는 50 초과는 구조화된
`invalid_input` 오류다. 모든 오류는 `CoreError`로 표현하며 JSON으로
`code`와 `details`를 직렬화할 수 있다.

## SQLite와 마이그레이션

DB는 `Database::open` 또는 테스트 전용 `Core::in_memory`로만 연다.
마이그레이션은 `schema_migrations`를 기준으로 각 버전을 별도 SQLite
transaction에서 적용한다.

- v1: `sources`, `snapshots`, FTS5 `snapshot_fts`, `relations`,
  `sync_states`, `memories`, `memory_evidence`
- v2: `memory_revisions`, `idempotency_records`
- v3: snapshot metadata/explicit link JSON
- v4: `ui_memories` presentation metadata linked to Core Memory

마이그레이션 SQL, version 기록, commit이 하나의 transaction에 들어가므로
실패하면 해당 version과 새 테이블이 함께 rollback된다. 외래 키를 켜며,
테스트는 임시 DB 또는 in-memory DB만 사용한다.

## 읽기 전용 어댑터와 fixture

`ReadOnlyConnector`는 `manifest`, `connection_id`, `connect`, `sync`, `fetch`,
`disconnect`만 제공한다. `write`, `execute`, token 반환 계약은 없다.
현재 `FixtureAdapter`가 실제 어댑터의 경계를 재현한다. `sources_refresh`는
그 중 읽기 전용 snapshot batch만 코어 저장소에 반영한다.

- `src-tauri/fixtures/obsidian.json`
- `src-tauri/fixtures/github.json`

각 fixture에는 정상 자료, 같은 원본의 중복, `unavailable`,
`permission_denied` 사례가 있다. fixture 테스트는 계정·볼트에 연결하지
않으며 native Keychain, iCloud hydration, 실제 GitHub 권한을 증명하지
않는다.

## 후속 IPC 경계

M1 이후 Tauri command를 추가할 때 command는 앱 데이터 디렉터리에서
`Core::open`을 한 번 관리하고 위 request/response 타입만 전달한다. M4의
CLI/MCP도 같은 코어에 인증된 로컬 IPC로 연결한다. 여러 MCP 프로세스가
SQLite 파일을 직접 열지 않도록 코어 소유 프로세스와 소켓 권한을 별도로
검증해야 한다.
