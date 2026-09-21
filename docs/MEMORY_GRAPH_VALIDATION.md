# 메모리·지식 그래프 G1–G4 전달 및 검증

검증일: 2026-09-21. 조사 기준은 [AGENT_MEMORY_GRAPH_RESEARCH.md](./AGENT_MEMORY_GRAPH_RESEARCH.md), 구현 계약은 [MEMORY_GRAPH_DESIGN.md](./MEMORY_GRAPH_DESIGN.md), 계획은 [ROADMAP.md](./ROADMAP.md)다.

## 완료 범위

| 단계 | 결과 |
| --- | --- |
| G1 | SQLite 파생 문서 그래프, namespace 기반 URL/wikilink 해석, bounded BFS, stale·삭제·권한 철회 배제, rebuild, 이전 백업의 staged migration |
| G2 | observation→candidate 수명주기, version/idempotency, 승인 시 canonical memory·revision·evidence와 후보 상태를 원자적으로 저장, 거부 이력 |
| G3 | `context.query.v1`에 자료·메모·그래프·freshness·bounds 통합, CLI/MCP/IPC/Tauri 연결, 기존 여섯 method 보존 |
| G4 | 결정적 Markdown export, 본문 fence·후행 공백 보존, 검토 후보만 만드는 원자적 import, 설정 → 메모리와 그래프 UI |

UI는 후보 검토와 승인·거부 이력, 맥락 검색과 연결 목록, 그래프 재빌드, Markdown 다운로드·미리보기·선택 파일 읽기·붙여넣기를 제공한다. Core import는 텍스트만 받아 외부 vault를 수정하지 않는다. 후보 승인·거부는 Tauri 검토 경로에만 있고 MCP의 13개 도구에는 없다.

완전한 코드 심볼 그래프, LLM 추출, 벡터 검색, 외부 그래프 DB는 이번 설계 범위에 포함하지 않았다.

## 자동 검증

통합 revision `zpunmsrx / 89253731`에서 다음을 통과했다. Rust 테스트는 **51개**, 실패 0개다.

| 명령 | 증거 |
| --- | --- |
| `npm run build` | TypeScript + Vite production build 성공 |
| `cargo test --manifest-path src-tauri/Cargo.toml` | unit 11, candidate 4, context 6, graph 7, Markdown 5, M0–M5 18 |
| `cargo check --manifest-path src-tauri/Cargo.toml` | 성공 |
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` | 성공 |
| `git diff --check 21a219ce 89253731` | 공백 오류 없음 |
| `python3 scripts/smoke-memory-graph.py` | 실제 debug Core/CLI/MCP subprocess와 임시 DB/socket: capture→distill→propose→list, context query, 13개 도구, accept/reject 미노출 |

재현 시 먼저 debug 바이너리를 빌드한다. Python smoke는 표준 라이브러리만 사용하고 임시 DB·owner process를 종료·정리한다.

```sh
cargo build --manifest-path src-tauri/Cargo.toml --bin aidebook-core --bin aidebook-cli
python3 scripts/smoke-memory-graph.py
```

이번 isolated workspace에서는 `CARGO_TARGET_DIR=/Users/yoonho.go/workspace/aidebook/src-tauri/target`을 공유했고, smoke에 같은 위치의 `debug`를 `AIDEBOOK_BIN_DIR`로 지정했다.

주요 회귀는 acceptance 도중 DB 실패의 전체 rollback, 충돌·재시도, stale URL/link와 삭제한 명시 관계의 즉시 제외, 순환 그래프의 최단 hop, 권한 철회, multi-source memory 검색, 캐시 삭제 후 근거 unavailable, malformed/unclosed Markdown의 전체 rollback, fenced heading과 후행 공백 roundtrip이다. 10,000건 benchmark 테스트도 통과했으나 이번 실행의 p95 수치는 별도로 수집하지 않았다. 기존 상태 문서의 p95는 2026-09-19 측정값이다.

## UI 검증과 한계

Aside 브라우저의 로컬 Vite 페이지에서 Tauri invoke를 fixture로 대체해 확인했다.

- 일반 브라우저에서 native 작업이 비활성화됨.
- 후보 근거·이유 표시, 승인·거부와 이력, version conflict 오류 표시와 동일 idempotency key 재시도.
- 맥락 검색의 depth/root/bounds 전달, stale 표시, 관계 목록과 rebuild 결과.
- Markdown Blob 다운로드 내용과 미리보기, 붙여넣기 후 후보 import 결과.
- 파일 읽기 실패 메시지는 확인했다. 자동화의 파일 지정 후 textarea 반영은 확인을 끝내지 못했으므로 파일 선택 흐름은 미검증으로 남긴다.

이는 실제 Tauri command 실행 또는 앱 재시작 후 UI persistence 증거가 아니다. 실제 macOS Tauri 창, native 다운로드·파일 선택, Keychain, 실계정 GitHub, 실제 Obsidian/iCloud/FSEvents, third-party MCP host와 release package는 별도 검증이 필요하다. 이번 작업에서 외부 계정·vault에 쓰거나 배포하지 않았다.

## jj 전달

Luna max 서브에이전트가 코어 구현을 담당했고, 부모 에이전트가 독립 회귀 검토·UI·최종 통합을 담당했다. 마지막 Markdown 수정 중 Luna 사용량 제한이 발생해 부모가 남은 변경을 검증·커밋했다.

| 변경 | jj change / commit |
| --- | --- |
| 계획 | `lwlorruq / 6da201ee` |
| G1 | `ytylmkmv / aaffe293` |
| G2 | `rkqprzqs / 84a4a03f` |
| G3 | `tusvpzqv / 7684bb97` |
| G4 Core | `ntomptrr / 923feb68` |
| Markdown 본문 보존 | `zpooqrmk / 0ac8a8c0` |
| 검토·검색·교환 UI | `xtnpllvw / 9385be87` |
| 독립 그래프 검토 수정·CLI smoke | `usposssk / fdde8494` |
| 두 branch 통합 | `zpunmsrx / 89253731` |
| 최종 검증 문서 | `rmrzokoz` |

최종 문서 변경까지 로컬 `main`에 반영한다. 원래 workspace의 `docs/JEV_REVIEW.md`와 ROADMAP의 Jev 추가 내용은 별도 미커밋 변경으로 유지한다. 원격 push는 수행하지 않는다.
