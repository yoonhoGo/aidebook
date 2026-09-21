# Aidebook 구현 로드맵

작성: 2026-09-18. 기준: `비서의 노트 MVP 설계.md`, `비서의 노트 기획서.md`.
원문 위치: `/Users/yoonho.go/Library/Mobile Documents/iCloud~md~obsidian/Documents/Zettelkasten/notes/`.
범위가 다르면 구체적인 MVP 설계를 우선한다. 아래 항목은 완료 보고가 아니다.

## 출발점

- 기준 부모: `f86381e3` (`main`), 시작 작업 사본은 깨끗함.
- React/Tauri UI, 작업 탐색·메모 편집·복원·설정과 localStorage 저장이 존재한다.
- Rust는 `core_status` 경계만 제공한다. SQLite, 실제 커넥터, CLI/MCP는 아직 없다.
- 기존 디자인·아이콘·브라우저 데모와 저장 데이터를 보존한다. 네이티브 DB 전환 시 명시적인 가져오기와 실패 복구를 제공하며 기존 저장소를 자동 삭제하지 않는다.

## 제품 경계

macOS Apple Silicon, 볼트 1개와 선택한 GitHub 저장소 읽기, 로컬 메모 쓰기만 지원한다. 자체 LLM, 외부 서비스 쓰기, 웹훅 서버, 원격 MCP, 벡터 검색, 다른 공급자와 마켓플레이스는 후속 범위다. 공급자 본문은 비신뢰 데이터이며 권한을 바꾸지 않는다.

## 단계와 완료 기준

| 단계 | 구현 범위 | 검증 및 완료 기준 | 의존성 |
|---|---|---|---|
| M0 계약·저장 기반 | Rust 공통 모델/API, SQLite 마이그레이션·FTS5, 구조화 오류, 읽기 전용 어댑터 계약, Obsidian/GitHub fixture | 재시작 영속성, 중복 원본 제거, 권한 거부, 실패 시각 보존, 마이그레이션 롤백, 검색 필터·기본 20/최대 50 검증 | 현재 코드 |
| M1 Obsidian 수직 슬라이스 | 명시적 볼트 선택, Markdown·허용 frontmatter·링크 색인, 변경 감시·해시 재검사, Tauri 검색 연결 | 임시 볼트에서 생성·수정·삭제 수렴, 허용 경로 밖 접근 거부, 원본 무변경, iCloud 미다운로드/충돌 상태 표시 | M0 |
| M2 GitHub 수직 슬라이스 | 선택 저장소와 계정 범위, Keychain 토큰, 이슈·PR·댓글·상태, 페이지 순회·수동 refresh | fixture 오류 분류와 범위 제한; 실제 계정/Keychain smoke 별도 기록. 실패가 fetched_at/마지막 성공을 갱신하지 않음 | M0, M1 설정 |
| M3 맥락·메모 UI | 명시 URL/ID/wikilink 관계, 파생 WorkContext, 관계 해제, 근거·이유·작성자·주장 형태, 수정·철회·복원, localStorage 가져오기 | 동일 제목만으로 병합 금지, 멱등성·버전 충돌·동시 수정, 사용자 메모 보존, 근거 접근 불가와 재검토 표시, 10개 저장/생략/추론 예시 | M1, M2 |
| M4 에이전트 연결 | 단일 코어 소유 프로세스와 인증된 로컬 IPC, CLI JSON/stdout 및 로그/stderr, MCP stdio 6개 도구 | UI/CLI/MCP 결과·권한 일치, 복수 MCP가 DB를 직접 열지 않음, 창 닫기 후 호출, 종료 시 구조화 오류 또는 재시작, 프로토콜 클라이언트 smoke | M3 |
| M5 출시 검증 | 연결 해제/캐시 삭제 분리, 백업·복원, 접근성·폰트·Reduce Motion, arm64 빌드, CLI 포함 패키지와 Cask 템플릿 | 새 설치·오프라인·재시작·권한 철회·마이그레이션 실패·복원, 10,000건 검색 p95 ≤300ms 측정, 실제 macOS smoke 기록 | M4 |

M0 → M1 → M2 → M3 → M4 → M5 순서로 작은 jj 변경을 만든다. 메모 저장 불변조건은 M0에서 먼저 검증하고 사용자 흐름은 M3에서 통합한다. 예상 일정은 M0 검증 후 산정한다.

## 메모리·지식 그래프 확장 계획

`docs/AGENT_MEMORY_GRAPH_RESEARCH.md`의 조사 결과를 현재 Core에 적용하는
후속 G1–G4 작업이다. 이 계획은 외부 Neo4j·벡터 DB·LLM·완전한 언어
파서를 추가하지 않고, SQLite/FTS5와 기존 단일 Core 소유권을 확장한다.
각 단계는 앞 단계의 계약과 테스트가 통과된 뒤 별도 jj 변경으로 기록한다.

| 단계 | 구현 범위 | 완료 기준 | 명시적 경계 |
|---|---|---|---|
| G1 파생 문서 그래프 | snapshot에서 명시 URL/Obsidian wikilink를 namespace·path로 해석하고 별도 graph node/edge 테이블에 deterministic/extracted/inferred provenance, 근거 위치, confidence, content+link digest, build ID를 저장한다. bounded 1-hop/다중 hop traversal, stale·접근 불가·삭제 edge 제외, rebuild를 제공한다. | 같은 제목의 다른 namespace는 연결하지 않음, 모호하거나 외부인 target은 edge를 만들지 않음, 링크만 바뀌어도 digest/build이 갱신됨, 재빌드가 중복 없이 수렴하며 이전 명시 `relations`를 바꾸지 않음, revoked/deleted snapshot 본문·snippet이 graph/context에서 노출되지 않음. | 완전한 Tree-sitter/코드 컴파일러 분석과 모델 추론은 후속 작업이다. graph는 canonical user relation이 아닌 derived data다. |
| G2 관찰·메모리 후보 | observation을 captured→distilled→proposed→accepted/rejected로 보존하고 evidence/version/idempotency digest를 가진 candidate를 만든다. accepted만 기존 `memories`/`memory_revisions`를 재사용해 canonical memory로 promotion한다. | 상태 전이 규칙과 actor/reason/evidence를 검증하고, acceptance가 candidate 상태·canonical memory·revision을 하나의 transaction으로 갱신한다. 중복 acceptance와 conflicting idempotency가 원자적으로 거부/재생된다. 사용자 검토 경계는 Tauri route에만 둔다. | MCP에는 observation/candidate 조회·propose만 노출하고 자동 acceptance 도구를 노출하지 않는다. 외부 서비스/LLM은 사용하지 않는다. |
| G3 작업 중심 context.query.v1 | lexical snapshot 검색, memory 검색, graph traversal, freshness/evidence/unavailable/conflict를 한 응답에 묶은 versioned query contract와 bounds를 추가한다. 기존 여섯 IPC method semantics를 유지하고 IPC/CLI/MCP/Tauri에서 사용 가능한 read/propose 경로를 추가한다. | 결과에 source/memory/graph provenance와 fresh/stale/unavailable 상태가 명시되고, max nodes/edges/memories와 query length가 강제된다. legacy method regression, access revocation·stale/rebuild·candidate lifecycle 검증이 통과한다. | 현재 검색은 FTS5 lexical이며 vector/rerank는 후속이다. context.query.v1은 read-only다. |
| G4 Markdown 교환·검토 UI | evidence/revision/wikilink를 포함하는 deterministic Markdown export와 review candidate 전용 safe import를 제공한다. canonical memory 자동 승격이나 외부 vault write 없이 Tauri review/graph/query/exchange 설정 패널을 기존 UI 스타일에 맞춰 추가한다. | export가 반복 실행해도 같은 결과를 내고 evidence/revision이 보존된다. import는 경로·형식·evidence를 검증해 후보로만 저장하며 traversal/path escape와 임의 외부 쓰기를 거부한다. 기존 UI 상태·스타일과 localStorage를 보존한다. | 실제 native Tauri window, Keychain, iCloud/FSEvents, live provider, WebGL은 이 작업으로 증명하지 않는다. |

상태 문서에는 단계별 change ID와 실행한 검증을 기록한다. 구현 계획에 없는
기능은 이 확장 작업에 포함하지 않으며, native/live 경계는 fixture·in-memory
테스트의 통과와 분리해 보고한다.

## 공통 계약

- SourceRef: 공급자·계정·외부 ID·정규 URL·유형. Snapshot: 본문·제목·내용 해시·source_updated_at·fetched_at·접근 상태.
- Memory: 본문·저장 이유·근거·작성 주체·주장 형태·버전·멱등성 키·이력. Relation과 SyncState는 별도 테이블, WorkContext는 조회 모델.
- `context.search`, `context.get`, `memory.upsert`, `memory.retract`, `sources.refresh`, `connections.status`는 공통 코어를 사용한다.
- 검색/맥락은 원본 URL·수정/수집 시각·freshness를 반환한다. max-age 초과는 stale, 갱신 실패는 unavailable. 권한 철회/삭제 자료는 검색에서 제외하고 메모 본문은 보존한다.
- 수정은 expected_version과 idempotency_key로 보호한다. 동일 키의 다른 요청은 충돌로 처리한다. 복원은 과거 이력을 지우지 않고 새 버전을 만든다.
- 토큰은 Keychain에만 저장하며 DB·로그·모델 응답에 반환하지 않는다. 외부 자료가 명령처럼 쓰여 있어도 계정/범위 검사는 코어가 수행한다.

## 첫 실행 작업: M0

담당: GPT-5.6 Luna, reasoning=max. 실제 코드·fixture·검증 결과를 제출한다.

1. 현재 Rust/React 경계와 저장소 규칙을 읽고 `docs/CORE_CONTRACT.md`에 모델, 오류, 프로세스 소유권과 향후 IPC 경계를 고정한다.
2. SQLite 저장·트랜잭션 마이그레이션·FTS5 및 공통 타입/검색/최신성/범위 검사를 구현한다. 테스트는 임시 DB만 사용한다.
3. 메모 생성/수정/철회/복원의 멱등성·버전·근거 검증을 구현하고 동기화가 메모를 덮어쓰지 않게 한다.
4. 정상·중복·실패·권한 거부의 두 공급자 fixture와 회귀 테스트를 추가한다. 실제 볼트/계정에 자동 연결하지 않는다.
5. `npm run build`, `cargo test --manifest-path src-tauri/Cargo.toml`, `cargo check --manifest-path src-tauri/Cargo.toml`, `git diff --check` 결과를 기록한다. 10,000건 성능 측정은 재현 명령과 환경·p95를 남기며 측정하지 않았다면 미검증으로 표시한다.
6. README의 현재 구현 상태와 `docs/IMPLEMENTATION_STATUS.md`에 완료·남은 항목·테스트·네이티브 미검증 경계를 기록한다. 이 작업의 완료 범위는 M0이며 전체 MVP 완료로 보고하지 않는다.

## jj 작업 규칙

- 작업 시작/종료 및 변경 분리 전 `jj status`, `jj diff --summary`, `jj log`를 확인한다.
- 로드맵 변경 위에 `jj new`로 구현 변경을 만든다. 의미 있는 단위마다 `jj describe -m '...'` 후 다음 변경을 만든다.
- 사용자 변경을 흡수하거나 기존 main 이력을 재작성하지 않는다. Git commit/reset/checkout 대신 jj로 변경을 관리한다.
- 최종 change ID와 검증 결과를 상태 문서와 보고에 남긴다. main 이동, 원격 push, 릴리스/tap 공개는 이번 작업에 포함하지 않는다.
- 별도 작업 사본이 필요하면 `jj workspace add`를 사용하고 경로·기준 revision·소유 작업을 기록한다. 동일 파일을 여러 에이전트가 동시에 수정하지 않는다.

## 출시 전 결정과 후속 범위

최소 macOS 버전, Intel 지원, Developer ID/공증, 공개 GitHub OAuth, 제품명 사용 가능성은 출시 전 결정한다. 개발 단계는 사용자가 선택한 읽기 전용 토큰과 저장 직후 되돌릴 수 있는 로컬 메모를 기준으로 한다.

MVP 인수 기준을 통과한 뒤 Jira/Google/Slack/Confluence → 선택적 웹훅 릴레이 → 검증된 Apple 어댑터/Windows/외부 플러그인 순서로 검토한다. 자동 테스트는 실제 Keychain, iCloud, GitHub, UI, MCP 클라이언트, Homebrew 설치 검증을 대신하지 않는다.
