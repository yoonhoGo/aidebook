import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";

type Provider = "obsidian" | "github" | "jira";
type Connection = { id: string; provider: Provider; label: string; account: string; scope: string; project: string; auth: "local" | "gh_cli" | "token"; auto_sync: boolean };
type SyncStatus = { id: string; running: boolean; last_checked_at: string | null; last_success_at: string | null; last_graph_at: string | null; indexed: number; inaccessible: number; changed: number; removed: number; error: string | null };
function syncLabel(status?: SyncStatus) {
  if (!status) return "첫 자동 확인 대기 중";
  if (status.running) return "문서와 관계를 확인하는 중…";
  if (status.error) return `자동 갱신 실패 · 기존 자료 유지 · ${status.error}`;
  const checked = status.last_checked_at ? new Date(status.last_checked_at).toLocaleTimeString() : "—";
  const graph = status.last_graph_at ? new Date(status.last_graph_at).toLocaleTimeString() : "—";
  return `${status.indexed}개 문서${status.inaccessible ? ` · 읽기 불가 ${status.inaccessible}개 (삭제 판정 보류)` : ""} · 변경 ${status.changed}개 · 삭제 ${status.removed}개 · 확인 ${checked} · 관계 갱신 ${graph}`;
}
const names = { obsidian: "Obsidian", github: "GitHub", jira: "Jira" };
const order: Provider[] = ["obsidian", "github", "jira"];
const empty = (provider: Provider): Connection => ({ id: "", provider, label: "", account: "", scope: "", project: "", auto_sync: true, auth: provider === "obsidian" ? "local" : provider === "github" ? "gh_cli" : "token" });
function legacyScopes(): Partial<Record<Provider, string>> {
  try {
    const value: unknown = JSON.parse(localStorage.getItem("aidebook-native-scopes-v1") ?? "{}");
    if (!value || typeof value !== "object") return {};
    const scopes = value as Record<string, unknown>;
    return { obsidian: typeof scopes.obsidian === "string" ? scopes.obsidian : undefined, github: typeof scopes.github === "string" ? scopes.github : undefined };
  } catch { return {}; }
}
function errorMessage(error: unknown): string {
  if (typeof error === "object" && error && "details" in error) {
    const details = (error as { details?: { message?: string } }).details;
    if (details?.message) return details.message;
  }
  return typeof error === "string" ? error : "연결 작업을 완료하지 못했습니다.";
}
export default function PluginsPanel() {
  const formRef = useRef<HTMLFormElement>(null);
  const [connections, setConnections] = useState<Connection[]>([]);
  const [draft, setDraft] = useState<Connection>(empty("obsidian"));
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [syncStatuses, setSyncStatuses] = useState<Record<string, SyncStatus>>({});
  const [statuses, setStatuses] = useState<Record<string, string>>({});
  const [tokenId, setTokenId] = useState<string | null>(null);
  const [token, setToken] = useState("");
  const [removeId, setRemoveId] = useState<string | null>(null);
  const [deleteCredential, setDeleteCredential] = useState(false);
  const native = isTauri();
  const [legacy] = useState(legacyScopes);
  function editConnection(connection: Connection) {
    setDraft(connection);
    requestAnimationFrame(() => formRef.current?.scrollIntoView({ block: "start", behavior: "smooth" }));
  }
  async function load() { setConnections(await invoke<Connection[]>("plugin_list")); }
  useEffect(() => { if (native) void load().catch(e => setMessage(errorMessage(e))); }, [native]);
  useEffect(() => {
    if (!native) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const result = await invoke<SyncStatus[]>("plugin_sync_status");
        if (!cancelled) setSyncStatuses(Object.fromEntries(result.map(s => [s.id, s])));
      } catch { /* Connection actions surface errors; the next status poll retries. */ }
      if (!cancelled) timer = setTimeout(() => void poll(), 2000);
    }
    void poll();
    return () => { cancelled = true; clearTimeout(timer); };
  }, [native]);
  async function toggleSync(connection: Connection) {
    setBusy(true);
    try { await invoke("plugin_sync_configure", { id: connection.id, enabled: !connection.auto_sync }); await load(); }
    catch (error) { setMessage(errorMessage(error)); } finally { setBusy(false); }
  }
  async function add(event: FormEvent) {
    event.preventDefault(); setBusy(true); setMessage("");
    try {
      const input = { ...draft, id: crypto.randomUUID(), label: draft.label.trim() || names[draft.provider], account: draft.account.trim(), scope: draft.scope.trim(), project: draft.project.trim() };
      const saved = await invoke<Connection>("plugin_add", { input });
      await load(); setDraft(empty(draft.provider));
      if (saved.auth === "token") { setTokenId(saved.id); setToken(""); }
      setMessage(saved.provider === "obsidian" ? "연결을 저장했습니다. 앱이 실행 중이면 10초 간격으로 문서와 관계를 자동 갱신합니다." : "연결 범위를 저장했습니다. 읽기를 실행하면 접근 권한과 자료를 확인합니다.");
    } catch (error) { setMessage(errorMessage(error)); } finally { setBusy(false); }
  }
  async function refresh(connection: Connection) {
    setStatuses(s => ({ ...s, [connection.id]: "읽는 중…" }));
    try {
      const result = await invoke<{ indexed: number }>("plugin_refresh", { id: connection.id });
      setStatuses(s => ({ ...s, [connection.id]: `${result.indexed}개 색인 · ${new Date().toLocaleTimeString()} 확인` }));
    } catch (error) { setStatuses(s => ({ ...s, [connection.id]: `갱신 실패 · 기존 캐시 유지 · ${errorMessage(error)}` })); }
  }
  async function refreshAll() {
    setBusy(true);
    try { for (const provider of order) for (const c of connections.filter(c => c.provider === provider)) await refresh(c); }
    finally { setBusy(false); }
  }
  async function saveToken(event: FormEvent) {
    event.preventDefault(); if (!tokenId) return; setBusy(true);
    try { await invoke("plugin_token_set", { id: tokenId, token }); setToken(""); setTokenId(null); setMessage("인증 정보를 macOS Keychain에 저장했습니다."); }
    catch (error) { setMessage(errorMessage(error)); } finally { setBusy(false); }
  }
  async function remove() {
    setBusy(true);
    try { await invoke("plugin_remove", { id: removeId, deleteCredential }); await load(); if (tokenId === removeId) { setTokenId(null); setToken(""); } setRemoveId(null); setMessage("연결을 해제했습니다. 수집한 캐시와 메모는 유지됩니다."); }
    catch (error) { setMessage(errorMessage(error)); } finally { setBusy(false); }
  }
  return <div className="page"><section className="section overview-section"><div className="container">
    <p className="eyebrow">플러그인 / 읽기 전용 연결</p><h1>연결 관리</h1>
    <p className="muted">Obsidian 로컬 경로를 먼저 읽습니다. 같은 플러그인에 여러 계정과 경로를 추가할 수 있습니다.</p>
    {!native && <div className="notice">브라우저 미리보기입니다. 실제 경로 연결과 인증은 데스크톱 앱에서 사용할 수 있습니다.</div>}
    <div className="connection-list">{order.map(provider => <article className="card" key={provider}>
      <div className="row-between"><h2>{names[provider]}</h2><span className="tag">{provider === "obsidian" ? "로컬 우선 · 인증 불필요" : "읽기 전용"}</span></div>
      <p>{provider === "obsidian" ? "명시적으로 추가한 볼트의 Markdown 문서" : provider === "github" ? "계정별 저장소의 이슈·PR·댓글" : "사이트·계정·프로젝트별 이슈 제목·상태·원문 링크"}</p>
      {connections.filter(c => c.provider === provider).map(c => <div className="plugin-connection" key={c.id}>
        <strong>{c.label}</strong><p className="small">{c.account && `${c.account} · `}{c.scope}{c.project && ` · ${c.project}`}</p>
        <p className="small muted">{c.auth === "local" ? "로컬 경로" : c.auth === "gh_cli" ? "기존 GitHub CLI 인증 재사용" : "연결 전용 Keychain 토큰"}</p>
        {c.provider === "obsidian" && <label className="small"><input type="checkbox" checked={c.auto_sync} disabled={busy} onChange={() => void toggleSync(c)} /> 자동 갱신 · 앱 실행 중 10초 간격 · 관계도 함께 반영</label>}
        <p className="small" role="status">{c.provider === "obsidian" ? (syncStatuses[c.id] ? syncLabel(syncStatuses[c.id]) : c.auto_sync ? "첫 자동 확인 대기 중" : "자동 갱신 꺼짐 · 수동 읽기 가능") : statuses[c.id] ?? "저장된 연결 · 이 화면에서 아직 확인하지 않음"}</p>
        {c.provider === "obsidian" && !c.auto_sync && <p className="small muted">자동 갱신이 꺼져 있습니다.</p>}
        <div className="card-actions"><button className="btn btn-secondary" disabled={busy} onClick={() => { setBusy(true); void refresh(c).finally(() => setBusy(false)); }}>읽기 / 다시 확인</button>
          {c.auth === "token" && <button className="btn btn-ghost" disabled={busy} onClick={() => { setToken(""); setTokenId(c.id); }}>인증 정보 설정</button>}
          <button className="btn btn-ghost" disabled={busy} onClick={() => { setDeleteCredential(false); setRemoveId(c.id); }}>연결 해제</button></div>
      </div>)}
      {!connections.some(c => c.provider === provider) && <p className="small muted">아직 연결한 계정이나 경로가 없습니다.</p>}
      <button className="btn btn-secondary" disabled={busy} onClick={() => editConnection(empty(provider))}>{names[provider]} 연결 추가</button>
      {legacy[provider] && <button className="btn btn-ghost" disabled={busy} onClick={() => editConnection({ ...empty(provider), scope: legacy[provider]!, label: `이전 ${names[provider]} 연결` })}>이전 선택 범위 가져오기</button>}
    </article>)}</div>
    <button className="btn btn-secondary" disabled={!native || busy || !connections.length} onClick={() => void refreshAll()}>전체 읽기 · 로컬부터</button>
    <form ref={formRef} className="card plugin-form" onSubmit={add}><h2>{names[draft.provider]} 연결 추가</h2>
      <label>플러그인<select value={draft.provider} disabled={busy} onChange={e => setDraft(empty(e.target.value as Provider))}>{order.map(p => <option key={p} value={p}>{names[p]}</option>)}</select></label>
      <label>연결 이름<input value={draft.label} onChange={e => setDraft({ ...draft, label: e.target.value })} placeholder="개인 / 회사 / 연구 노트" /></label>
      {draft.provider !== "obsidian" && <label>{draft.provider === "github" ? "GitHub 로그인 계정 (저장소 소유자와 별개)" : "Atlassian 계정 이메일"}<input required value={draft.account} onChange={e => setDraft({ ...draft, account: e.target.value })} /></label>}
      <label>{draft.provider === "obsidian" ? "볼트 절대 경로" : draft.provider === "github" ? "저장소 owner/repository" : "Jira Cloud 사이트"}<input required value={draft.scope} onChange={e => setDraft({ ...draft, scope: e.target.value })} placeholder={draft.provider === "obsidian" ? "/Users/me/Notes" : draft.provider === "github" ? "team/project" : "https://team.atlassian.net"} /></label>
      {draft.provider === "jira" && <label>프로젝트 키<input required value={draft.project} onChange={e => setDraft({ ...draft, project: e.target.value })} placeholder="PROJ" /></label>}
      {draft.provider === "github" && <label>인증 방식<select value={draft.auth} onChange={e => setDraft({ ...draft, auth: e.target.value as Connection["auth"] })}><option value="gh_cli">기존 gh 로그인 사용</option><option value="token">Personal access token</option></select></label>}
      <p className="small muted">{draft.provider === "obsidian" ? "앱 실행 중 선택한 경로를 10초 간격으로 확인하고 문서와 관계를 자동 갱신합니다. 원본 파일은 수정하지 않습니다." : draft.provider === "github" ? "gh 인증은 지정한 계정에서만 가져옵니다. 새 브라우저 인증은 터미널에서 gh auth login으로 진행하거나, 읽기 권한의 fine-grained PAT를 사용하세요." : "현재 개인용 연결은 이메일 + unscoped API token을 지원합니다. OAuth 2.0 (3LO), scoped token, 에이전트 OAuth 세션 공유는 아직 지원하지 않습니다."}</p>
      <button className="btn btn-primary" disabled={!native || busy}>연결 범위 저장</button>
    </form>
    {tokenId && <form className="card plugin-form" onSubmit={saveToken}><h2>{connections.find(c => c.id === tokenId)?.label} 인증 정보</h2><label>API token<input type="password" autoComplete="new-password" required value={token} onChange={e => setToken(e.target.value)} /></label><p className="small muted">이 연결의 Keychain에 저장합니다. 토큰은 연결 설정 파일과 브라우저 저장소에 남기지 않습니다.</p><div className="card-actions"><button className="btn btn-primary" disabled={busy}>인증 저장</button><button className="btn btn-ghost" type="button" disabled={busy} onClick={() => { setToken(""); setTokenId(null); }}>취소</button></div></form>}
    {removeId && <div className="card plugin-form"><h2>{connections.find(c => c.id === removeId)?.label} 연결 해제</h2><p>이 연결만 해제합니다. 기존 캐시와 사용자 메모는 유지합니다.</p><label><input type="checkbox" checked={deleteCredential} onChange={e => setDeleteCredential(e.target.checked)} /> 이 연결의 저장 토큰도 삭제 (gh 로그인은 유지)</label><div className="card-actions"><button className="btn btn-primary" disabled={busy} onClick={() => void remove()}>해제</button><button className="btn btn-ghost" disabled={busy} onClick={() => setRemoveId(null)}>취소</button></div></div>}
    <p role="status">{message}</p>
  </div></section></div>;
}
