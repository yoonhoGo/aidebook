import { describe, expect, it } from "vitest";
import { createGraphModel, noteGraphId } from "./graph-model";
import type { Note, Source, CoreSourceRef } from "./App";

const source: Source = { id: 0, provider: "GitHub", title: "Example", ref: "example", time: "", body: "Fixture", stale: false };
const note: Note = { id: 1, work: 0, kind: "결정", title: "Decision", body: "Keep", author: "User", time: "", reason: "Evidence", sources: [0] };
const ref: CoreSourceRef = { provider: "github", account_id: "personal", external_id: "repo/issues/1", kind: "issue", url: "https://github.com/example/repo/issues/1" };

describe("graph integration adapter", () => {
  it("uses exact native references rather than provider-matched demo sources", () => {
    const native = { ...note, id: -1, nativeId: "core-memory", nativeEvidence: [ref, ref, { ...ref, account_id: "work" }] };
    const model = createGraphModel(0, [native], [source]);
    const sources = model.nodes.filter((node) => node.kind === "source");
    expect(sources).toHaveLength(2);
    expect(sources.every((node) => node.sourceRef && node.sourceId === undefined)).toBe(true);
    expect(model.nodes.some((node) => node.id === "source:0")).toBe(false);
    expect(model.links.filter((link) => link.kind === "evidence")).toHaveLength(2);
    expect(noteGraphId(native)).toBe(noteGraphId({ ...native, id: -9 }));
  });

  it("does not invent evidence when a native memory has no references", () => {
    const model = createGraphModel(0, [{ ...note, nativeId: "core-memory", nativeEvidence: [] }], [source]);
    expect(model.nodes.filter((node) => node.kind === "source")).toEqual([]);
    expect(model.links.filter((link) => link.kind === "evidence" || link.kind === "delivery")).toEqual([]);
  });

  it("preserves source data and creates only valid, deduplicated explicit edges", () => {
    const notes = [{ ...note, sources: [0, 0, 99] }];
    const before = JSON.stringify({ notes, source });
    const model = createGraphModel(0, notes, [source]);
    expect(model.links.filter((link) => link.kind === "evidence")).toHaveLength(1);
    expect(model.links.every((link) => model.nodes.some((node) => node.id === link.sourceId) && model.nodes.some((node) => node.id === link.targetId))).toBe(true);
    expect(JSON.stringify({ notes, source })).toBe(before);
    expect(model.demoEvents.every((event) => event.observer === "demo_fixture")).toBe(true);
  });
});


describe("information map layout", () => {
  it("keeps many native references in separate, non-overlapping districts", () => {
    const notes = Array.from({ length: 36 }, (_, index) => ({
      ...note, id: index + 1, nativeId: `memory-${index}`,
      nativeEvidence: [{ ...ref, external_id: `issue-${index}` }],
    }));
    const model = createGraphModel(0, notes, []);
    const sources = model.nodes.filter((node) => node.kind === "source");
    const memories = model.nodes.filter((node) => node.kind === "memory");
    const request = model.nodes.find((node) => node.kind === "request")!;
    expect(new Set(model.nodes.map(({ x, y, z }) => `${x}:${y}:${z}`)).size).toBe(model.nodes.length);
    expect(Math.max(...sources.map((node) => node.x))).toBeLessThan(Math.min(...memories.map((node) => node.x)));
    expect(request.x).toBeGreaterThan(Math.max(...memories.map((node) => node.x)));
    expect(model.nodes.every(({ x, y, z }) => [x, y, z].every(Number.isFinite))).toBe(true);
  });
});
