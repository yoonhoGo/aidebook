import { useMemo, useState } from "react";
import type { GraphModel } from "./graph-model";
import { graphNodeKindLabel } from "./graph-model";

export default function MapContents({ model, selectedId, activeIds, onSelect }: {
  model: GraphModel; selectedId?: string; activeIds: string[]; onSelect: (id: string) => void;
}) {
  const [query, setQuery] = useState("");
  const normalized = query.trim().toLocaleLowerCase();
  const matches = useMemo(() => model.nodes.filter((node) =>
    `${node.title} ${node.body} ${node.label}`.toLocaleLowerCase().includes(normalized)), [model, normalized]);
  const related = useMemo(() => new Set(model.links.flatMap((link) =>
    link.sourceId === selectedId ? [link.targetId] : link.targetId === selectedId ? [link.sourceId] : [])), [model, selectedId]);
  const groups = [
    { id: "source", title: "자료", nodes: matches.filter((node) => node.kind === "source") },
    { id: "memory", title: "저장한 메모", nodes: matches.filter((node) => node.kind === "memory" && !node.candidate) },
    { id: "candidate", title: "검토할 후보", nodes: matches.filter((node) => node.candidate) },
    { id: "request", title: "데모 요청", nodes: matches.filter((node) => node.kind === "request") },
  ];
  function entry(node: GraphModel["nodes"][number]) {
    return <button type="button" key={node.id} className="map-contents-entry" aria-current={selectedId === node.id ? "location" : undefined} onClick={() => onSelect(node.id)}>
      <span className={`map-entry-marker map-entry-${node.candidate ? "candidate" : node.kind}`} aria-hidden="true" />
      <span><strong>{node.title}</strong><small>{node.label}{activeIds.includes(node.id) ? " · 활성" : ""}</small></span>
      <span aria-hidden="true">↗</span>
    </button>;
  }
  return <nav className="map-contents" aria-label="Map of contents">
    <header><span className="eyebrow">MAP OF CONTENTS</span><h2>정보 목차</h2><p>현재 작업의 정보를 지도에서 찾아보세요.</p></header>
    <label className="map-contents-search"><span className="sr-only">지도 목차 검색</span><input type="search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="제목·내용 검색" /></label>
    <p className="map-contents-count" role="status">{matches.length}개 항목 · 목차 선택 시 지도 필터 해제</p>
    <div className="map-contents-scroll">
      {normalized ? <section aria-label="목차 검색 결과">{matches.map(entry)}{!matches.length && <p className="map-contents-empty">검색 결과가 없습니다.</p>}</section> : groups.map((group) => <details key={group.id} open><summary>{group.title}<span>{group.nodes.length}</span></summary>{group.nodes.map(entry)}{!group.nodes.length && <p className="map-contents-empty">아직 항목이 없습니다.</p>}</details>)}
      {selectedId && <section className="map-contents-related" aria-label="선택한 항목의 연결"><h3>연결 따라가기</h3>{model.nodes.filter((node) => related.has(node.id)).map(entry)}{related.size === 0 && <p className="map-contents-empty">연결된 항목이 없습니다.</p>}<p className="map-contents-empty">{graphNodeKindLabel(model.nodes.find((node) => node.id === selectedId)?.kind ?? "memory")}에서 직접 연결된 항목</p></section>}
    </div>
  </nav>;
}
