# W0 계약·W1a 업무와 할 일 검증

> 후속 구현과 검증은 [W1b·W2 검증 기록](WORKFLOW_W1B_W2_VALIDATION.md)을 참조한다. 이 문서는 W1a 시점의 기록이다.

작성: 2026-09-23. jj change: `qoyspyzk`.

## 구현과 남은 경계

W0의 기존 데이터 매핑, 로컬 상태·소유권·API 계약, 가져오기/복원 인수 절차를 [계약 문서](WORKFLOW_CONTRACT.md)에 기록했다. 캘린더 공급자는 사용자 확인 대기이고, native 캘린더·알림·창 닫기·앱 종료 실험은 미완료다. 따라서 W0 전체 완료가 아니다.

공급자와 무관한 W1a 수직 구현을 먼저 추가했다.

- SQLite v8 업무/할 일/활동. 버전 검사, 멱등성 응답, 원자적 저장.
- 업무·로컬 할 일 생성/수정/상태 변경/재오픈, 개인 목표일·우선순위·막힘, 여러 작업 시간.
- 업무에서 하위 할 일 탐색, 페이지 조회, 저장 후 재조회, 로딩/빈 상태/실패 안내.
- Tauri `workflow_save/get/list`, IPC/MCP `workflow.save/get/list`, CLI `workflow save|get|list --params JSON`.
- 업무 완료는 데스크톱 완료 검토 후 확정. CLI/MCP의 업무 done 요청 및 trusted 응답 재생은 거부.
- 기존 고정 작업 묶음·localStorage·메모·원본 관계 보존.

W1의 원본/메모 수동 연결, 명시 가져오기 미리보기·매핑은 아직 없다. 외부 티켓·일정·대시보드·알림·필수 산출물 검사도 후속 단계다. 앱 재시작은 실제 Core owner 프로세스 수준에서 확인했고, Tauri 화면에서 저장하고 앱을 다시 여는 인수는 아직 하지 않았다. 브라우저 fixture와 native/live 완료를 혼동하지 않는다.

## 로컬 자동 검증

환경: macOS, 로컬 Rust/Tauri debug binaries, npm/TypeScript/Vite. 테스트 DB·계정·home은 임시 디렉터리이며 실제 사용자 DB, 계정, 캘린더, 알림 권한을 사용하지 않았다.

| 실행 | 결과 |
| --- | --- |
| `cargo test --manifest-path src-tauri/Cargo.toml --no-fail-fast` | 86개 통과, 실패 0. [전체 결과](../artifacts/workflow-w1a/rust-tests.txt) |
| `cargo build --manifest-path src-tauri/Cargo.toml --bins` | 통과 |
| `python3 scripts/smoke-workflow.py` | 실제 CLI 생성/멱등 재생, Core 프로세스 종료·재시작 후 할 일/작업 시간 보존, MCP 목록·공통 데이터 조회 통과. [결과](../artifacts/workflow-w1a/process-smoke.json) |
| `npm run build` | TypeScript/Vite 통과. 기존 GraphCanvas 500kB 초과 chunk 경고 유지 |
| `npm test` | 기존 프런트엔드 8개 통과. 신규 폼 자동 테스트를 의미하지 않음 |
| `node scripts/test-agent-extension.mjs` | Pi 23개 도구 등록, 인수 격리·한글·취소 검증 통과 |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` | 통과 |
| `git diff --check` | 통과 |

신규 workflow 저장소 테스트 9개: 같은 제목 분리·멱등 재생/충돌, stale version·완료 검토·재오픈, 동시 작성자 중 하나만 commit, 부모/날짜/시간 검증, 활동 기록 실패 rollback, v8 마이그레이션 실패 rollback·재실행, 재시작·백업 복원·할 일 완료와 업무 상태 독립, 페이지 경계, Core/IPC 공통 데이터.

## 발견하고 수정한 IPC 회귀

처음 전체 검사에서 기존 IPC 인증 및 설치 앱 probe가 간헐적으로 실패했다. 새로 연결한 뒤 100ms 후 요청을 보내는 회귀 테스트에서 `Resource temporarily unavailable (os error 35)`와 `BrokenPipe`로 재현했다.

비차단 listener에서 접수한 macOS 소켓에 비차단 상태가 남아, body가 도착하기 전에 read가 실패했다. connection worker의 stream을 blocking으로 설정하고 읽기/쓰기 timeout 30초를 적용했다. listener의 종료 polling은 유지한다. 동일 지연 테스트와 원래 두 실패 테스트를 포함한 전체 86개가 통과했다.

v8 추가에 맞춰 저장소 회귀 테스트의 예상 schema version을 갱신하고, v4 복원 fixture에서 v5–v8 테이블을 실제로 제거하도록 수정했다. 마이그레이션 충돌을 무시하도록 production SQL을 바꾸지 않았다.

## 화면 점검

Aside 브라우저, 별도 localhost:1421, 테스트용 Tauri invoke fixture를 사용했다. 브라우저 fixture는 메모리 배열만 수정했고, 실제 SQLite·Tauri·외부 계정에는 연결되지 않았다.

- native bridge 없는 브라우저에서 저장 불가 안내와 쓰기 폼 미노출 확인.
- 업무 생성·목표일 입력·저장 후 목록 반영 확인.
- 업무에서 하위 할 일로 탐색, 작업 시간 입력, 완료 저장 확인.
- 완료 할 일을 진행 중으로 재오픈하고 버전 2·시간 유지 확인.
- 저장 실패 주입 후 편집 내용 보존, 재시도 요청 두 개의 idempotency_key 동일, 성공 후 내용 반영 확인.
- [화면 이미지](../artifacts/workflow-w1a/workflow-task-browser-fixture.png)는 완료 상태의 브라우저 fixture다.

## 다음 작업

1. W1b: 원본/메모 수동 연결·해제와 localStorage 가져오기 미리보기/매핑. 원본 보존·재실행·잘못된 참조 테스트.
2. W0 잔여: 첫 캘린더 공급자 확정, 인증/읽기 범위, 실제 native 알림·프로세스 수명 검증.
3. W1 native 인수 후 W2: 오늘·다음 행동·확인 필요 대시보드와 날짜/시간대·10k 조회 성능.
