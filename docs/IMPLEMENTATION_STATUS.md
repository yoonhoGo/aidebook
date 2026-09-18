# M0 구현 상태

기준: `docs/ROADMAP.md`의 “첫 실행 작업: M0”, 두 원문 기획 노트의 구체적인
MVP 설계 우선. 이 문서는 전체 MVP 완료 보고서가 아니다.

## 완료한 M0 산출물

- Rust `Core` 공통 모델/API와 구조화 `CoreError` 추가
- SQLite v1/v2 transaction 마이그레이션과 외래 키
- SQLite bundled FTS5 snapshot 색인 및 검색
- source identity 중복 제거, provider/account namespace, 기간·종류 필터
- 접근 상태(`accessible`, `permission_denied`, `not_found`, `unavailable`)와
  freshness(`fresh`, `stale`, `unavailable`)
- 마지막 정상 snapshot을 보존하는 실패 기록 및 연결별 SyncState
- 메모 근거·저장 이유·작성 주체·claim type 검증
- `expected_version`, `idempotency_key`, 동시 수정 충돌, 철회, revision 복원
- 읽기 전용 `ReadOnlyConnector` 계약과 Obsidian/GitHub 정상·중복·실패·권한
  거부 fixture
- 임시/in-memory DB 회귀 테스트와 마이그레이션 rollback 테스트
- [CORE_CONTRACT.md](./CORE_CONTRACT.md)에 저장·IPC 경계 고정

## 검증 기록

아래는 M0 변경 후 실행한 명령이다.

| 명령 | 결과 |
| --- | --- |
| `npm run build` | 통과: `tsc` + Vite production build |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 통과: Rust unit 1개, M0 integration 5개 |
| `cargo check --manifest-path src-tauri/Cargo.toml` | 통과 |
| `git diff --check` | 통과 |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` | 통과 |

## 검증하지 않은 경계

- 실제 macOS Tauri 실행, native SQLite app-data 경로, UI persistence 연결
- 실제 Obsidian vault/iCloud download·충돌·file watcher
- 실제 GitHub 계정·토큰·Keychain·권한 철회
- CLI/MCP stdio 프로세스와 인증된 local IPC
- 10,000건 검색 p95 300ms 목표의 실제 측정
- arm64 패키징, 서명·공증, Homebrew Cask 설치

fixture 및 in-memory 테스트는 위 native/live 경계를 대신하지 않는다. 현재
React UI의 `localStorage` 메모와 아이콘은 보존했으며 M0에서 자동 migration이나
삭제를 하지 않는다.

## 다음 작업

M1에서 사용자가 명시한 Obsidian vault 한 곳을 읽기 전용으로 스캔하고,
허용 frontmatter·wikilink·URL·해시 재검사·Tauri 검색 command를 추가한다.
M2에서 선택 GitHub 저장소와 Keychain 경계를 검증한다. M3 이전까지 실제
provider 연결을 자동으로 시작하지 않는다.

## jj 기록

로드맵 변경 `sqonnulz` 위의 구현 change에서 작업했으며, 최종 change ID는
jj 작업 단위를 마친 뒤 아래에 기록한다.
