import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./MemoryGraphPanel.css";

type Source = { provider: string; account_id: string; external_id: string; url: string; kind: string };
type Candidate = { id: string; body: string; reason: string; evidence: Source[]; author: string; claim_type: string; state: "distilled" | "proposed" | "accepted" | "rejected"; version: number; rejection_reason: string | null };
type Memory = { id: string; body: string; reason: string; evidence: Source[]; version: number };
type Packet = {
  sources: { source: Source; title: string; snippet: string; freshness: string; fetched_at: string }[];
  memories: Memory[];
  graph: { nodes: { id: string; title: string; source: Source }[]; edges: { id: string; from: Source; to: Source; relation_type: string; provenance: string }[]; diagnostics: string[]; truncated: boolean } | null;
  unavailable_sources: Source[]; stale_sources: Source[]; bounds: { truncated: boolean };
};
type Section = "review" | "context" | "exchange";
const labels: Record<Candidate["state"], string> = { distilled: "정제됨", proposed: "검토 대기", accepted: "승인됨", rejected: "거부됨" };
const claimLabels: Record<string, string> = { decision: "결정", preference: "선호", constraint: "제약", inferred: "추론", fact: "사실", question: "미해결 질문" };
const sourceKey = (source: Source) => JSON.stringify([source.provider, source.account_id, source.external_id, source.kind]);

function EvidenceLinks({ evidence }: { evidence: Source[] }) {
  return <ul className="mg-evidence">{evidence.map((source) => <li key={sourceKey(source)}><span className="tag">{source.provider}</span> <a href={source.url} target="_blank" rel="noreferrer">{source.external_id}</a></li>)}</ul>;
}
function errorMessage(error: unknown) {
  if (typeof error === "object" && error !== null && "code" in error) {
    const code = String(error.code);
    if (code === "version_conflict") return "다른 곳에서 내용이 변경됐습니다. 목록을 새로고침한 뒤 다시 검토해 주세요.";
    if (code === "sensitive_data_rejected") return "인증 정보로 보이는 내용은 메모로 가져올 수 없습니다.";
    if (code === "idempotency_conflict") return "이 요청은 이전 요청과 내용이 다릅니다. 목록을 새로고침해 주세요.";
    if (code === "not_found") return "연결된 근거를 찾을 수 없습니다. 원본 자료를 먼저 연결해 주세요.";
    if (code === "invalid_input") return "입력 형식을 확인해 주세요. 가져오기는 Aidebook에서 내보낸 Markdown과 연결된 근거가 필요합니다.";
  }
  return "요청을 처리하지 못했습니다. 데스크톱 앱의 연결 상태와 입력을 확인한 뒤 다시 시도해 주세요.";
}

export default function MemoryGraphPanel({ native }: { native: boolean }) {
  const [section, setSection] = useState<Section>("review");
  const [candidates, setCandidates] = useState<Candidate[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [busy, setBusy] = useState(false);
  const lock = useRef(false);
  const requestKeys = useRef(new Map<string, string>());
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const [rejectReasons, setRejectReasons] = useState<Record<string, string>>({});
  const [query, setQuery] = useState("");
  const [root, setRoot] = useState<Source | null>(null);
  const [depth, setDepth] = useState(1);
  const [packet, setPacket] = useState<Packet | null>(null);
  const [markdown, setMarkdown] = useState("");
  const [exported, setExported] = useState("");

  useEffect(() => {
    if (!native) return;
    let active = true;
    invoke<Candidate[]>("candidate_list", { stateFilter: null }).then((value) => {
      if (active) { setCandidates(value); setLoaded(true); }
    }).catch((failure: unknown) => { if (active) setError(errorMessage(failure)); });
    return () => { active = false; };
  }, [native]);

  async function perform(action: () => Promise<void>) {
    if (!native || lock.current) return;
    lock.current = true; setBusy(true); setError(""); setMessage("");
    try { await action(); } catch (failure) { setError(errorMessage(failure)); }
    finally { lock.current = false; setBusy(false); }
  }
  async function refresh() {
    await perform(async () => {
      setCandidates(await invoke<Candidate[]>("candidate_list", { stateFilter: null }));
      setLoaded(true); setMessage("검토 목록을 새로고침했습니다.");
    });
  }
  async function review(candidate: Candidate, accept: boolean) {
    await perform(async () => {
      const key = `${candidate.id}:${candidate.version}:${accept ? "accept" : "reject"}`;
      const rejectionReason = rejectReasons[candidate.id]?.trim() || "사용자 검토에서 거부했습니다.";
      const payloadKey = accept ? key : `${key}:${rejectionReason}`;
      const idempotencyKey = requestKeys.current.get(payloadKey) ?? `review:${crypto.randomUUID()}`;
      requestKeys.current.set(payloadKey, idempotencyKey);
      const result = await invoke<{ candidate: Candidate }>(accept ? "candidate_accept" : "candidate_reject", {
        request: { id: candidate.id, expected_version: candidate.version, idempotency_key: idempotencyKey,
          ...(accept ? {} : { reason: rejectionReason }) },
      });
      setCandidates((previous) => previous.map((item) => item.id === candidate.id ? result.candidate : item));
      setMessage(accept ? "메모를 승인해 저장했습니다. 아래 검토 이력과 맥락 검색에서 다시 확인할 수 있습니다." : "후보를 거부했습니다. 검토 이력은 보존됩니다.");
    });
  }
  async function search(event?: FormEvent) {
    event?.preventDefault();
    await perform(async () => {
      const value = await invoke<Packet>("context_query", { request: { query, source: root, max_depth: depth, max_sources: 20, max_nodes: 50, max_edges: 100, max_memories: 30, max_age_seconds: 86400 } });
      setPacket(value); setMessage("맥락을 조회했습니다.");
    });
  }
  async function rebuild() {
    await perform(async () => {
      const value = await invoke<{ build: { node_count: number; edge_count: number } }>("graph_rebuild", { request: {} });
      setPacket(null); setMessage(`연결 지도를 갱신했습니다. 자료 ${value.build.node_count}개, 관계 ${value.build.edge_count}개. 검색으로 확인해 주세요.`);
    });
  }
  async function exportMarkdown() {
    await perform(async () => {
      const value = await invoke<string>("memory_export_markdown");
      setExported(value);
      const url = URL.createObjectURL(new Blob([value], { type: "text/markdown;charset=utf-8" }));
      const anchor = document.createElement("a"); anchor.href = url; anchor.download = "aidebook-memories.md"; anchor.click();
      window.setTimeout(() => URL.revokeObjectURL(url), 1000);
      setMessage("Markdown을 준비했습니다. 다운로드가 열리지 않으면 아래 내용을 복사해 저장할 수 있습니다.");
    });
  }
  async function importMarkdown() {
    await perform(async () => {
      const result = await invoke<{ imported: number; idempotent: number; candidates: Candidate[] }>("memory_import_markdown", { markdown });
      setCandidates((previous) => { const next = new Map(previous.map((item) => [item.id, item])); result.candidates.forEach((item) => next.set(item.id, item)); return [...next.values()]; });
      setMessage(`${result.imported}개를 검토 후보로 가져왔습니다. 이미 가져온 항목 ${result.idempotent}개. 검토 탭에서 승인 여부를 결정해 주세요.`);
    });
  }
  const proposed = candidates.filter((item) => item.state === "proposed");
  const reviewed = candidates.filter((item) => item.state === "accepted" || item.state === "rejected");

  return <div className="memory-graph-panel" aria-busy={busy}>
    <p className="muted">근거를 확인하고 기억할 내용을 결정합니다. 자료 사이의 연결과 메모를 함께 찾아보세요.</p>
    {!native && <div className="notice" role="status">데스크톱 앱에서 사용할 수 있습니다. 브라우저에서는 로컬 메모와 연결 자료를 읽거나 변경하지 않습니다.</div>}
    <nav className="mg-tabs" aria-label="메모리 도구">{([["review", "후보 검토"], ["context", "맥락 검색"], ["exchange", "Markdown 교환"]] as const).map(([value, label]) => <button className={`btn ${section === value ? "btn-primary" : "btn-ghost"}`} type="button" aria-pressed={section === value} key={value} onClick={() => { setSection(value); setError(""); setMessage(""); }}>{label}</button>)}</nav>
    {error && <div className="notice mg-error" role="alert">{error}</div>}
    {message && <p className="mg-status" role="status">{message}</p>}
    {busy && <p className="small muted" role="status">처리 중…</p>}
    {section === "review" && <section aria-label="메모리 후보 검토">
      <div className="row-between"><h3>검토할 후보 <span className="tag">{proposed.length}</span></h3><button className="btn btn-secondary" type="button" disabled={!native || busy} onClick={() => void refresh()}>목록 새로고침</button></div>
      {native && !loaded && !error && <p className="small muted">검토 목록을 불러오는 중입니다.</p>}
      {native && loaded && proposed.length === 0 && <p className="empty">검토할 후보가 없습니다. 에이전트의 제안이나 Markdown 가져오기로 후보를 모을 수 있습니다.</p>}
      {proposed.map((candidate) => <article className="card mg-card" key={candidate.id}>
        <div className="tag-row"><span className="tag">{claimLabels[candidate.claim_type] ?? candidate.claim_type}</span><span className="small muted">{candidate.author} · 버전 {candidate.version}</span></div>
        <p className="mg-body">{candidate.body}</p><p><strong>저장 이유</strong><br />{candidate.reason}</p>
        <EvidenceLinks evidence={candidate.evidence} /><p className="small muted">연결된 원문을 확인한 뒤 승인 여부를 결정해 주세요.</p>
        <label className="mg-field">거부 이유 (선택)<input value={rejectReasons[candidate.id] ?? ""} disabled={busy} onChange={(event) => setRejectReasons((previous) => ({ ...previous, [candidate.id]: event.target.value }))} /></label>
        <div className="actions"><button className="btn btn-primary" type="button" disabled={busy} onClick={() => void review(candidate, true)}>승인하여 메모 저장</button><button className="btn btn-ghost" type="button" disabled={busy} onClick={() => void review(candidate, false)}>후보 거부</button></div>
      </article>)}
      {reviewed.length > 0 && <details className="mg-history"><summary>검토 이력 {reviewed.length}개</summary>{reviewed.map((candidate) => <article className="card mg-card" key={candidate.id}><span className="tag">{labels[candidate.state]}</span><p className="mg-body">{candidate.body}</p>{candidate.rejection_reason && <p className="small muted">{candidate.rejection_reason}</p>}<EvidenceLinks evidence={candidate.evidence} /></article>)}</details>}
    </section>}
    {section === "context" && <section aria-label="메모리와 자료 검색">
      <form onSubmit={(event) => void search(event)}><fieldset disabled={!native || busy} className="mg-controls"><legend className="sr-only">맥락 검색 조건</legend>
        <label className="mg-field">검색어<input value={query} maxLength={500} onChange={(event) => setQuery(event.target.value)} placeholder="예: 릴리스 결정, 작업 제약" /></label>
        <div className="mg-options"><label className="mg-field">연결 탐색 범위<select value={depth} onChange={(event) => setDepth(Number(event.target.value))}><option value={1}>한 단계</option><option value={2}>두 단계</option><option value={3}>세 단계</option></select></label>
        <label className="mg-field">기준 자료<select value={root ? sourceKey(root) : ""} onChange={(event) => setRoot(packet?.sources.find((item) => sourceKey(item.source) === event.target.value)?.source ?? null)}><option value="">검색 결과에서 선택</option>{root && !packet?.sources.some((item) => sourceKey(item.source) === sourceKey(root)) && <option value={sourceKey(root)}>{root.external_id}</option>}{packet?.sources.map((item) => <option key={sourceKey(item.source)} value={sourceKey(item.source)}>{item.title}</option>)}</select></label></div>
        <div className="actions"><button className="btn btn-primary" type="submit">맥락 검색</button><button className="btn btn-secondary" type="button" onClick={() => void rebuild()}>연결 지도 갱신</button></div>
      </fieldset></form>
      {packet && <div className="mg-results">
        {(packet.stale_sources.length > 0 || packet.unavailable_sources.length > 0) && <div className="notice">다시 확인할 자료 {packet.stale_sources.length}개 · 접근할 수 없는 근거 {packet.unavailable_sources.length}개. 저장한 메모는 유지됩니다.</div>}
        {packet.bounds.truncated && <p className="small muted">일부 결과만 표시됩니다. 검색어 또는 기준 자료를 좁혀 주세요.</p>}
        <h3>저장한 메모 {packet.memories.length}개</h3>{packet.memories.map((memory) => <article className="card mg-card" key={memory.id}><p className="mg-body">{memory.body}</p><p className="small muted">{memory.reason} · 버전 {memory.version}</p><EvidenceLinks evidence={memory.evidence} /></article>)}
        <h3>연결 자료 {packet.sources.length}개</h3>{packet.sources.map((item) => <article className="card mg-card" key={sourceKey(item.source)}><a href={item.source.url} target="_blank" rel="noreferrer">{item.title}</a><p>{item.snippet.replace(/<\/?mark>/g, "")}</p><span className="small muted">{item.source.provider} · {item.freshness === "stale" ? "오래된 자료" : "수집 자료"} · {item.fetched_at}</span></article>)}
        <h3>자료 사이의 관계 {packet.graph?.edges.length ?? 0}개</h3>{!packet.graph && <p className="small muted">연결 지도를 갱신한 뒤 기준 자료를 선택해 조회해 주세요.</p>}
        {packet.graph?.edges.map((edge) => <div className="mg-edge" key={edge.id}><span>{edge.from.external_id}</span><span aria-label="연결">→</span><span>{edge.to.external_id}</span><span className="tag">{edge.provenance === "explicit" ? "직접 연결" : edge.provenance === "inferred" ? "추론" : "원문 링크"}</span></div>)}
        {packet.sources.length === 0 && packet.memories.length === 0 && <p className="empty">검색 결과가 없습니다. 연결 상태와 검색어를 확인해 주세요.</p>}
      </div>}
    </section>}
    {section === "exchange" && <section aria-label="Markdown 메모 교환"><h3>메모 내보내기</h3><p className="small muted">저장한 메모와 근거·수정 이력을 읽을 수 있는 Markdown으로 보관합니다.</p><button className="btn btn-secondary" type="button" disabled={!native || busy} onClick={() => void exportMarkdown()}>Markdown 다운로드</button>
      {exported && <label className="mg-field">내보낸 Markdown<textarea readOnly rows={8} value={exported} /></label>}
      <h3>검토 후보로 가져오기</h3><p className="small muted">Aidebook에서 내보낸 문서를 선택하거나 붙여넣으세요. 근거 자료가 먼저 연결되어 있어야 합니다. 가져온 내용은 승인하기 전까지 후보로 남습니다.</p>
      <label className="mg-field">Markdown 파일<input type="file" accept=".md,.markdown,text/markdown,text/plain" disabled={!native || busy} onChange={(event) => { const file = event.target.files?.[0]; if (!file) return; setError(""); setMessage(""); if (file.size > 2 * 1024 * 1024) { setError("2MB 이하의 Markdown 파일을 선택해 주세요."); return; } void file.text().then(setMarkdown).catch(() => setError("파일을 읽지 못했습니다.")); }} /></label>
      <label className="mg-field">가져올 Markdown<textarea rows={8} disabled={!native || busy} value={markdown} onChange={(event) => setMarkdown(event.target.value)} placeholder="Aidebook Markdown 내용을 붙여넣으세요." /></label>
      <button className="btn btn-primary" type="button" disabled={!native || busy || !markdown.trim()} onClick={() => void importMarkdown()}>검토 후보로 가져오기</button>
    </section>}
  </div>;
}
