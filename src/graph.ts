export type ReferenceStage = "search_result" | "response_prepared" | "answer_cited";

export type ReferenceObserver = "core" | "transport" | "agent" | "demo_fixture";

export type ReferenceEvent = {
  eventId: string;
  requestId: string;
  sequence: number;
  stage: ReferenceStage;
  observer: ReferenceObserver;
  nodeIds: string[];
  edgeIds: string[];
  references: Array<{
    nodeId: string;
    snapshotId?: string;
    contentHash?: string;
    memoryVersion?: number;
  }>;
  occurredAt: string;
  recordedAt: string;
  answerId?: string;
  citationIds?: string[];
};

export type ActivationStatus = "idle" | "playing" | "complete";

export type ActivationState = {
  requestId: string | null;
  events: ReferenceEvent[];
  cursor: number;
  status: ActivationStatus;
  activeNodeIds: string[];
  activeEdgeIds: string[];
  nodeStages: Record<string, ReferenceStage[]>;
  edgeStages: Record<string, ReferenceStage[]>;
  currentStage: ReferenceStage | null;
};

export type ActivationAction =
  | { type: "load"; events: ReferenceEvent[] }
  | { type: "start" }
  | { type: "step" }
  | { type: "reset" };

function unique<T>(values: T[]) {
  return Array.from(new Set(values));
}

export function createActivationState(events: ReferenceEvent[] = []): ActivationState {
  return {
    requestId: events[0]?.requestId ?? null,
    events,
    cursor: -1,
    status: "idle",
    activeNodeIds: [],
    activeEdgeIds: [],
    nodeStages: {},
    edgeStages: {},
    currentStage: null,
  };
}

export function activationReducer(state: ActivationState, action: ActivationAction): ActivationState {
  if (action.type === "load") return createActivationState(action.events);

  if (action.type === "start") {
    if (!state.events.length) return state;
    return {
      ...createActivationState(state.events),
      status: "playing",
    };
  }

  if (action.type === "reset") return createActivationState(state.events);

  if (state.status !== "playing") return state;

  const nextCursor = state.cursor + 1;
  if (nextCursor >= state.events.length) return { ...state, status: "complete" };

  const event = state.events[nextCursor];
  const eventsThroughCursor = state.events.slice(0, nextCursor + 1);
  const nodeStages: Record<string, ReferenceStage[]> = {};
  const edgeStages: Record<string, ReferenceStage[]> = {};
  for (const item of eventsThroughCursor) {
    for (const nodeId of item.nodeIds) nodeStages[nodeId] = unique([...(nodeStages[nodeId] ?? []), item.stage]);
    for (const edgeId of item.edgeIds) edgeStages[edgeId] = unique([...(edgeStages[edgeId] ?? []), item.stage]);
  }

  return {
    ...state,
    cursor: nextCursor,
    status: nextCursor === state.events.length - 1 ? "complete" : "playing",
    activeNodeIds: unique(eventsThroughCursor.flatMap((item) => item.nodeIds)),
    activeEdgeIds: unique(eventsThroughCursor.flatMap((item) => item.edgeIds)),
    nodeStages,
    edgeStages,
    currentStage: event.stage,
  };
}

export type DemoEventInput = {
  requestId: string;
  nodeIds: string[];
  sourceNodeId?: string;
  memoryNodeId?: string;
  deliveryEdgeId?: string;
  citationEdgeId?: string;
  memoryVersion?: number;
};

export function createDemoEvents(input: DemoEventInput): ReferenceEvent[] {
  const base = {
    requestId: input.requestId,
    observer: "demo_fixture" as const,
    occurredAt: "2026-09-19T10:00:00.000Z",
    recordedAt: "2026-09-19T10:00:00.000Z",
  };
  const events: ReferenceEvent[] = [
    {
      ...base,
      eventId: `${input.requestId}:search-result`,
      sequence: 1,
      stage: "search_result",
      nodeIds: input.nodeIds,
      edgeIds: [],
      references: input.nodeIds.map((nodeId) => ({ nodeId })),
    },
  ];

  if (input.sourceNodeId && input.deliveryEdgeId) {
    events.push({
      ...base,
      eventId: `${input.requestId}:response-prepared`,
      sequence: 2,
      stage: "response_prepared",
      nodeIds: [input.sourceNodeId, input.requestId],
      edgeIds: [input.deliveryEdgeId],
      references: [{ nodeId: input.sourceNodeId }],
    });
  }

  if (input.memoryNodeId && input.citationEdgeId) {
    events.push({
      ...base,
      eventId: `${input.requestId}:answer-cited`,
      sequence: 3,
      stage: "answer_cited",
      nodeIds: [input.memoryNodeId, input.requestId],
      edgeIds: [input.citationEdgeId],
      references: [{ nodeId: input.memoryNodeId, memoryVersion: input.memoryVersion }],
      answerId: `${input.requestId}:answer`,
      citationIds: [`${input.requestId}:citation:1`],
    });
  }

  return events;
}

export function stageLabel(stage: ReferenceStage) {
  return {
    search_result: "검색 결과 포함",
    response_prepared: "반환 응답 생성",
    answer_cited: "답변 인용",
  }[stage];
}

export function stageShortLabel(stage: ReferenceStage) {
  return {
    search_result: "검색",
    response_prepared: "반환",
    answer_cited: "인용",
  }[stage];
}
