import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import ForceGraph3D from "react-force-graph-3d";
import type { ForceGraphMethods } from "react-force-graph-3d";
import * as THREE from "three";
import type { ActivationState } from "./graph";
import type { GraphNode, GraphLink, GraphModel } from "./graph-model";
import { graphNodeKindLabel, graphLinkKindLabel } from "./graph-model";

type GraphKindFilter = "all" | "source" | "memory";

function createNodeLabel(text: string, active: boolean, selected: boolean) {
  const canvas = document.createElement("canvas");
  const context = canvas.getContext("2d");
  if (!context) return null;
  const font = '600 25px "Pretendard", sans-serif';
  context.font = font;
  const label = text.length > 22 ? `${text.slice(0, 21)}…` : text;
  const textWidth = Math.ceil(context.measureText(label).width);
  const width = textWidth + 34;
  const height = 48;
  canvas.width = width * 2;
  canvas.height = height * 2;
  context.scale(2, 2);
  context.font = font;
  context.textBaseline = "middle";
  context.fillStyle = selected ? "#19364c" : active ? "#315f9e" : "#41586b";
  context.fillText(label, 17, height / 2);
  const texture = new THREE.CanvasTexture(canvas);
  texture.minFilter = THREE.LinearFilter;
  texture.colorSpace = THREE.SRGBColorSpace;
  const material = new THREE.SpriteMaterial({ map: texture, transparent: true, depthWrite: false });
  const sprite = new THREE.Sprite(material);
  sprite.scale.set(width / height * 17, 17, 1);
  sprite.position.y = -22;
  return sprite;
}

function createGraphNodeObject(node: GraphNode, active: boolean, selected: boolean, showLabel: boolean, dimmed: boolean) {
  const group = new THREE.Group();
  const color = node.kind === "source" ? "#426f9b" : node.kind === "memory" ? (node.candidate ? "#9a7335" : "#667da5") : "#667b8e";
  const activeColor = active ? "#346bb3" : selected ? "#203e65" : color;
  const geometry = node.kind === "source"
    ? new THREE.BoxGeometry(10, 12, 6)
    : node.kind === "memory"
      ? (node.candidate ? new THREE.OctahedronGeometry(10, 0) : new THREE.SphereGeometry(7, 24, 16))
      : new THREE.IcosahedronGeometry(9, 1);
  const mesh = new THREE.Mesh(geometry, new THREE.MeshBasicMaterial({ color: activeColor, transparent: true, opacity: dimmed ? .18 : 1 }));
  mesh.scale.setScalar(1.2);
  group.add(mesh);
  if (active || selected) {
    const ring = new THREE.Mesh(
      new THREE.TorusGeometry(node.kind === "request" ? 17 : 14, 1.25, 8, 32),
      new THREE.MeshBasicMaterial({ color: selected ? "#203e65" : "#346bb3", transparent: true, opacity: .65 }),
    );
    ring.rotation.x = Math.PI / 2;
    ring.name = active ? "activation-ring" : "selection-ring";
    group.add(ring);
  }
  const label = showLabel && !dimmed ? createNodeLabel(node.title, active, selected) : null;
  if (label) group.add(label);
  return group;
}

function graphTooltip(text: string) {
  const element = document.createElement("span");
  element.textContent = text;
  return element.innerHTML;
}

export default function GraphCanvas({ model, kindFilter, activeOnly, activation, graphMotionReduced, selectedNodeId, onNodeSelect, onLinkSelect, onClear }: {
  model: GraphModel; kindFilter: GraphKindFilter; activeOnly: boolean; activation: ActivationState;
  graphMotionReduced: boolean; selectedNodeId?: string; onNodeSelect: (id: string) => void;
  onLinkSelect: (id: string) => void; onClear: () => void;
}) {
  const graphRef = useRef<ForceGraphMethods<GraphNode, GraphLink> | undefined>(undefined);
  const containerRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 600, height: 420 });
  const activeNodeSet = new Set(activation.activeNodeIds);
  const activeEdgeSet = new Set(activation.activeEdgeIds);
  const visibleNodes = activeOnly ? activation.activeNodeIds : null;
  const visibleEdges = activeOnly ? activation.activeEdgeIds : null;
  const graphData = useMemo(() => {
    const nodes = model.nodes.filter((node) => (kindFilter === "all" || node.kind === kindFilter) && (!visibleNodes || visibleNodes.includes(node.id)));
    const ids = new Set(nodes.map((node) => node.id));
    return { nodes: nodes.map((node) => ({ ...node })), links: model.links.filter((link) => ids.has(link.sourceId) && ids.has(link.targetId) && (!visibleEdges || visibleEdges.includes(link.id))).map((link) => ({ ...link })) };
  }, [model, kindFilter, visibleNodes, visibleEdges]);
  useEffect(() => {
    const element = containerRef.current;
    if (!element) return;
    const observer = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      if (width > 0 && height > 0) setSize({ width, height });
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const [hoveredId, setHoveredId] = useState<string>();
  const [allLabels, setAllLabels] = useState(false);
  const [motionPaused, setMotionPaused] = useState(false);
  const animateActivation = !graphMotionReduced && !motionPaused;
  const focusId = hoveredId ?? selectedNodeId;
  const neighborhood = useMemo(() => {
    const ids = new Set<string>();
    if (focusId && graphData.nodes.some((node) => node.id === focusId)) {
      ids.add(focusId);
      graphData.links.forEach((link) => {
        if (link.sourceId === focusId) ids.add(link.targetId);
        if (link.targetId === focusId) ids.add(link.sourceId);
      });
    }
    return ids;
  }, [focusId, graphData]);
  useEffect(() => {
    const graph = graphRef.current;
    if (!graph) return;
    graph.cameraPosition({ x: 420, y: 460, z: 640 }, { x: 0, y: 0, z: 0 }, 0);
    const extent = Math.max(800, ...model.nodes.flatMap((node) => [Math.abs(node.x) * 2 + 200, Math.abs(node.z) * 2 + 200]));
    const grid = new THREE.GridHelper(extent, Math.ceil(extent / 50), "#b4c3d2", "#dce4ec");
    grid.position.y = -24;
    grid.material.transparent = true;
    grid.material.opacity = .35;
    graph.scene().add(grid);
    return () => { graph.scene().remove(grid); grid.geometry.dispose(); grid.material.dispose(); };
  }, [model]);
  const duration = graphMotionReduced ? 0 : 650;
  const fit = useCallback(() => {
    const graph = graphRef.current;
    if (!graph || !graphData.nodes.length) return;
    const box = new THREE.Box3().setFromPoints(graphData.nodes.map((node) => new THREE.Vector3(node.x, node.y, node.z)));
    box.expandByScalar(35);
    const center = box.getCenter(new THREE.Vector3());
    const direction = new THREE.Vector3(.55, .8, 1).normalize();
    const right = new THREE.Vector3().crossVectors(new THREE.Vector3(0, 1, 0), direction).normalize();
    const up = new THREE.Vector3().crossVectors(direction, right);
    const camera = graph.camera() as THREE.PerspectiveCamera;
    const tanV = Math.tan(THREE.MathUtils.degToRad(camera.fov / 2));
    const tanH = tanV * size.width / size.height;
    let distance = 0;
    for (const x of [box.min.x, box.max.x]) for (const y of [box.min.y, box.max.y]) for (const z of [box.min.z, box.max.z]) {
      const point = new THREE.Vector3(x, y, z).sub(center);
      distance = Math.max(distance, point.dot(direction) + Math.max(Math.abs(point.dot(right)) / (tanH * .78), Math.abs(point.dot(up)) / (tanV * .66)));
    }
    const position = center.clone().addScaledVector(direction, Math.max(250, distance));
    graph.cameraPosition(position, center, duration);
  }, [graphData, duration, size.width, size.height]);
  useEffect(() => {
    const frame = requestAnimationFrame(fit);
    return () => cancelAnimationFrame(frame);
  }, [fit, size.width, size.height]);
  // Follow an explicit selection, never hover. Preserve the viewing direction.
  useEffect(() => {
    const node = graphData.nodes.find((item) => item.id === selectedNodeId);
    const graph = graphRef.current;
    if (!node || !graph) return;
    const direction = graph.camera().position.clone().sub(new THREE.Vector3(node.x, node.y, node.z));
    if (direction.lengthSq() < 1) direction.set(0, 0, 1);
    direction.normalize().multiplyScalar(300);
    graph.cameraPosition({ x: node.x + direction.x, y: node.y + direction.y, z: node.z + direction.z }, node, duration);
  }, [selectedNodeId, graphData, duration]);
  const nodeObjects = useMemo(() => new Map(graphData.nodes.map((node) => [node.id,
    createGraphNodeObject(node, activeNodeSet.has(node.id), selectedNodeId === node.id,
      allLabels || graphData.nodes.length <= 8 || neighborhood.has(node.id) || activeNodeSet.has(node.id),
      neighborhood.size > 0 && !neighborhood.has(node.id) && !activeNodeSet.has(node.id)),
  ])), [graphData, allLabels, neighborhood, selectedNodeId, activation.activeNodeIds]);
  useEffect(() => () => {
    nodeObjects.forEach((object) => object.traverse((child) => {
      if (child instanceof THREE.Mesh || child instanceof THREE.Sprite) {
        if (child instanceof THREE.Mesh) child.geometry.dispose();
        const materials = Array.isArray(child.material) ? child.material : [child.material];
        materials.forEach((material) => {
          if ("map" in material && material.map instanceof THREE.Texture) material.map.dispose();
          material.dispose();
        });
      }
    }));
  }, [nodeObjects]);
  useEffect(() => {
    const rings = [...nodeObjects.values()].flatMap((object) => {
      const ring = object.getObjectByName("activation-ring");
      return ring instanceof THREE.Mesh ? [ring] : [];
    });
    if (!animateActivation || !rings.length) return;
    let frame = 0;
    const start = performance.now();
    const tick = (now: number) => {
      const phase = ((now - start) % 2200) / 2200;
      rings.forEach((ring) => {
        ring.scale.setScalar(1 + phase * .85);
        (ring.material as THREE.MeshBasicMaterial).opacity = .65 * (1 - phase);
      });
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => {
      cancelAnimationFrame(frame);
      rings.forEach((ring) => { ring.scale.setScalar(1); (ring.material as THREE.MeshBasicMaterial).opacity = .65; });
    };
  }, [nodeObjects, animateActivation]);
  const adjacent = (link: GraphLink) => link.sourceId === focusId || link.targetId === focusId;
  return <div ref={containerRef} className={`graph-canvas ${graphMotionReduced ? "reduce-motion" : ""}`} aria-label="3차원 정보 지도">
    <ForceGraph3D ref={graphRef} graphData={graphData} nodeId="id" linkSource="source" linkTarget="target"
      controlType="orbit" backgroundColor="#f2f5f8" showNavInfo={false} width={size.width} height={size.height}
      nodeLabel={(node) => graphTooltip(`${graphNodeKindLabel(node.kind)} · ${node.title}`)}
      nodeThreeObject={(node) => nodeObjects.get(String(node.id))!} nodeThreeObjectExtend={false}
      linkLabel={(link) => graphTooltip(`${graphLinkKindLabel(link.kind)} · ${link.label}`)}
      linkColor={(link) => activeEdgeSet.has(link.id) ? "#346bb3" : adjacent(link) ? "#536f87" : neighborhood.size ? "#d3dde5" : "#95a9b9"}
      linkWidth={(link) => activeEdgeSet.has(link.id) ? 1.8 : adjacent(link) ? 1 : .45}
      linkDirectionalArrowLength={(link) => link.kind === "evidence" ? 0 : 3}
      linkDirectionalArrowColor={(link) => activeEdgeSet.has(link.id) ? "#346bb3" : "#71829a"}
      linkDirectionalParticles={(link) => animateActivation && activeEdgeSet.has(link.id) ? 3 : 0}
      linkDirectionalParticleSpeed={0.006} linkDirectionalParticleWidth={1.5} linkDirectionalParticleColor="#346bb3"
      linkOpacity={0.65} cooldownTicks={0} warmupTicks={0} enableNodeDrag enableNavigationControls
      onNodeHover={(node) => setHoveredId(node ? String(node.id) : undefined)}
      onNodeClick={(node) => onNodeSelect(String(node.id))} onLinkClick={(link) => onLinkSelect(link.id)} onBackgroundClick={onClear} />
    <section className="graph-map-legend" aria-label="노드 범례">
      <span><i className="legend-source" />자료</span><span><i className="legend-memory" />메모</span><span><i className="legend-candidate" />후보</span><span><i className="legend-request" />데모 요청</span>
    </section>
    <nav className="graph-camera-tools" aria-label="3D 카메라 설정">
      <button type="button" onClick={fit} disabled={!graphData.nodes.length} title="전체 그래프를 화면에 맞춤">화면 맞춤</button>
      <button type="button" disabled={graphMotionReduced} aria-pressed={motionPaused || graphMotionReduced} onClick={() => setMotionPaused(!motionPaused)}>{graphMotionReduced ? "모션 감소 적용" : motionPaused ? "애니메이션 재개" : "애니메이션 정지"}</button>
      <button type="button" aria-pressed={allLabels} onClick={() => setAllLabels(!allLabels)}>모든 이름</button>
      {selectedNodeId && <button type="button" onClick={onClear}>선택 해제</button>}
    </nav>
    {graphData.nodes.length === 0 && <section className="graph-map-empty" role="status"><strong>표시할 노드가 없습니다</strong><span>자료 유형이나 활성 자료 필터를 변경해 보세요.</span></section>}
    <p className="graph-map-help">드래그하여 회전 · 스크롤로 확대 · 우클릭 드래그로 이동</p>
  </div>;
}
