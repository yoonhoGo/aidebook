# 구현 상태

기준: `docs/ROADMAP.md`와 두 원문 기획 노트의 구체적인 MVP 설계를 우선했다.
이 문서는 단계별 구현·검증 경계를 기록하며, native/live 경계는 별도로 표시한다.

## Jira 개인 범위·Confluence — 2026-09-22

- [구현 계획](ATLASSIAN_IMPLEMENTATION_PLAN.md)에 따라 서브에이전트 3개와 주 에이전트가 구현·통합했다.
- Jira 프로젝트/내 티켓 모드, 선택적 보고자·상위 티켓 포함, 기존 프로젝트 연결 호환.
- Confluence 작성/Watch/선택 모드, 검색 후보 선택·같은 사이트 URL/ID 추가 및 본문 색인.
- 연결 편집 UI, Tauri 검색 명령, MCP `plugins.confluence.search`, CLI `plugins search` 및 설정 옵션.
- 전체 Rust 테스트 통과. 후속 Jira·Confluence 단위 회귀도 통과. CLI/MCP 설정 영구 저장 통합 검증 포함.
- Vitest 7개와 TypeScript/Vite build 통과. Pi 확장 20개 도구 등록·호출·취소 검증 통과.
- `cargo fmt -- --check`, `git diff --check` 검사. Vite 기존 graph chunk 크기 경고 유지.
- 엄격한 Clippy는 기존 db/obsidian/types/mod의 lint 때문에 실패했다. 신규 Confluence trim lint는 수정했다.
- 실제 Jira·Confluence 계정, native UI/Keychain, release 앱 재설치·배포는 미검증/미실행이다.
- 상위 관계는 parent key/URL로 보존하며 canonical graph Relation이나 전용 트리 UI는 없다.
  최근 열람 기록·OAuth/scoped token·원격 자동 갱신은 미지원이다. 선택 해제는 기존 캐시를 삭제하지 않는다.

## MCP·CLI 플러그인 연결 관리 — 2026-09-22

- `plugins.list/get/add/update/remove/refresh` 6개 도구와 CLI 하위 명령 추가.
- 기존 다중 연결 registry를 앱·IPC가 공유하며, 부분 수정 검증과 원자적 저장,
  읽기/수정/해제 직렬화, 다음 스캔의 범위 재확인과 관계 갱신을 적용했다.
- 해제 시 캐시·메모·원본·Keychain 보존. 토큰은 에이전트 인수에 노출하지 않는다.
- 앱 연결 목록은 외부 변경도 재조회한다. standalone Core owner도 같은 API를 제공한다.
- 임시 vault/SQLite 및 실제 CLI·MCP 프로세스 통합 테스트 4개 통과.
- 전체 Rust 테스트, frontend build/Vitest 7개, Pi 확장 19개 등록/호출,
  Core/CLI/MCP smoke, release app bundle build를 통과했다.
- 실제 배포 번들을 실행해 Codex 설치 스킬·MCP를 갱신하고 19개 도구 통신을 확인했다.
  기존 개인 vault를 유지하며 CLI로 bbros를 추가, 설치된 MCP로 732개 문서 색인
  (접근 불가 0개), native 화면에서 개인 1,136개/bbros 732개 자동 갱신을 확인했다.
  GitHub/Jira 실계정 CRUD·인증 변경과 장시간/절전 복귀 동작은 이번에 검증하지 않았다.
- 사용법: [PLUGIN_CONNECTIONS.md](./PLUGIN_CONNECTIONS.md).

## 에이전트 원클릭 연결 — 2026-09-22

- 설정 → 에이전트 연결에서 Codex·Claude Code·Hermes의 사용자 스킬+MCP,
  Pi의 스킬+확장 플러그인을 설치·갱신·해제하고 로컬 통신을 확인할 수 있다.
- 앱 실행 파일에 UI를 띄우지 않는 MCP/호출 모드를 포함해 별도 CLI 설치를 제거했다.
  절대 경로, 설치 기록, 충돌 검사, 백업, 다른 설정/사용자 수정 파일 보존을 구현했다.
- Codex·Claude용 플러그인 번들도 생성한다. 기본 설치와 네이티브 플러그인
  마켓플레이스 등록은 구분하며 후자는 자동 활성화하지 않는다.
- 임시 home 설치 회귀와 실제 복사한 실행 파일의 IPC/MCP 왕복 통신,
  Pi 확장 자식 프로세스 호출·취소를 검증했다. 실제 에이전트 대화와 배포 앱은 미검증이다.
- 사용법과 공식 자료: [AGENT_CONNECTIONS.md](./AGENT_CONNECTIONS.md).

## Obsidian 자동 갱신 — 2026-09-21

- 앱 수명 동안 실행하는 Rust worker가 연결된 로컬 볼트를 시작 시 및 순회 완료 후
  10초 간격으로 읽고, 추가·수정·삭제가 있으면 내부 관계 지도를 갱신한다.
- 연결별 켜기/끄기 영구 저장, 수동/자동 읽기 중복 방지, 해제 후 대기 작업 차단,
  재시작 중 누락된 삭제 반영, 실패한 경로의 캐시 보존과 다음 경로 처리를 구현했다.
- 화면에서 마지막 확인/관계 갱신 시각과 오류를 표시한다. 원본에는 쓰지 않는다.
- 임시 실제 파일·SQLite를 사용하는 `local_auto_sync` 통합 테스트를 통과했다.
  사용자 볼트/iCloud와 native 앱에서의 지속 실행·절전 복귀는 아직 검증하지 않았다.
- GitHub·Jira 자동 조회와 현재 열린 검색 결과/메인 3D 화면의 자동 재조회는 미구현이다.

## 플러그인 다중 연결 — 2026-09-21

- Obsidian → GitHub → Jira 순서의 연결 관리 화면, 연결별 UUID/계정/범위 저장·복원,
  로컬 우선 전체 읽기, 연결별 해제와 토큰 삭제 분리.
- GitHub 계정별 기존 gh 인증 재사용과 PAT, Jira Cloud 프로젝트별 개인 API token
  읽기를 지원한다. native Security Framework로 Keychain에 접근한다.
- 기존 단일 선택 범위는 새 폼으로 가져올 수 있으며 캐시·메모·Keychain을 보존한다.
- 다중 경로·계정·재시작·Jira 페이지 처리 회귀 테스트, 기존 Rust 테스트,
  frontend build/Vitest와 브라우저 폼 전환을 검증했다.
- 실제 gh 인증 상태는 유효했으나, native Tauri 연결·실자료 갱신·Keychain
  저장/삭제는 미검증이다. 내장 OAuth flow·Jira scoped token은 미구현이다.
- 자세한 인증 재사용 판단과 공식 자료: [PLUGIN_CONNECTIONS.md](./PLUGIN_CONNECTIONS.md).

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

## G4 — Core 교환·검토 UI 구현 완료 · native UI 검증 대기 (Markdown review exchange)

- `memory_export_markdown`가 canonical memory를 ID 순으로 정렬하고 현재 본문,
  evidence SourceRef/canonical URL/wikilink, revision history를 같은 DB와
  같은 데이터에서 byte-for-byte 재현 가능한 v1 Markdown으로 내보냄
- `memory_import_markdown`가 제한된 header/metadata/evidence/body fence와
  현재 source identity를 파싱하고 body 편집을 그대로 candidate에 반영; 모든
  문서를 먼저 검증한 뒤 하나의 transaction에서 `proposed` candidate만 생성
- content digest 기반 import idempotency와 credential marker 거부를 적용하고
  malformed/unknown evidence 입력에서 partial candidate를 남기지 않음
- Tauri `memory_export_markdown`/`memory_import_markdown` command를 제공하며
  import는 canonical memory 승격이나 외부 vault 쓰기를 수행하지 않음
- Markdown exchange integration 5개가 deterministic output, edited body,
  idempotent reimport, fenced heading·후행 공백 보존, unclosed fence·malformed 입력의 atomic rollback을 검증

설정 → 메모리와 그래프에서 후보 승인·거부/이력, bounded 맥락 검색, 그래프 재빌드,
Markdown 다운로드·파일 읽기·복사/붙여넣기를 제공한다. 브라우저 fixture로
승인/거부, version conflict 재시도, 검색·재빌드, 다운로드·붙여넣기를 확인했다.
파일 선택의 실제 네이티브 동작은 미검증이다. 패널은 기존 UI 스타일과 localStorage를
보존하는 native/browser 경계를 유지한다. 실제 Tauri window, 외부 vault 파일
선택, live provider와 WebGL은 이 text-only exchange 구현으로 증명하지 않는다.

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
  native Security Framework API로 credential을 읽고 저장하며
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
- arm64 staged `aidebook-core` + `aidebook-cli` local smoke(2026-09-19)는
  임시 DB/socket, 구조화된 stderr 오류와 MCP `tools/list` 6개를 확인했다.
- debug `aidebook-core` + `aidebook-cli` smoke(2026-09-21)는 임시 DB/socket에서
  observation capture → candidate distill/propose/list와 `context.query.v1`,
  MCP `tools/list` 13개(accept/reject 미노출)를 확인했다. 이는 staged package
  또는 third-party host/native window 검증이 아니다.
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
| `cargo test --manifest-path src-tauri/Cargo.toml` | 통과: Rust unit 11개, graph integration 7개, candidate lifecycle/review integration 4개, context review 6개, Markdown exchange 5개, M0 integration 6개, M1 integration 1개, M2 integration 2개, M3 integration 3개, M4 integration 1개, M5 storage 4개, M5 benchmark 1개 |
| `cargo check --manifest-path src-tauri/Cargo.toml` | 통과 |
| `git diff --check` | 통과 |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` | 통과 |
| `cargo test --manifest-path src-tauri/Cargo.toml --test m5_benchmark -- --nocapture` | 통과(2026-09-19 측정): seed=20260919, n=10000, warmup=5, runs=30, p95=24.509ms, macOS aarch64 |
| (2026-09-19) `AIDEBOOK_PACKAGE_DIR=/tmp/aidebook-arm64-package scripts/package-arm64.sh` | 통과: arm64 local binaries 3개 staged; 서명/공증/릴리스 미실행 |
| staged `aidebook-core` + `aidebook-cli` local smoke (2026-09-19) | 통과: `connections.status` structured stderr, MCP `tools/list` 6개; third-party host/native window 미검증 |
| debug Core/CLI/MCP review smoke (2026-09-21) | 통과: candidate capture→distill→propose→list, `context.query.v1`, MCP `tools/list` 13개; staged package/third-party host/native window 미검증 |

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

## 이전 MVP 작업의 jj 기록

로드맵 변경 `sqonnulz` 위에 M0 구현 `sxonqxql`/`yptolqwy`, M1 구현
`mqkpvwql`, M2 구현 `qswsrmtk`, M3 구현 `lqxykmkx`, M4 구현 `svykzmml`,
M5 구현 `ympmpyot`을 순서대로 기록했다. 당시 main 이력·원격·push는 건드리지
않았다. 2026-09-21 G1–G4의 main 병합과 검증은 아래 기록을 따른다.

## G1–G4 전달 기록 (2026-09-21)

로컬 main에 기능별 jj 변경과 독립 검토 수정을 병합했다. 원격 push는 수행하지
않았으며 기존 Jev 작업은 별도 작업 변경으로 보존했다. 재현 명령·검증 범위와
변경 ID는 [MEMORY_GRAPH_VALIDATION.md](./MEMORY_GRAPH_VALIDATION.md)에 기록한다.

## 그래프 탐색 UI — 통합 완료 · native/live 검증 대기 (2026-09-21)

사이드바의 그래프에서 현재 작업 메모·근거를 3D 또는 키보드 목록으로 탐색한다.
`neurun`의 미커밋 UI를 현재 메인의 후보 검토·맥락 검색·Markdown 교환과 함께 통합했다.
메모에 저장된 native 근거 참조는 정확한 계정·자료 식별자로 구분하며 원문·freshness는
미조회로 표시한다. 이 화면은 Core의 전체 파생 문서 그래프를 조회하지 않는다.
AI 활성화는 사용자가 실행하는 fixture 재생이며 실제 agent 추적은 아니다.

빌드, 그래프 회귀 테스트, 브라우저 UI 확인 및 남은 경계는
[그래프 UI 통합 검토](GRAPH_UI_INTEGRATION.md)에 기록한다.

### 2026-09-22: 정보 지도 탐색 UI

- 별자리 컨셉을 자료/메모 구역과 격자 기반 3차원 정보 지도로 대체.
- Map of contents 검색/유형별 목차/연결 따라가기와 선택·카메라·근거 패널 연동.
- 이벤트 활성 노드 확산 링, 활성 연결 방향 입자, 정지/재개 및 모션 감소 지원.
- 프런트엔드 빌드와 8개 테스트 통과, 브라우저 목차/데모/정지 동작 및 연속 캡처 비교 완료. native/live 추적 검증은 아님.
- 세부 기록: `docs/GRAPH_UX_REFINEMENT.md`.
