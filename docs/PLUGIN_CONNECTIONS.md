# 플러그인 연결과 인증

검토일: 2026-09-21. 우선 대상은 **Obsidian → GitHub → Jira**다.
외부 시스템에는 쓰지 않으며, 로컬 경로 연결은 네트워크 로그인에 의존하지 않는다.

## 구현

- 플러그인 종류와 연결 인스턴스를 분리했다. 연결마다 UUID, 이름, 계정,
  읽기 범위, 인증 방식을 저장한다. 같은 플러그인에 여러 계정·저장소·볼트를
  추가할 수 있다. 저장소 소유자와 로그인 계정도 별도 필드다.
- 앱 데이터 디렉터리의 `connections.json`을 임시 파일 + rename으로 저장한다.
  재시작 후 목록을 복원하며 연결 ID 중복으로 기존 연결을 덮어쓰지 않는다.
- 목록과 전체 읽기는 Obsidian을 먼저 처리한다. 원격 연결 하나의 실패 후에도
  다음 연결을 읽는다. 연결 목록 잠금은 네트워크 요청 중 보유하지 않는다.
- Obsidian은 실제 경로를 정규화하고 기존 adapter의 경로·symlink 경계를
  사용한다. 볼트별 namespace가 달라 동일 상대 경로의 문서가 충돌하지 않는다.
- GitHub는 기존 읽기 adapter에 연결별 계정·저장소를 전달한다.
  `/user` 응답으로 토큰 계정과 선택 계정이 일치하는지 확인한다.
- Jira Cloud는 선택 프로젝트의 `POST /rest/api/3/search/jql` 조회만 사용한다.
  제목·상태·수정 시각·원문 링크를 저장하며 `nextPageToken`을 순회한다.
  다른 프로젝트의 결과, 반복 cursor, 불완전 페이지는 실패 처리한다.
- 토큰은 연결별 macOS Keychain 항목에 저장한다. 기존 Keychain service/account
  규칙을 유지하면서 `security` 프로세스의 prompt 의존을 native Security
  Framework API로 교체했다. UI에는 password 입력을 사용하고 localStorage,
  연결 파일, SQLite, 오류 메시지에 토큰을 보관하지 않는다.
- 연결 해제와 토큰 삭제를 구분한다. 기존 캐시·사용자 메모·다른 연결과
  `gh` 원본 로그인은 유지한다. 삭제 실패를 성공으로 숨기지 않는다.
- HTTP 요청은 HTTPS, 리다이렉트 미사용, curl 기본 설정 무시, 30초 제한,
  8 MiB 응답 제한을 적용한다. GitHub 댓글 URL은 선택 저장소에서 직접 만든다.

## Obsidian 자동 갱신

- 앱이 실행 중이면 시작 시 한 번, 이후 각 순회 완료 후 10초마다 연결된 볼트를
  읽는다. 절전·앱 종료 중에는 실행되지 않고 재실행 시 현재 파일 상태를 대조한다.
  큰 볼트는 스캔 시간만큼 반영이 늦어질 수 있다. native FSEvents 대신 polling이다.
- 새 연결과 기존 `auto_sync` 필드가 없는 연결 모두 기본 켜짐이다.
  연결별 체크박스로 중지·재개하고 설정을 영구 저장한다.
- 문서 추가·수정·링크 변경을 색인하고, 변경이 있을 때 앱 내부 관계 지도를
  재구성한다. 첫 확인도 지도를 구성한다. 변경이 없으면 관계 재구성을 생략한다.
  수동 읽기도 같은 경로를 사용해 관계까지 갱신한다.
- 파일 삭제·이름 변경은 이전 DB 자료와 대조한다. 사라진 원문은 내부에서 삭제
  표시하고 검색·관계에서 제외한다. 사용자 메모와 원문의 마지막 내용은 보존한다.
  앱 종료 중 삭제한 문서도 재실행 후 반영하며 복원한 파일은 다시 수집한다.
- 전체 스캔 실패 시 삭제 판정을 하지 않는다. 접근 불가 파일이 있으면 해당
  순회의 누락 파일 삭제 판정을 보류한다. 한 볼트의 실패는 다음 볼트를 막지 않는다.
  선택 경로가 다른 위치의 symlink로 바뀌면 자동으로 범위를 확장하지 않는다.
- 수동/자동 읽기는 직렬화하며, 연결 해제·자동 갱신 끄기가 반환된 뒤에는
  이전에 대기하던 스캔이 시작되지 않는다. 파일/폴더/Obsidian 설정에 쓰지 않는다.
- 연결 화면은 갱신 중 상태, 문서·변경·삭제·접근 불가 개수, 마지막 확인·관계
  갱신 시각과 오류를 조회한다. 다른 화면으로 이동해도 backend 작업은 계속된다.
- 캐시 삭제 후 자동 갱신이 켜져 있으면 다시 수집된다. 유지하려면 해당 연결의
  자동 갱신을 먼저 끈다. 이미 열린 검색 결과와 메인 3D 데모 화면은 자동 갱신
  대상이 아니며, 맥락 검색을 다시 실행하면 최신 내부 자료를 조회한다.

## 기존 에이전트 인증 재사용 검토

| 인증 위치 | 판단 | 이번 처리 |
| --- | --- | --- |
| 로컬 Obsidian 경로 | API 인증 불필요. 앱 프로세스에 파일 읽기 권한이 있으면 가능 | 경로를 추가하고 즉시 읽기 가능 |
| 로컬 GitHub CLI | `gh auth token --hostname github.com --user <login>`으로 특정 계정 선택 가능 | `gh_cli` 연결이 요청 시 메모리에서 사용. Keychain에 복제하지 않음 |
| 에이전트에만 연결된 원격 GitHub/Jira 도구 | 도구 호출 가능 여부와 credential export는 별개 | 이 세션의 노출 도구에서 credential 전달 계약을 확인하지 못했으므로 직접 공유 미지원 |
| 다른 프로그램의 OAuth 세션 | client, audience, scope, refresh 소유권 확인 필요 | 다른 앱의 비공개 저장소·브라우저 쿠키를 읽어 복사하지 않음 |
| 기존 Aidebook Keychain 항목 | 항목은 보존됨. 구버전은 저장소 소유자를 계정으로 사용해 실제 로그인과 구분 불가 | 자동 계정 추정·복사를 하지 않음. 새 연결에서 gh 사용 또는 연결별 토큰 등록 |

로컬 확인: `gh 2.74.2`가 설치되어 있고 `gh auth status --hostname github.com`에서
`yoonhoGo` 인증이 유효했다. 토큰 값은 출력하지 않았다. `gh auth token --help`에서
`--user` 지원도 확인했다. 이 버전의 `gh auth status`는 JSON 옵션을 지원하지 않아
기본 상태 명령의 결과에서 계정명과 성공 여부만 확인했다.
`GH_TOKEN` 등의 환경 변수가 다른 계정을 선택하지 않도록 gh 재사용 명령에서는
해당 변수들을 제거한다. 전역 active account를 전환하지 않는다.

## 신규 인증 선택

| 공급자 | 우선 검토 방식 | 구현 / 조건 |
| --- | --- | --- |
| Obsidian | 없음 | 로컬 경로 최우선 |
| GitHub | 기존 gh 로그인 → `gh auth login` 브라우저 인증 → fine-grained PAT | gh 재사용과 PAT 지원. 저장소와 Issues/Pull requests 읽기 권한을 최소화 |
| GitHub 자체 OAuth | Device flow | 별도 OAuth/GitHub App 등록과 client ID가 필요. 이번에 내장 로그인 flow는 구현하지 않음 |
| Jira 개인 연결 | API token + 계정 이메일 | 현재 unscoped 개인 API token과 `https://<site>.atlassian.net` 지원 |
| Jira OAuth | OAuth 2.0 authorization code grant (3LO) | 배포용 우선 방향. 앱 등록, callback, state 검증, 토큰 교환·갱신, accessible-resources/cloudId 선택이 필요. 아직 미구현 |
| Jira scoped API token | API gateway + cloudId | `https://api.atlassian.com/ex/jira/{cloudId}` 경로 필요. 현재 연결 폼에는 미지원으로 명시 |

Atlassian은 배포용 cloud integration에 3LO를 권장한다. API token 수집 방식에는
별도 배포 정책 제약이 있으므로 이번 개인용 직접 연결을 marketplace 배포용
인증 설계로 확정하지 않는다. 토큰 만료·권한 부족은 재인증 오류로 표시한다.

## 호환성과 남은 경계

- 구버전 `aidebook-native-scopes-v1` localStorage와 기존 Keychain/캐시를 삭제하지
  않는다. 기존 단일 선택 값은 새 연결 폼으로 가져와 사용자가 저장할 수 있다.
  GitHub 로그인 계정은 새로 지정해야 한다. 기존 Tauri 단일 연결 명령은 호환용으로
  남겼고 새 연결 화면은 `plugin_*` 명령을 사용한다.
- Obsidian 다중 연결은 아래 자동 갱신을 지원한다. GitHub·Jira는 수동 읽기다.
  CLI/MCP 연결 관리는 아래 명령으로 지원한다. 자체 OAuth 로그인/refresh flow는 아직 미구현이다.
- DB 백업은 `connections.json`과 Keychain을 포함하지 않는다.
- 브라우저 미리보기는 실제 연결 저장·토큰 저장을 차단한다.

## 검증

- TypeScript/Vite build, Vitest 기존 7개 통과.
- 자동 갱신 통합 회귀: 문서·링크 변경, 삭제·이름 변경·복원, 무변경 시 지도 유지,
  재시작 후 삭제 감지, 중지 설정 복원, 수동 읽기, 연결 해제, 경로 일시 오류와 회복,
  다른 볼트 갱신 지속, 종료 플래그, 원본 내용·수정시각 보존을 임시 볼트로 검증.
- Rust 최종 전체 테스트 통과(신규 플러그인 회귀 4개 포함).
- `cargo fmt -- --check`, `git diff --check` 통과. Vite의 기존 대형 graph chunk 경고는 남아 있다.
- 회귀: 복수 볼트 재시작 복원·같은 파일명 분리 색인, 로컬 우선 순서,
  같은 저장소의 복수 계정, ID 덮어쓰기 거부, 연결별 해제,
  Jira host/project 검증·페이지 순회·반복/불완전 cursor 실패.
- Aside 브라우저에서 Obsidian 기본 선택, GitHub/Jira 폼 전환, 브라우저 저장
  비활성화를 확인했다. 이것은 native Tauri IPC나 실계정 자료 읽기 검증이 아니다.
- 미검증: native 앱에서 실제 볼트·Jira·GitHub 자료 갱신, Keychain 저장/삭제
  프롬프트와 접근 권한, iCloud 상태. gh 인증 상태 확인은 자료 수집 성공의 증거가 아니다.

## 공식 자료

- [GitHub CLI: 계정별 토큰 조회](https://cli.github.com/manual/gh_auth_token)
- [GitHub CLI: 새 로그인](https://cli.github.com/manual/gh_auth_login)
- [GitHub OAuth와 device flow](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps)
- [GitHub fine-grained PAT](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens)
- [Jira OAuth 2.0 (3LO)](https://developer.atlassian.com/cloud/jira/platform/oauth-2-3lo-apps/)
- [Jira 이메일 + API token 인증 및 배포 제약](https://developer.atlassian.com/cloud/jira/platform/basic-auth-for-rest-apis/)
- [Atlassian API token과 scoped token URL](https://support.atlassian.com/atlassian-account/docs/manage-api-tokens-for-your-atlassian-account)
- [Jira enhanced JQL search](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/)


## MCP / CLI 연결 관리 — 2026-09-22

앱·MCP·CLI는 Core owner의 같은 `connections.json` registry와 동기화 잠금을 사용한다.
여기서 플러그인은 내장 Obsidian/GitHub/Jira의 연결 인스턴스다. 실행 코드를 설치하거나
Codex 플러그인 패키지를 삭제하는 기능은 아니다.

| MCP | CLI | 동작 |
| --- | --- | --- |
| `plugins.list` | `plugins list` | 저장된 연결 목록 |
| `plugins.get` | `plugins get --id ID` | 한 연결의 설정 |
| `plugins.add` | `plugins add` | 고유 ID로 새 연결 추가; 중복 ID는 거부 |
| `plugins.update` | `plugins update --id ID` | 지정한 필드만 수정; ID/provider 고정 |
| `plugins.remove` | `plugins remove --id ID` | 해당 연결 해제; 캐시·메모·원본·인증 보존 |
| `plugins.refresh` | `plugins refresh --id ID` | 실제 저장된 경로/저장소/프로젝트 읽기 |

앱을 실행한 뒤 아래처럼 사용한다. 개발 빌드 CLI는 `src-tauri/target/debug/aidebook-cli`다.
`--data-dir`는 `--socket`/`--token-file` 쌍 또는 기존 IPC 환경 변수 대신 사용할 수 있다.

```sh
AIDEBOOK_DATA_DIR="$HOME/Library/Application Support/com.yoonhogo.aidebook"
aidebook-cli plugins list --data-dir "$AIDEBOOK_DATA_DIR"
aidebook-cli plugins add --data-dir "$AIDEBOOK_DATA_DIR" \
  --id bbros --provider obsidian --label bbros \
  --scope "$HOME/Library/Mobile Documents/iCloud~md~obsidian/Documents/bbros"
aidebook-cli plugins update --data-dir "$AIDEBOOK_DATA_DIR" \
  --id bbros --label '회사 노트' --auto-sync false
aidebook-cli plugins refresh --data-dir "$AIDEBOOK_DATA_DIR" --id bbros
aidebook-cli plugins get --data-dir "$AIDEBOOK_DATA_DIR" --id bbros
# 실제로 해제할 때만 실행:
aidebook-cli plugins remove --data-dir "$AIDEBOOK_DATA_DIR" --id bbros
```

MCP `plugins.add` 인수는 `id, provider, label, account, scope, auth`를 포함한다.
Obsidian은 `account: ""`, `auth: "local"`; 선택적 `auto_sync` 기본값은 true다.
CLI는 Obsidian/local, GitHub/gh_cli, Jira/token을 기본 인증 방식으로 사용한다.
GitHub는 `--account LOGIN --scope OWNER/REPO`, Jira는 `--account EMAIL
--scope https://TENANT.atlassian.net --project KEY`가 필요하다.
토큰 자체는 앱에서 저장하며 도구 인수로 받지 않는다.

부분 수정의 MCP 인수 및 CLI `--params`는 같은 형태다:

```json
{"id":"bbros","changes":{"label":"회사 노트","auto_sync":false}}
```

`--params`를 사용하면 해당 JSON이 일반 필드 옵션을 대체한다. 수정 가능한 필드는
`label/account/scope/project/auth/auto_sync`다. GitHub·Jira의 auto_sync는 자동 조회를
활성화하지 않으며 현재 로컬 vault에만 적용된다. 인증 방식/계정을 바꾸어도 기존
Keychain 항목은 자동 변경하지 않는다. 필요한 인증은 앱에서 설정한다.
연결 해제는 재추가로 복구할 수 있고 원본 문서는 변경하지 않는다.

자동 갱신/수동 읽기와 수정/해제는 직렬화한다. 경로 수정 후 다음 읽기부터 새 경로를
사용한다. 연결 관리 요청의 IPC 응답 대기는 120초이며 시간 초과가 작업 취소를
뜻하지는 않는다. 재시도 전 `plugins.get/list`와 `connections.status`로 확인한다.
앱 연결 화면은 외부 변경을 2초 간격으로 다시 읽는다.

기존 설치는 업데이트한 앱을 실행하고 설정 → 에이전트 연결에서 설치/갱신한 뒤
에이전트의 새 세션을 시작한다. 앱 Core와 복사된 MCP 실행 파일 모두 새 버전이어야 한다.
앱 내장 CLI도 `--aidebook-agent call plugins.list --data-dir DIR`로 사용할 수 있으며
JSON 인수는 stdin으로 전달한다. `describe`로 현재 도구 목록을 확인할 수 있다.

검증: 임시 실제 vault와 SQLite, 인증 Unix socket, CLI/MCP 자식 프로세스로 다중 연결,
부분 수정·재시작 복원·경로 변경 후 읽기·해제 후 수집 중단·캐시 보존·실패 시 설정 보존을
검증한다. 실제 원격 계정/Keychain 변경은 별도 검증 경계다.

2026-09-22 실사용 확인: release 앱과 설치된 Codex bridge를 갱신했다. 기존 개인 vault를
유지하고 CLI로 bbros를 추가했으며, 설치된 MCP `plugins.list/refresh`로 두 연결과 bbros
732개 색인·접근 불가 0개를 확인했다. native 연결 화면에서도 두 vault의 자동 갱신이
표시되었다. 위 수치는 해당 시점의 검증 결과다. Codex의 새 대화에서 새 도구 목록을 불러온다.
