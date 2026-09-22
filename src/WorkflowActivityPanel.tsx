import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type Kind = "work" | "task";
type Activity = { previous_state: string | null; next_state: string; version: number; occurred_at: string };
const statuses: Record<string, string> = { planned: "예정", in_progress: "진행 중", review: "완료 검토", done: "완료", on_hold: "보류", cancelled: "취소" };
const label = (status: string) => statuses[status] ?? status;

// Keyed inner state prevents rows/page/open state from leaking across targets.
export default function WorkflowActivityPanel({ kind, id }: { kind: Kind; id: string }) {
  return <ActivityDetails key={`${kind}:${id}`} kind={kind} id={id} />;
}
function ActivityDetails({ kind, id }: { kind: Kind; id: string }) {
  const [opened, setOpened] = useState(false);
  const [offset, setOffset] = useState(0);
  const [events, setEvents] = useState<Activity[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [reload, setReload] = useState(0);
  useEffect(() => {
    if (!opened) return;
    let active = true;
    setLoading(true); setError(""); setEvents([]); setHasMore(false);
    void invoke<Activity[]>("workflow_activity_list", { input: { kind, id, limit: 21, offset } }).then((rows) => {
      if (active) { setEvents(rows.slice(0, 20)); setHasMore(rows.length > 20); }
    }).catch(() => {
      if (active) setError("활동 기록을 불러오지 못했습니다. 다시 시도하세요.");
    }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [opened, kind, id, offset, reload]);
  return <details className="workflow-activity" onToggle={(event) => setOpened(event.currentTarget.open)}>
    <summary>활동 기록</summary>
    {opened && <div aria-busy={loading}>
      <p className="muted">최근 변경부터 표시합니다.</p>
      {loading && <p role="status">활동 기록을 불러오는 중…</p>}
      {error && <p role="alert">{error} <button type="button" onClick={() => setReload((n) => n + 1)}>다시 시도</button></p>}
      {!loading && !error && events.length === 0 && <p className="muted">표시할 활동이 없습니다.</p>}
      {!error && <ol>{events.map((event) => <li key={event.version}>
        <strong>{event.previous_state === null ? `생성 · ${label(event.next_state)}` : event.previous_state === event.next_state ? `저장 · ${label(event.next_state)}` : `${label(event.previous_state)} → ${label(event.next_state)}`}</strong>
        {" · "}<time dateTime={event.occurred_at}>{new Date(event.occurred_at).toLocaleString("ko-KR")}</time>
      </li>)}</ol>}
      <div className="workflow-actions">
        <button type="button" disabled={loading || offset === 0} onClick={() => setOffset((n) => Math.max(0, n - 20))}>이전 활동</button>
        <span aria-live="polite">{Math.floor(offset / 20) + 1}페이지</span>
        <button type="button" disabled={loading || !hasMore || !!error} onClick={() => setOffset((n) => n + 20)}>다음 활동</button>
        <button type="button" disabled={loading} onClick={() => { setOffset(0); setReload((n) => n + 1); }}>새로고침</button>
      </div>
    </div>}
  </details>;
}
