# 에이전트 메모리·그래프 지식 저장소 도구 조사

작성: 2026-09-19

이 문서는 [Aside 공유 조사 페이지](https://share.aside.com/s/VIwPKWnghfvzxwYp)에 나온 도구와 공식 저장소 문서를 함께 검토한 결과를 기록한다. 목적은 Aidebook의 여러 컨텍스트 연결, 장기 메모리, 코드·문서 관계 탐색 기능을 설계할 때 참고할 구현 패턴을 남기는 것이다.

## 핵심 분류

공유 페이지는 도구를 다음 세 층으로 나눈다.

| 층 | 도구 | 주된 역할 |
|---|---|---|
| 코드 구조 그래프 | [Graphify](https://github.com/Graphify-Labs/graphify), [CodeGraph](https://github.com/colbymchenry/codegraph), [Serena](https://github.com/oraios/serena), [Codanna](https://github.com/bartolli/codanna) | 파일·심볼·호출·의존 관계를 분석하고 에이전트의 코드 탐색을 줄인다. |
| 에이전트 메모리 | [agentmemory](https://github.com/rohitg00/agentmemory), [Mem0](https://docs.mem0.ai/platform/features/graph-memory), [Letta](https://docs.letta.com/), [Zep/Graphiti](https://github.com/getzep/graphiti), [Cognee](https://github.com/topoteretes/cognee) | 세션 관찰을 정리하고 검색 가능한 장기 메모리와 시간적 관계를 만든다. |
| 개인 지식베이스 | [Basic Memory](https://github.com/basicmachines-co/basic-memory), [claude-obsidian](https://github.com/AgriciDaniel/claude-obsidian), OpenHuman | 사람이 읽고 수정할 수 있는 Markdown·그래프·개인 기록을 제공한다. |

Understand Anything, Repowise, blarify는 공유 페이지에서 코드 그래프의 추가 후보로 언급된다. 이들은 각각 시각적 탐색, Git 이력·문서·ADR 연결, Neo4j/SCIP 기반 그래프라는 방향을 대표한다.

핵심 결론은 코드 구조 그래프와 에이전트 메모리를 하나의 저장소 유형으로 합치지 않는 것이다. 코드 그래프는 현재 저장소의 구조와 변경 영향을 설명하고, 에이전트 메모리는 세션에서 얻은 사실·선호·결정을 보존한다. 개인 지식베이스는 사람이 검토하고 이동할 수 있는 표현 계층으로 보는 편이 안전하다.

## 도구별 구현 패턴

### 코드 구조 그래프

- **Graphify**는 Tree-sitter로 코드를 로컬에서 분석하고 graph.json, HTML 그래프, 보고서를 만든다. 코드 관계는 결정적으로 추출하고, 문서·미디어의 의미 관계에는 모델 호출이 사용될 수 있다. 엣지를 추출된 관계와 추론된 관계로 구분하는 방식이 Aidebook의 근거·신뢰도 모델에 참고할 만하다. 벡터 저장소 없이 탐색 가능한 그래프를 만드는 점도 유용하다.
- **CodeGraph**는 Rust 커널, SQLite/FTS5, 파일 감시, 디바운스, 파일 해시·수정 시각 재검사를 조합한다. 코드 변경 뒤 그래프가 자동으로 갱신되고, 여러 클라이언트가 하나의 writer를 공유한다. 이는 Aidebook의 단일 Core owner와 freshness 관리에 직접 참고할 수 있다.
- **Serena**는 별도 장기 그래프 DB보다 LSP 또는 JetBrains 백엔드를 사용해 심볼 단위 검색·참조 탐색·진단·편집·리팩터링을 제공한다. 메모리 저장소라기보다 에이전트가 코드를 정확히 읽고 수정하는 작업 계층이다.
- **Codanna**는 한 번의 요청으로 의미 검색 결과에 심볼 식별자, 시그니처, 문서 문자열, callers, callees, 재귀적 영향 범위를 함께 반환한다. 여러 개의 작은 도구 호출을 하나의 작업 중심 Context Packet으로 묶는 방식이 Aidebook의 context.query 설계에 적합하다.

### 에이전트 메모리

- **agentmemory**는 에이전트 작업을 자동 수집하고 observation, memory, crystal 형태의 요약, graph 관계로 정리한다. 키가 없어도 BM25 검색을 사용할 수 있고 로컬 임베딩은 선택 사항이다. 다만 자동 생성 결과는 Aidebook에서 바로 정본 메모리로 승격하지 않고 검토 대기 후보로 두어야 한다.
- **Mem0**는 관련 컨텍스트를 모은 뒤 벡터·키워드·엔티티 후보를 평가하고 메모리를 추가·수정·삭제·무시하는 흐름을 제공한다. 현재 공식 문서는 Platform의 Graph Memory와 OSS 범위를 구분하므로, 공유 페이지의 이전 비용·외부 저장소 설명은 현재 버전 선택의 근거로 사용하지 않는다.
- **Letta**는 영속 메모리 블록과 검색 가능한 archival memory를 에이전트가 직접 수정하는 런타임에 가깝다. 장기 실행 에이전트에는 강하지만, Aidebook의 단일 Core와 명시적 사용자 검토 경계에 비해 런타임 결합도가 크다.
- **Zep/Graphiti**는 사실의 유효 기간, 과거와 현재의 차이, 사건의 출처를 그래프에 보존한다. 정책·상태·고객 정보처럼 시간이 지나며 바뀌는 컨텍스트를 다룰 때 참고할 수 있다.
- **Cognee**는 문서·코드·세션을 엔티티, 관계, 청크로 정리하고 remember와 recall을 통해 영속 메모리와 세션 메모리를 분리한다. 정제 파이프라인의 각 단계를 교체할 수 있다는 점이 후보 메모리와 사용자 승인 흐름에 적합하다.

### 개인 지식베이스

- **Basic Memory**는 일반 Markdown과 wikilink를 그래프의 정본으로 사용하고 MCP를 통해 에이전트가 접근한다. 사람과 에이전트가 같은 파일을 읽고 수정할 수 있으며, 세션 시작 요약과 pre-compaction checkpoint도 제공한다.
- **claude-obsidian**은 Obsidian vault의 Markdown을 중심으로 출처가 포함된 페이지와 연결된 지식 그래프를 만든다. 평범한 파일 형식을 유지해 이식성을 확보하는 방향이 Aidebook의 내보내기·백업 계층에 적합하다.
- **OpenHuman**은 공유 페이지에서 전 생애 범위의 로컬 메모리 도구로 분류된다. 범위가 넓고 GPL 계열 라이선스 검토가 필요하므로 Aidebook의 기본 Core에 바로 포함할 후보는 아니다.

공유 페이지의 별 수와 벤치마크는 탐색용 참고값으로만 사용한다. [agentmemory의 비교 설명](https://github.com/rohitg00/agentmemory)은 서로 다른 데이터셋의 수치와 시점별 별 수를 동일 조건의 대결 결과로 해석하지 말아야 한다고 명시한다. 실제 도입 전에는 Aidebook의 문서·코드·세션 샘플로 별도 측정한다.

## Aidebook에 적용할 구조

현재 Aidebook의 기본 계약은 source/snapshot을 외부 자료 캐시로, memory를 근거와 수정 이력을 가진 사용자 소유 데이터로 구분한다. 명시적인 source relation, evidence, expected version, idempotency, retraction/restore 규칙도 이미 고정되어 있다. 이 경계를 유지하면서 다음 기능을 파생 계층으로 추가한다.

~~~mermaid
flowchart LR
  S[Sources and snapshots] --> G[Derived project graph]
  A[Agent session] --> O[Observations]
  O --> C[Memory candidates]
  C --> R[User review]
  R --> M[Durable memories]
  G --> Q[Context query]
  M --> Q
  S --> Q
  M --> X[Markdown and Obsidian export]
~~~

1. **파생 프로젝트 그래프**

   기존 relations는 사용자가 명시한 source-to-source 관계로 유지한다. 코드·문서 그래프는 별도 파생 테이블에 저장한다. 최소 필드는 다음과 같다.

   - node: 심볼·문서·파일 종류, 경로, 줄 번호, source 또는 snapshot ID
   - edge: 호출·참조·의존·포함 종류, 출발 노드·도착 노드
   - provenance: 명시 추출인지 모델 추론인지, 근거 위치, confidence
   - freshness: 콘텐츠 해시, 분석 시각, 그래프 빌드 ID

   CodeGraph의 감시·해시·단일 writer와 Graphify의 그래프 산출물·엣지 종류를 조합한다. 초기 구현은 외부 Neo4j 없이 SQLite 파생 테이블과 FTS5로 시작한다.

2. **세션 메모리와 정본 메모리 분리**

   다음 생명주기를 사용한다.

   captured → distilled → proposed → accepted/rejected

   관찰과 세션 요약은 짧은 수명의 작업 데이터로 두고, 승인된 후보만 기존 memories에 evidence, reason, author, claim type과 함께 저장한다. 철회·복원은 기존 revision 규칙을 그대로 사용한다.

3. **작업 중심 Context Packet**

   향후 context.query는 여러 도구를 호출한 원시 결과 대신 다음 정보를 한 번에 반환한다.

   - lexical 검색 결과와 관련 memory
   - 관련 심볼·문서·source
   - callers, callees, 영향 경로
   - evidence, 원본 위치, 수집 시각
   - freshness, 충돌, 접근 불가 source

   현재 M4의 context.search, context.get 및 여섯 IPC 메서드는 공통 Core 계약으로 유지하고, 그래프 결합 검색은 버전이 있는 새 메서드로 추가한다. 기존 호출의 의미를 조용히 바꾸지 않는다.

4. **사람이 읽는 교환 계층**

   Basic Memory와 claude-obsidian의 Markdown·wikilink 방식을 export/import 및 백업 포맷으로 참고한다. SQLite Core는 실행 정본으로 두고, Markdown에는 원본 링크·evidence ID·revision 정보를 남겨 사람이 확인할 수 있게 한다.

## 도입 순서

1. CodeGraph에서 파일 감시, 콘텐츠 해시, stale 표시, 단일 Core writer 패턴을 가져온다.
2. Graphify에서 파생 graph.json 형태와 extracted/inferred 관계 구분을 가져온다.
3. Codanna와 Serena에서 심볼·호출·영향 범위를 포함한 작업 중심 검색과 정확한 코드 작업 인터페이스를 가져온다.
4. agentmemory와 Cognee에서 observation, session summary, candidate, 승인 흐름을 가져온다.
5. Basic Memory와 claude-obsidian에서 Markdown export와 사람이 검토할 수 있는 연결 형식을 가져온다.

외부 Neo4j, Letta 런타임, 별도 벡터 그래프 서비스를 초기 구현의 필수 의존성으로 추가하지 않는다. Aidebook의 macOS 로컬 우선, 단일 Core, 사용자 검토, 근거 보존 경계가 먼저 안정된 뒤 규모와 검색 품질을 측정해 외부 저장소를 검토한다.

이 문서는 조사 결과를 저장한 것이며 구현 상태를 변경하지 않는다. 실제 도입 시에는 이 문서의 도구별 설명과 현재 공식 문서를 다시 확인하고, Aidebook 샘플 데이터에 대한 정확도·검색 지연·저장 용량을 별도로 측정한다.
