# 3D 그래프 구현 계획

작성: 2026-09-19

> 이 문서의 기존 완료·검증 표는 neurun 작업 당시 기록이다. 2026-09-21 통합 과정에서 발견한 수정 사항과 재검증 결과는 [통합 검토 기록](GRAPH_UI_INTEGRATION.md)을 따른다.

설계 기준: [그래프 활성화 설계](GRAPH_ACTIVATION_DESIGN.md).

## 첫 번째 구현 범위

기존 React 화면의 자료와 메모, evidence 연결을 탐색 가능한 그래프로 투영한다. 현재 UI가 사용하는 자료의 출처와 데모 여부를 그대로 보존한다. 실제 Core 요청 추적이 없는 상태에서는 AI 사용을 표시하지 않는다. 사용자가 명시적으로 실행한 데모 재생으로 활성화 동작을 검토한다.

Luna Max 서브에이전트가 프런트엔드, 필요한 의존성, 상태 전이 검증을 담당한다. 주 에이전트는 계약과 수용 기준을 정리한다. Rust Core와 IPC의 새 계약 구현은 후속 단계다.

## 컴포넌트와 상태 경계

- GraphView: 작업 범위, 선택 노드, 보기 모드, 데모 재생 제어.
- Graph adapter: 기존 Source와 Note를 안정적인 ID의 노드 및 evidence 간선으로 변환. 원본 데이터를 수정하지 않는다.
- Graph renderer: 3D 위치와 카메라, 노드 선택, 강조 표현. 도메인 상태를 렌더러 객체에 저장하지 않는다.
- Activation reducer: 요청 ID·이벤트 ID·순서를 이용해 활성 상태를 계산. 시간 기반 시각 효과와 도메인 이벤트를 분리한다.
- Evidence inspector: 선택 자료의 본문, 근거, 버전과 freshness를 표시. 없는 정보는 추측하지 않는다.
- Accessible list: WebGL 실패와 키보드 사용 시 같은 노드·근거에 접근할 수 있는 목록.

노드 ID는 source와 memory namespace를 구분한다. 간선 ID는 관계 종류와 양끝 ID를 포함해 결정적으로 생성한다. 렌더링 라이브러리가 좌표나 source/target 필드를 변경하더라도 원본 상태가 오염되지 않도록 렌더러 전용 데이터를 만든다.

## 데모 재생 계약

초기 상태는 정지다. 데모 실행 전과 실행 중에 데모임을 표시한다. 실제 에이전트의 관측 이벤트로 설명하지 않는다.

재생 이벤트는 requestId, eventId, sequence, stage, nodeIds, edgeIds를 가진다. 검색 결과, 응답 생성, 답변 인용 단계를 구분하고 근거가 있는 간선만 강조한다. 리셋은 이벤트 강조를 초기화하되 사용자의 자료나 메모를 변경하지 않는다. 완료 후 이동 효과는 종료한다.

동작 줄이기 설정에서는 정적 배지와 외곽선으로 같은 상태를 전달한다. 화면 이탈 시 타이머와 그래픽 리소스를 정리한다.

## 수용 기준

1. 기존 네비게이션에서 그래프를 열고 기존 메모 화면으로 돌아갈 수 있다.
2. 현재 작업의 자료·메모 및 evidence 관계가 안정적인 ID로 표시된다.
3. 노드 선택으로 실제 연결된 근거를 확인할 수 있다.
4. 데모 실행 전에는 AI 활성 상태나 실제 사용 주장이 나타나지 않는다.
5. 데모 이벤트가 지정한 간선만 활성화된다. 두 노드가 활성화돼도 다른 간선은 켜지지 않는다.
6. 재생 완료·리셋·화면 이탈 시 애니메이션이 올바르게 정리된다.
7. 동작 줄이기와 목록 보기에서도 같은 상태와 자료를 확인할 수 있다.
8. 기존 메모 편집과 localStorage 데이터가 보존된다.
9. 프런트엔드 빌드가 통과하며 상태 전이에 대한 검증 결과를 기록한다.

## 후속 Core 연동

Core owner에서 검색과 응답 생성 기록을 수집하고, 버전 있는 그래프 조회·이벤트 조회 계약을 추가한다. 클라이언트 수신, 컨텍스트 포함, 인용은 해당 관측 주체가 제공할 때만 표시한다. 기존 IPC 여섯 메서드의 의미를 변경하지 않는다.

기록에는 자료 snapshot/hash 또는 memory revision을 연결한다. 이벤트 보존 정책과 재연결 cursor를 정한 뒤 실시간 구독을 구현한다. fixture와 실제 에이전트 연동, 브라우저와 native WebView 검증은 각각 기록한다.

## 구현된 UI 계약

- `source:<id>`와 `memory:<id>`는 현재 React가 사용하는 `Source.id`와 `Note.id`를
  namespace로 감싼 안정적인 그래프 ID다. `request:demo:<work>`는 실제 AI 요청이
  아니라 사용자가 재생을 누를 때만 쓰는 fixture 요청 노드다.
- evidence 간선은 현재 메모의 `Note.sources`에서만 만든다. 제목 유사도, 양끝
  활성화, 벡터 추정으로 새 간선을 만들지 않는다. 데모 반환·인용 간선은
  `provenance: demo_fixture`인 별도 레이어로 표시한다.
- 활성화 상태는 `src/graph.ts`의 reducer가 `requestId`, `eventId`, `sequence`,
  `stage`, `nodeIds`, `edgeIds`를 기준으로 계산한다. 현재 단계는
  `search_result`(검색 결과 포함), `response_prepared`(반환 응답 생성),
  `answer_cited`(답변 인용)로 분리한다. `activeEdgeIds`는 이벤트가 명시한 ID의
  합집합이며 활성 노드의 양끝으로 추론하지 않는다.
- fixture 이벤트의 `observer`는 `demo_fixture`다. 현재 Core/IPC에는 AI 추적 계약이
  없으므로 `transport`, `agent`, `context_included`를 실제 상태처럼 표시하지 않는다.
  동일 이벤트는 고정된 시각과 ID로 재생되며, 화면이 그래프를 떠나면 타이머를
  정리하고 활성 상태를 리셋한다.
- 3D 렌더러용 node/link 객체는 기존 `Source`·`Note`를 복사해 만든다. 선택 패널은
  기존 자료 본문·메모 본문·저장 이유·버전·freshness를 직접 읽고, 수정·철회·이력
  동작은 기존 핸들러를 재사용한다. WebGL을 사용할 수 없거나 키보드가 필요한 경우
  같은 graph model과 activation state를 접근 가능한 목록 보기로 노출한다.
- `settings.reduceMotion`과 OS `prefers-reduced-motion`을 함께 존중한다. 이때
  시안 외곽선·텍스트 배지는 유지하고 directional particle을 만들지 않는다.

## 수용 기준 및 결과

1. [x] 기존 사이드바에서 `그래프`를 열고 기존 작업·메모 화면으로 돌아갈 수 있다.
2. [x] 현재 작업의 실제 자료·메모·evidence가 안정적인 namespace ID로 표시된다.
3. [x] 3D 노드 또는 목록 노드 선택으로 연결 원문, 메모 본문, 저장 이유와 버전을
   확인한다.
4. [x] 초기 화면에는 실제 AI 사용·인용 주장이 없고 `실제 AI 추적 없음`과
   `demo_fixture` 경계가 표시된다.
5. [x] 데모 이벤트가 지정한 간선만 활성화된다. reducer 테스트에서 검색 단계는
   간선 0개, 반환 단계는 delivery 간선 1개, 인용 단계는 citation 간선만 추가되는
   것을 확인한다.
6. [x] 데모 재생은 버튼 선택 전 시작하지 않으며, 완료·리셋·그래프 이탈 시 타이머와
   활성 상태를 정리한다.
7. [x] 목록 대안과 모션 감소 상태에서도 동일한 노드·간선·단계·근거에 접근한다.
8. [x] 기존 localStorage 메모와 편집/저장/철회 흐름은 변경하지 않고 그래프는 파생
   데이터만 사용한다.
9. [x] `npm run build`, `npm run test:graph`, `git diff --check`를 통과한다.

검증 기록:

| 명령 | 결과 |
| --- | --- |
| `npm run build` | 통과: `tsc` + Vite production build; 3D 번들 크기 경고만 있음 |
| `npm run test:graph` | 통과: Vitest 3개; 정지/단계 분리/지정 간선/리셋 검증 |
| `git diff --check` | 통과 |
| Chrome headless CDP UI smoke | 통과: 그래프 메뉴, 목록 대안, 메모 선택 근거 패널, 데모 완료·인용 단계 확인 |
| 외부 브라우저/Orca UI 확인 | macOS AX window read가 blocked되어 직접 클릭·상태 판독은 미완료 |

## 작업 상태

- 설계 및 첫 구현 범위: 구현·문서화 완료.
- Luna Max 프런트엔드 수직 구현: `src/graph.ts`, `src/App.tsx`, `src/App.css`,
  패키지 의존성과 상태 전이 테스트까지 완료.
- Core 이벤트와 실제 에이전트 연동: 후속 단계. 현재 화면의 데모 fixture를 Core/AI
  관측으로 승격하지 않는다.
- native Tauri WebView, 실제 Core/IPC 이벤트, live agent delivery/context/citation,
  macOS WebGL 성능은 아직 검증하지 않았다.
