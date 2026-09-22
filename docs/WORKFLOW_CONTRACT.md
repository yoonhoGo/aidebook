# 업무·할 일 계약과 기존 데이터 매핑

작성: 2026-09-23. W0 계약과 W1a/W1b·W2 구현 범위를 기록한다. 후속 검증은 [W1b·W2 기록](WORKFLOW_W1B_W2_VALIDATION.md)을 따른다.
전체 단계 완료 여부는 [검증 기록](WORKFLOW_VALIDATION.md)을 따른다.

## 기존 저장소 조사

| 현재 데이터 | 실제 위치·식별자 | 신규 대응·보존 정책 |
| --- | --- | --- |
| 작업 묶음 | `src/App.tsx`의 `works` 세 제목, 인덱스 0–2 | 영속 업무 ID가 아니다. 자동 변환하지 않음. 이후 명시 가져오기에서 namespace와 인덱스로 매핑 |
| 브라우저 메모 | `aidebook-notes-v1`, 숫자 `id`, 숫자 `work`, 선택적 `nativeId` | 원본 JSON 보존. nativeId가 있으면 기존 canonical memory를 참조. 없는 메모는 기존 명시 메모 가져오기 경로와 연동 |
| Core UI 메모 | `ui_memories(memory_id, title, work, kind)` | 기존 Memory ID와 숫자 work를 유지. 이후 별도 매핑 테이블을 추가해 새 업무에 연결 |
| 메모 근거 | `memory_evidence` → sources | 근거·본문·revision 복사 없이 참조 |
| 원본 관계 | `relations(from_source_id,to_source_id,relation_type,reason)` | 원본 간 명시 관계 유지. 업무 관계로 재해석하지 않음 |
| 그래프 | `graph_builds/nodes/edges` | 파생 탐색 자료. 확정 업무 관계나 상태의 저장소로 사용하지 않음 |
| 활동 | `aidebook-activities-v1`의 UI 문구·시각 | 감사 기록으로 승격하지 않음. 원본 보존, 가져오기 미리보기에서 구분 |
| 세션 | `aidebook-session-v1`의 page/work/selected 등 | 탐색 상태만 유지. `workflow` 페이지를 추가하되 기존 작업 묶음 경로 유지 |
| 설정 | `aidebook-settings-v1`, `aidebook-scopes-v1` | background/autostart/notifications는 UI 값. 실행·예약 성공으로 해석하지 않음 |

동일 제목의 업무도 독립 UUID로 생성한다. 기존 localStorage와 Core 메모를 자동 수정하거나 삭제하지 않는다. 새 ‘업무와 할 일’ 화면은 Core만 조회하며 브라우저 대체 저장소를 만들지 않는다.

## W1a에서 확정한 로컬 모델

SQLite v8에서 `work_items`, `tasks`, `activity_events`를 추가한다. 각 업무/할 일의 버전 있는 aggregate를 `data_json`에 보존하고, ID·갱신 시각·할 일의 work_id를 별도 컬럼으로 두어 관계 무결성과 페이지 조회를 지원한다. 이후 W2 조회 지표에 따라 상태·목표일 컬럼을 추가하는 마이그레이션을 작성한다.

- 업무/할 일: UUID, kind, version, created_at, updated_at, fields.
- 공통 fields: title, status, purpose, blocked_reason, target_date, priority(0–3, 3이 가장 높음).
- 할 일에만: 선택적 work_id(현재는 로컬 업무 하나), time_blocks 배열(최대 100개).
- time_blocks는 할 일 aggregate 안에서 원자적으로 수정한다. 각 start/end는 오프셋 있는 RFC3339이며 종료가 시작보다 늦어야 한다. 별도 예약 ID·캘린더 회차와 연결하는 W3에서 전용 테이블로 분리한다.
- target_date는 YYYY-MM-DD의 **개인 목표일**이다. 외부 티켓의 원본 기한이 아니다.
- blocked_reason은 null 또는 비어 있지 않은 이유다. 상태와 독립적이다.
- activity_events는 로컬 수정 때 대상·이전/다음 상태·버전·서버 기록 시각을 남긴다. 지금은 사용자/에이전트별 작성 주체와 원격 발생/수집 시각을 구분하지 않으므로 W4 감사 이력 완성으로 보지 않는다.

원격 티켓은 이번 API로 생성하지 않는다. 이후 provider+account+scope+external_id의 외부 identity 테이블, 원본 소유 필드, 다대다 work_links를 추가한다. 원격 제목·상태·담당자·기한이 로컬 계획 필드를 덮어쓰는 경로는 허용하지 않는다.

## 상태·완료·삭제 계약

| 종류 | 허용 상태 | 특별 전이 |
| --- | --- | --- |
| 업무 | planned, in_progress, review, done, on_hold, cancelled | done 진입은 기존 review에서 데스크톱 사용자가 저장할 때만 허용 |
| 로컬 할 일 | planned, in_progress, done, cancelled | 할 일 완료는 업무 상태를 바꾸지 않음 |
| 완료/취소된 항목 | 기존 상태 수정 또는 planned/in_progress로 재오픈 | review 등 다른 상태로 바로 전이 불가 |

새 업무를 done으로 만들 수 없다. CLI/MCP는 업무 done 요청을 멱등 재생까지 포함해 거부한다. UI는 ‘완료 검토’로 먼저 저장한 다음 ‘완료’를 선택하여 확정한다. 필수 산출물 자동 검사·완료 제안은 W4 후속 작업이다.

물리 삭제 API는 제공하지 않고 취소 상태로 보존한다. 관계 해제는 향후 `work_links`만 해제하며 원본·메모·개인 계획을 삭제하지 않는다. 연결 권한 철회에 따른 접근 가능성은 기존 SourceRef/Snapshot의 access_status 계약을 따른다.

## 후속 명시 관계 계약

`work_links`의 endpoint는 업무/할 일/일정 ID 또는 기존 SourceRef/Memory ID를 참조한다. 문서·티켓·PR 본문을 복제하지 않는다. 관계 종류는 `context`(관련 맥락), `meeting_minutes`(회의록), `specification`(설계), `implements`(티켓/PR 실행), `verification`(검증), `completion_record`(작업 기록)로 시작한다. 하나의 자료를 여러 업무에서 참조할 수 있다.

동일 endpoint 쌍·관계 종류는 중복 생성하지 않는다. 이유, evidence, author, confirmed, version, 생성/해제 시각을 둔다. 사용자가 명시적으로 연결한 관계는 confirmed이며 자동 추출·유사 제목은 후보로만 둔다. 해제는 관계만 비활성화하고 원본·메모·계획·다른 업무의 링크는 보존한다. 접근 권한은 링크가 아니라 현재 원본의 접근 상태로 판단한다. v9에서 업무 → 원본/기존 메모의 부분집합을 구현했다. 일정/할 일 endpoint 확장은 후속이다. reason과 target의 기존 근거를 사용하며 별도 evidence 사본은 저장하지 않는다. author는 explicit_request다. 링크 조회용 제목/SourceRef/수집 시각은 현재 자료에서 계산하고 저장하지 않는다.

## 공통 API

| API | Tauri | CLI | MCP |
| --- | --- | --- | --- |
| workflow.save | workflow_save(input), trusted 완료 검토 가능 | workflow save --params JSON | workflow.save, 업무 완료 불가 |
| workflow.get | workflow_get(input) | workflow get --params JSON | workflow.get |
| workflow.list | workflow_list(input) | workflow list --params JSON | workflow.list |

save는 부분 patch가 아니라 fields 전체 교체다. 생성은 id/expected_version 생략 또는 null, 수정은 id와 양의 expected_version이 필수다. 저장 성공 후 Core 결과를 재조회한다. 같은 idempotency_key와 payload는 처음 응답을 재생하고, 다른 payload는 충돌한다. 저장·활동·멱등성 응답은 하나의 transaction이다.

list: kind 필수, 선택적 work_id는 task에만 허용. limit 기본 50/최대 100, offset 기본 0. updated_at DESC, id ASC로 결정적으로 정렬한다. offset 페이지를 조회하는 사이 다른 수정이 있으면 경계가 이동할 수 있으며 UI는 변경 뒤 재조회한다. W2 대시보드 정렬/날짜 분류는 dashboard.get으로 구현했다.

```sh
# 실행 중인 Core owner의 데이터 디렉터리를 사용
# 예시는 테스트 owner에만 실행한다.
aidebook-cli workflow save --data-dir /tmp/aidebook-workflow-demo --params '{"kind":"work","idempotency_key":"example-work-1","fields":{"title":"첫 업무","status":"planned","purpose":"범위 확인"}}'
aidebook-cli workflow list --data-dir /tmp/aidebook-workflow-demo --params '{"kind":"work","limit":20}'
```

추가 API: `work_link.add/remove/list`(명시 참조·이유·버전·멱등성), `dashboard.get`(날짜·시간대·페이지·정렬 이유), `reminder.save/cancel/snooze/list`(대상·예약시각·버전·멱등성). work_link와 dashboard는 구현·노출했고 reminder는 아직 설계이며 노출하지 않는다. review가 필요한 관계 제안/완료 확정은 generic 자동화 경로와 구분한다.

## 명시 가져오기·복원 인수 절차

1. 기존 localStorage JSON 내보내기와 Core `backup_to`를 별도로 수행한다. SQLite 백업은 localStorage를 포함하지 않는다.
2. 가져오기 미리보기에서 원본 namespace, 묶음 인덱스, 제목, 메모 수, nativeId 유무, 잘못된 참조를 표시한다.
3. 사용자가 선택한 항목에만 `(namespace, legacy_work_index) → work_id` 매핑을 transaction으로 생성한다. 제목으로 매칭하지 않는다. 같은 매핑 재실행은 기존 업무/관계를 반환하고, 기존 사용자 수정은 덮어쓰지 않는다.
4. 메모 본문은 기존 가져오기·근거 검토 경로를 사용한다. 업무 링크 추가를 메모 승인으로 해석하지 않는다.
5. 실패 주입으로 새 행·매핑·멱등 기록의 rollback을 확인한다. 원본 localStorage는 성공/실패 모두 보존한다.
6. Core 복원 시 v7 백업은 현재 마이그레이션으로 업그레이드하되 백업 당시 없던 업무는 복원되지 않는다는 점을 미리 표시한다. v8 백업은 업무·할 일·작업 시간·활동·멱등 기록을 함께 복원한다.

2–5의 묶음 가져오기 미리보기/명시 적용/매핑/rollback 검증은 v9에 구현했다. nativeId가 없는 메모 본문은 자동 승격하지 않으며 기존 메모 가져오기 경로로 안내한다. 기존 메모 가져오기와 업무 묶음 가져오기는 독립적이다.

## 캘린더·native 수명 결정

첫 공급자는 사용자 확인 대기다. Google/Outlook/EventKit 중 무엇을 실제 사용하는지 확인 전 연결·권한 요청을 만들지 않는다. 이에 독립적인 로컬 모델 구현을 먼저 진행했다.

현재 `lib.rs`는 Tauri window Destroyed 때 Core와 LocalSync에 stop 신호를 보낸다. 앱 종료 후 IPC/동기화 지속을 보장하지 않는다. standalone aidebook-core는 별도 실행 가능하지만 자동 시작 agent/daemon 설치가 아니다. 현재 Cargo dependencies에는 OS notification plugin이 없고, 캘린더/알림 권한·예약 구현도 없다.

W0 native 검증은 아직 미완료다. 독립적인 테스트 앱으로 권한 상태/거부, 창 닫기와 Cmd-Q, pending OS 예약의 잔존, 알림 클릭 라우팅을 측정한다. 창을 숨길지 종료할지 제품 동작을 결정하고, 앱 종료 중에도 OS 예약만 전달할 수 있는지와 원격 변경을 알 수 없는 경계를 구분한다. SDK 컴파일이나 소스 읽기를 실제 전달 검증으로 기록하지 않는다.

## W2 조회와 활동

`dashboard.get`은 timezone(IANA), section(today/next/attention/in_progress/completed), 선택적 now(RFC3339), kind, limit(기본20/최대100), offset을 받는다. 각 페이지에 date, timezone, generated_at, entries, total, has_more와 calendar_connected=false를 반환한다. fields.pinned는 기본 false로 기존 JSON과 호환된다. 최근 완료는 활동의 실제 done 전이를 기준으로 오늘 포함 7개 날짜만 반환한다.

`workflow.activity.list`는 kind/id와 limit/offset을 받고 버전 역순의 상태 전이·기록 시각을 반환한다. 출처 없는 작성 주체는 추정하지 않는다. UI는 두 조회 결과를 별도 영속 상태로 저장하지 않는다.
