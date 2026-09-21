# 구현 상태

기준: `docs/ROADMAP.md`와 두 원문 기획 노트의 구체적인 MVP 설계를 우선했다.
이 문서는 단계별 구현·검증 경계를 기록하며, native/live 경계는 별도로 표시한다.

## M0 — 완료

- Rust `Core` 공통 모델/API와 구조화 `CoreError` 추가
- SQLite transaction 마이그레이션(v1–v7), 외래 키, bundled FTS5 snapshot 검색
- source identity 중복 제거, provider/account namespace, 기간·종류 필터
- 접근 상태와 freshness, 마지막 정상 snapshot을 보존하는 실패 기록, 연결별 SyncState
- 메모 근거·저장 이유·작성 주체·claim type 검증
- `expected_version`, `idempotency_key`, 동시 수정 충돌, 철회, revision 복원
- 읽기 전용 `ReadOnlyConnector` 계약과 Obsidian/GitHub fixture
- 임시/in-memory DB 회귀 테스트와 마이그레이션 rollback 테스트
- [CORE_CONTRACT.md](./CORE_CONTRACT.md)에 저장·IPC 경계 고정

## G1 — 구현 완료 · native/live 검증 대기 (파생 문서 그래프)

- migration v5/v6으로 canonical `relations`와 분리된 `graph_builds`,
  `graph_nodes`, `graph_edges` 투영 계층 추가
- snapshot의 정규 URL·Obsidian wikilink를 provider/account/path namespace와
  exact canonical URL 규칙으로 결정적으로 해석; title만으로 연결하거나
  모호한 target을 연결하지 않음
- extracted/explicit provenance, evidence metadata, snapshot content/link
  digest, source/target URL, deterministic build ID와 rebuild diagnostics 보존
- 접근 철회·삭제·stale hash/link/URL edge를 traversal에서 제외하고,
  `max_depth`/`max_nodes`/`max_edges` bounded traversal과 inbound edge 중복
  제거를 제공
- `cache_clear`가 snapshot/FTS와 파생 graph build를 함께 지우며 source와
  canonical memory/evidence는 보존
- graph review integration 7개와 v4 backup을 staged copy에서만 최신 schema로
  migrate하는 restore regression을 통과

G1은 완전한 Tree-sitter/코드 심볼 parser, 모델 기반 inferred edge, native
Tauri/WebGL 그래프 화면을 포함하지 않는다. fixture/in-memory에서 확인한
freshness와 접근 상태는 실제 provider, iCloud/FSEvents, native window 검증을
대신하지 않는다.

## G2 — 구현 완료 · native UI 검증 대기 (관찰·메모리 후보 review gate)

- migration v7로 `observations`와 `memory_candidates`를 추가하고
  `captured → distilled → proposed → accepted/rejected` 상태와 version,
  evidence, actor/author, reason, claim type, idempotency digest를 보존
- `observation_capture`, `candidate_distill`, `candidate_propose`,
  `candidate_accept`, `candidate_reject`, 조회/list Core API와 Tauri review
  commands를 추가
- accepted 승격은 기존 memories/memory_revisions/memory_evidence를 재사용하는
  단일 SQLite transaction으로 실행하며, candidate 상태·canonical memory·두
  idempotency 기록이 함께 commit되거나 함께 rollback됨
- expected version과 상태 전이, credential marker, evidence source를 검증하고
  중복/충돌 idempotency 요청을 재생 또는 거부; rejected candidate는 canonical
  memory를 만들지 않음
- acceptance는 generic IPC/MCP tool에 노출하지 않고 trusted Tauri review
  command 경계에 둔다. observation/candidate read·distill·propose transport는
  G3에서 연결했다.

G2의 candidate 결과는 로컬 fixture/in-memory Core에서 검증했으며 자동화된
agent가 acceptance를 호출하는 경로를 제공하지 않는다. native Tauri review
화면과 live provider evidence는 G4 및 별도 native/live 검증 경계다.

## G3 — 구현 완료 · native/live 검증 대기 (bounded context query와 transport)

- `context.query.v1`가 FTS5 lexical source, canonical memory, graph traversal를
  한 응답으로 묶고 provider/kind, freshness, max-age, evidence
  unavailable/stale, missing provider와 conflict diagnostics를 반환
- 명시적인 root가 없을 때 첫 lexical source가 memory 검색 범위를 제한하지
  않으며, cache clear·새 graph node·revoked/deleted source도 전체 packet을
  실패시키지 않고 상태 배열/diagnostics로 보고
- query length, depth/node/edge/source/memory bounds를 검증하고 source 또는
  memory limit 도달도 `bounds.truncated`로 표시; graph expansion은 filter와
  동일한 provider/kind namespace를 적용
- 기존 여섯 IPC method를 유지한 채 `context.query.v1`, observation
  capture/get, candidate distill/propose/get/list를 authenticated IPC,
  `aidebook-cli`, MCP stdio, Tauri command에 연결
- `candidate.accept`와 `candidate.reject`는 generic IPC/MCP allowlist와 MCP
  `tools/list`에서 제외하고 trusted Tauri review command에만 유지

G3 context review 6개와 M4 IPC/MCP integration 1개, Rust unit 11개가 통과했다.
CLI/MCP transport와 fixture/in-memory Core는 검증했지만 packaged external MCP
host, native Tauri window, live provider는 여전히 별도 검증 경계다.

## M1 — 구현 완료 · native 검증 대기 (선택된 Obsidian vault)

- 사용자가 선택한 한 경로만 canonicalize하여 읽는 `ObsidianAdapter` 추가
- Markdown title/body와 허용 목록 frontmatter만 색인하고 wikilink·URL을 명시적
  source link로 보존
- 외부 경로·비 Markdown 파일 접근 거부, vault 밖으로 나가는 symlink 무시
- iCloud placeholder·conflicted copy를 검색 불가 snapshot으로 표시하되 목록과
  상태는 보존
- content hash 기반 add/modify/remove 및 available/unavailable 변화 계산과
  재현 가능한 polling watcher 추가
- Tauri `vault_select`/`vault_scan` command가 선택 adapter와 polling watcher를
  보관하고 Core의 동일 refresh 경로에 해시 변경 목록을 연결
- 네이티브 연결 화면의 사용자가 vault 경로를 직접 입력해 선택·스캔하고,
  브라우저에서는 실제 경로를 요청하지 않는 경계를 표시
- 임시 vault에서 무쓰기·범위 제한·메타데이터 필터·링크·iCloud 상태·검색 회귀 검증

M1의 watcher는 재현 가능한 polling 구현이다. native FSEvents와 실제 iCloud
다운로드 상태는 아직 검증하지 않았다.

## M2 — 구현 완료 · live 검증 대기 (선택된 GitHub repository)

- `GitHubConfig`가 account/connection/owner/repository를 하나의 명시적 scope로
  검증하고 경로 주입·범위 이탈을 거부
- `GitHubAdapter`가 이슈·pull request·댓글·상태·labels를 읽기 전용 snapshot으로
  변환하고 page cursor를 순회
- `CredentialStore` 경계와 in-memory fixture store 추가; macOS에서는
  `security` Keychain 명령의 stdin prompt로만 credential을 읽고 저장하며
  token을 argv·로그·SQLite에 두지 않음
- 401/403/404/429/5xx·network·malformed 응답을 구조화 오류로 분류
- `github_select`/`github_credential_set`/`github_refresh` Tauri command를 통해
  사용자의 명시적 선택·저장·수동 refresh만 허용
- 네이티브 연결 화면에서 owner/repository를 직접 선택하고 빈 token은 기존
  Keychain 조회로 남기며, GitHub 연결 해제와 credential 삭제를 분리
- fixture 페이지·권한 거부·누락 credential·페이지 순회 테스트와 실패 시
  마지막 성공 `fetched_at`/snapshot 보존 테스트 통과

실제 GitHub 계정·토큰·macOS Keychain·네트워크 curl refresh는 실행하지 않았다.
라이브 transport는 수동 선택 경로에서만 동작하도록 구현했으며 fixture 증거가
실계정 smoke를 대신하지 않는다.

## M3 — 구현 완료 · UI/native 검증 대기 (관계·Core 메모 UI 경계)

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

## M4 — 구현 완료 · native/protocol-host 검증 대기 (단일 Core owner · authenticated IPC · CLI/MCP)

- Tauri setup이 앱 데이터 디렉터리에서 Core를 한 번 열고, 같은 프로세스의
  Unix socket `CoreServer`를 owner로 시작; `Arc<Database>` 외부 직접 open 경로 없음
- socket/lock/token 파일을 `0600`으로 만들고 `create_new` lock과 stale socket
  검사로 복수 owner를 거부
- bearer token과 constant-time 비교, 구조화 `unauthenticated`/owner 오류,
  SQLite·SQL path를 받지 않는 line-delimited JSON IPC 구현
- `aidebook-cli`가 기존 six method와 versioned context/candidate transport를
  호출하고 성공 JSON은 stdout, 오류 JSON은 stderr에 출력;
  `aidebook-core` standalone owner도 제공
- `mcp serve --stdio`가 initialize, tools/list, tools/call과 동일 13개 tool을
  노출하며 tool 결과·오류를 Core IPC로 전달
- IPC integration test에서 direct Core/CLI client/MCP tool 결과 일치, fixture
  refresh, 잘못된 token 거부, legacy method 호환과 13개 tool count를 검증
- Tauri `WindowEvent::Destroyed`가 Core server stop flag를 설정해 socket/token/lock
  정리를 요청하며, 실제 native 창 종료·재시작 smoke는 별도로 남김
- arm64 staged `aidebook-core` + `aidebook-cli`를 임시 DB/socket으로 실행해
  구조화된 stderr 오류와 MCP `tools/list` 13개 응답(accept/reject 미노출)을 확인
- standalone owner를 강제 종료한 뒤 stale PID lock/socket을 회수하고 같은
  endpoint로 재시작하는 local smoke도 통과

실제 패키지 바이너리를 외부 MCP host에 연결하거나 macOS 창 종료·재시작,
Keychain/실계정 provider와 함께 실행하는 native smoke는 아직 검증하지 않았다.

## M5 — 구현 완료 · release/native 검증 대기 (보호·접근성·arm64 local package)

- `Core::backup_to`는 SQLite `VACUUM INTO` 임시 파일과 integrity check 후
  명시된 새 경로로 commit; `restore_from`은 corrupt backup 실패 시 원래 DB를
  보존하고 검증된 backup만 교체
- `cache_clear`는 snapshots/FTS만 삭제하고 sources·memory·evidence를 보존;
  GitHub disconnect의 credential 삭제는 별도 명시 옵션
- UI에 백업·복원·캐시만 삭제 control을 추가하고 browser에서는 네이티브 작업을
  실행하지 않으며 localStorage를 자동 삭제하지 않음
- existing 44px controls, visible focus outline, keyboard splitter, semantic
  labels/fieldset, `prefers-reduced-motion`와 in-app reduce-motion CSS를 보존
- `scripts/package-arm64.sh`가 `aidebook`, `aidebook-cli`, `aidebook-core`를
  `aarch64-apple-darwin` local staging directory에 생성; Cask는 placeholder
  template만 제공하며 signing/notarization/tap publish를 수행하지 않음
- deterministic FTS benchmark: seed `20260919`, seed count `10000`, warmup
  `5`, measured `30`, p95 `24.509ms`, environment `macos/aarch64`, rustc
  `1.94.1`, parallelism `12`; reproducible command는 아래 표에 기록

## 검증 기록

| 명령 | 결과 |
| --- | --- |
| `npm run build` | 통과: `tsc` + Vite production build |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 통과: Rust unit 11개, graph integration 7개, candidate lifecycle/review integration 4개, context review 6개, M0 integration 6개, M1 integration 1개, M2 integration 2개, M3 integration 3개, M4 integration 1개, M5 storage 4개, M5 benchmark 1개 |
| `cargo check --manifest-path src-tauri/Cargo.toml` | 통과 |
| `git diff --check` | 통과 |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` | 통과 |
| `cargo test --manifest-path src-tauri/Cargo.toml --test m5_benchmark -- --nocapture` | 통과: seed=20260919, n=10000, warmup=5, runs=30, p95=24.509ms, macOS aarch64 |
| `AIDEBOOK_PACKAGE_DIR=/tmp/aidebook-arm64-package scripts/package-arm64.sh` | 통과: arm64 local binaries 3개 staged; 서명/공증/릴리스 미실행 |
| staged `aidebook-core` + `aidebook-cli` local smoke | 통과: `connections.status` structured stderr, MCP `tools/list` 13개(accept/reject 미노출); third-party host/native window 미검증 |

## 검증하지 않은 경계

- 실제 macOS Tauri 실행, native SQLite app-data 경로, UI persistence 연결
- 실제 Obsidian vault/iCloud download·충돌·FSEvents watcher
- 실제 GitHub 계정·토큰·Keychain·권한 철회
- 외부 MCP host가 실행한 packaged CLI/MCP stdio와 macOS 창 종료·재시작
- 실제 10,000건 benchmark를 제외한 사용자 데이터 규모·실계정 환경의 성능
- arm64 패키징은 local binary staging까지 검증했으며 서명·공증·Homebrew Cask 설치는 미검증

fixture 및 in-memory 테스트는 위 native/live 경계를 대신하지 않는다. 현재
React UI의 `localStorage` 메모와 아이콘은 보존했으며 자동 migration이나 삭제를
하지 않는다. provider는 계정·경로를 자동 발견하지 않고 사용자가 명시한 선택
경로에서만 시작한다.

## 다음 작업

M5까지 로컬 구현은 완료했지만 위 native/live/release 검증이 남아 있어 로드맵의
최종 end-to-end acceptance 완료로 표시하지 않는다. 남은 것은
signing/notarization, public release asset/Cask ownership, 실계정·iCloud·native
window 및 외부 MCP host smoke다.

## jj 기록

로드맵 변경 `sqonnulz` 위에 M0 구현 `sxonqxql`/`yptolqwy`, M1 구현
`mqkpvwql`, M2 구현 `qswsrmtk`, M3 구현 `lqxykmkx`, M4 구현 `svykzmml`,
M5 구현 `ympmpyot`을 순서대로 기록한다. main 이력·원격·push는 건드리지
않았다.
