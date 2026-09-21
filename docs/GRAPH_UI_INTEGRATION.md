# 그래프 UI 통합 검토

2026-09-21. `neurun`의 `neuron` 기준 미커밋 UI를 현재 `main`의 메모리 검토·교환 UI에 통합했다. 원래 `neurun` 작업 파일은 보존했다. 메인에 없는 독립 커밋을 merge한 것이 아니라 공통 기준 `21a219ce`를 사용해 UI 변경을 3-way 통합하고 검토 수정했다.

## 통합 범위

- 사이드바 그래프, 현재 작업 메모와 evidence, 3D 선택·목록 대안·근거 패널.
- 명시적으로 실행하는 데모 재생, 단계별 강조, 자료 유형·활성 자료 필터, 모션 감소.
- 기존 설정의 후보 검토·맥락 검색·Markdown 교환 유지.
- 순수 그래프 어댑터(`graph-model.ts`), fixture reducer(`graph.ts`), 지연 로딩 렌더러(`GraphCanvas.tsx`) 분리.

## 검토 중 수정

1. App 안에 선언된 GraphPage 컴포넌트가 상태 변경마다 다시 mount되는 문제를 제거했다. 렌더러 데이터는 복사·memoize해 카메라·좌표를 보존한다.
2. 완료 뒤에도 계속 나오던 입자를 재생 중에만 표시한다. 리셋 뒤 늦게 도착한 step은 무시한다.
3. 실제 캔버스 크기를 ResizeObserver로 전달한다. CSS로 전체 창 크기 캔버스를 축소하던 문제와 초기 카메라의 노드 잘림을 수정했다.
4. native 메모는 안정적인 native ID로 식별한다. 근거는 provider/account/external ID/kind로 구분하며 같은 provider의 예시 원문에 연결하지 않는다. 실제 원문과 freshness가 없으면 미조회로 표시한다.
5. 중복 근거 간선을 제거한다. 활성 자료 필터에서도 반환·인용 간선이 유지되도록 fixture 요청 노드도 해당 단계에 활성화한다.
6. tooltip에 제목 HTML이 해석되지 않도록 이스케이프한다. 목록 선택 시 키보드 focus를 보존하고 닫힌 근거 패널을 연다.
7. 3D 의존성을 lazy load한다. 기본 JS 약 305 kB, 3D chunk 약 1,417 kB로 분리했으며 3D chunk 크기 경고는 남아 있다.
8. Vitest 대상을 루트 `src/**/*.test.ts`로 제한해 `neurun`의 원본 테스트를 중복 실행하지 않는다.

렌더러 크기·tooltip·입자 API는 [react-force-graph 공식 문서](https://github.com/vasturiano/react-force-graph)와 설치된 패키지 구현·타입을 확인했다.

## 검증

- `npm ci --ignore-scripts`: 성공, audit 취약점 0.
- `npm run build`: TypeScript 및 production build 통과. 3D chunk 크기 경고만 남음.
- `npm run test:graph`: 2개 파일, 7개 테스트 통과. 명시적 간선 강조·단계·리셋 후 timer·정확한 native 근거 식별·근거 없는 메모·입력 보존 검증.
- `git diff --check`: 통과.
- Aside의 로컬 Vite 브라우저: 실제 WebGL 캔버스와 화면 캡처 확인, 데모 완료, 재생 전후 동일 canvas DOM 유지 확인.
- 목록 노드 선택과 메모 근거 패널, 선택 버튼 focus 유지 확인.
- 활성 자료 필터에서 반환·인용 간선 2개만 표시하고 reset 후 활성 노드·간선 0개 확인.
- 설정 → 메모리와 그래프의 후보 검토·맥락 검색·Markdown 교환 메뉴와 브라우저 native 작업 비활성 상태 유지 확인.

## 검증 경계

이 화면은 현재 UI의 메모·근거 투영이다. Core `graph_neighbors`의 전체 파생 문서 그래프와 연결하거나 실제 agent delivery/context/citation을 수집하지 않는다. native 근거 어댑터는 단위 테스트이며 실제 Tauri UI와 Core 데이터로 검증한 것은 아니다.

native Tauri WebView, WebGL 실패 강제 주입, 실제 agent 이벤트, native 편집·철회 persistence, 장시간 GPU 사용은 미검증이다. Rust 코드는 변경하지 않았다. 기존 `docs/JEV_REVIEW.md`, `docs/ROADMAP.md`, `src-tauri/Cargo.toml` 변경은 이번 통합에서 제외한다.
