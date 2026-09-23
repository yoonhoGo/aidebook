import { useCallback, useEffect, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { Button, Card } from "@radix-ui/themes";
import { motion } from "motion/react";
import { SegmentedControl } from "./ui/AppComponents";
import "./DashboardPanel.css";

type Kind = "work" | "task";
type Section = "today" | "next" | "attention" | "in_progress" | "completed";
type Item = {
  id: string; kind: Kind; version: number;
  fields: {
    title: string; status: string; purpose: string; blocked_reason: string | null;
    work_id: string | null; target_date: string | null; priority: number; pinned: boolean;
    time_blocks: { start: string; end: string }[];
  };
};
type Entry = { entry_id: string; item: Item; reasons: string[]; starts_at: string | null; ends_at: string | null; completed_at: string | null };
type Page = { date: string; timezone: string; generated_at: string; entries: Entry[]; total: number; has_more: boolean };
const sections: { id: Section; title: string; description: string }[] = [
  { id: "today", title: "오늘", description: "시간순 작업 계획과 오늘 목표일" },
  { id: "next", title: "다음 행동", description: "고정 → 기한 초과 → 3일 이내 목표 → 우선순위" },
  { id: "attention", title: "확인 필요", description: "막힌 일, 보류, 완료 검토와 지난 목표일" },
  { id: "in_progress", title: "진행 중 업무", description: "지금 진행하는 업무의 목적과 상태" },
  { id: "completed", title: "최근 완료", description: "오늘을 포함한 최근 7일의 완료" },
];
const statuses: Record<string, string> = { planned: "예정", in_progress: "진행 중", review: "완료 검토", done: "완료", cancelled: "취소", on_hold: "보류" };
const reasons: Record<string, string> = { scheduled: "작업 시간", due_today: "오늘 목표", pinned: "직접 고정", overdue: "목표일 지남", due_soon: "3일 이내 목표", priority_1: "낮은 우선순위", priority_2: "보통 우선순위", priority_3: "높은 우선순위", ready: "진행 가능", blocked: "막힘", review: "완료 검토", on_hold: "보류", parent_inactive: "상위 업무 상태 확인", in_progress: "진행 중", completed: "완료" };
const firstPages = (): Record<Section, number> => ({ today: 0, next: 0, attention: 0, in_progress: 0, completed: 0 });
function explain(error: unknown): string {
  const e = error as { code?: string; details?: { message?: string }; message?: string } | null;
  if (e?.code === "version_conflict") return "다른 곳에서 수정한 항목입니다. 새로고침 후 다시 확인하세요.";
  return e?.details?.message ?? e?.message ?? (typeof error === "string" ? error : "조회하지 못했습니다. Core 연결 상태를 확인하세요.");
}

export default function DashboardPanel({ onOpen, reducedMotion }: { onOpen: (selection: { kind: Kind; id: string }) => void; reducedMotion: boolean }) {
  const [timezone, setTimezone] = useState(() => Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC");
  const [timezoneDraft, setTimezoneDraft] = useState(timezone);
  const [kind, setKind] = useState<Kind | "all">("all");
  const [offsets, setOffsets] = useState(firstPages);
  const [pages, setPages] = useState<Partial<Record<Section, Page>>>({});
  const [errors, setErrors] = useState<Partial<Record<Section, string>>>({});
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [notice, setNotice] = useState("");
  const generation = useRef(0);
  const savingRef = useRef(false);
  const requestKeys = useRef(new Map<string, string>());
  const native = isTauri();
  const refresh = useCallback(async () => {
    if (!native) return;
    const current = ++generation.current;
    setLoading(true);
    const now = new Date().toISOString();
    const results = await Promise.allSettled(sections.map(({ id }) => invoke<Page>("dashboard_get", {
      input: { timezone, now, section: id, kind: kind === "all" ? null : kind, limit: 5, offset: offsets[id] },
    })));
    if (generation.current !== current) return;
    const nextPages: Partial<Record<Section, Page>> = {};
    const nextErrors: Partial<Record<Section, string>> = {};
    results.forEach((result, index) => {
      const id = sections[index].id;
      if (result.status === "fulfilled") nextPages[id] = result.value;
      else nextErrors[id] = explain(result.reason);
    });
    setPages(nextPages); setErrors(nextErrors); setLoading(false);
  }, [native, timezone, kind, offsets]);
  useEffect(() => {
    void refresh();
    const interval = window.setInterval(() => { if (!document.hidden) void refresh(); }, 30000);
    const onFocus = () => void refresh();
    window.addEventListener("focus", onFocus);
    return () => { ++generation.current; window.clearInterval(interval); window.removeEventListener("focus", onFocus); };
  }, [refresh]);
  function time(value: string): string {
    // Display only in the timezone acknowledged by the Core, so invalid draft zones never throw.
    return new Intl.DateTimeFormat("ko-KR", { timeZone: timezone, month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(new Date(value));
  }
  async function complete(item: Item) {
    if (savingRef.current) return;
    savingRef.current = true; setSaving(true); setNotice("");
    const key = `${item.id}:${item.version}:done`;
    if (!requestKeys.current.has(key)) requestKeys.current.set(key, crypto.randomUUID());
    try {
      await invoke("workflow_save", { input: {
        kind: "task", id: item.id, expected_version: item.version,
        idempotency_key: requestKeys.current.get(key), fields: { ...item.fields, status: "done" },
      } });
      setNotice("할 일을 완료했습니다."); await refresh();
    } catch (e) { setNotice(explain(e)); }
    finally { savingRef.current = false; setSaving(false); }
  }
  return <div className="page dashboard-page"><section className="section"><div className="container">
    <header className="dashboard-intro">
      <p className="eyebrow">개인 작업 공간</p><h1>오늘의 업무</h1>
      <p className="dashboard-lede muted">오늘의 다음 행동과 확인이 필요한 업무를 한곳에서 살핍니다.</p>
    </header>
    {!native ? <p className="notice" role="status">대시보드는 데스크톱 앱의 업무와 할 일을 조회합니다. 브라우저에서는 저장소에 연결되지 않습니다.</p> : <>
      <form className="dashboard-controls" onSubmit={(event) => { event.preventDefault(); setPages({}); setTimezone(timezoneDraft); setOffsets(firstPages()); }}>
        <label>시간대<input value={timezoneDraft} onChange={(event) => setTimezoneDraft(event.target.value)} placeholder="Asia/Seoul" /></label>
        <Button className="app-button" variant="surface" color="gray" type="submit">시간대 적용</Button>
        <SegmentedControl label="대시보드 항목" id="dashboard-kind" options={[{ value: "all", label: "전체" }, { value: "work", label: "업무" }, { value: "task", label: "할 일" }]} value={kind} onChange={(next) => { setPages({}); setKind(next); setOffsets(firstPages()); }} reducedMotion={reducedMotion} />
        <Button className="app-button" variant="surface" color="gray" type="button" disabled={loading || saving} onClick={() => void refresh()}>새로고침</Button>
      </form>
      <p className="muted small">{Object.values(pages)[0]?.date ?? "오늘"} · {timezone} · 캘린더 연결 전: 로컬 작업 시간과 개인 목표일을 표시합니다.</p>
      {loading && <p role="status">업무를 조회하는 중…</p>}
      {notice && <p role="status">{notice}</p>}
      <div className="dashboard-summary" role="group" aria-label="오늘 업무 요약" aria-busy={loading}>
        <article className="dashboard-stat"><span>오늘 항목</span><strong>{pages.today?.total ?? "—"}</strong><small>작업 시간 · 개인 목표</small></article>
        <article className="dashboard-stat"><span>확인 필요</span><strong>{pages.attention?.total ?? "—"}</strong><small>막힘 · 보류 · 검토 · 목표일 지남</small></article>
        <article className="dashboard-stat"><span>진행 중 업무</span><strong>{pages.in_progress?.total ?? "—"}</strong><small>현재 진행 중인 업무</small></article>
      </div>
      <div className="dashboard-grid">{sections.map(({ id, title, description }, index) => {
        const page = pages[id];
        return <Card asChild size="2" variant="surface" key={id}><motion.section className="dashboard-section" aria-labelledby={`dashboard-${id}`} initial={reducedMotion ? false : { opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} transition={reducedMotion ? { duration: 0 } : { duration: 0.28, delay: index * 0.045 }}>
          <h2 id={`dashboard-${id}`}>{title}{page && <span className="small muted"> {page.total}개</span>}</h2><p className="small muted">{description}</p>
          {errors[id] && <p role="alert">{errors[id]}</p>}
          {page && page.entries.length === 0 && <p className="empty">{offsets[id] ? "이 페이지에 항목이 없습니다." : "표시할 항목이 없습니다."}</p>}
          <ul>{page?.entries.map((entry) => <li key={entry.entry_id}>
            <div className="row-between"><button className="dashboard-title" onClick={() => onOpen({ kind: entry.item.kind, id: entry.item.id })}>{entry.item.fields.title}</button><span className="tag">{statuses[entry.item.fields.status]}</span></div>
            <p className="small">{entry.item.kind === "work" ? "업무" : "할 일"} · {entry.reasons.map((reason) => reasons[reason] ?? reason).join(" · ")}</p>
            {entry.starts_at && <p className="small">{time(entry.starts_at)} – {entry.ends_at && time(entry.ends_at)}</p>}
            {entry.item.fields.target_date && <p className="small">개인 목표일 {entry.item.fields.target_date}</p>}
            {entry.item.fields.blocked_reason && <p className="dashboard-purpose">막힌 이유: {entry.item.fields.blocked_reason}</p>}
            {id === "in_progress" && entry.item.fields.purpose && <p className="dashboard-purpose">{entry.item.fields.purpose}</p>}
            {entry.completed_at && <p className="small">완료 {time(entry.completed_at)}</p>}
            <div className="dashboard-actions"><button className="btn btn-ghost" onClick={() => onOpen({ kind: entry.item.kind, id: entry.item.id })}>열기 · 계획하기</button>{entry.item.kind === "task" && entry.item.fields.status !== "done" && <button className="btn btn-secondary" disabled={saving} onClick={() => void complete(entry.item)}>완료</button>}</div>
          </li>)}</ul>
          <nav aria-label={`${title} 페이지`} className="dashboard-pagination"><button className="btn btn-ghost" disabled={!offsets[id] || loading} onClick={() => setOffsets((old) => ({ ...old, [id]: Math.max(0, old[id] - 5) }))}>이전</button><span className="small">{offsets[id] / 5 + 1} 페이지</span><button className="btn btn-ghost" disabled={!page?.has_more || loading} onClick={() => setOffsets((old) => ({ ...old, [id]: old[id] + 5 }))}>다음</button></nav>
        </motion.section></Card>;
      })}</div>
    </>}
  </div></section></div>;
}
