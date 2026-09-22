export const legacyNamespace = "aidebook-notes-v1";
export const legacyTitles = ["첫 번째 릴리스", "로컬 코어 설계", "커넥터 조사"];
export type ImportGroup = { legacy_work_index: number; title: string; native_memory_ids: string[]; unmigrated_note_count: number };
export function parseLegacyNotes(raw: string | null): { groups: ImportGroup[]; warnings: string[] } {
  const groups = legacyTitles.map((title, legacy_work_index) => ({ legacy_work_index, title, native_memory_ids: [] as string[], unmigrated_note_count: 0 }));
  if (raw === null) return { groups, warnings: ["저장된 브라우저 메모가 없습니다."] };
  let notes: unknown;
  try { notes = JSON.parse(raw); } catch { throw new Error("기존 메모 JSON이 손상되어 미리보기를 중단했습니다. 원본은 유지됩니다."); }
  if (!Array.isArray(notes)) throw new Error("기존 메모가 배열 형식이 아닙니다. 원본은 유지됩니다.");
  const warnings: string[] = [];
  notes.forEach((note, index) => {
    if (!note || typeof note !== "object" || !Number.isInteger(note.work) || note.work < 0 || note.work >= groups.length) { warnings.push(`메모 ${index + 1}: 잘못된 묶음 인덱스 — 제외`); return; }
    const group = groups[note.work];
    if (typeof note.nativeId === "string" && note.nativeId.trim()) group.native_memory_ids.push(note.nativeId);
    else group.unmigrated_note_count += 1;
  });
  return { groups, warnings };
}
export function selectedGroups(groups: ImportGroup[], selected: number[]): ImportGroup[] {
  return groups.filter((group) => selected.includes(group.legacy_work_index));
}
/** Keep a failed request's key for identical retries, including nonconsecutive ones. */
export function retryKey(keys: Map<string, string>, payload: unknown, create: () => string = () => crypto.randomUUID()): string {
  const serialized = JSON.stringify(payload);
  const existing = keys.get(serialized);
  if (existing) return existing;
  const key = create(); keys.set(serialized, key); return key;
}
