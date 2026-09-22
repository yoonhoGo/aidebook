import { describe, expect, it } from "vitest";
import { parseLegacyNotes, selectedGroups, retryKey } from "./workflow-import";
describe("legacy work import", () => {
  it("rejects corrupt and non-array storage without mutating it", () => {
    for (const raw of ["{", "{}", "null"]) expect(() => parseLegacyNotes(raw)).toThrow();
  });
  it("separates unmigrated notes and invalid groups without promoting content", () => {
    const raw = JSON.stringify([{ work: 0, nativeId: "memory-1", body: "keep" }, { work: 0 }, { work: 1, nativeId: "" }, { work: 5 }, null]);
    const result = parseLegacyNotes(raw);
    expect(result.groups[0].native_memory_ids).toEqual(["memory-1"]);
    expect(result.groups[0].unmigrated_note_count).toBe(1);
    expect(result.groups[1].unmigrated_note_count).toBe(1);
    expect(result.warnings).toHaveLength(2);
    expect(JSON.parse(raw)[0].body).toBe("keep");
  });
  it("applies only explicitly selected stable indexes", () => {
    const groups = parseLegacyNotes(null).groups;
    expect(selectedGroups(groups, [])).toEqual([]);
    expect(selectedGroups(groups, [2, 2, 99]).map((g) => g.legacy_work_index)).toEqual([2]);
  });
  it("retains retry identity across failures and changed selections", () => {
    const keys = new Map<string, string>(); let index = 0;
    const create = () => `key-${++index}`;
    expect(retryKey(keys, { selected: [0] }, create)).toBe("key-1");
    expect(retryKey(keys, { selected: [1] }, create)).toBe("key-2");
    expect(retryKey(keys, { selected: [0] }, create)).toBe("key-1");
  });
});
