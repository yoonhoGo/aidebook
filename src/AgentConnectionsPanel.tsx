import { useEffect, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";

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
  return <div className="settings-list">
    <div className="notice"><strong>내 노트를 에이전트에서 사용하기</strong><br />에이전트별 설치 버튼으로 기본 사용자 설정에 스킬과 연결 도구를 추가합니다. Aidebook 앱이 실행 중일 때 자료를 검색하고 메모 후보를 제안할 수 있습니다.</div>
    <p className="small muted">기존 설정은 백업하고 다른 연결은 유지합니다. 에이전트 프로그램 자체는 별도로 설치해야 합니다. 설치 후 새 세션을 시작하세요. Pi는 /reload로 확장을 다시 불러올 수 있습니다.</p>
    {!native && <p className="notice">브라우저 미리보기에서는 설치하지 않습니다. 데스크톱 앱에서 사용할 수 있습니다.</p>}
    {items.map(item => <article className="card" key={item.agent}>
      <div className="row-between"><h3>{item.label}</h3><span className="tag">{item.installed ? "연결 설정 설치됨" : item.issue ? "확인 필요" : "설치 전"}</span></div>
      <p>{item.mode}</p>
      {item.issue && <p className="small" role="alert">{item.issue}</p>}
      <div className="card-actions"><button className="btn btn-primary" disabled={!native || !ready || busy !== null} onClick={() => void act(item.agent)}>{busy === item.agent ? "처리 중…" : item.installed ? "다시 설치 / 업데이트" : "한 번에 설치"}</button>
        {item.managed && <button className="btn btn-ghost" disabled={busy !== null} onClick={() => void act(item.agent, true)}>연결 해제</button>}
      </div>
      {item.skill_path && <details><summary className="small">설치 경로와 플러그인 번들</summary><p className="small plugin-path">스킬: {item.skill_path}</p>{item.config_path && <p className="small plugin-path">MCP 설정: {item.config_path}</p>}<p className="small plugin-path">번들: {item.bundle_path}</p><p className="small muted">Codex·Claude용 번들도 생성합니다. 기본 설치는 스킬 + MCP이며, 플러그인 마켓플레이스에 자동 등록하지 않습니다. 두 방식을 동시에 등록하면 도구가 중복될 수 있습니다.</p></details>}
    </article>)}
    <div className="card-actions"><button className="btn btn-secondary" disabled={!native || busy !== null || !items.some(i => i.installed)} onClick={() => void check()}>로컬 연결 확인</button><button className="btn btn-ghost" disabled={!native || busy !== null} onClick={() => void load().catch(e => setError(messageOf(e)))}>설치 상태 새로고침</button></div>
    {message && <p role="status">{message}</p>}{error && <p role="alert">{error}</p>}
    {backups.length > 0 && <details><summary>이전 파일 백업 {backups.length}개</summary>{backups.map(path => <p className="small plugin-path" key={path}>{path}</p>)}</details>}
    <p className="small muted">예시 요청: “Aidebook에서 최근 결정과 관련 노트를 찾아줘.” 메모 후보는 Aidebook에서 검토 후 승인합니다. GitHub·Jira 토큰은 에이전트 설정에 복사하지 않습니다.</p>
  </div>;
}
