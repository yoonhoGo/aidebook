# Workflow native 검증

2026-09-23. macOS 27 arm64, Swift 6.4, Tauri 2.11.5 / tao 0.35.3 debug 실행. 이 기록은 UI 인수 완료나 알림 전달 완료를 뜻하지 않는다.

## 실제 실행 결과

| 항목 | 관측 결과 |
| --- | --- |
| 별도 Tauri 앱 시작 | 성공. `dev.aidebook.wp923`, 창 제목 `Aidebook Workflow Probe`. 사용자 앱/DB 대신 별도 Application Support 디렉터리 사용 |
| native host의 Core IPC | 실제 Tauri 프로세스가 연 SQLite에 CLI로 업무·할 일·작업 시간 저장 성공 |
| 프로세스 종료·재시작 | SIGTERM 뒤 IPC 조회 실패 확인. 같은 native binary 재실행 후 할 일·작업 시간 동일성 검증 성공 |
| native UI 저장/재열기 | 미검증. Orca가 visible window를 찾았으나 AX 읽기는 `permission_denied`로 실패 |
| 창 닫기·Cmd+Q 수명 | 미검증. AX 차단을 우회하거나 OS 권한을 변경하지 않음 |
| macOS UserNotifications 연동 | 별도 임시 ad-hoc 서명 앱에서 `getNotificationSettings`와 pending 조회 성공 |
| 알림 권한/전달 | probe bundle의 `authorizationStatus=0`(notDetermined), pending=0. 권한 요청·예약·배너 전달·클릭·재부팅 검증은 실행하지 않음 |

[Native process 결과](../artifacts/workflow-native/native-process.json), [AX 실패](../artifacts/workflow-native/initial-state.json), [권한 상태 조회](../artifacts/workflow-native/computer-permissions.json), [알림 상태](../artifacts/workflow-native/notification-status.json), [시작 로그](../artifacts/workflow-native/native-launch-short.txt).

실제 계정·캘린더·사용자 DB·자격 증명은 사용하지 않았다. probe 시작/종료 외 사용자 앱 프로세스는 조작하지 않았다. probe 데이터는 재실행 비교를 위해 `~/Library/Application Support/dev.aidebook.wp923`에 남겨 두었다. 원래 Aidebook 식별자와 분리되어 있다. 임시 notification 앱은 실행 후 삭제한다.

## 현재 수명 계약의 제한

현재 `src-tauri/src/lib.rs`는 `WindowEvent::Destroyed`에서 `core_stop=true`를 설정한다. 같은 플래그를 IPC server와 background sync가 사용한다. 이는 코드 검토 결과이며 창 닫기 실측은 아니다. 현재 구현을 창을 닫아도 Core가 계속 동기화하는 구조로 설명하면 안 된다. SIGTERM은 창 닫기/정상 종료 이벤트와 다르다. 상시 Core가 필요하면 명시적인 tray/background 정책과 종료 후 서비스 소유권을 먼저 결정해야 한다.

첫 시도에서는 긴 probe bundle identifier가 macOS UNIX socket 최대 경로 길이를 초과해 setup panic이 발생했다([로그](../artifacts/workflow-native/native-launch.txt)). 짧은 별도 식별자로 재실행해 성공했다. 임의로 긴 사용자 경로에 대한 endpoint 단축은 별도 개선 사항이다.

## 재현

프로젝트 루트에서 별도 터미널로 실행한다. 기존 실행 중인 probe가 있으면 먼저 해당 probe만 종료한다.

```sh
npx tauri dev --no-watch --config scripts/native-workflow-config.json
```

`orca computer list-apps --json`으로 probe PID를 확인한다. `get-app-state --app pid:<PID> --json`으로 실제 UI 접근 가능 여부를 확인한다. 접근이 막히면 완료라고 처리하지 않는다. OS 설정 변경은 이 검증에 포함하지 않는다.

```sh
python3 scripts/native-workflow-process.py --pid <PROBE_PID>
python3 scripts/native-workflow-notification.py
```

process harness는 해당 PID가 전용 probe DB를 실제로 열고 있는지 `lsof`로 검사한 뒤, 업무/할 일을 IPC로 저장하고 그 PID만 종료한다. 재시작 binary는 별도 임시 복사본으로 고정한다. probe config로 빌드한 binary를 유지하고 실행해야 한다. UI 쓰기 검증을 대체하지 않는다.

## 공식 문서에 따른 다음 검증

[Apple 권한 안내](https://developer.apple.com/documentation/usernotifications/asking-permission-to-use-notifications)는 예약 전 현재 권한 조회를 안내한다. [authorizationStatus](https://developer.apple.com/documentation/usernotifications/unnotificationsettings/authorizationstatus)에 따라 denied 상태에서는 시스템 전달을 기대할 수 없다. 이번 probe는 조회 API가 실제 macOS 앱에서 동작함만 확인했다.

[Tauri 이벤트 문서](https://v2.tauri.app/develop/plugins/)는 window와 app exit 이벤트 처리 지점을 제공한다. 실제 앱의 창 닫기, 정상 종료, 절전 복귀, 재실행은 개별 시나리오로 검증해야 한다. W3 전에 UI 접근이 가능한 환경에서 저장/재열기와 수명을 확인하고, 사용자가 선택한 캘린더 공급자 및 알림 권한 정책에 맞춰 예약/취소/실제 전달/클릭을 검증한다.
