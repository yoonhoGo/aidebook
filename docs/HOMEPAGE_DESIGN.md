# Aidebook 제품 소개 홈페이지

작성: 2026-09-22

## 작업 위치와 근거

- 독립 jj workspace: `/Users/yoonho.go/workspace/aidebook-homepage`, 이름 `aidebook-homepage`.
- 분기 기준: 원래 workspace의 `lxvktpls` 작업 스냅샷. 기존 미완료 앱 작업은 수정하지 않고 새 workspace의 `site/`에서만 구현.
- 제품 판단은 현재 `README.md`, `IMPLEMENTATION_STATUS.md`, `src/App.css`, 기존 노트 모양 브랜드 SVG를 참고했다.
- 핵심 가치: AI 비서가 다시 쓸 작업 맥락을 로컬에 저장하고, 원본 근거와 사용자 검토를 보존한다.
- Obsidian/GitHub/Jira 연결은 개발 중인 기능으로 소개. 실제 계정/네이티브/공개 배포 검증 완료를 주장하지 않는다.

## 공식 레퍼런스와 적용

2026-09-22 공식 홈페이지를 직접 열고 브라우저 화면을 캡처했다. 검색 결과의 오래된 Heptabase 문구 대신 현재 공식 홈페이지를 기준으로 판단했다.

| 레퍼런스 | 관찰 | Aidebook 적용 |
| --- | --- | --- |
| [mymind](https://mymind.com/) | 큰 세리프 제목, 기억에 대한 짧은 가치 제안, 넓은 여백 | “다음 대화도, 이어서.”, 종이와 노트의 인상 |
| [Heptabase](https://heptabase.com/) | 중앙 정렬 메시지, 대형 제품 영상, 자료별 예시 | 자료→기억→근거 패널을 하나의 예제로 이해시키는 UI |
| [Obsidian](https://obsidian.md/) | 개인 데이터와 로컬 저장에 대한 직접적인 문장, 노트/그래프 화면 | 로컬 보관, 읽기 전용 연결, 검토와 이력 영역 |

검토용 캡처와 설명은 `site/references.html`. 공개 빌드에서는 이 페이지와 타사 캡처를 제외한다.

## 디자인 결정

- 방향: Quiet notebook, visible evidence.
- Paper `#f5f8fd`, Ink `#18345b`, Memory `#dcecff`, Evidence `#0866df`.
- 제목은 시스템 명조/세리프, 본문은 시스템 산세리프, 주석은 작은 모노스페이스.
- 사용자가 제공한 aidebook-assistant-logo.png를 헤더·푸터에, aidebook-assistant-icon.png를 파비콘·Apple touch icon·마지막 소개 영역에 적용. 원본 비율과 색상을 보존.
- 페이지 순서: 히어로 → 인터랙티브 예시 → 문제 → 연결/검토/조회 → 통제권 → FAQ → CTA.
- 메인 그래프는 정교한 3D 우주보다 자료와 결정의 이유를 읽을 수 있는 2D 카드로 구성. 이 홈페이지의 설명용 모델이며 실제 앱 레이아웃 캡처가 아니다.
- 초기 개선: 가짜 다운로드/가입 CTA를 제외하고 실제 작동하는 데모 앵커로 통일. 임의 고객 수/후기/성능 수치 없음.
- 모바일은 사이드바를 숨기고 근거를 캔버스 아래에 펼침. hover에 의존하지 않는 버튼과 native details 사용.
- 키보드 포커스, skip link, 선택 상태 aria-pressed, 근거 갱신 aria-live, reduced-motion 처리.

## 실행과 산출물

`cd site && npm run dev` → `http://127.0.0.1:4178`.

`cd site && npm run build` → `site/dist/`. 외부 패키지 설치가 필요 없는 HTML/CSS/JS 사이트.

공개 배포/원격 push는 하지 않았다. 검증 결과는 아래에 기록한다.

## 검증 결과

- `npm run build --prefix site`: 통과. `site/dist` 생성, 내부 레퍼런스 보드/캡처 제외.
- `node --check site/main.js`: 통과.
- 별도 headless Chrome/Playwright: 자료 3종 선택과 선택 상태 단일성, FAQ 열기/닫기, 히어로 CTA의 `#demo` 이동 통과.
- 375 / 768 / 1024 / 1440px에서 가로 overflow 없음.
- 레퍼런스 보드의 세 캡처 이미지 정상 로딩, 브라우저 pageerror 없음.
- 실제 렌더 캡처를 열어 데스크톱과 모바일의 배치 확인.
- 캡처: `artifacts/homepage/desktop.png`, `mobile.png`, `references.png`.
- Aside의 viewport 변경 API는 지원하지 않아 별도 headless Chrome에서 반응형 검증을 수행했다.
- jj 추가 workspace는 Git 작업 트리가 아니어서 `git diff --check` 대신 새 텍스트 파일의 후행 공백과 최종 개행을 직접 검사했다.
- macOS 앱/외부 계정의 동작 검증이 아니라 독립 홈페이지 검증이다. 실제 배포는 수행하지 않았다.

## 파란 브랜드 색상 적용

사용자의 아이콘 색상에 맞춰 청백색 배경, 남색 제목, 파란 CTA와 연결선, 연한 파란 기억 카드로 통일했다. 원본 로고·아이콘과 자료 제공자 구분색은 유지했다. 레퍼런스 보드의 팔레트와 설명도 갱신했다.
