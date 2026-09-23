import { useCallback, useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { Button, Card } from "@radix-ui/themes";
import { motion } from "motion/react";
import { SegmentedControl } from "./ui/AppComponents";
import "./WorkflowPanel.css";
import WorkflowActivityPanel from "./WorkflowActivityPanel";
import WorkflowLinksPanel from "./WorkflowLinksPanel";
import WorkflowImportPanel from "./WorkflowImportPanel";

type Kind = "work" | "task";
type Fields = { pinned: boolean; title: string; status: string; purpose: string; blocked_reason: string | null; work_id: string | null; target_date: string | null; priority: number; time_blocks: { start: string; end: string }[] };
type Item = { id: string; kind: Kind; version: number; fields: Fields; created_at: string; updated_at: string };
const labels: Record<string, string> = { planned: "예정", in_progress: "진행 중", review: "완료 검토", done: "완료", on_hold: "보류", cancelled: "취소" };
const blank = (workId: string | null = null): Fields => ({ pinned: false, title: "", status: "planned", purpose: "", blocked_reason: null, work_id: workId, target_date: null, priority: 0, time_blocks: [] });
function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error && typeof error === "object") {
    const value = error as { code?: string; details?: { message?: string }; message?: string };
    if (value.code === "version_conflict") return "다른 곳에서 이 항목을 수정했습니다. 편집을 닫고 새로고침한 뒤 다시 수정하세요.";
    if (value.code === "idempotency_conflict") return "이전 저장 요청과 내용이 다릅니다. 최신 내용을 확인한 뒤 다시 수정하세요.";
    if (value.code === "not_found") return "항목을 찾을 수 없습니다. 목록을 새로고침하세요.";
    return value.details?.message ?? value.message ?? "연결 상태를 확인하고 다시 시도하세요.";
  }
  return "요청을 처리하지 못했습니다. 다시 시도하세요.";
}
const localTime = (value: string) => { if (!value) return ""; const date = new Date(value); return new Date(date.getTime() - date.getTimezoneOffset() * 60000).toISOString().slice(0, 16); };

export default function WorkflowPanel({ initialSelection, reducedMotion = false }: { initialSelection?: { kind: Kind; id: string }; reducedMotion?: boolean } = {}) {
  const [kind, setKind] = useState<Kind>("work");
  const [items, setItems] = useState<Item[]>([]);
  const [parent, setParent] = useState<Item | null>(null);
  const [offset, setOffset] = useState(0);
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [editing, setEditing] = useState<{ item: Item | null; fields: Fields } | null>(null);
  const [saving, setSaving] = useState(false);
  const savingRef = useRef(false);
  const requestRef = useRef<{ payload: string; key: string } | null>(null);
  const generation = useRef(0);
  const native = isTauri();
  const refresh = useCallback(async () => {
    if (!native) return;
    const current = ++generation.current;
    setLoading(true); setError("");
    try {
      const data = await invoke<Item[]>("workflow_list", { input: { kind, work_id: kind === "task" ? parent?.id ?? null : null, limit: 21, offset } });
      if (current === generation.current) { setItems(data.slice(0, 20)); setHasMore(data.length > 20); }
    } catch (e) { if (current === generation.current) setError(errorText(e)); }
    finally { if (current === generation.current) setLoading(false); }
  }, [kind, offset, parent, native]);
  useEffect(() => { void refresh(); return () => { generation.current += 1; }; }, [refresh]);
  useEffect(() => {
    if (!native || !initialSelection) return;
    let active = true;
    void invoke<Item>("workflow_get", { input: initialSelection }).then((item) => {
      if (!active) return;
      setKind(item.kind); setParent(null); setOffset(0);
      requestRef.current = null;
      setEditing({ item, fields: structuredClone(item.fields) });
    }).catch((e) => { if (active) setError(errorText(e)); });
    return () => { active = false; };
  }, [native, initialSelection?.kind, initialSelection?.id]);
  function navigate(next: Kind, work: Item | null = null) { setKind(next); setParent(work); setOffset(0); setItems([]); setEditing(null); setMessage(""); }
  function edit(item: Item | null) { requestRef.current = null; setEditing({ item, fields: item ? structuredClone(item.fields) : blank(kind === "task" ? parent?.id ?? null : null) }); setError(""); setMessage(""); }
  function field<K extends keyof Fields>(key: K, value: Fields[K]) { setEditing((draft) => draft ? { ...draft, fields: { ...draft.fields, [key]: value } } : null); }
  async function save(event: FormEvent) {
    event.preventDefault();
    if (!editing || savingRef.current) return;
    savingRef.current = true; setSaving(true); setError(""); setMessage("");
    const payload = { kind, id: editing.item?.id ?? null, expected_version: editing.item?.version ?? null, fields: editing.fields };
    const serialized = JSON.stringify(payload);
    if (requestRef.current?.payload !== serialized) requestRef.current = { payload: serialized, key: crypto.randomUUID() };
    try {
      await invoke("workflow_save", { input: { ...payload, idempotency_key: requestRef.current.key } });
      setEditing(null); requestRef.current = null; setMessage("저장했습니다."); await refresh();
    } catch (e) { setError(`${errorText(e)} 입력 내용은 편집기에 유지했습니다.`); }
    finally { savingRef.current = false; setSaving(false); }
  }
  return <div className="page workflow-page"><section className="section"><div className="container">
    <header className="workflow-intro"><p className="eyebrow">개인 작업 계획</p><h1>업무와 할 일</h1><p className="workflow-lede muted">목적과 다음 행동을 기록하고, 필요한 시간을 계획하세요.</p></header>
    {!native ? <div className="notice" role="status">업무 저장은 Aidebook 데스크톱 앱에서 사용할 수 있습니다. 브라우저에서는 저장소에 연결되지 않습니다.</div> : <>
      <div className="workflow-toolbar workflow-primary-toolbar"><SegmentedControl label="업무 유형" id="workflow-kind" options={[{ value: "work", label: "업무" }, { value: "task", label: "모든 할 일" }]} value={kind} onChange={(next) => navigate(next)} disabled={saving} reducedMotion={reducedMotion} /><Button className="app-button" variant="surface" color="gray" type="button" disabled={loading || saving} onClick={() => void refresh()}>새로고침</Button><Button className="app-button" size="3" type="button" disabled={saving} onClick={() => edit(null)}>{kind === "work" ? "업무 만들기" : "할 일 만들기"}</Button></div>
      <WorkflowImportPanel onApplied={() => void refresh()} />
      {parent && <h2 className="workflow-parent-title">{parent.fields.title}의 할 일</h2>}
      {message && <p role="status">{message}</p>}{error && <div className="notice" role="alert">{error}</div>}
      {editing && <motion.form className="workflow-editor workflow-form" aria-labelledby="workflow-editor-heading" onSubmit={(event) => void save(event)} initial={reducedMotion ? false : { opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }} transition={reducedMotion ? { duration: 0 } : { duration: 0.24 }}><h2 id="workflow-editor-heading">{editing.item ? "수정" : "새로 만들기"}</h2><fieldset disabled={saving}>
        <legend className="sr-only">{editing.item ? "업무 또는 할 일 수정" : "새 업무 또는 할 일"}</legend>
        <label className="workflow-field workflow-field--title">제목<input required maxLength={250} value={editing.fields.title} onChange={(e) => field("title", e.target.value)} /></label>
        <label className="workflow-field workflow-field--purpose">{kind === "work" ? "목적" : "작업 설명"}<textarea maxLength={5000} value={editing.fields.purpose} onChange={(e) => field("purpose", e.target.value)} /></label>
        <label className="workflow-field">상태<select value={editing.fields.status} onChange={(e) => field("status", e.target.value)}>{Object.entries(labels).filter(([key]) => kind === "work" || !["review", "on_hold"].includes(key)).map(([key, label]) => <option key={key} value={key} disabled={kind === "work" && key === "done" && !["review", "done"].includes(editing.item?.fields.status ?? "")}>{label}</option>)}</select></label>
        {kind === "work" && editing.fields.status === "done" && <p className="notice">저장하면 업무 완료를 확정합니다. 필요한 결과를 직접 확인한 뒤 확정하세요.</p>}
        <label className="workflow-field">개인 목표일<input type="date" value={editing.fields.target_date ?? ""} onChange={(e) => field("target_date", e.target.value || null)} /></label>
        <label className="workflow-field">우선순위<select value={editing.fields.priority} onChange={(e) => field("priority", Number(e.target.value))}>{["지정 없음", "낮음", "보통", "높음"].map((label, i) => <option key={i} value={i}>{label}</option>)}</select></label>
        <label className="workflow-check"><span><input type="checkbox" checked={editing.fields.pinned ?? false} onChange={(e) => field("pinned", e.target.checked)} /> 다음 행동에 고정</span></label>
        <label className="workflow-field">막힌 이유<input maxLength={500} value={editing.fields.blocked_reason ?? ""} onChange={(e) => field("blocked_reason", e.target.value || null)} /></label>
        {kind === "task" && <div className="workflow-blocks"><h3>작업 시간</h3>{editing.fields.time_blocks.map((block, index) => <div className="workflow-block" key={index}>{(["start", "end"] as const).map((key) => <label key={key}>{key === "start" ? "시작" : "종료"}<input type="datetime-local" required value={localTime(block[key])} onChange={(e) => { const value = e.target.value ? new Date(e.target.value).toISOString() : ""; field("time_blocks", editing.fields.time_blocks.map((b, i) => i === index ? { ...b, [key]: value } : b)); }} /></label>)}<button className="btn btn-ghost" type="button" aria-label={`작업 시간 ${index + 1} 삭제`} onClick={() => field("time_blocks", editing.fields.time_blocks.filter((_, i) => i !== index))}>삭제</button></div>)}<button className="btn btn-secondary" type="button" onClick={() => field("time_blocks", [...editing.fields.time_blocks, { start: "", end: "" }])}>작업 시간 추가</button><p className="small muted">시간은 이 기기의 시간대를 기준으로 입력합니다.</p></div>}
        <div className="workflow-toolbar"><Button className="app-button" size="3" type="submit" loading={saving}>{saving ? "저장 중…" : "저장"}</Button><button className="btn btn-ghost" type="button" onClick={() => setEditing(null)}>편집 닫기</button></div>
      </fieldset></motion.form>}
      {loading ? <p role="status">불러오는 중…</p> : !error && items.length === 0 ? <p className="empty">{offset ? "이 페이지에 항목이 없습니다." : "아직 항목이 없습니다. 첫 계획을 기록해 보세요."}</p> : <ul className="workflow-list">{items.map((item, index) => <Card asChild size="2" variant="surface" key={item.id}><motion.li initial={reducedMotion ? false : { opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} transition={reducedMotion ? { duration: 0 } : { duration: 0.22, delay: Math.min(index, 8) * 0.025 }}><div className="row-between"><h2>{item.fields.title}</h2><span className="tag">{labels[item.fields.status]}</span></div>{item.fields.purpose && <p className="workflow-purpose">{item.fields.purpose}</p>}<p className="small muted workflow-meta">{item.fields.target_date ? `목표일 ${item.fields.target_date}` : "목표일 없음"} · 우선순위 {item.fields.priority} · 버전 {item.version}</p>{item.fields.blocked_reason && <p className="workflow-blocked">막힘: {item.fields.blocked_reason}</p>}{item.fields.time_blocks.map((b, i) => <p key={i} className="small workflow-time">{new Date(b.start).toLocaleString()} – {new Date(b.end).toLocaleString()}</p>)}<div className="workflow-toolbar"><button className="btn btn-secondary" type="button" disabled={saving} onClick={() => edit(item)}>수정 · 상태 변경</button>{kind === "work" && <button className="btn btn-ghost" type="button" disabled={saving} onClick={() => navigate("task", item)}>할 일 열기</button>}</div>{kind === "work" && <WorkflowLinksPanel workId={item.id} />}<WorkflowActivityPanel kind={item.kind} id={item.id} /></motion.li></Card>)}</ul>}
      <div className="workflow-toolbar workflow-list-pagination"><button className="btn btn-ghost" type="button" disabled={offset === 0 || loading || saving} onClick={() => { setOffset(Math.max(0, offset - 20)); setEditing(null); }}>이전</button><span aria-live="polite">{offset / 20 + 1} 페이지</span><button className="btn btn-ghost" type="button" disabled={!hasMore || loading || saving} onClick={() => { setOffset(offset + 20); setEditing(null); }}>다음</button></div>
    </>}
  </div></section></div>;
}
