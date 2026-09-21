# Aidebook 메모리·지식 그래프 설계

작성: 2026-09-21

이 문서는 [`AGENT_MEMORY_GRAPH_RESEARCH.md`](AGENT_MEMORY_GRAPH_RESEARCH.md)의
조사 결과를 Aidebook의 현재 SQLite/FTS5 Core에 적용하는 G1–G4 구현 계약이다.
정본 메모리와 파생 그래프를 분리하고, 관찰에서 만들어진 후보를 사람이
검토한 뒤에만 정본으로 승격한다.

## 데이터 경계

```text
SourceRef -> Snapshot -> derived graph node/edge -> context.query.v1
Agent session -> Observation -> Candidate -> review -> Memory -> ContextResponse
Memory/Snapshot -> deterministic Markdown exchange
```

`sources`, `snapshots`, `relations`, `memories`, `memory_revisions`는 기존
계약을 유지한다. 그래프는 `graph_nodes`, `graph_edges`, `graph_builds`라는
별도 파생 계층이며 사용자가 명시한 `relations`를 덮어쓰거나 canonical
memory를 생성하지 않는다. snapshot의 본문은 외부 자료 캐시이고 memory는
사용자 소유 데이터다. 접근 철회나 삭제가 발생하면 snapshot 본문과 snippet은
보존하되 검색·context·graph 결과에서는 제외한다.

## G1 파생 그래프

### 노드

노드는 하나의 source 또는 snapshot에 대응하는 `source`/`document` 종류다.
identity는 `provider + account_id + external_id + kind`로 결정하고 title로
병합하지 않는다. 각 노드는 source ID, snapshot content hash, 접근 상태,
삭제 상태, 마지막 build ID를 가진다. 접근 불가·삭제 snapshot의 node는 감사와
rebuild를 위해 남길 수 있지만 query/traversal 응답에는 포함하지 않는다.

### 엣지와 link 해석

엣지는 `wikilink`, `url`, `reference`, `contains` 등의 종류와 다음 provenance를
가진다.

- `explicit`: 기존 `relations`처럼 사용자가 명시한 link.
- `extracted`: snapshot body의 정규 URL 또는 Obsidian wikilink에서 결정적으로
  읽은 link. target은 같은 provider/account namespace와 vault-relative path를
  모두 만족해야 한다.
- `inferred`: 현재 구현에서는 모델 호출을 하지 않으므로 저장하지 않는다.

wikilink target이 다른 namespace이거나 동일 path가 여러 source에 매칭되면
edge를 만들지 않고 ambiguity를 build diagnostic에 기록한다. URL은 provider가
달라도 canonical URL이 현재 source 하나와 정확히 일치할 때 연결할 수 있다.
동일 URL이 여러 source에 매칭되면 edge를 만들지 않는다. 링크가 있는 snapshot도
본문·title만 같을 수 있으므로 node freshness는 `content_hash`와 정렬된
link target/kind payload를 함께 해시한다. build ID는 scope와 digest를 포함한
결정적 값이다.

### 최신성·rebuild·순회

각 edge는 source/target snapshot digest, build ID, stale 여부를 기록한다.
rebuild는 현재 accessible, non-deleted snapshot에서만 extracted edge를
재계산하고 기존 build를 보존한 채 새 build를 만든다. 이전 build와 일치하는
edge는 재사용할 수 있지만 결과는 중복 없이 수렴해야 한다. context query와
traversal은 `max_nodes`, `max_edges`, `max_depth` 상한을 필수로 적용한다.
stale·revoked·deleted target은 결과에서 제외하고 diagnostics로만 표시한다.

코드 심볼·호출 관계는 이 단계에서 완전한 언어 파서로 추출하지 않는다.
향후 parser adapter가 추가되어도 같은 node/edge/provenance/freshness 계약을
사용한다.

## G2 후보 메모리

관찰은 짧은 세션 사실을 `captured` 상태로 저장한다. 정제 결과는
`distilled`, 사용자 검토 대상은 `proposed`, 승인·거부 결과는 각각 `accepted`,
`rejected`다. 전이는 다음 규칙만 허용한다.

```text
captured -> distilled -> proposed -> accepted
                                  \-> rejected
```

각 observation/candidate에는 session/actor, body, reason, evidence source,
claim type, created/updated time, request digest와 idempotency key가 있다.
후보는 evidence source가 실제로 존재하는지 확인하고 credential marker를
거부한다. `accepted` 전이는 candidate 상태 변경, 기존 `memories` insert,
`memory_revisions` insert, evidence 연결, idempotency 기록을 하나의 SQLite
transaction에서 처리한다. 같은 key와 같은 요청은 저장된 결과를 재생하고,
다른 payload는 conflict다. 이미 accepted/rejected인 candidate의 재승인은
새 canonical memory를 만들지 않는다.

MCP는 candidate를 읽고 propose하는 호출까지만 제공한다. acceptance는
검토 화면 또는 명시적으로 trusted한 Tauri command에서만 호출한다.

## G3 context.query.v1

요청은 `query`, optional root source, `max_age_seconds`, `max_depth`,
`max_nodes`, `max_edges`, `max_memories`, provider/kind filters를 가진다.
응답은 다음 배열을 포함한다.

- lexical `sources`: FTS5 result와 source updated/fetched time, freshness,
  access status, evidence location.
- `memories`: canonical memory와 revision/version, evidence, retracted state.
- `graph`: traversed node/edge, relation type, provenance, confidence,
  freshness/build ID.
- `unavailable_sources`, `stale_sources`, `conflicts`, `missing_providers`,
  `bounds` diagnostics.

기존 `context.search`, `context.get`, `memory.upsert`, `memory.retract`,
`sources.refresh`, `connections.status`의 request/response와 의미는 바꾸지
않는다. `context.query.v1`는 별도 IPC method, CLI subcommand, read-only MCP
tool, Tauri command를 통해 추가한다. G2의 observation capture/get,
candidate distill/propose/get/list도 같은 인증된 IPC/CLI/MCP 경계에서 사용할
수 있어 로컬 에이전트가 관찰 → 후보 정제 → 검토 제안 흐름을 끝까지 만들 수
있다. `candidate.accept`와 `candidate.reject`는 이 transport 목록에 없고
trusted Tauri review command에만 남는다.

## G4 Markdown 교환·검토

export는 source identity와 canonical URL, evidence IDs, revision history,
claim type, provenance, wikilink를 안정된 정렬 순서로 기록한다. 동일 DB와
동일 옵션에서 반복 export하면 byte-for-byte 결과가 같아야 한다.

import는 Markdown을 parser가 이해할 수 있는 제한된 frontmatter/body 형식으로
검증하고 candidate `proposed`로만 저장한다. 현재 교환은 파일 경로를 받지 않는
text-only 경계이며(UI는 선택 파일 읽기·다운로드·복사/붙여넣기 제공), export에 포함된 SourceRef가 현재 Core source와
일치하는지 확인한다. import가 canonical memory를 직접 만들거나 외부 vault를
수정하는 경로는 없다.

검토 UI는 기존 Aidebook 색상·패널 구조를 재사용해 review queue, graph query,
export/import 설정을 노출한다. UI에서 보이는 성공은 Core commit 후에만
표시한다. 브라우저 fixture/in-memory 검증은 native Tauri window, live provider,
Keychain, iCloud hydration을 대신하지 않는다.

## 검증 매트릭스

| 영역 | 핵심 회귀 |
|---|---|
| 그래프 | namespace/path ambiguity, link-only digest, stale/revoked/deleted exclusion, deterministic rebuild, traversal bounds |
| 후보 | lifecycle transition, invalid transition, evidence/version checks, atomic acceptance, duplicate/conflicting idempotency |
| context | lexical+memory+graph composition, unavailable/freshness, old six IPC methods |
| 교환 | deterministic export, bounded text import, fenced body and whitespace roundtrip, no auto promotion/external write |
| 빌드 | `npm run build`, `cargo test --manifest-path src-tauri/Cargo.toml`, `cargo check --manifest-path src-tauri/Cargo.toml`, `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`, `git diff --check` |
