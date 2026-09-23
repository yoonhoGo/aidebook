# 플러그인 연결과 인증

검토일: 2026-09-23. 우선 대상은 **Obsidian → GitHub → Atlassian(Jira·Confluence)**다.
외부 시스템에는 쓰지 않으며, 로컬 경로 연결은 네트워크 로그인에 의존하지 않는다.

## 통합 Atlassian 연결 — 2026-09-23

- 새 연결은 사이트·계정 하나와 `jira_enabled`/`confluence_enabled` 선택을 저장한다.
  티켓·문서의 읽기 범위는 각각 유지한다. 같은 사이트의 기존 `jira`/`confluence`
  연결은 자동 병합하거나 삭제하지 않고 계속 읽고 수정할 수 있다.
- 연결 설정 폼에서 API token을 입력하면 선택한 제품의 읽기 API 접근을 확인한 뒤
  연결별 Keychain에 저장한다. OAuth를 고르면 Client ID와 secret을 설정 폼에서
  입력하고 저장 후 브라우저 승인을 시작한다. OAuth token과 secret은 Keychain에
  두고 연결 파일에는 저장하지 않는다. 인증 후 제품별 접근 확인 결과를 표시한다.
- Atlassian 3LO 앱에는 선택한 제품에 따라 `read:jira-work`,
  `search:confluence`, `read:confluence-content.all` 및 `offline_access`가 필요하다.
  승인된 같은 사이트의 cloudId와 각 scope를 대조한다. OAuth API 요청은
  `api.atlassian.com/ex/jira|confluence/{cloudId}`로 보낸다.
- 문서 검색은 후보만 반환한다. 인증·검색·범위 저장만으로 원문을 가져오지 않으며
  `읽기 / 다시 확인`에서 선택된 제품을 읽는다. 한 제품의 읽기가 실패해도 다른
  제품의 읽기는 시도하고 실패를 표시한다.
- MCP/CLI에서는 `provider: "atlassian"`과 제품 선택 필드를 설정할 수 있다.
  CLI 예: `aidebook-cli plugins add --id work --provider atlassian --label Work
  --account me@example.com --scope https://team.atlassian.net --jira-scope mine
  --jira-enabled true --confluence-enabled true`. 인증 정보는 데스크톱 앱에서 설정한다.
- 이 앱의 현재 3LO 방식은 개인 로컬 설정이다. 공유 배포 시 고객별 Client secret
  입력 방식은 별도 배포용 인증 설계로 교체해야 한다.

근거: [Atlassian 3LO의 복수 제품 scope와 API gateway](https://developer.atlassian.com/cloud/jira/platform/oauth-2-3lo-apps/),
[Confluence CQL 검색 scope](https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-search/).

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
- Jira Cloud는 `POST /rest/api/3/search/jql`로 프로젝트 또는 내 생성·담당 티켓을 조회한다.
  보고자 포함과 상위 티켓 포함을 선택할 수 있다. 기존 연결은 프로젝트 범위를 유지한다.
  상위 티켓은 프로젝트를 넘어 조회하며 제목·상태·수정 시각·원문과 부모 링크를 저장한다.
  반복 cursor, 불완전 페이지, 접근 불가 상위 티켓은 전체 수집을 실패 처리한다.
- Confluence Cloud는 작성/Watch CQL 또는 선택 ID로 페이지를 읽는다. 검색은 후보만 반환하고
  선택 ID 저장 후 수동 읽기를 실행한다. storage 본문은 실행하지 않는 텍스트로 변환한다.
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
| GitHub 자체 OAuth | Device flow | Device flow를 켠 GitHub App client ID를 연결에 저장하고 브라우저에서 승인. 실제 접근 범위는 GitHub App에 부여한 저장소 권한에 따름 |
| Jira 개인 연결 | API token + 계정 이메일 | 현재 unscoped 개인 API token과 `https://<site>.atlassian.net` 지원 |
| Jira OAuth | OAuth 2.0 authorization code grant (3LO) | 3LO 앱의 client ID와 Keychain client secret을 사용. loopback callback, state 검증, 토큰 교환·갱신, accessible-resources/cloudId 대조를 구현 |
| Jira scoped API token | API gateway + cloudId | `https://api.atlassian.com/ex/jira/{cloudId}` 경로 필요. 현재 연결 폼에는 미지원으로 명시 |

GitHub OAuth 연결은 GitHub App에서 device flow를 활성화하고 필요한 저장소에
읽기 권한을 부여한 후 client ID를 입력한다. 브라우저의 사용자 코드를 승인하면
앱이 최소 polling 간격을 지켜 결과를 확인한다. `/user` 로그인 이름이 연결의
GitHub 계정과 다르면 토큰을 저장하지 않는다. 만료형 토큰의 refresh token도
연결별 Keychain에 보관하고 갱신한다.

Jira OAuth 연결은 Atlassian Developer Console에서 3LO 앱을 준비하고
`read:jira-work`, `offline_access`를 활성화한다. Callback URL은
`http://127.0.0.1:48913/aidebook/oauth`로 정확히 등록한다. 연결에는 client ID만
저장하고 client secret과 access/refresh token은 각각 연결별 Keychain 항목에 저장한다.
앱은 loopback listener로 콜백을 받고 state를 검증하며, 승인된 사이트 목록에서
설정된 `https://<site>.atlassian.net`과 일치하는 cloudId만 사용한다.
OAuth 연결도 명시적으로 선택한 Jira 티켓 범위만 읽는다. 3LO는 앱 등록이
필요하며 개인용 로컬 설정을 대상으로 구현했다. 공유 배포를 위해서는 앱 소유자가
client secret을 안전하게 운영하는 별도 인증 구성이 필요하다.

API 키 방식은 기존 GitHub fine-grained PAT, Jira 계정 이메일 + unscoped API
token을 유지한다. Atlassian의 배포 정책상 고객의 API token을 수집하거나
각 고객에게 3LO 앱 생성을 요구하는 cloud 앱에는 제약이 있으므로 이 개인용
설정을 marketplace 배포용 인증으로 취급하지 않는다.

## 호환성과 남은 경계

- 구버전 `aidebook-native-scopes-v1` localStorage와 기존 Keychain/캐시를 삭제하지
  않는다. 기존 단일 선택 값은 새 연결 폼으로 가져와 사용자가 저장할 수 있다.
  GitHub 로그인 계정은 새로 지정해야 한다. 기존 Tauri 단일 연결 명령은 호환용으로
  남겼고 새 연결 화면은 `plugin_*` 명령을 사용한다.
- Obsidian 다중 연결은 아래 자동 갱신을 지원한다. GitHub·Jira는 수동 읽기다.
  CLI/MCP 연결 관리는 아래 명령으로 지원한다. OAuth 브라우저 승인과 client
  secret 등록은 데스크톱 앱에서 진행한다.
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

## 에이전트에 이미 연결된 MCP

현재 Aidebook MCP는 Aidebook Core의 자료를 Codex·Claude Code에 **제공**하는
서버다. 에이전트에 등록된 다른 GitHub/Jira MCP 서버의 세션과 인증 정보를
Aidebook이 역방향으로 읽는 클라이언트는 구현되어 있지 않다. 해당 MCP 서버가
독립적인 접속 방식과 읽기 도구를 제공한다면, 서버 주소·권한·도구 계약을
명시적으로 확인한 다음 별도의 Aidebook MCP 클라이언트 연결로 가져올 수 있다.
에이전트 설정 파일의 토큰이나 브라우저 세션을 자동 복사하지 않는다.

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


## Jira 개인 범위 / Confluence — 2026-09-22

구현 계획: [ATLASSIAN_IMPLEMENTATION_PLAN.md](ATLASSIAN_IMPLEMENTATION_PLAN.md).

- 연결 화면의 **설정 편집**에서 기존 Jira 연결을 `내 티켓`으로 전환할 수 있다.
  신규 UI 연결은 내 티켓 + 상위 티켓 포함이 기본이다. reporter는 creator와 구분한다.
- Jira 상위 티켓은 20단계까지 수집하고 중복/순환을 막는다. 부모 key/URL을 원문 근거로
  보존하지만 별도 canonical Relation 생성이나 계층 트리 전용 UI는 이번 범위가 아니다.
- Confluence는 사이트·계정 이메일·개인 API token 연결을 저장하고 인증 정보를 설정한다.
  `검색 / URL로 선택한 문서`를 선택하면 저장된 계정으로 검색하거나 같은 사이트의
  `/wiki/spaces/.../pages/ID/...`, `/wiki/pages/viewpage.action?pageId=ID` 또는 ID를 추가한다.
  선택 범위를 저장한 다음 `읽기 / 다시 확인`을 누른다. 검색만으로는 색인하지 않는다.
- Watch는 구독 상태이며 최근 열람 기록은 포함하지 않는다. 첨부파일/매크로 실행은 미지원이다.
- Jira·Confluence 모두 수동 갱신이다. 범위 축소나 선택 해제는 다음 수집 범위를 바꾸며,
  이미 색인한 원문 캐시와 메모를 삭제하지 않는다.
- Confluence 인증은 기존 사이트 직접 접근용 unscoped token 방식이다.
  Jira OAuth/cloudId 지원은 위 신규 인증 선택 절을 따른다. scoped API token은 미지원이다.

CLI 예시:

```sh
aidebook-cli plugins add --id my-jira --provider jira --label '내 티켓' \
  --account me@example.com --scope https://TEAM.atlassian.net \
  --jira-scope mine --jira-include-parents true
aidebook-cli plugins update --id my-jira --jira-include-reporter true
aidebook-cli plugins search --id my-confluence --query '설계'
aidebook-cli plugins update --id my-confluence --confluence-mode selected --confluence-page-ids 123,456
aidebook-cli plugins refresh --id my-confluence
```

CLI는 `--project`가 있으면 project, 없으면 mine을 사용하며 상위 포함은 명시적으로 켠다.
MCP `plugins.confluence.search`는 `{ "id": "my-confluence", "query": "설계" }`를 받는다.
`plugins.add/update`의 추가 필드는 계획 문서의 데이터 계약과 같다.
앱/bridge 업데이트 후 새 에이전트 세션에서 갱신된 도구 목록을 사용한다.
