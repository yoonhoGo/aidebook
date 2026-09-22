# W1b 연결·가져오기와 W2 로컬 대시보드 검증

2026-09-23. jj feature change `lunssuwp`. 서브에이전트 3개가 백엔드/화면/native 검증 및 대시보드 회귀를 나눠 실행하고, 주 에이전트가 transport·대시보드 화면을 구현하고 통합 검증했다.

## 구현

- SQLite v9 `work_links`, `legacy_work_mappings`. 명시 원본/메모 연결·해제·재연결, 버전 검사·멱등성·원자적 가져오기.
- 기존 작업 묶음의 namespace+인덱스를 영속 업무에 매핑한다. 사용자가 미리보기에서 선택한 묶음만 가져오며 기존 업무 수정·localStorage·메모·원본을 보존한다. nativeId 없는 메모는 별도 기존 메모 가져오기 경로로 남기고 제외 내용을 표시한다.
- 링크 제목·SourceRef·수집 시각은 현재 접근 가능한 원본에서 조회하며 링크 JSON/멱등 응답에 복제 저장하지 않는다. 접근 철회 후 조회/재생에서도 이 필드는 숨긴다. 사용자 메모 자체를 삭제하지 않는다.
- 업무 자료 연결 화면은 펼칠 때 20개 페이지로 조회한다. 현재 원본의 인라인 미리보기, 수집 시각, 접근 불가 안내를 제공한다.
- ‘오늘의 업무’에서 오늘 작업 시간·목표일, 다음 행동, 확인 필요, 진행 중 업무, 최근 완료를 Core에서 파생 조회한다. 별도 업무 상태 사본은 없다.
- 직접 고정 → 기한 초과 → 3일 이내 목표일 → 개인 우선순위로 정렬하고 이유를 표시한다. 시간 계획은 시간순이다.
- 완료·취소·보류 업무 및 그 하위 할 일의 행동 분류를 구분한다. 최근 완료는 단순 updated_at이 아니라 활동의 완료 전이 시각을 사용한다.
- IANA 시간대, 날짜 변경, DST 23/25시간 날짜, 자정 종료의 반열린 시간 구간을 처리한다. 화면은 선택한 시간대로 30초마다/포커스 복귀/변경 후 재조회한다.
- 대시보드에서 업무/할 일의 편집 화면으로 이동하며 할 일은 바로 완료할 수 있다. 업무 완료 확정은 기존 trusted 검토 경로만 사용한다.
- 업무/할 일별 활동 타임라인은 펼칠 때 버전 역순으로 20개씩 조회한다. 기록에 없는 작성자를 추정하지 않는다.

## 공통 API

| IPC/MCP | Tauri | CLI (`--params JSON`) |
| --- | --- | --- |
| work_link.add/remove/list | workflow_link_add/remove/list | work-link add/remove/list |
| workflow.import.preview/apply | workflow_import_preview/apply | workflow import-preview/import-apply |
| dashboard.get | dashboard_get | dashboard get |
| workflow.activity.list | workflow_activity_list | workflow activity |

기존 API는 유지하며 총 MCP/Pi 도구는 30개다. work_link confirmed는 요청자가 명시 선택한 링크를 뜻하고 작성자는 `explicit_request`다. canonical memory 후보 승인이나 외부 서비스 쓰기를 뜻하지 않는다.

## 자동 검증

| 명령 | 결과 |
| --- | --- |
| `cargo test --manifest-path src-tauri/Cargo.toml --no-fail-fast` | **102개 통과**, 실패 0. [로그](../artifacts/workflow-w1b/rust-tests.txt) |
| `cargo build --manifest-path src-tauri/Cargo.toml --bins` | 통과 |
| `npm test` | **12개 통과**: 기존 8개 + 가져오기 helper 4개 |
| `npm run build` | TypeScript/Vite 통과. 기존 GraphCanvas chunk 크기 경고 유지 |
| `python3 scripts/smoke-workflow.py` | 실제 별도 Core/CLI/MCP 프로세스로 생성/재시작/자료 연결·해제/가져오기 재생/대시보드 조회 통과. [결과](../artifacts/workflow-w1b/process-smoke.json) |
| `node scripts/test-agent-extension.mjs` | 30개 도구 등록·인수 격리·한글·취소 검증 통과 |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` 및 `git diff --check` | 통과 |

신규 backend 검증: 잘못된/철회된 참조, SourceRef lookup만 수행하고 자동 수집하지 않음, 접근 상태의 조회/멱등 재생 시 재검사, 동시 해제 충돌, 같은 제목의 namespace 분리, 기존 업무 수정 보존, 전체 가져오기 rollback, v9 migration rollback, 백업 복원/매핑 보존, 링크/멱등 JSON의 원본 제목·URL 비복제, 활동 버전·페이지·재오픈.

## 10,000개 조회 측정

macOS arm64, debug test, 메모리 SQLite의 업무 1개+할 일 9,999개. 페이지당 최대 100개를 반환하는 Core 메서드, 영역별 warmup 3회+측정 30회. [원본 측정값](../artifacts/workflow-w2/dashboard-benchmark.json).

| 영역 | p95 |
| --- | ---: |
| 오늘 | 79.01ms |
| 다음 행동 | 104.69ms |
| 확인 필요 | 81.44ms |
| 진행 중 업무 | 70.98ms |
| 최근 완료 | 76.67ms |

영역별 목표 300ms 이하다. **메모리 DB의 단일 영역 Core 조회** 측정으로 디스크 I/O·다섯 영역 전체 화면 로드·native 렌더·원격 일정 성능을 증명하지 않는다. 현재 구현은 업무 aggregate를 한 번 순회하므로 더 큰 자료량에서 인덱스/조회 모델 개선이 필요할 수 있다. 기존 source FTS 10k 벤치마크와 구분한다.

## 화면·native 경계

- [W1b 브라우저 검증](../artifacts/workflow-w1b/ui-verification.md): 명시 선택, 첫 적용 실패→같은 키 재시도, 원본 JSON byte 보존, 연결 입력·지연 조회·페이지 이동·원본 미리보기. mock invoke 사용.
- [대시보드 브라우저 이미지](../artifacts/workflow-w2/dashboard-browser-fixture.png): 다섯 영역·빈 상태, 상세 선택 callback, 할 일 완료 후 오늘/다음 행동 제외와 최근 완료 반영, 잘못된 시간대 오류 및 복구 확인. mock invoke 사용.
- 테스트 fixture는 실제 Core 저장/외부 계정 동작을 대신하지 않는다. fixture HTML은 개발 서버에서만 사용하는 재현 자료다.
- 작은 native 창·전체 키보드 경로 인수는 미완료다. 1120px 이하 단일 열 CSS와 기본 버튼/레이블/상태 텍스트는 구현했으나 실제 작은 창 검증으로 표시하지 않았다.
- [native 검증](WORKFLOW_NATIVE_VALIDATION.md): 별도 Tauri 앱이 소유한 Core에 IPC 저장 후 프로세스 재시작 보존을 확인했다. UI 접근성 조회는 `permission_denied`로 차단되어 UI 저장·창 닫기·Cmd-Q는 검증하지 못했다. 우회하거나 OS 권한을 변경하지 않았다.
- 별도 Swift 앱의 알림 권한/pending 조회는 동작했고 `notDetermined`, pending=0이었다. 권한 요청·예약·실제 전달은 수행하지 않았다.

## 남은 단계

W1 기능과 로컬 회귀는 완료했으나 native UI 인수가 남아 있다. W2 로컬 조회/화면 구현·성능은 확인했고 작은 창/키보드 인수가 남아 있다. W0 캘린더 공급자 선택과 native 알림 수명 실험, W3 실제 캘린더·예약/전달, W4 외부 구조화 상태·완료 조건·기록 템플릿, W5 실제 계정/출시 인수는 미완료다. W6 AI와 W7 외부 쓰기는 원래대로 별도 범위다.

IANA 변환 구현은 [Chrono-TZ 공식 API](https://docs.rs/chrono-tz/0.10.4/chrono_tz/)를 확인해 기존 chrono 모델에 연결했고, 의존성은 Cargo.lock에 고정했다. 실제 사용 시각/날짜 경계는 위 테스트로 검증했다.
