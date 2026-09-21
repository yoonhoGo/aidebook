import { describe, expect, it } from "vitest";
import { activationReducer, createActivationState, createDemoEvents } from "./graph";

const events = createDemoEvents({
  requestId: "request:demo:test",
  nodeIds: ["source:0", "memory:1", "memory:2"],
  sourceNodeId: "source:0",
  memoryNodeId: "memory:1",
  deliveryEdgeId: "delivery:source:0:request:demo:test",
  citationEdgeId: "citation:memory:1:request:demo:test",
  memoryVersion: 4,
});

describe("demo graph activation", () => {
  it("starts idle with no active nodes or edges", () => {
    const state = createActivationState(events);
    expect(state.status).toBe("idle");
    expect(state.cursor).toBe(-1);
    expect(state.activeNodeIds).toEqual([]);
    expect(state.activeEdgeIds).toEqual([]);
  });

  it("keeps search, return, and citation as distinct deterministic transitions", () => {
    const started = activationReducer(createActivationState(events), { type: "start" });
    const searched = activationReducer(started, { type: "step" });
    const returned = activationReducer(searched, { type: "step" });
    const cited = activationReducer(returned, { type: "step" });

    expect(searched.currentStage).toBe("search_result");
    expect(searched.activeNodeIds).toEqual(["source:0", "memory:1", "memory:2"]);
    expect(searched.activeEdgeIds).toEqual([]);

    expect(returned.currentStage).toBe("response_prepared");
    expect(returned.activeEdgeIds).toEqual(["delivery:source:0:request:demo:test"]);
    expect(returned.activeNodeIds).toEqual(["source:0", "memory:1", "memory:2", "request:demo:test"]);

    expect(cited.currentStage).toBe("answer_cited");
    expect(cited.status).toBe("complete");
    expect(cited.activeEdgeIds).toEqual([
      "delivery:source:0:request:demo:test",
      "citation:memory:1:request:demo:test",
    ]);
    expect(cited.activeEdgeIds).not.toContain("evidence:1:0");
    expect(cited.nodeStages["memory:1"]).toEqual(["search_result", "answer_cited"]);
  });

  it("resets replay state without changing the fixture event order", () => {
    const started = activationReducer(createActivationState(events), { type: "start" });
    const complete = activationReducer(activationReducer(activationReducer(started, { type: "step" }), { type: "step" }), { type: "step" });
    const reset = activationReducer(complete, { type: "reset" });

    expect(reset.status).toBe("idle");
    expect(reset.events.map((event) => event.eventId)).toEqual(events.map((event) => event.eventId));
    expect(reset.activeNodeIds).toEqual([]);
    expect(reset.activeEdgeIds).toEqual([]);
  });
});

it("ignores timer steps after reset or before explicit playback", () => {
  const idle = createActivationState(events);
  expect(activationReducer(idle, { type: "step" })).toBe(idle);
  const playing = activationReducer(idle, { type: "start" });
  const reset = activationReducer(playing, { type: "reset" });
  expect(activationReducer(reset, { type: "step" })).toBe(reset);
});
