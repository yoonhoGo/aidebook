# 에이전트 연결 설치

2026-09-22. 설정 → 에이전트 연결에서 대상별 **한 번에 설치**를 누른다.
설치는 기본 사용자 설정만 변경하며 에이전트 프로그램 자체나 로그인은 설치하지 않는다.
Aidebook 앱은 실행 중이어야 한다. 실제 사용자 환경에는 작업 중 자동 설치하지 않았으며,
사용자가 앱의 설치 버튼을 누를 때 적용된다.

## 지원 경로

| 대상 | 설치되는 기능 | 기본 사용자 위치 |
| --- | --- | --- |
| Codex | 스킬 + stdio MCP | `~/.agents/skills/aidebook-codex/SKILL.md`, `~/.codex/config.toml`의 `mcp_servers.aidebook` |
| Claude Code | 스킬 + stdio MCP | `~/.claude/skills/aidebook-claude-code/SKILL.md`, `~/.claude.json`의 사용자 `mcpServers.aidebook` |
| Hermes | 스킬 + stdio MCP | `~/.hermes/skills/aidebook-hermes/SKILL.md`, `~/.hermes/config.yaml`의 `mcp_servers.aidebook` |
| Pi | 스킬 + TypeScript 확장 플러그인 | `~/.pi/agent/skills/aidebook-pi/SKILL.md`, `~/.pi/agent/extensions/aidebook/index.ts`와 `config.json` |

현재 자동 설치 대상은 위 기본 사용자 경로다. `CODEX_HOME`, `CLAUDE_CONFIG_DIR`,
`HERMES_HOME`, 별도 Pi profile 등의 사용자 지정 환경은 자동 추적하지 않는다.
설치 전에 카드의 실제 대상 경로를 확인하고, 다른 profile을 쓰는 경우 생성된
설정/번들을 해당 profile에 별도로 등록한다. 해당 에이전트의 실행/설치 여부를
추정해 "연결됨"으로 표시하지 않고 **연결 설정 설치됨**으로 구분한다.

Codex·Claude·Hermes는 새 세션을 시작한다. Pi는 새 세션 또는 `/reload`를 사용한다.
예시 요청: “Aidebook에서 이 작업과 관련된 노트와 최근 결정을 찾아줘.”

## 앱에 포함된 실행 기능

별도 `aidebook-cli`, Node, Python, Homebrew가 필요하지 않도록 앱의 실행 파일에
`--aidebook-agent` 모드를 포함한다. 설치 시 현재 앱 실행 파일을 앱 데이터 폴더의
`agent-integrations/bin/aidebook-agent`로 복사하고, 절대 경로와 앱 데이터 경로를
MCP 설정에 기록한다. 경로의 공백은 배열 인수로 보존하며 shell을 거치지 않는다.
Pi 확장은 Pi 자체 Node 런타임과 내장 모듈만 사용한다.

- `mcp`: stdin/stdout JSON-RPC. UI 창이나 두 번째 DB owner를 시작하지 않는다.
- `call <method>`: Pi가 stdin으로 전달한 JSON을 기존 인증 IPC로 전달한다.
- `describe`: 동일한 MCP 도구 schema 목록을 제공한다.
- `probe`: 기존 코어에 읽기 요청을 보내고 연결 여부와 도구 수만 반환한다.

MCP 메시지마다 IPC 토큰 파일을 다시 읽어 앱 재시작의 토큰 교체를 반영한다.
앱이 꺼져 있어도 initialize/tools/list는 동작하며 실제 데이터 요청은 오류를 반환한다.
토큰 값이나 GitHub/Jira credential은 에이전트 설정·스킬·설치 기록으로 복사하지 않는다.
IPC 읽기/쓰기에는 15초 제한, 연결 검사에는 10초 제한, Pi 호출에는 30초 제한을 둔다.

## 스킬과 도구

공통 스킬은 `integrations/aidebook/skills/aidebook-memory/SKILL.md`에서 관리한다.
현재 19개 도구는 기존 Core의 whitelist/schema를 공유한다. Pi는 동일한 도구 이름의
점을 밑줄로 치환하고 `aidebook_` 접두사를 붙인다.

- `plugins.list/get/add/update/remove/refresh`: 내장 공급자의 다중 연결 관리와 실제 자료 갱신
- 자료 검색, 전체 맥락 조회, 근거 확인, freshness/접근 상태 확인
- 명시적 요청에 따른 관찰 → 후보 추출 → 제안
- 명시적 메모 쓰기/철회와 연결 상태 조회

candidate 승인·거부는 에이전트 도구에 노출하지 않는다. 자동 스킬 발견은 켜 두되,
스킬 설치만으로 모든 대화를 수집하거나 원본 문서를 수정하지 않는다.

## Codex·Claude 플러그인 번들

각 대상의 설치 디렉터리 아래 `aidebook/`에 `.codex-plugin/plugin.json`,
`.claude-plugin/plugin.json`, 실행 경로가 채워진 `.mcp.json`, 공통 스킬을 생성한다.
이 디렉터리는 Codex·Claude의 플러그인 배포/등록에 사용할 수 있는 번들이다.
원본 저장소의 `.mcp.json`은 템플릿이며 설치 시 실제 앱 실행 경로로 대체된다.

**기본 원클릭 설치는 스킬 + MCP 직접 등록이다. Codex·Claude 플러그인 마켓플레이스에
자동 등록/활성화하지 않는다.** Pi의 확장 플러그인은 자동 발견 위치에 직접 설치한다.
Codex·Claude에서 네이티브 플러그인 형태로 따로 등록하려면 기존 직접 연결과 중복하지
않도록 한 방식만 사용한다. 설치 화면에서도 이 차이를 표시한다.

## 설정 보존과 해제

- 설치 전 전체 설정 파싱과 충돌 검사를 수행한다. 다른 `aidebook` MCP 항목이 있으면
  덮어쓰지 않는다. 손상된 설정이나 지원하지 않는 형식도 변경하지 않는다.
- TOML은 기존 주석을 유지한다. JSON/YAML은 다른 설정 값을 유지하되 재직렬화하므로
  서식과 YAML 주석은 달라질 수 있다. 원본 파일은 먼저 백업한다.
- 변경 파일 옆에 UUID가 포함된 `.aidebook-backup-*` 백업을 0600으로 저장한다.
  설정에는 기존 비밀 값이 있을 수 있으므로 백업도 그대로 보호한다.
- 임시 파일 + rename으로 저장하고, 실패 시 이번에 쓴 파일만 되돌리도록 시도한다.
  설치 중 다른 프로그램이 바꾼 파일은 다시 덮어쓰지 않는다. 백업 경로는 UI에 표시한다.
- 반복 설치는 같은 설정을 중복 추가하지 않는다. 앱 업데이트 후 다시 설치하면 실행
  파일과 관리 대상 패키지를 교체한다. 사용자가 수정한 스킬/확장은 덮어쓰지 않는다.
- 연결 해제는 기록과 현재 내용이 일치하는 Aidebook 항목만 제거한다. 다른 MCP 서버,
  사용자 수정 파일, 앱의 메모/자료, 다른 에이전트 연결은 유지한다. 공용 실행 파일과
  백업은 유지한다. 심볼릭 링크인 설정 파일은 자동 변경하지 않는다.

## 검증과 제한

- Rust 임시 home 테스트: 네 대상 설치·재설치·해제, 기존 설정 유지, 기존 항목 충돌,
  파싱 실패 시 무변경, 사용자 수정 스킬 보존.
- 실제 복사한 앱 실행 파일 + 임시 CoreServer: probe, MCP initialize/tools/list/search,
  Pi용 stdin 호출, 승인 API 거부, 앱 종료 후 도구 목록 응답.
- `scripts/test-agent-extension.mjs`: 확장 등록 19개, 실제 자식 프로세스 전달,
  공백/한글/셸 표현식의 인수 보존, 취소를 임시 실행 환경에서 검증.
- skill-creator와 plugin-creator validator, Claude Code plugin validator 사용.
- frontend build·Vitest와 전체 Rust 테스트 통과. 브라우저에서 설정 → 에이전트 연결의
  네 카드 표시 및 미리보기 설치 비활성화를 확인했다.
- 실제 Codex·Claude·Hermes·Pi 대화에서의 도구 호출, native Tauri 창에서 설치 버튼,
  서명된 배포 앱 복사 후 실행은 별도 검증이 필요하다. 현재 Hermes·Pi 명령은 이 기기의
  PATH에서 발견되지 않았다. 설정 설치와 외부 에이전트 실행 성공은 구분한다.

## 공식 자료

- [Codex MCP 설정](https://developers.openai.com/codex/mcp/)
- [Codex 사용자 스킬 경로](https://developers.openai.com/codex/skills/)
- [Claude Code 사용자 MCP 범위](https://code.claude.com/docs/en/mcp)
- [Claude 플러그인 구조](https://code.claude.com/docs/en/plugins-reference)
- [Hermes MCP](https://hermes-agent.nousresearch.com/docs/user-guide/features/mcp)
- [Hermes 스킬](https://hermes-agent.nousresearch.com/docs/user-guide/features/skills/)
- [Pi 확장 API와 발견 경로](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/extensions.md)
- [Pi 스킬](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/skills.md)
