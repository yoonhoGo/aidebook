import { useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { legacyNamespace, parseLegacyNotes, retryKey, selectedGroups } from "./workflow-import";
import type { ImportGroup } from "./workflow-import";
type Preview = { groups: { legacy_work_index: number; title: string; existing_work_id: string | null; linkable_memory_ids: string[]; skipped: { memory_id: string | null; reason: string }[] }[] };
export default function WorkflowImportPanel({ onApplied }: { onApplied: () => void }) {
  const [groups, setGroups] = useState<ImportGroup[]>([]);
  const [selected, setSelected] = useState<number[]>([]);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [warnings, setWarnings] = useState<string[]>([]);
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const lock = useRef(false), keys = useRef(new Map<string, string>());
  async function inspect() {
    if (lock.current) return; lock.current = true; setBusy(true); setMessage(""); setPreview(null); setSelected([]); setWarnings([]);
    try {
      const parsed = parseLegacyNotes(localStorage.getItem(legacyNamespace));
      setGroups(parsed.groups); setWarnings(parsed.warnings);
      setPreview(await invoke<Preview>("workflow_import_preview", { input: { namespace: legacyNamespace, groups: parsed.groups } }));
    } catch (e) { setMessage(e instanceof Error ? e.message : "미리보기를 불러오지 못했습니다. 원본은 유지됩니다."); }
    finally { lock.current = false; setBusy(false); }
  }
  async function apply() {
    if (lock.current || !selected.length) return; lock.current = true; setBusy(true); setMessage("");
    const payload = { namespace: legacyNamespace, groups: selectedGroups(groups, selected) };
    try {
      const result = await invoke<{ works: unknown[]; linked_count: number; skipped: unknown[] }>("workflow_import_apply", { input: { ...payload, idempotency_key: retryKey(keys.current, payload) } });
      setMessage(`업무 ${result.works.length}개 확인, 메모 연결 ${result.linked_count}개, 제외 ${result.skipped.length}개. 기존 메모와 작업 묶음은 유지했습니다.`); onApplied();
    } catch { setMessage("가져오기 결과를 확인하지 못했습니다. 선택을 유지했습니다. 같은 선택으로 다시 적용하면 기존 요청을 안전하게 재확인합니다."); }
    finally { lock.current = false; setBusy(false); }
  }
  return <details className="workflow-editor"><summary>기존 작업 묶음 가져오기</summary><p>원본 메모는 보존하며 선택한 묶음만 업무로 연결합니다. Core에 아직 가져오지 않은 메모는 기존 메모 가져오기 화면에서 먼저 처리하세요.</p><button className="btn btn-secondary" disabled={busy} onClick={() => void inspect()}>가져오기 미리보기</button><p className="small">원본 namespace: {legacyNamespace}</p>{warnings.map((warning, i) => <p key={i}>{warning}</p>)}{preview && <fieldset disabled={busy}>{preview.groups.map((group) => { const original = groups.find((item) => item.legacy_work_index === group.legacy_work_index)!; return <label key={group.legacy_work_index}><span><input type="checkbox" checked={selected.includes(group.legacy_work_index)} onChange={(e) => setSelected((current) => e.target.checked ? [...current, group.legacy_work_index] : current.filter((index) => index !== group.legacy_work_index))} /> [{group.legacy_work_index}] {group.title}</span><span className="small">메모 {original.native_memory_ids.length + original.unmigrated_note_count}개 · nativeId 유효 {group.linkable_memory_ids.length}개 · 미이관 {original.unmigrated_note_count}개 · {group.existing_work_id ? `기존 업무 ${group.existing_work_id}` : "새 업무 생성"}</span>{group.skipped.map((skip, i) => <span className="small" key={i}>제외: {skip.memory_id ?? "nativeId 없음"} — {skip.reason}</span>)}</label>; })}<button className="btn btn-primary" disabled={!selected.length} onClick={() => void apply()}>선택한 {selected.length}개 묶음 적용</button></fieldset>}{message && <p role="status">{message}</p>}</details>;
}
