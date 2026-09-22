import type { Note, Source, CoreSourceRef } from "./App";
import { createDemoEvents } from "./graph";
import type { ReferenceEvent } from "./graph";

export type GraphNodeKind = "source" | "memory" | "request";

export type GraphNode = {
  id: string;
  kind: GraphNodeKind;
  title: string;
  body: string;
  label: string;
  work: number;
  sourceId?: number;
  sourceRef?: CoreSourceRef;
  noteId?: number;
  stale?: boolean;
  candidate?: boolean;
  x: number;
  y: number;
  z: number;
};

export type GraphLinkKind = "evidence" | "delivery" | "citation";

export type GraphLink = {
  id: string;
  source: string;
  target: string;
  sourceId: string;
  targetId: string;
  kind: GraphLinkKind;
  label: string;
  provenance: "explicit" | "demo_fixture";
};

export type GraphSelection =
  | { type: "node"; nodeId: string }
  | { type: "edge"; edgeId: string }
  | null;

export type GraphModel = {
  nodes: GraphNode[];
  links: GraphLink[];
  demoEvents: ReferenceEvent[];
  requestId: string;
};

export function sourceGraphId(sourceId: number) {
  return `source:${sourceId}`;
}

export function noteGraphId(note: Note) {
  return `memory:${note.nativeId ?? note.id}`;
}

export function graphNodeKindLabel(kind: GraphNodeKind) {
  return { source: "자료", memory: "메모", request: "데모 요청" }[kind];
}

export function graphLinkKindLabel(kind: GraphLinkKind) {
  return { evidence: "근거 연결", delivery: "데모 반환 연결", citation: "데모 인용 연결" }[kind];
}

export function stablePosition(index: number, total: number, side: "source" | "memory" | "request") {
  // Separate map districts. Rows extend in depth instead of wrapping into a circle.
  if (side === "request") return { x: 240, y: 36, z: 0 };
  const columns = Math.max(1, Math.ceil(Math.sqrt(total)));
  const rows = Math.ceil(total / columns);
  return {
    x: side === "source" ? -180 - (index % columns) * 110 : (index % columns) * 110,
    y: side === "source" ? 0 : 24,
    z: (Math.floor(index / columns) - (rows - 1) / 2) * 110,
  };
}

export function createGraphModel(workIndex: number, notesForWork: Note[], sources: Source[]): GraphModel {
  const referencedSourceIds = new Set(notesForWork.filter((note) => !note.nativeId).flatMap((note) => note.sources));
  const graphSources = sources.filter((source) => referencedSourceIds.has(source.id));
  const requestId = `request:demo:${workIndex}`;
  const memoryForDemo = notesForWork[0];
  const sourceForDemo = memoryForDemo?.nativeId ? undefined : graphSources.find((source) => memoryForDemo?.sources.includes(source.id));
  const nodes: GraphNode[] = [
    ...graphSources.map((source, index) => ({
      id: sourceGraphId(source.id),
      kind: "source" as const,
      title: source.title,
      label: source.provider,
      body: source.body,
      work: workIndex,
      sourceId: source.id,
      stale: source.stale,
      ...stablePosition(index, graphSources.length, "source"),
    })),
    ...notesForWork.map((note, index) => ({
      id: noteGraphId(note),
      kind: "memory" as const,
      title: note.title,
      label: note.kind,
      body: note.body,
      work: workIndex,
      noteId: note.id,
      candidate: note.kind === "후보",
      ...stablePosition(index, notesForWork.length, "memory"),
    })),
    {
      id: requestId,
      kind: "request" as const,
      title: "데모 요청",
      label: "fixture",
      body: "실제 에이전트 추적이 없는 상태에서 재생하는 결정적 예시 이벤트입니다.",
      work: workIndex,
      ...stablePosition(0, 1, "request"),
    },
  ];

  const links: GraphLink[] = [];
  for (const note of notesForWork) {
    if (note.nativeId) {
      for (const ref of note.nativeEvidence ?? []) {
        const id = `source:${JSON.stringify([ref.provider, ref.account_id, ref.external_id, ref.kind])}`;
        if (!nodes.some((node) => node.id === id)) nodes.push({
          id, kind: "source", title: ref.external_id, label: ref.provider,
          body: "이 화면에는 원문 snapshot을 불러오지 않았습니다.", work: workIndex,
          sourceRef: { ...ref }, ...stablePosition(nodes.length, nodes.length + 1, "source"),
        });
        const edgeId = `evidence:${JSON.stringify([id, noteGraphId(note)])}`;
        if (!links.some((link) => link.id === edgeId)) links.push({
          id: edgeId, source: id, target: noteGraphId(note), sourceId: id, targetId: noteGraphId(note),
          kind: "evidence", label: "저장된 메모의 근거 참조", provenance: "explicit",
        });
      }
      continue;
    }
    for (const sourceId of new Set(note.sources)) {
      const source = graphSources.find((item) => item.id === sourceId);
      if (!source) continue;
      links.push({
        id: `evidence:${note.id}:${source.id}`,
        source: sourceGraphId(source.id),
        target: noteGraphId(note),
        sourceId: sourceGraphId(source.id),
        targetId: noteGraphId(note),
        kind: "evidence",
        label: "사용자가 연결한 근거",
        provenance: "explicit",
      });
    }
  }

  const sourceNode = memoryForDemo?.nativeId ? links.find((link) => link.targetId === noteGraphId(memoryForDemo))?.sourceId : sourceForDemo ? sourceGraphId(sourceForDemo.id) : undefined;
  const memoryNode = memoryForDemo ? noteGraphId(memoryForDemo) : undefined;
  const deliveryEdgeId = sourceNode ? `delivery:${sourceNode}:${requestId}` : undefined;
  const citationEdgeId = memoryNode ? `citation:${memoryNode}:${requestId}` : undefined;
  if (sourceNode && deliveryEdgeId) {
    links.push({
      id: deliveryEdgeId,
      source: sourceNode,
      target: requestId,
      sourceId: sourceNode,
      targetId: requestId,
      kind: "delivery",
      label: "데모에서 반환 응답을 만든 연결",
      provenance: "demo_fixture",
    });
  }
  if (memoryNode && citationEdgeId) {
    links.push({
      id: citationEdgeId,
      source: memoryNode,
      target: requestId,
      sourceId: memoryNode,
      targetId: requestId,
      kind: "citation",
      label: "데모에서 인용을 표시한 연결",
      provenance: "demo_fixture",
    });
  }

  // Native references are collected incrementally; position after deduplication.
  for (const kind of ["source", "memory", "request"] as const) {
    const district = nodes.filter((node) => node.kind === kind);
    district.forEach((node, index) => Object.assign(node, stablePosition(index, district.length, kind)));
  }

  const request = nodes.find((node) => node.kind === "request");
  if (request) request.x = Math.max(0, ...nodes.filter((node) => node.kind === "memory").map((node) => node.x)) + 180;

  return {
    nodes,
    links,
    requestId,
    demoEvents: createDemoEvents({
      requestId,
      nodeIds: nodes.filter((node) => node.kind !== "request").map((node) => node.id),
      sourceNodeId: sourceNode,
      memoryNodeId: memoryNode,
      deliveryEdgeId,
      citationEdgeId,
      memoryVersion: memoryForDemo?.version ?? 1,
    }),
  };
}
