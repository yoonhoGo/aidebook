import { useEffect, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import "./ConnectionsPanels.css";

type Agent = "codex" | "claude_code" | "hermes" | "pi";
type Status = { agent: Agent; label: string; installed: boolean; managed: boolean; mode: string; config_path: string | null; skill_path: string; bundle_path: string; issue: string | null };
type Result = { message: string; backups: string[]; bundle_path: string };
const preview: Status[] = ([['codex', 'Codex'], ['claude_code', 'Claude Code'], ['hermes', 'Hermes'], ['pi', 'Pi']] as const).map(([agent,label]) => ({ agent, label, installed:false, managed:false, mode:agent === 'pi' ? '스킬 + 확장 플러그인' : '스킬 + MCP', config_path:null, skill_path:'', bundle_path:'', issue:null }));
function messageOf(error: unknown) {
  if (error && typeof error === "object" && "details" in error) return (error as { details?: { message?: string } }).details?.message ?? "작업을 완료하지 못했습니다.";
  return typeof error === "string" ? error : "작업을 완료하지 못했습니다.";
}
export default function AgentConnectionsPanel() {
  const native = isTauri();
  const [items, setItems] = useState(preview);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const [backups, setBackups] = useState<string[]>([]);
  const [ready, setReady] = useState(false);
  async function load() { setItems(await invoke<Status[]>("agent_connections_status")); setReady(true); }
  useEffect(() => { if (native) void load().catch(e => setError(messageOf(e))); }, [native]);
  async function act(agent: Agent, remove = false) {
    setBusy(agent); setError(""); setMessage(""); setBackups([]);
    try {
      const result = await invoke<Result>(remove ? "agent_connection_uninstall" : "agent_connection_install", { agent });
      setMessage(result.message); setBackups(result.backups); await load();
      if (!remove) {
        try {
          const probe = await invoke<{ connected: boolean; tools: number }>("agent_connection_probe");
          if (probe.connected) setMessage(`${result.message} 로컬 코어 통신 확인 · 도구 ${probe.tools}개.`);
        } catch (e) { setError(`설정은 설치되었습니다. 통신 확인: ${messageOf(e)}`); }
      }
    } catch (e) { setError(messageOf(e)); } finally { setBusy(null); }
  }
  async function check() {
    setBusy("check"); setError("");
    try { const result = await invoke<{ connected: boolean; tools: number }>("agent_connection_probe"); setMessage(`설치된 연결 실행 파일과 로컬 코어 통신 확인 · 도구 ${result.tools}개. 에이전트에서의 도구 사용 여부는 별도입니다.`); }
    catch (e) { setError(messageOf(e)); } finally { setBusy(null); }
  }
  return <section className="settings-list agent-connections" aria-label="에이전트 연결 설정" aria-busy={busy !== null || undefined}>
    <div className="notice agent-callout"><strong>내 노트를 에이전트에서 사용하기</strong><br />에이전트별 설치 버튼으로 기본 사용자 설정에 스킬과 연결 도구를 추가합니다. Aidebook 앱이 실행 중일 때 자료를 검색하고 메모 후보를 제안할 수 있습니다.</div>
    <p className="agent-intro-copy">기존 설정은 백업하고 다른 연결은 유지합니다. 에이전트 프로그램 자체는 별도로 설치해야 합니다. 설치 후 새 세션을 시작하세요. Pi는 /reload로 확장을 다시 불러올 수 있습니다.</p>
    {!native && <p className="notice agent-preview-note" role="note">브라우저 미리보기에서는 설치하지 않습니다. 데스크톱 앱에서 사용할 수 있습니다.</p>}
    <div className="agent-panel-heading"><div><h3>지원 에이전트</h3><p>각 에이전트의 설치 상태와 로컬 연결을 관리합니다.</p></div></div>
    <div className="agent-list" role="list" aria-label="지원 에이전트 목록">
      {items.map(item => <article className="card agent-card" key={item.agent} role="listitem" aria-labelledby={`agent-name-${item.agent}`}>
        <div className="agent-card-heading"><h4 id={`agent-name-${item.agent}`}>{item.label}</h4><span className="tag">{item.installed ? "연결 설정 설치됨" : item.issue ? "확인 필요" : "설치 전"}</span></div>
        <p className="agent-mode">{item.mode}</p>
        {item.issue && <p className="agent-issue" role="alert">{item.issue}</p>}
        <div className="agent-actions"><button type="button" className="btn btn-primary" disabled={!native || !ready || busy !== null} aria-label={`${item.label} ${item.installed ? "연결 설정 다시 설치 또는 업데이트" : "연결 설정 설치"}`} onClick={() => void act(item.agent)}>{busy === item.agent ? "처리 중…" : item.installed ? "다시 설치 / 업데이트" : "한 번에 설치"}</button>
          {item.managed && <button type="button" className="btn btn-ghost" disabled={busy !== null} aria-label={`${item.label} 연결 해제`} onClick={() => void act(item.agent, true)}>연결 해제</button>}
        </div>
        {item.skill_path && <details className="agent-details"><summary>설치 경로와 플러그인 번들</summary><p className="plugin-path">스킬: {item.skill_path}</p>{item.config_path && <p className="plugin-path">MCP 설정: {item.config_path}</p>}<p className="plugin-path">번들: {item.bundle_path}</p><p className="agent-intro-copy">Codex·Claude용 번들도 생성합니다. 기본 설치는 스킬 + MCP이며, 플러그인 마켓플레이스에 자동 등록하지 않습니다. 두 방식을 동시에 등록하면 도구가 중복될 수 있습니다.</p></details>}
      </article>)}
    </div>
    <div className="agent-toolbar"><button type="button" className="btn btn-secondary" disabled={!native || busy !== null || !items.some(i => i.installed)} onClick={() => void check()}>로컬 연결 확인</button><button type="button" className="btn btn-ghost" disabled={!native || busy !== null} onClick={() => void load().catch(e => setError(messageOf(e)))}>설치 상태 새로고침</button></div>
    {message && <p className="agent-feedback" role="status" aria-live="polite" aria-atomic="true">{message}</p>}{error && <p className="agent-feedback" role="alert">{error}</p>}
    {backups.length > 0 && <details className="agent-backups"><summary>이전 파일 백업 {backups.length}개</summary>{backups.map(path => <p className="plugin-path" key={path}>{path}</p>)}</details>}
    <p className="agent-footnote">예시 요청: “Aidebook에서 최근 결정과 관련 노트를 찾아줘.” 메모 후보는 Aidebook에서 검토 후 승인합니다. GitHub·Jira 토큰은 에이전트 설정에 복사하지 않습니다.</p>
  </section>;
}
