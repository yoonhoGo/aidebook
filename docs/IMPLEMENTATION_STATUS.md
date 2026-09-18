# 구현 상태

기준: `docs/ROADMAP.md`와 두 원문 기획 노트의 구체적인 MVP 설계를 우선했다.
이 문서는 단계별 구현·검증 경계를 기록하며, native/live 경계는 별도로 표시한다.

## M0 — 완료

- Rust `Core` 공통 모델/API와 구조화 `CoreError` 추가
- SQLite transaction 마이그레이션(v1–v3), 외래 키, bundled FTS5 snapshot 검색
- source identity 중복 제거, provider/account namespace, 기간·종류 필터
- 접근 상태와 freshness, 마지막 정상 snapshot을 보존하는 실패 기록, 연결별 SyncState
- 메모 근거·저장 이유·작성 주체·claim type 검증
- `expected_version`, `idempotency_key`, 동시 수정 충돌, 철회, revision 복원
- 읽기 전용 `ReadOnlyConnector` 계약과 Obsidian/GitHub fixture
- 임시/in-memory DB 회귀 테스트와 마이그레이션 rollback 테스트
- [CORE_CONTRACT.md](./CORE_CONTRACT.md)에 저장·IPC 경계 고정

## M1 — 완료 (선택된 Obsidian vault)

- 사용자가 선택한 한 경로만 canonicalize하여 읽는 `ObsidianAdapter` 추가
- Markdown title/body와 허용 목록 frontmatter만 색인하고 wikilink·URL을 명시적
  source link로 보존
- 외부 경로·비 Markdown 파일 접근 거부, vault 밖으로 나가는 symlink 무시
- iCloud placeholder·conflicted copy를 검색 불가 snapshot으로 표시하되 목록과
  상태는 보존
- content hash 기반 add/modify/remove 및 available/unavailable 변화 계산과
  재현 가능한 polling watcher 추가
- Tauri `vault_select`/`vault_scan` command가 Core의 동일 refresh 경로를 사용
- 임시 vault에서 무쓰기·범위 제한·메타데이터 필터·링크·iCloud 상태·검색 회귀 검증

M1의 watcher는 재현 가능한 polling 구현이다. native FSEvents와 실제 iCloud
다운로드 상태는 아직 검증하지 않았다.

## M2 — 완료 (선택된 GitHub repository)

- `GitHubConfig`가 account/connection/owner/repository를 하나의 명시적 scope로
  검증하고 경로 주입·범위 이탈을 거부
- `GitHubAdapter`가 이슈·pull request·댓글·상태·labels를 읽기 전용 snapshot으로
  변환하고 page cursor를 순회
- `CredentialStore` 경계와 in-memory fixture store 추가; macOS에서는
  `security` Keychain 명령으로만 credential을 읽고 저장
- 401/403/404/429/5xx·network·malformed 응답을 구조화 오류로 분류
- `github_select`/`github_credential_set`/`github_refresh` Tauri command를 통해
  사용자의 명시적 선택·저장·수동 refresh만 허용
- fixture 페이지·권한 거부·누락 credential·페이지 순회 테스트와 실패 시
  마지막 성공 `fetched_at`/snapshot 보존 테스트 통과

실제 GitHub 계정·토큰·macOS Keychain·네트워크 curl refresh는 실행하지 않았다.
라이브 transport는 수동 선택 경로에서만 동작하도록 구현했으며 fixture 증거가
실계정 smoke를 대신하지 않는다.

## M3 — 완료 (관계·Core 메모 UI 경계)

- 관계는 SourceRef의 명시 URL/외부 ID와 이유로만 추가하며 `relation_remove`로
  해제; 제목 일치만으로 병합하지 않음
- 1-hop 명시 관계를 기반으로 `context_get` WorkContext를 파생하고, 접근 불가
  근거는 `unavailable_sources`로 분리하면서 사용자 메모 본문은 보존
- `ui_memories` v4 마이그레이션이 Core Memory의 title/work/kind를 연결하고
  멱등성·expected_version·철회·복원 이력을 공유
- Tauri `ui_memory_upsert`/`retract`/`restore`/`list` command와 UI 근거 선택,
  저장 이유, 작성 주체, inferred claim type 연결
- UI는 Core commit 성공 뒤에만 네이티브 성공을 표시하고, 브라우저 fallback은
  “브라우저 데모 저장”으로 명확히 표시
- localStorage 메모 → Core 가져오기는 설정에서 사용자가 누르는 명시적 작업이며
  원본 localStorage를 삭제하지 않음
- 제목 중복 비병합, 관계 해제, 10개 UI 메모 저장, 멱등성/버전 충돌, 접근 불가
  evidence와 사용자 메모 보존을 integration test로 검증

## 검증 기록

| 명령 | 결과 |
| --- | --- |
| `npm run build` | 통과: `tsc` + Vite production build |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 통과: Rust unit 6개, M0 integration 6개, M1 integration 1개, M2 integration 2개, M3 integration 3개 |
| `cargo check --manifest-path src-tauri/Cargo.toml` | 통과 |
| `git diff --check` | 통과 |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` | 통과 |

## 검증하지 않은 경계

- 실제 macOS Tauri 실행, native SQLite app-data 경로, UI persistence 연결
- 실제 Obsidian vault/iCloud download·충돌·FSEvents watcher
- 실제 GitHub 계정·토큰·Keychain·권한 철회
- CLI/MCP stdio 프로세스와 인증된 local IPC
- 10,000건 검색 p95 300ms 목표의 실제 측정
- arm64 패키징, 서명·공증, Homebrew Cask 설치

fixture 및 in-memory 테스트는 위 native/live 경계를 대신하지 않는다. 현재
React UI의 `localStorage` 메모와 아이콘은 보존했으며 자동 migration이나 삭제를
하지 않는다. provider는 계정·경로를 자동 발견하지 않고 사용자가 명시한 선택
경로에서만 시작한다.

## 다음 작업

M4에서 단일 Core
소유 프로세스와 인증된 local IPC/CLI/MCP를 추가한다. M5에서 백업·복구,
접근성·arm64 패키징 템플릿·재현 가능한 10,000건 p95 측정을 마무리한다.

## jj 기록

로드맵 변경 `sqonnulz` 위에 M0 구현 `sxonqxql`/`yptolqwy`, M1 구현
`mqkpvwql`, M2 구현 `qswsrmtk`, M3 구현 `lqxykmkx`를 순서대로 기록한다.
각 단계는 다음 단계의 빈 child change에서 계속하며 main 이력·원격·push는
건드리지 않았다.
