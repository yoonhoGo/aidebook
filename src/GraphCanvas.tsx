import { useEffect, useMemo, useRef, useState } from "react";
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
  const font = "600 25px -apple-system, BlinkMacSystemFont, system-ui, sans-serif";
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
  context.fillStyle = selected ? "#d9ffff" : active ? "#baf7f2" : "#d5e0e8";
  context.fillText(label, 17, height / 2);
  const texture = new THREE.CanvasTexture(canvas);
  texture.minFilter = THREE.LinearFilter;
  texture.colorSpace = THREE.SRGBColorSpace;
  const material = new THREE.SpriteMaterial({ map: texture, transparent: true, depthWrite: false });
  const sprite = new THREE.Sprite(material);
  sprite.scale.set(Math.max(32, width / 4.2), 15, 1);
  sprite.position.y = -22;
  return sprite;
}

function createGraphNodeObject(node: GraphNode, active: boolean, selected: boolean) {
  const group = new THREE.Group();
  const color = node.kind === "source" ? "#91a7b8" : node.kind === "memory" ? (node.candidate ? "#d1a667" : "#dae5ea") : "#76dcd7";
  const activeColor = active || selected ? "#62e3dd" : color;
  const geometry = node.kind === "source"
    ? new THREE.BoxGeometry(14, 18, 8)
    : node.kind === "memory"
      ? (node.candidate ? new THREE.OctahedronGeometry(10, 0) : new THREE.SphereGeometry(10, 16, 12))
      : new THREE.IcosahedronGeometry(12, 1);
  const mesh = new THREE.Mesh(geometry, new THREE.MeshBasicMaterial({ color: activeColor, transparent: true, opacity: active || selected ? 1 : .82 }));
  group.add(mesh);
  if (active || selected) {
    const ring = new THREE.Mesh(
      new THREE.TorusGeometry(node.kind === "request" ? 17 : 14, 1.25, 8, 32),
      new THREE.MeshBasicMaterial({ color: "#62e3dd", transparent: true, opacity: selected ? .95 : .72 }),
    );
    ring.rotation.x = Math.PI / 2;
    group.add(ring);
  }
  const label = createNodeLabel(node.title, active, selected);
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
  useEffect(() => {
    const frame = requestAnimationFrame(() => graphRef.current?.cameraPosition({ x: 0, y: 0, z: Math.max(520, 420 * size.height / size.width) }, { x: 0, y: 0, z: 0 }, 0));
    return () => cancelAnimationFrame(frame);
  }, [size.width, size.height, model.requestId]);
  return <div ref={containerRef} className={`graph-canvas ${activation.status === "playing" ? "is-playing" : ""} ${graphMotionReduced ? "reduce-motion" : ""}`} aria-label="3D 근거 탐색 그래프"><ForceGraph3D ref={graphRef} graphData={graphData} nodeId="id" linkSource="source" linkTarget="target" backgroundColor="#0b1118" showNavInfo={false} width={size.width} height={size.height} nodeRelSize={4} nodeVal={(node: GraphNode) => node.kind === "request" ? 2.2 : 1.4} nodeLabel={(node) => graphTooltip(`${graphNodeKindLabel(node.kind)} · ${node.title}`)} nodeThreeObject={(node) => createGraphNodeObject(node as GraphNode, activeNodeSet.has(String(node.id)), selectedNodeId === node.id)} nodeThreeObjectExtend={false} linkLabel={(link) => graphTooltip(`${graphLinkKindLabel((link as GraphLink).kind)} · ${(link as GraphLink).label}`)} linkColor={(link) => { const item = link as GraphLink; if (activeEdgeSet.has(item.id)) return "#62e3dd"; if (item.kind === "evidence") return "#536473"; return "#34515d"; }} linkWidth={(link) => { const item = link as GraphLink; return activeEdgeSet.has(item.id) ? 2.4 : item.kind === "evidence" ? 0.8 : 1.2; }} linkDirectionalArrowLength={(link) => { const item = link as GraphLink; return item.kind === "evidence" ? 0 : 5; }} linkDirectionalArrowColor={(link) => activeEdgeSet.has((link as GraphLink).id) ? "#62e3dd" : "#55727c"} linkDirectionalParticles={(link) => !graphMotionReduced && activation.status === "playing" && activeEdgeSet.has((link as GraphLink).id) ? 2 : 0} linkDirectionalParticleSpeed={0.014} linkDirectionalParticleWidth={1.7} linkDirectionalParticleColor="#62e3dd" linkOpacity={0.85} cooldownTicks={0} warmupTicks={0} enableNodeDrag enableNavigationControls onNodeClick={(node) => onNodeSelect(String(node.id))} onLinkClick={(link) => onLinkSelect(String((link as GraphLink).id))} onBackgroundClick={() => onClear()} /></div>;
}
