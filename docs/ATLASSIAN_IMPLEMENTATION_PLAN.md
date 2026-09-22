# Jira 개인 범위와 Confluence 연결 구현 계획

작성: 2026-09-22 · 상태: 구현 및 로컬 검증 완료 · 실계정/native 검증 대기

## 목표와 사용자 흐름

- Jira: 새 연결은 프로젝트/보드 제한 없는 내 생성·담당 티켓을 기본으로 한다. 보고자 포함은 선택한다. 기존 저장 연결은 프로젝트 범위를 유지한다.
- 상위 티켓: 선택 시 parent 관계를 따라 상위 작업/Epic을 추가 조회한다. 중복과 순환을 방지하고 조회 권한/불완전 응답 오류에서 기존 캐시를 보존한다. 일반 관련 링크를 부모로 추정하지 않는다.
- Confluence: 내가 작성한 페이지, Watch 중인 페이지, 직접 선택한 페이지 모드를 제공한다. 저장된 계정으로 검색하고 결과 또는 같은 사이트 URL/ID를 선택한 뒤 명시적으로 읽는다. 검색 자체는 색인하지 않는다.
- 최근 열람 기록, 임의 연관 티켓 확장, 원격 자동 갱신, OAuth/scoped-token 인증은 이번 범위 밖이다. 원격 원문을 수정하지 않는다.

## 데이터 및 호환 계약

`PluginConnection`과 부분 수정 계약에 다음 기본값을 추가한다.

| 필드 | 저장 호환 기본값 | 신규 UI 기본값 |
| --- | --- | --- |
| jira_scope | project | mine |
| jira_include_reporter | false | false |
| jira_include_parents | false | true |
| confluence_mode | authored | authored |
| confluence_page_ids | [] | [] |

Provider에 confluence를 추가한다. 기존 connections.json은 마이그레이션 없이 역직렬화하며 계정별 Keychain을 유지한다. 범위 편집은 같은 연결 ID를 보존한다. Jira site+key, Confluence site+page ID로 원문 정체성을 유지한다.

## 작업 분담

1. jira_core 서브에이전트: 공통 연결 모델·검증·레지스트리, Jira 개인 JQL·상위 티켓 수집·회귀 테스트.
2. confluence_core 서브에이전트: 독립 Confluence connector, 검색 API, 본문 변환, pagination/URL 검증·회귀 테스트.
3. plugin_ui 서브에이전트: 연결 편집, Jira 조회 옵션, Confluence 검색·선택·URL 추가 화면.
4. 주 에이전트: Tauri/IPC/MCP/CLI 연결 계약, 통합 빌드·테스트·문서 및 결과 검토.

파일 소유권을 분리하고 공통 모델은 jira_core만 변경한다. 통합 후 전체 Rust·TypeScript 검증을 수행한다.

## 완료 기준

- 보드/프로젝트 제한 없는 개인 Jira 조회, 선택적 보고자 포함, 상위 관계 수집과 중복 방지.
- 기존 프로젝트 연결 복원, 부분 수정 시 누락 설정 유지.
- Confluence 작성/Watch/선택 모드, 검색 결과 선택 및 같은 사이트 URL 추가.
- 네트워크 오류·반복 pagination·불완전 수집 시 기존 캐시 보존; 토큰은 로그/설정/DB에 노출하지 않음.
- npm test/build, cargo test, cargo fmt 검사 및 diff 검사.
- fixture/로컬 검증과 실제 Jira·Confluence 계정 및 native Keychain 검증은 구분하여 기록.

## 공식 API 참고

- https://support.atlassian.com/jira-software-cloud/docs/jql-functions/
- https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/
- https://developer.atlassian.com/cloud/confluence/cql-fields/
- https://developer.atlassian.com/cloud/confluence/rest/v1/api-group-search/

## 결과

세 작업을 통합했고 로컬 Rust/CLI/MCP 회귀와 frontend build/test를 통과했다. 실제 계정과 native 검증, 엄격한 Clippy의 기존 lint 경계는 [구현 상태](IMPLEMENTATION_STATUS.md)에 기록했다.
