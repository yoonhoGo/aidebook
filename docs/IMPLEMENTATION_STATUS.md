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

## 검증 기록

| 명령 | 결과 |
| --- | --- |
| `npm run build` | 통과: `tsc` + Vite production build |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 통과: Rust unit 3개, M0 integration 6개, M1 integration 1개 |
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

M2에서 사용자가 선택한 GitHub 저장소의 페이지 순회·Keychain 경계·구조화 오류를
구현한다. M3에서 명시적 관계와 메모 UI 저장 경계를 연결하고, M4에서 단일 Core
소유 프로세스와 인증된 local IPC/CLI/MCP를 추가한다. M5에서 백업·복구,
접근성·arm64 패키징 템플릿·재현 가능한 10,000건 p95 측정을 마무리한다.

## jj 기록

로드맵 변경 `sqonnulz` 위에 M0 구현 `sxonqxql`/`yptolqwy`, M1 구현
`mqkpvwql`을 순서대로 기록한다. 각 단계는 다음 단계의 빈 child change에서
계속하며 main 이력·원격·push는 건드리지 않았다.
