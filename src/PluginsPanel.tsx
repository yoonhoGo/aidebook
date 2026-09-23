import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import "./ConnectionsPanels.css";

type Provider = "obsidian" | "github" | "jira" | "confluence";
type Connection = { id: string; provider: Provider; label: string; account: string; scope: string; project: string; auth: "local" | "gh_cli" | "token"; auto_sync: boolean; jira_scope: "mine" | "project"; jira_include_reporter: boolean; jira_include_parents: boolean; confluence_mode: "authored" | "watched" | "selected"; confluence_page_ids: string[] };
type PageResult = { id: string; title: string; url: string };
type SyncStatus = { id: string; running: boolean; last_checked_at: string | null; last_success_at: string | null; last_graph_at: string | null; indexed: number; inaccessible: number; changed: number; removed: number; error: string | null };
function syncLabel(status?: SyncStatus) {
  if (!status) return "첫 자동 확인 대기 중";
  if (status.running) return "문서와 관계를 확인하는 중…";
  if (status.error) return `자동 갱신 실패 · 기존 자료 유지 · ${status.error}`;
  const checked = status.last_checked_at ? new Date(status.last_checked_at).toLocaleTimeString() : "—";
  const graph = status.last_graph_at ? new Date(status.last_graph_at).toLocaleTimeString() : "—";
  return `${status.indexed}개 문서${status.inaccessible ? ` · 읽기 불가 ${status.inaccessible}개 (삭제 판정 보류)` : ""} · 변경 ${status.changed}개 · 삭제 ${status.removed}개 · 확인 ${checked} · 관계 갱신 ${graph}`;
}
const names = { obsidian: "Obsidian", github: "GitHub", jira: "Jira", confluence: "Confluence" };
const order: Provider[] = ["obsidian", "github", "jira", "confluence"];
const empty = (provider: Provider): Connection => ({ id: "", provider, label: "", account: "", scope: "", project: "", auto_sync: true, jira_scope: "mine", jira_include_reporter: false, jira_include_parents: true, confluence_mode: "authored", confluence_page_ids: [], auth: provider === "obsidian" ? "local" : provider === "github" ? "gh_cli" : "token" });
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
  const [query, setQuery] = useState("");
  const [pageInput, setPageInput] = useState("");
  const [results, setResults] = useState<PageResult[]>([]);
  const native = isTauri();
  const [legacy] = useState(legacyScopes);
  const savedDraft = connections.find(c => c.id === draft.id);
  const searchNeedsSave = !savedDraft || savedDraft.scope !== draft.scope.trim() || savedDraft.account !== draft.account.trim();
  function editConnection(connection: Connection) {
    setDraft({ ...connection, jira_scope: connection.jira_scope ?? "project", jira_include_reporter: connection.jira_include_reporter ?? false, jira_include_parents: connection.jira_include_parents ?? false, confluence_mode: connection.confluence_mode ?? "authored", confluence_page_ids: connection.confluence_page_ids ?? [] });
    setResults([]); setQuery(""); setPageInput(""); setMessage("");
    requestAnimationFrame(() => {
      const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches || document.body.classList.contains("reduce-motion");
      formRef.current?.scrollIntoView({ block: "start", behavior: reducedMotion ? "auto" : "smooth" });
    });
  }
  async function load() { setConnections(await invoke<Connection[]>("plugin_list")); }
  useEffect(() => { if (native) void load().catch(e => setMessage(errorMessage(e))); }, [native]);
  useEffect(() => {
    if (!native) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const [result, saved] = await Promise.all([
          invoke<SyncStatus[]>("plugin_sync_status"), invoke<Connection[]>("plugin_list"),
        ]);
        if (!cancelled) {
          setSyncStatuses(Object.fromEntries(result.map(s => [s.id, s])));
          setConnections(saved);
        }
      } catch { /* Connection actions surface errors; the next status poll retries. */ }
      if (!cancelled) timer = setTimeout(() => void poll(), 2000);
    }
    void poll();
    return () => { cancelled = true; clearTimeout(timer); };
  }, [native]);
  async function toggleSync(connection: Connection) {
    if (!native) return;
    setBusy(true);
    try { await invoke("plugin_sync_configure", { id: connection.id, enabled: !connection.auto_sync }); await load(); }
    catch (error) { setMessage(errorMessage(error)); } finally { setBusy(false); }
  }
  async function add(event: FormEvent) {
    event.preventDefault(); if (!native) return; setBusy(true); setMessage("");
    try {
      const input = { ...draft, id: draft.id || crypto.randomUUID(), label: draft.label.trim() || names[draft.provider], account: draft.account.trim(), scope: draft.scope.trim(), project: draft.project.trim() };
      const { id, provider: _provider, ...changes } = input;
      const saved = draft.id ? await invoke<Connection>("plugin_update", { id, changes }) : await invoke<Connection>("plugin_add", { input });
      await load(); setDraft(saved.provider === "confluence" ? saved : empty(draft.provider));
      if (!draft.id && saved.auth === "token") { setTokenId(saved.id); setToken(""); }
      setMessage(saved.provider === "obsidian" ? "연결을 저장했습니다. 앱이 실행 중이면 10초 간격으로 문서와 관계를 자동 갱신합니다." : "연결 범위를 저장했습니다. 인증 정보를 설정한 뒤 읽기 / 다시 확인을 실행하세요. 검색과 범위 저장만으로 문서를 가져오지는 않습니다.");
    } catch (error) { setMessage(errorMessage(error)); } finally { setBusy(false); }
  }
  async function refresh(connection: Connection) {
    if (!native) return;
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
    event.preventDefault(); if (!native || !tokenId) return; setBusy(true);
    try { await invoke("plugin_token_set", { id: tokenId, token }); setToken(""); setTokenId(null); setMessage("인증 정보를 macOS Keychain에 저장했습니다."); }
    catch (error) { setMessage(errorMessage(error)); } finally { setBusy(false); }
  }
  async function remove() {
    if (!native || !removeId) return;
    setBusy(true);
    try { await invoke("plugin_remove", { id: removeId, deleteCredential }); await load(); if (tokenId === removeId) { setTokenId(null); setToken(""); } if (draft.id === removeId) setDraft(empty(draft.provider)); setRemoveId(null); setMessage("연결을 해제했습니다. 수집한 캐시와 메모는 유지됩니다."); }
    catch (error) { setMessage(errorMessage(error)); } finally { setBusy(false); }
  }
  async function searchPages() {
    if (!native || !draft.id || !query.trim() || searchNeedsSave) return;
    setBusy(true); setMessage(""); setResults([]);
    try { const pages = await invoke<PageResult[]>("plugin_confluence_search", { id: draft.id, query: query.trim() }); setResults(pages); setMessage(`${pages.length}개 문서를 찾았습니다. 가져올 문서를 선택하고 범위를 저장하세요.`); }
    catch (error) { setMessage(errorMessage(error)); } finally { setBusy(false); }
  }
  function selectPage(id: string, selected: boolean) {
    setDraft(d => ({ ...d, confluence_page_ids: selected ? [...new Set([...d.confluence_page_ids, id])] : d.confluence_page_ids.filter(value => value !== id) }));
  }
  function addPage() {
    const value = pageInput.trim();
    let id = value;
    if (!/^\d+$/.test(value)) {
      try {
        const url = new URL(value);
        if (url.origin !== new URL(draft.scope).origin || url.username || url.password) throw new Error();
        id = url.pathname.match(/\/pages\/(\d+)(?:\/|$)/)?.[1] ?? (url.pathname.endsWith("/viewpage.action") ? url.searchParams.get("pageId") ?? "" : "");
      } catch { id = ""; }
    }
    if (!/^\d+$/.test(id)) { setMessage("페이지 ID 또는 연결한 사이트의 /pages/ID, viewpage.action?pageId=ID 주소를 입력하세요."); return; }
    selectPage(id, true); setPageInput(""); setMessage("선택 목록에 추가했습니다. 연결 범위를 저장한 뒤 읽기를 실행하세요.");
  }
  return <div className="page plugin-page" aria-busy={busy || undefined}><section className="section overview-section plugin-overview"><div className="container">
    <p className="eyebrow">플러그인 / 읽기 전용 연결</p><h1 id="plugin-panel-title">연결 관리</h1>
    <p className="muted">Obsidian 로컬 경로를 먼저 읽습니다. 같은 플러그인에 여러 계정과 경로를 추가할 수 있습니다.</p>
    {!native && <div className="notice plugin-preview-notice" role="note">브라우저 미리보기입니다. 실제 경로 연결과 인증은 데스크톱 앱에서 사용할 수 있습니다.</div>}
    <div className="plugin-list-heading">
      <div><h2 id="plugin-list-title">연결된 자료</h2><p>연결별 범위와 마지막 확인 상태를 관리합니다.</p></div>
      <button type="button" className="btn btn-secondary" disabled={!native || busy || !connections.length} onClick={() => void refreshAll()}>전체 읽기 · 로컬부터</button>
    </div>
    <div className="connection-list" role="list" aria-labelledby="plugin-list-title">
      {order.map(provider => <article className="plugin-provider-group" key={provider} role="listitem" aria-labelledby={`plugin-provider-${provider}`}>
        <div className="plugin-provider-heading">
          <div>
            <div className="plugin-provider-title"><h3 id={`plugin-provider-${provider}`}>{names[provider]}</h3><span className="tag">{provider === "obsidian" ? "로컬 우선 · 인증 불필요" : "읽기 전용"}</span></div>
            <p className="plugin-provider-description">{provider === "obsidian" ? "명시적으로 추가한 볼트의 Markdown 문서" : provider === "github" ? "계정별 저장소의 이슈·PR·댓글" : provider === "jira" ? "보드·프로젝트에 관계없이 내가 생성하거나 담당하는 티켓과 상위 티켓" : "내가 작성하거나 Watch 중인 문서, 검색해서 선택한 문서"}</p>
          </div>
          <div className="plugin-provider-actions">
            <button type="button" className="btn btn-secondary" disabled={!native || busy} onClick={() => editConnection(empty(provider))}>{names[provider]} 연결 추가</button>
            {legacy[provider] && <button type="button" className="btn btn-ghost" disabled={!native || busy} onClick={() => editConnection({ ...empty(provider), scope: legacy[provider]!, label: `이전 ${names[provider]} 연결` })}>이전 선택 범위 가져오기</button>}
          </div>
        </div>
        <div className="plugin-connection-list" role="list">
          {connections.filter(c => c.provider === provider).map(c => <div className="plugin-connection-row" key={c.id} role="listitem" aria-labelledby={`plugin-connection-${c.id}`}>
            <div className="plugin-connection-copy">
              <h4 id={`plugin-connection-${c.id}`}>{c.label}</h4>
              <p>{c.account && `${c.account} · `}{c.scope}{c.provider === "jira" ? ` · ${c.jira_scope === "mine" ? "내 티켓" : c.project}${c.jira_include_parents ? " · 상위 티켓 포함" : ""}` : c.project && ` · ${c.project}`}{c.provider === "confluence" && ` · ${{ authored: "내가 작성", watched: "Watch 중", selected: "선택한 문서" }[c.confluence_mode ?? "authored"]}`}</p>
              <p className="plugin-connection-auth">{c.auth === "local" ? "로컬 경로" : c.auth === "gh_cli" ? "기존 GitHub CLI 인증 재사용" : "연결 전용 Keychain 토큰"}</p>
            </div>
            <p className="plugin-connection-status">{c.provider === "obsidian" ? (syncStatuses[c.id] ? syncLabel(syncStatuses[c.id]) : c.auto_sync ? "첫 자동 확인 대기 중" : "자동 갱신 꺼짐 · 수동 읽기 가능") : statuses[c.id] ?? "저장된 연결 · 이 화면에서 아직 확인하지 않음"}</p>
            {c.provider === "obsidian" && <label className="plugin-toggle-row" htmlFor={`auto-sync-${c.id}`}><input id={`auto-sync-${c.id}`} type="checkbox" checked={c.auto_sync} disabled={!native || busy} onChange={() => void toggleSync(c)} />자동 갱신 · 앱 실행 중 10초 간격 · 관계도 함께 반영</label>}
            <div className="plugin-connection-actions">
              <button type="button" className="btn btn-ghost" disabled={!native || busy} aria-label={`${c.label} 연결 범위 수정`} onClick={() => editConnection(c)}>범위 수정</button>
              <button type="button" className="btn btn-secondary" disabled={!native || busy} aria-label={`${c.label} 읽기 또는 다시 확인`} onClick={() => { setBusy(true); void refresh(c).finally(() => setBusy(false)); }}>읽기 / 다시 확인</button>
              {c.auth === "token" && <button type="button" className="btn btn-ghost" disabled={!native || busy} aria-label={`${c.label} 인증 정보 설정`} onClick={() => { setToken(""); setTokenId(c.id); }}>인증 정보 설정</button>}
              <button type="button" className="btn btn-ghost" disabled={!native || busy} aria-label={`${c.label} 연결 해제`} onClick={() => { setDeleteCredential(false); setRemoveId(c.id); }}>연결 해제</button>
            </div>
          </div>)}
        </div>
        {!connections.some(c => c.provider === provider) && <p className="plugin-empty-state">아직 연결한 계정이나 경로가 없습니다.</p>}
      </article>)}
    </div>
    <form ref={formRef} className="card plugin-form" aria-labelledby="plugin-form-title" onSubmit={add}>
      <h2 id="plugin-form-title">{names[draft.provider]} 연결 {draft.id ? "수정" : "추가"}</h2>
      <label>플러그인<select value={draft.provider} disabled={!native || busy || !!draft.id} onChange={e => setDraft(empty(e.target.value as Provider))}>{order.map(p => <option key={p} value={p}>{names[p]}</option>)}</select></label>
      <label>연결 이름<input value={draft.label} onChange={e => setDraft({ ...draft, label: e.target.value })} placeholder="개인 / 회사 / 연구 노트" /></label>
      {draft.provider !== "obsidian" && <label>{draft.provider === "github" ? "GitHub 로그인 계정 (저장소 소유자와 별개)" : "Atlassian 계정 이메일"}<input required value={draft.account} disabled={busy} onChange={e => { setDraft({ ...draft, account: e.target.value }); setResults([]); }} /></label>}
      <label>{draft.provider === "obsidian" ? "볼트 절대 경로" : draft.provider === "github" ? "저장소 owner/repository" : "Atlassian Cloud 사이트"}<input required value={draft.scope} disabled={busy} onChange={e => { setDraft({ ...draft, scope: e.target.value }); setResults([]); }} placeholder={draft.provider === "obsidian" ? "/Users/me/Notes" : draft.provider === "github" ? "team/project" : "https://team.atlassian.net"} /></label>
      {draft.provider === "jira" && <>
        <label>가져올 티켓<select value={draft.jira_scope} onChange={e => setDraft({ ...draft, jira_scope: e.target.value as Connection["jira_scope"] })}><option value="mine">내가 생성하거나 담당하는 티켓 · 모든 프로젝트</option><option value="project">특정 프로젝트</option></select></label>
        {draft.jira_scope === "project" && <label>프로젝트 키<input required value={draft.project} onChange={e => setDraft({ ...draft, project: e.target.value })} placeholder="PROJ" /></label>}
        {draft.jira_scope === "mine" && <label className="plugin-check-row"><input type="checkbox" checked={draft.jira_include_reporter} onChange={e => setDraft({ ...draft, jira_include_reporter: e.target.checked })} />내가 보고자인 티켓도 포함</label>}
        <label className="plugin-check-row"><input type="checkbox" checked={draft.jira_include_parents} onChange={e => setDraft({ ...draft, jira_include_parents: e.target.checked })} />상위 티켓도 함께 가져오기</label>
        <p className="small muted">연결한 계정에 조회 권한이 있는 티켓만 읽습니다. 상위 티켓은 부모 관계를 따르며 일반 관련 링크와는 구분됩니다.</p>
      </>}
      {draft.provider === "confluence" && <>
        <label>가져올 문서<select value={draft.confluence_mode} onChange={e => setDraft({ ...draft, confluence_mode: e.target.value as Connection["confluence_mode"] })}><option value="authored">내가 작성한 문서</option><option value="watched">내가 Watch 중인 문서</option><option value="selected">검색 / URL로 선택한 문서</option></select></label>
        <p className="small muted">Watch는 Confluence에서 지켜보기로 설정한 문서입니다. 최근 열람 목록은 포함하지 않습니다.</p>
        {draft.confluence_mode === "selected" && <>
          <label>문서 검색<input value={query} onChange={e => setQuery(e.target.value)} placeholder="제목 또는 본문 검색어" /></label>
          <div className="plugin-form-actions"><button type="button" className="btn btn-secondary" disabled={!native || busy || searchNeedsSave || !query.trim()} onClick={() => void searchPages()}>저장된 연결로 검색</button></div>
          <p className="small muted">먼저 사이트와 계정을 저장하고 인증 정보를 설정하세요. 사이트나 계정을 변경하면 저장 후 검색할 수 있습니다. 검색만으로 문서를 가져오지는 않습니다.</p>
          {results.map(page => <label className="plugin-page-result" key={page.id}><input type="checkbox" checked={draft.confluence_page_ids.includes(page.id)} onChange={e => selectPage(page.id, e.target.checked)} /><span>{page.title} · {page.id}<span className="small muted">{page.url}</span></span></label>)}
          <label>페이지 ID 또는 같은 사이트의 페이지 URL<input value={pageInput} onChange={e => setPageInput(e.target.value)} placeholder="123456 또는 https://team.atlassian.net/wiki/spaces/TEAM/pages/123456" /></label>
          <div className="plugin-form-actions"><button type="button" className="btn btn-secondary" disabled={!native || busy || !pageInput.trim()} onClick={addPage}>선택 목록에 추가</button></div>
          <p className="small">선택한 문서 {draft.confluence_page_ids.length}개 · 저장 후 읽기 / 다시 확인을 실행하세요.</p>
          {draft.confluence_page_ids.map(id => <div key={id} className="plugin-page-selection"><span>{results.find(page => page.id === id)?.title ?? `페이지 ${id}`}</span><button type="button" className="btn btn-ghost" disabled={!native || busy} onClick={() => selectPage(id, false)}>선택 해제</button></div>)}
        </>}
      </>}
      {draft.provider === "github" && <label>인증 방식<select value={draft.auth} onChange={e => setDraft({ ...draft, auth: e.target.value as Connection["auth"] })}><option value="gh_cli">기존 gh 로그인 사용</option><option value="token">Personal access token</option></select></label>}
      <p className="small muted">{draft.provider === "obsidian" ? "앱 실행 중 선택한 경로를 10초 간격으로 확인하고 문서와 관계를 자동 갱신합니다. 원본 파일은 수정하지 않습니다." : draft.provider === "github" ? "gh 인증은 지정한 계정에서만 가져옵니다. 새 브라우저 인증은 터미널에서 gh auth login으로 진행하거나, 읽기 권한의 fine-grained PAT를 사용하세요." : "현재 개인용 연결은 이메일 + unscoped API token을 지원합니다. OAuth 2.0 (3LO), scoped token, 에이전트 OAuth 세션 공유는 아직 지원하지 않습니다."}</p>
      <div className="plugin-form-actions"><button className="btn btn-primary" disabled={!native || busy}>연결 범위 저장</button>{draft.id && <button type="button" className="btn btn-ghost" disabled={busy} onClick={() => editConnection(empty(draft.provider))}>수정 닫기</button>}</div>
    </form>
    {tokenId && <form className="card plugin-token-form plugin-form" aria-labelledby="plugin-token-title" onSubmit={saveToken}>
      <h2 id="plugin-token-title">{connections.find(c => c.id === tokenId)?.label} 인증 정보</h2>
      <label>API token<input type="password" autoComplete="new-password" required value={token} onChange={e => setToken(e.target.value)} /></label>
      <p className="small muted">이 연결의 Keychain에 저장합니다. 토큰은 연결 설정 파일과 브라우저 저장소에 남기지 않습니다.</p>
      <div className="plugin-form-actions"><button className="btn btn-primary" disabled={!native || busy}>인증 저장</button><button className="btn btn-ghost" type="button" disabled={!native || busy} onClick={() => { setToken(""); setTokenId(null); }}>취소</button></div>
    </form>}
    {removeId && <section className="card plugin-remove-confirmation" aria-labelledby="plugin-remove-title">
      <h2 id="plugin-remove-title">{connections.find(c => c.id === removeId)?.label} 연결 해제</h2>
      <p>이 연결만 해제합니다. 기존 캐시와 사용자 메모는 유지합니다.</p>
      <label className="plugin-check-row"><input type="checkbox" checked={deleteCredential} onChange={e => setDeleteCredential(e.target.checked)} />이 연결의 저장 토큰도 삭제 (gh 로그인은 유지)</label>
      <div className="plugin-form-actions"><button type="button" className="btn btn-primary" disabled={!native || busy} onClick={() => void remove()}>해제</button><button type="button" className="btn btn-ghost" disabled={!native || busy} onClick={() => setRemoveId(null)}>취소</button></div>
    </section>}
    {message && <p className="plugin-feedback" role="status" aria-live="polite" aria-atomic="true">{message}</p>}
  </div></section></div>;
}
