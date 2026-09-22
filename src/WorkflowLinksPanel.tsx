import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { retryKey } from "./workflow-import";
type Source = { provider: string; account_id: string; external_id: string; url: string; kind: string };
type Link = { target_title: string | null; target_source: Source | null; fetched_at: string | null; id: string; target_kind: string; target_id: string; relation_type: string; reason: string; version: number; removed_at: string | null; access_status: string };
type Choice = { id: string; title: string; source?: Source; unavailable: boolean; freshness?: string };
const sourceKey = (source: Source) => JSON.stringify([source.provider, source.account_id, source.external_id, source.kind]);
const relations: Record<string, string> = { context: "관련 맥락", meeting_minutes: "회의록", specification: "설계", implements: "실행 자료", verification: "검증", completion_record: "작업 기록" };
export default function WorkflowLinksPanel({ workId }: { workId: string }) {
  const [links, setLinks] = useState<Link[]>([]), [choices, setChoices] = useState<Choice[]>([]);
  const [kind, setKind] = useState<"source" | "memory">("source"), [query, setQuery] = useState("");
  const [selected, setSelected] = useState<Choice | null>(null), [relation, setRelation] = useState("context"), [reason, setReason] = useState("");
  const [message, setMessage] = useState(""), [busy, setBusy] = useState(false);
  const [opened, setOpened] = useState(false), [offset, setOffset] = useState(0), [hasMore, setHasMore] = useState(false), [loading, setLoading] = useState(false);
  const [excerpt, setExcerpt] = useState<{ id: string; text: string } | null>(null);
  const generation = useRef(0);
  const lock = useRef(false), keys = useRef(new Map<string, string>());
  const refresh = useCallback(async () => {
    const current = ++generation.current;
    setLoading(true); setExcerpt(null);
    try {
      const page = await invoke<Link[]>("workflow_link_list", { input: { work_id: workId, include_removed: true, limit: 21, offset } });
      if (current === generation.current) { setLinks(page.slice(0, 20)); setHasMore(page.length > 20); }
    } finally { if (current === generation.current) setLoading(false); }
  }, [workId, offset]);
  useEffect(() => {
    if (!opened) return;
    void refresh().catch(() => setMessage("연결 목록을 불러오지 못했습니다."));
    return () => { generation.current += 1; };
  }, [opened, refresh]);
  async function previewSource(link: Link) {
    setExcerpt(null);
    const result = await invoke<{ sources: { source: Source; title: string; snippet: string; access_status: string; freshness: string }[]; stale: boolean }>("context_get", { request: { source_id: link.target_id, max_age_seconds: 86400 } });
    const available = result.sources.filter((source) => source.access_status === "accessible" && !!link.target_source && sourceKey(source.source) === sourceKey(link.target_source));
    if (!available.length) { await refresh(); setMessage("현재 원본에 접근할 수 없습니다."); return; }
    setExcerpt({ id: link.id, text: available.map((source) => `${source.title}\n${source.snippet}${source.freshness !== "fresh" || result.stale ? "\n오래된 자료일 수 있습니다." : ""}`).join("\n\n") });
  }
  async function run(action: () => Promise<void>) { if (lock.current) return; lock.current = true; setBusy(true); setMessage(""); try { await action(); } catch { setMessage("처리 결과를 확인하지 못했습니다. 입력은 유지했습니다. 다시 시도하거나 목록을 새로고침하세요."); } finally { lock.current = false; setBusy(false); } }
  async function search() {
    setSelected(null); setChoices([]);
    if (kind === "source") {
      const result = await invoke<{ results: { source: Source; title: string; freshness: string; access_status: string }[] }>("context_search", { request: { query, limit: 30, max_age_seconds: 86400 } });
      setChoices(result.results.map((item) => ({ id: sourceKey(item.source), title: item.title, source: item.source, unavailable: item.access_status !== "accessible", freshness: item.freshness })));
    } else {
      const result = await invoke<{ id: string; title: string; memory: { retracted_at: string | null } }[]>("ui_memory_list");
      setChoices(result.filter((item) => item.title.toLocaleLowerCase().includes(query.toLocaleLowerCase())).map((item) => ({ id: item.id, title: item.title, unavailable: !!item.memory.retracted_at })));
    }
  }
  async function add() {
    if (!selected || !reason.trim()) return;
    const duplicate = links.find((link) => link.target_kind === kind && (selected.source ? !!link.target_source && sourceKey(link.target_source) === sourceKey(selected.source) : link.target_id === selected.id) && link.relation_type === relation);
    const payload = { work_id: workId, target_kind: kind, ...(selected.source ? { target_source: selected.source } : { target_id: selected.id }), relation_type: relation, reason: reason.trim(), ...(duplicate?.removed_at ? { expected_version: duplicate.version } : {}) };
    await invoke("workflow_link_add", { input: { ...payload, idempotency_key: retryKey(keys.current, payload) } }); keys.current.delete(JSON.stringify(payload)); await refresh(); setMessage("자료를 연결했습니다.");
  }
  async function restore(link: Link) {
    const payload = { work_id: workId, target_kind: link.target_kind, target_id: link.target_id, relation_type: link.relation_type, reason: link.reason, expected_version: link.version };
    await invoke("workflow_link_add", { input: { ...payload, idempotency_key: retryKey(keys.current, payload) } }); keys.current.delete(JSON.stringify(payload)); await refresh(); setMessage("기존 연결을 다시 활성화했습니다.");
  }
  async function remove(link: Link) {
    const payload = { id: link.id, expected_version: link.version };
    await invoke("workflow_link_remove", { input: { ...payload, idempotency_key: retryKey(keys.current, payload) } }); keys.current.delete(JSON.stringify(payload)); await refresh(); setMessage("연결을 해제했습니다. 원본과 메모는 유지됩니다.");
  }
  return <details className="workflow-editor" onToggle={(event) => setOpened(event.currentTarget.open)}><summary>자료 연결</summary><p className="small">확인한 기존 원본·메모를 명시적으로 연결합니다. 연결 해제는 자료를 삭제하지 않습니다.</p><button className="btn btn-ghost" disabled={busy || loading} onClick={() => void run(refresh)}>연결 새로고침</button>{loading && <p role="status">연결을 불러오는 중…</p>}<ul className="workflow-links">{links.map((link) => <li key={link.id}><strong>{link.removed_at ? "[해제됨] " : ""}{relations[link.relation_type] ?? link.relation_type}</strong> · {link.target_kind === "memory" ? "메모" : "원본"} <span>{link.access_status === "accessible" ? link.target_title ?? link.target_id : "접근할 수 없는 자료"}</span>{link.access_status === "accessible" && link.target_source && <p className="small muted">{link.target_source.provider} · {link.target_source.external_id}</p>}{link.access_status === "accessible" && link.fetched_at && <p className="small muted">마지막 수집 {new Date(link.fetched_at).toLocaleString()}</p>}<p>{link.reason}</p>{link.access_status === "accessible" && link.target_source && <button className="btn btn-secondary" disabled={busy || loading} onClick={() => void run(() => previewSource(link))}>원본 미리보기</button>}{excerpt?.id === link.id && <blockquote className="workflow-purpose">{excerpt.text}</blockquote>}{link.access_status !== "accessible" && <p role="status">현재 접근 불가 또는 철회됨 — 연결이 접근 권한을 부여하지 않습니다.</p>}<button className="btn btn-ghost" disabled={busy || loading || (!!link.removed_at && link.access_status !== "accessible")} onClick={() => void run(() => link.removed_at ? restore(link) : remove(link))}>{link.removed_at ? "다시 연결" : "연결만 해제"}</button></li>)}</ul><div className="workflow-toolbar"><button className="btn btn-ghost" disabled={offset === 0 || busy || loading} onClick={() => setOffset(Math.max(0, offset - 20))}>연결 이전</button><span>{offset / 20 + 1} 페이지</span><button className="btn btn-ghost" disabled={!hasMore || busy || loading} onClick={() => setOffset(offset + 20)}>연결 다음</button></div><fieldset disabled={busy || loading}><label>자료 종류<select value={kind} onChange={(e) => { setKind(e.target.value as "source" | "memory"); setChoices([]); setSelected(null); }}><option value="source">원본 자료</option><option value="memory">기존 메모</option></select></label><label>검색<input value={query} onChange={(e) => setQuery(e.target.value)} /></label><button className="btn btn-secondary" onClick={() => void run(search)}>자료 검색</button><ul className="workflow-links">{choices.map((choice) => <li key={choice.id}><label><span><input type="radio" name={`link-${workId}`} checked={selected?.id === choice.id} disabled={choice.unavailable} onChange={() => setSelected(choice)} /> {choice.title}</span><span className="small">{choice.source ? `${choice.source.provider} · ${choice.source.external_id}` : choice.id}{choice.unavailable ? " · 접근 불가/철회됨" : ""}{choice.freshness && choice.freshness !== "fresh" ? " · 오래된 자료일 수 있음" : ""}</span></label></li>)}</ul><label>관계<select value={relation} onChange={(e) => setRelation(e.target.value)}>{Object.entries(relations).map(([value, title]) => <option key={value} value={value}>{title}</option>)}</select></label><label>연결 이유<input maxLength={500} value={reason} onChange={(e) => setReason(e.target.value)} /></label><button className="btn btn-primary" disabled={!selected || !reason.trim()} onClick={() => void run(add)}>선택 자료 연결</button></fieldset>{message && <p role="status">{message}</p>}</details>;
}
