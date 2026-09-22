import { Component, lazy, Suspense, useEffect, useMemo, useReducer, useRef, useState } from "react";
import type { FormEvent, KeyboardEvent as ReactKeyboardEvent, PointerEvent as ReactPointerEvent, ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { activationReducer, createActivationState, stageLabel, stageShortLabel } from "./graph";
import type { ActivationState } from "./graph";
import { createGraphModel, sourceGraphId, noteGraphId, graphNodeKindLabel, graphLinkKindLabel } from "./graph-model";
import type { GraphNode, GraphLink, GraphSelection } from "./graph-model";
import "./App.css";
import MapContents from "./MapContents";
import MemoryGraphPanel from "./MemoryGraphPanel";
import PluginsPanel from "./PluginsPanel";
import WorkflowPanel from "./WorkflowPanel";
import AgentConnectionsPanel from "./AgentConnectionsPanel";

const GraphCanvas = lazy(() => import("./GraphCanvas"));

type Page = "workflow" | "work" | "home" | "notes" | "activity" | "graph" | "search" | "connections" | "settings";
type WorkTab = "all" | "candidate" | "sources" | "activity";
type NoteKind = "결정" | "다음 행동" | "미해결 질문" | "선호" | "후보";
type SettingCategory = "일반" | "모양" | "플러그인" | "에이전트 연결" | "메모와 데이터" | "메모리와 그래프" | "동기화" | "업데이트와 진단";
type GraphMode = "3d" | "list";
type GraphKindFilter = "all" | "source" | "memory";

type CoreStatus = {
  product: string;
  version: string;
  persistence: string;
  connectors: string;
};

type NoteRevision = {
  title: string;
  body: string;
  kind: NoteKind;
  author: string;
  reason: string;
};

export type Note = {
  id: number;
  work: number;
  kind: NoteKind;
  title: string;
  body: string;
  author: string;
  time: string;
  reason: string;
  sources: number[];
  version?: number;
  nativeId?: string;
  nativeEvidence?: CoreSourceRef[];
  retracted?: boolean;
  previous?: NoteRevision;
};

export type CoreSourceRef = {
  provider: string;
  account_id: string;
  external_id: string;
  url: string;
  kind: string;
};

type NativeUiMemory = {
  id: string;
  title: string;
  work: number;
  kind: string;
  memory: {
    id: string;
    body: string;
    reason: string;
    evidence: CoreSourceRef[];
    author: string;
    claim_type: string;
    version: number;
    retracted_at: string | null;
    updated_at: string;
  };
};

type NativeUiMemoryMutation = {
  memory: NativeUiMemory;
  created: boolean;
  idempotent_replay: boolean;
  action: string;
};

export type Source = {
  id: number;
  provider: "GitHub" | "Obsidian";
  title: string;
  ref: string;
  time: string;
  body: string;
  stale: boolean;
};

type Activity = {
  id: string;
  time: string;
  title: string;
  body: string;
};

type AppSettings = {
  autostart: boolean;
  background: boolean;
  notifications: boolean;
  reduceMotion: boolean;
  textSize: "15" | "16" | "18";
  exclude: string;
  syncPeriod: "5분" | "15분" | "수동";
};

type EditorState = {
  id: number | null;
  title: string;
  body: string;
  kind: NoteKind;
  reason: string;
  sources: number[];
};

type DetailState =
  | { type: "source"; source: Source }
  | { type: "history"; note: Note }
  | { type: "scope"; source: Source }
  | null;

type GraphCanvasBoundaryProps = { children: ReactNode; fallback: ReactNode };
type GraphCanvasBoundaryState = { hasError: boolean };

const STORAGE_NOTES = "aidebook-notes-v1";
const STORAGE_ACTIVITIES = "aidebook-activities-v1";
const STORAGE_SESSION = "aidebook-session-v1";
const STORAGE_SETTINGS = "aidebook-settings-v1";
const STORAGE_SCOPES = "aidebook-scopes-v1";
const DEFAULT_INSPECTOR_WIDTH = 330;

function isNativeRuntime() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function idempotencyKey(prefix: string) {
  const suffix = typeof crypto !== "undefined" && typeof crypto.randomUUID === "function" ? crypto.randomUUID() : `${Date.now()}`;
  return `${prefix}-${suffix}`;
}

function sourceRefFor(source: Source): CoreSourceRef {
  const provider = source.provider === "GitHub" ? "github" : "obsidian";
  return {
    provider,
    account_id: "ui-selected-scope",
    external_id: `${provider}/ui/${source.id}`,
    url: provider === "github" ? `https://github.com/aidebook/ui/${source.id}` : `obsidian://open?file=ui-${source.id}`,
    kind: provider === "github" ? "issue" : "note",
  };
}

function noteKindFromCore(kind: string): NoteKind {
  return noteKinds.includes(kind as NoteKind) ? kind as NoteKind : kind === "inferred" ? "후보" : "결정";
}

const works = ["첫 번째 릴리스", "로컬 코어 설계", "커넥터 조사"];
const settingCategories: SettingCategory[] = ["일반", "모양", "플러그인", "에이전트 연결", "메모와 데이터", "메모리와 그래프", "동기화", "업데이트와 진단"];
const noteKinds: NoteKind[] = ["결정", "다음 행동", "미해결 질문", "선호", "후보"];

const sources: Source[] = [
  {
    id: 0,
    provider: "GitHub",
    title: "테스트 환경 복구 및 재검증",
    ref: "aidebook / 이슈 #24",
    time: "09.18 09:12",
    body: "테스트 환경 오류를 조사하고 있습니다. 환경 복구 후 통합 테스트를 다시 실행해야 합니다.",
    stale: true,
  },
  {
    id: 1,
    provider: "Obsidian",
    title: "첫 릴리스 체크리스트",
    ref: "업무 / Aidebook / 릴리스.md",
    time: "09.18 10:40",
    body: "배포 조건: 테스트 환경 복구, 통합 테스트 통과, CLI와 MCP의 동일한 메모 확인.",
    stale: false,
  },
];

const initialNotes: Note[] = [
  {
    id: 1,
    work: 0,
    kind: "결정",
    title: "테스트 환경이 복구될 때까지 배포 보류",
    body: "테스트 환경을 먼저 복구하고, 통합 테스트를 다시 확인한 뒤 배포합니다. PR 병합 여부와 별개로 유지하는 결정입니다.",
    author: "에이전트 · 사용자 결정",
    time: "10:42",
    reason: "대화에서 명시한 배포 조건을 다음 작업에서도 유지하기 위해 저장했습니다.",
    sources: [0, 1],
  },
  {
    id: 2,
    work: 0,
    kind: "다음 행동",
    title: "복구 후 통합 테스트 다시 실행하기",
    body: "환경 복구를 확인한 뒤 테스트 결과를 릴리스 체크리스트에 기록합니다.",
    author: "내가 작성",
    time: "09:58",
    reason: "배포 전에 확인할 다음 행동을 직접 기록했습니다.",
    sources: [1],
  },
  {
    id: 3,
    work: 0,
    kind: "후보",
    title: "환경 변수 변경이 실패 원인일 수 있음",
    body: "최근 설정 변경과 테스트 실패의 시점이 겹칩니다. 원인으로 확정하기 전에 로그 확인이 필요합니다.",
    author: "에이전트 · 추론",
    time: "09:31",
    reason: "검증이 필요한 추론을 사실과 구분해 후보로 남겼습니다.",
    sources: [0],
  },
  {
    id: 4,
    work: 1,
    kind: "결정",
    title: "처음에는 읽기와 내부 메모만 지원",
    body: "외부 서비스의 댓글 작성과 상태 변경은 초기 버전에 포함하지 않습니다.",
    author: "내가 작성",
    time: "어제",
    reason: "초기 제품의 권한 범위를 유지하기 위해 기록했습니다.",
    sources: [1],
  },
  {
    id: 5,
    work: 2,
    kind: "미해결 질문",
    title: "오프라인 이후의 누락을 어떻게 복구할까?",
    body: "복귀 시 증분 조회와 전체 재조회의 기준을 정리해야 합니다.",
    author: "내가 작성",
    time: "어제",
    reason: "다음 설계에서 이어서 검토할 질문입니다.",
    sources: [1],
  },
];

const initialActivities: Activity[] = [
  { id: "seed-1", time: "10:42", title: "에이전트가 결정을 기록했습니다", body: "배포 조건을 다음 세션에도 유지하기 위해 저장" },
  { id: "seed-2", time: "09:58", title: "다음 행동을 직접 기록했습니다", body: "릴리스 체크리스트에 남길 확인 작업" },
  { id: "seed-3", time: "09:31", title: "검토할 후보를 구분했습니다", body: "환경 변수와 테스트 실패의 연관성" },
];

const defaultSettings: AppSettings = {
  autostart: false,
  background: false,
  notifications: true,
  reduceMotion: false,
  textSize: "15",
  exclude: "",
  syncPeriod: "5분",
};

function readStorage<T>(key: string, fallback: T): T {
  try {
    const value = localStorage.getItem(key);
    return value ? (JSON.parse(value) as T) : fallback;
  } catch {
    return fallback;
  }
}

function nowLabel() {
  return new Intl.DateTimeFormat("ko-KR", { hour: "2-digit", minute: "2-digit", hour12: false }).format(new Date());
}

function clampInspectorWidth(value: number) {
  return Math.min(440, Math.max(280, value));
}

class GraphCanvasBoundary extends Component<GraphCanvasBoundaryProps, GraphCanvasBoundaryState> {
  state: GraphCanvasBoundaryState = { hasError: false };

  static getDerivedStateFromError(): GraphCanvasBoundaryState {
    return { hasError: true };
  }

  render() {
    return this.state.hasError ? this.props.fallback : this.props.children;
  }
}

function Icon({ name }: { name: "brand" | "work" | "notes" | "activity" | "graph" | "panel-left" | "panel-right" | "search" | "settings" | "link" | "close" }) {
  const paths = {
    brand: <><path d="M5 3h17v18H5zM9 3v18" /><path d="M13 8h5M13 12h5M13 16h3" /></>,
    work: <><rect x="4" y="4" width="16" height="16" rx="3" /><path d="M4 10h16M10 10v10" /></>,
    notes: <><path d="M6 3h12v18H6zM9 8h6M9 12h6M9 16h4" /></>,
    activity: <><circle cx="12" cy="12" r="8" /><path d="M12 7v5l3 2" /></>,
    graph: <><circle cx="6" cy="7" r="2" /><circle cx="18" cy="5" r="2" /><circle cx="16" cy="18" r="2" /><path d="m7.8 6.4 8.4-1M7.4 8.5l7.3 8M17.5 6.8l-1 9.2" /></>,
    "panel-left": <><rect x="3" y="4" width="18" height="16" rx="2" /><path d="M9 4v16" /></>,
    "panel-right": <><rect x="3" y="4" width="18" height="16" rx="2" /><path d="M15 4v16" /></>,
    search: <><circle cx="10.5" cy="10.5" r="5.5" /><path d="m15 15 4 4" /></>,
    settings: <><circle cx="12" cy="12" r="3" /><path d="M19 12a7 7 0 0 0-.1-1.2l1.5-1.2-1.8-3.1-1.8.7a7 7 0 0 0-2.1-1.2L12.5 4h-3l-.3 2a7 7 0 0 0-2.1 1.2l-1.8-.7-1.8 3.1L5 10.8A7 7 0 0 0 5 13l-1.5 1.2 1.8 3.1 1.8-.7a7 7 0 0 0 2.1 1.2l.3 2h3l.3-2a7 7 0 0 0 2.1-1.2l1.8.7 1.8-3.1-1.5-1.2c.1-.4.1-.8.1-1.2Z" /></>,
    link: <><path d="M10 13.5 14 9.5" /><path d="M7.5 16.5H6a3.5 3.5 0 0 1 0-7h3" /><path d="M16.5 7.5H18a3.5 3.5 0 0 1 0 7h-3" /></>,
    close: <><path d="m6 6 12 12M18 6 6 18" /></>,
  }[name];

  return <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">{paths}</svg>;
}

function App() {
  const [status, setStatus] = useState<CoreStatus | null>(null);
  const [notes, setNotes] = useState<Note[]>(() => readStorage(STORAGE_NOTES, initialNotes));
  const [activities, setActivities] = useState<Activity[]>(() => readStorage(STORAGE_ACTIVITIES, initialActivities));
  const [settings, setSettings] = useState<AppSettings>(() => readStorage(STORAGE_SETTINGS, defaultSettings));
  const [scopes, setScopes] = useState<Record<string, string>>(() => readStorage(STORAGE_SCOPES, {}));
  const [page, setPage] = useState<Page>(() => readStorage(STORAGE_SESSION, { page: "work" as Page }).page);
  const [work, setWork] = useState(() => readStorage(STORAGE_SESSION, { work: 0 }).work);
  const [selected, setSelected] = useState(() => readStorage(STORAGE_SESSION, { selected: 1 }).selected);
  const [tab, setTab] = useState<WorkTab>(() => readStorage(STORAGE_SESSION, { tab: "all" as WorkTab }).tab);
  const [setting, setSettingCategory] = useState<SettingCategory>(() => readStorage(STORAGE_SESSION, { setting: "일반" as SettingCategory }).setting);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState("all");
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [detail, setDetail] = useState<DetailState>(null);
  const [undoStack, setUndoStack] = useState<Note[][]>([]);
  const [toast, setToast] = useState<string | null>(null);
  const [savingEditor, setSavingEditor] = useState(false);
  const [refreshingSource, setRefreshingSource] = useState<number | null>(null);
  const [refreshMessage, setRefreshMessage] = useState<string | null>(null);
  const [sidebarHidden, setSidebarHidden] = useState(() => readStorage(STORAGE_SESSION, { sidebarHidden: false }).sidebarHidden);
  const [sidebarMobileOpen, setSidebarMobileOpen] = useState(false);
  const [inspectorHidden, setInspectorHidden] = useState(() => readStorage(STORAGE_SESSION, { inspectorHidden: false }).inspectorHidden);
  const [inspectorWidth, setInspectorWidth] = useState(() => clampInspectorWidth(readStorage(STORAGE_SESSION, { inspectorWidth: DEFAULT_INSPECTOR_WIDTH }).inspectorWidth));
  const [graphMode, setGraphMode] = useState<GraphMode>("3d");
  const [graphKindFilter, setGraphKindFilter] = useState<GraphKindFilter>("all");
  const [graphActiveOnly, setGraphActiveOnly] = useState(false);
  const [graphSelection, setGraphSelection] = useState<GraphSelection>(null);
  const [prefersReducedMotion, setPrefersReducedMotion] = useState(false);
  const editorTitleRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    void invoke<CoreStatus>("core_status").then(setStatus).catch(() => setStatus(null));
  }, []);

  useEffect(() => {
    if (!isNativeRuntime()) return;
    void invoke<NativeUiMemory[]>("ui_memory_list").then((records) => {
      if (!records.length) return;
      const loaded = records.map((record, index): Note => ({
        id: -index - 1,
        work: record.work,
        kind: noteKindFromCore(record.kind),
        title: record.title,
        body: record.memory.body,
        author: record.memory.author,
        time: new Intl.DateTimeFormat("ko-KR", { hour: "2-digit", minute: "2-digit", hour12: false }).format(new Date(record.memory.updated_at)),
        reason: record.memory.reason,
        sources: record.memory.evidence.map((evidence) => sources.find((source) => source.provider.toLowerCase() === evidence.provider)?.id ?? -1).filter((id) => id >= 0),
        nativeId: record.id,
        nativeEvidence: record.memory.evidence,
        version: record.memory.version,
        retracted: Boolean(record.memory.retracted_at),
      }));
      setNotes((current) => [...loaded, ...current.filter((note) => !note.nativeId)]);
    }).catch(() => undefined);
  }, []);

  useEffect(() => {
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => setPrefersReducedMotion(media.matches);
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);

  useEffect(() => { localStorage.setItem(STORAGE_NOTES, JSON.stringify(notes)); }, [notes]);
  useEffect(() => { localStorage.setItem(STORAGE_ACTIVITIES, JSON.stringify(activities)); }, [activities]);
  useEffect(() => {
    localStorage.setItem(STORAGE_SETTINGS, JSON.stringify(settings));
    document.documentElement.style.setProperty("--note-size", `${settings.textSize}px`);
    document.body.classList.toggle("reduce-motion", settings.reduceMotion);
  }, [settings]);
  useEffect(() => { localStorage.setItem(STORAGE_SCOPES, JSON.stringify(scopes)); }, [scopes]);
  useEffect(() => {
    localStorage.setItem(STORAGE_SESSION, JSON.stringify({ page, work, selected, tab, setting, sidebarHidden, inspectorHidden, inspectorWidth }));
    document.documentElement.style.setProperty("--inspector-width", `${inspectorWidth}px`);
  }, [page, work, selected, tab, setting, sidebarHidden, inspectorHidden, inspectorWidth]);
  useEffect(() => { if (editor) window.setTimeout(() => editorTitleRef.current?.focus(), 0); }, [editor]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const command = event.metaKey || event.ctrlKey;
      if (command && event.key.toLowerCase() === "n") { event.preventDefault(); openEditor(); }
      else if (command && event.key.toLowerCase() === "k") { event.preventDefault(); setPage("search"); setSidebarMobileOpen(false); }
      else if (command && event.key === ",") { event.preventDefault(); setPage("settings"); setSidebarMobileOpen(false); }
      else if (event.key === "Escape") {
        if (editor) setEditor(null);
        else if (detail) setDetail(null);
        else setSidebarMobileOpen(false);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(null), 5000);
    return () => window.clearTimeout(timer);
  }, [toast]);

  const selectedNote = notes.find((note) => note.id === selected) ?? notes.find((note) => note.work === work) ?? notes[0];
  const activeWorkNotes = useMemo(() => notes.filter((note) => note.work === work && !note.retracted), [notes, work]);
  const noteCount = activeWorkNotes.length;
  const candidateCount = activeWorkNotes.filter((note) => note.kind === "후보").length;
  const graphModel = useMemo(() => createGraphModel(work, activeWorkNotes, sources), [work, activeWorkNotes]);
  const [activation, dispatchActivation] = useReducer(activationReducer, graphModel.demoEvents, createActivationState);
  const graphMotionReduced = settings.reduceMotion || prefersReducedMotion;

  useEffect(() => {
    dispatchActivation({ type: "load", events: graphModel.demoEvents });
    setGraphSelection(null);
  }, [graphModel.demoEvents]);

  useEffect(() => {
    if (page === "graph") return;
    if (activation.status !== "idle") dispatchActivation({ type: "reset" });
  }, [page, activation.status]);

  useEffect(() => {
    if (page !== "graph" || activation.status !== "playing") return;
    const timer = window.setTimeout(() => dispatchActivation({ type: "step" }), graphMotionReduced ? 360 : 820);
    return () => window.clearTimeout(timer);
  }, [page, activation.status, activation.cursor, graphMotionReduced]);

  function pushActivity(title: string, body: string) {
    setActivities((current) => [{ id: `${Date.now()}-${title}`, time: nowLabel(), title, body }, ...current].slice(0, 30));
  }

  function navigate(nextPage: Page) { setPage(nextPage); setSidebarMobileOpen(false); }

  function chooseWork(nextWork: number) {
    const firstNote = notes.find((note) => note.work === nextWork);
    setWork(nextWork); setSelected(firstNote?.id ?? 0); setTab("all"); navigate("work");
  }

  function selectNote(id: number) {
    const note = notes.find((item) => item.id === id);
    if (!note) return;
    setSelected(id); setWork(note.work);
    if (page !== "work") navigate("work");
  }

  function openEditor(id?: number) {
    const note = id ? notes.find((item) => item.id === id) : undefined;
    setEditor(note ? {
      id: note.id,
      title: note.title,
      body: note.body,
      kind: note.kind,
      reason: note.reason,
      sources: note.sources,
    } : {
      id: null,
      title: "",
      body: "",
      kind: "결정",
      reason: "현재 작업에서 다시 참고하기 위해 직접 기록했습니다.",
      sources: sources.map((source) => source.id),
    });
  }

  async function saveEditor(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const draft = editor;
    if (!draft || !draft.title.trim() || !draft.body.trim() || !draft.reason.trim()) return;
    if (!draft.sources.length) {
      setToast("메모를 저장하려면 근거를 하나 이상 선택하세요.");
      return;
    }
    setSavingEditor(true);
    const existing = draft.id === null ? undefined : notes.find((note) => note.id === draft.id);
    const evidence = draft.sources.map((sourceId) => sourceRefFor(sources.find((source) => source.id === sourceId) ?? sources[0]));
    const nativeRequest = {
      title: draft.title.trim(),
      work,
      kind: draft.kind,
      memory: {
        id: existing?.nativeId,
        body: draft.body.trim(),
        reason: draft.reason.trim(),
        evidence,
        author: existing?.author ?? "user",
        claim_type: draft.kind === "후보" ? "inferred" : "explicit",
        idempotency_key: idempotencyKey("ui-upsert"),
        expected_version: existing?.nativeId ? existing.version ?? 1 : undefined,
        supersedes_id: undefined,
      },
    };
    let mutation: NativeUiMemoryMutation | undefined;
    try {
      mutation = await invoke<NativeUiMemoryMutation>("ui_memory_upsert", { request: nativeRequest });
    } catch (error) {
      if (isNativeRuntime()) {
        setSavingEditor(false);
        setToast(`코어 저장에 실패했습니다: ${String(error)}`);
        return;
      }
    }
    setUndoStack((stack) => [...stack, structuredClone(notes)].slice(-10));
    const time = nowLabel();
    const nativeMemory = mutation?.memory;
    if (draft.id !== null) {
      if (!existing) {
        setSavingEditor(false);
        return;
      }
      setNotes((current) => current.map((note) => note.id === draft.id ? {
        ...note,
        title: draft.title.trim(),
        body: draft.body.trim(),
        kind: draft.kind,
        reason: draft.reason.trim(),
        sources: [...draft.sources],
        time,
        nativeId: nativeMemory?.id ?? note.nativeId,
        nativeEvidence: nativeMemory?.memory.evidence ?? note.nativeEvidence ?? evidence,
        version: nativeMemory?.memory.version ?? (note.version ?? 1) + 1,
        previous: { title: note.title, body: note.body, kind: note.kind, author: note.author, reason: note.reason },
      } : note));
      pushActivity("메모를 수정했습니다", draft.title.trim());
    } else {
      const nextId = Math.max(0, ...notes.map((note) => note.id)) + 1;
      const next: Note = {
        id: nextId,
        work,
        kind: draft.kind,
        title: draft.title.trim(),
        body: draft.body.trim(),
        author: "내가 작성",
        time,
        reason: draft.reason.trim(),
        sources: draft.sources,
        nativeId: nativeMemory?.id,
        nativeEvidence: nativeMemory?.memory.evidence ?? evidence,
        version: nativeMemory?.memory.version,
      };
      setNotes((current) => [next, ...current]); setSelected(nextId); pushActivity("새 메모를 기록했습니다", next.title); setToast("새 메모를 저장했습니다.");
    }
    setEditor(null);
    setSavingEditor(false);
    if (mutation) setToast("코어에 저장했습니다.");
    else setToast("브라우저 데모에 저장했습니다. 네이티브 코어에는 아직 기록하지 않았습니다.");
  }

  async function restoreNote(note: Note) {
    if (!note.previous) return;
    if (note.nativeId && isNativeRuntime()) {
      try {
        const mutation = await invoke<NativeUiMemoryMutation>("ui_memory_restore", {
          request: {
            id: note.nativeId,
            expected_version: note.version ?? 1,
            revision_version: Math.max(1, (note.version ?? 2) - 1),
            idempotency_key: idempotencyKey("ui-restore"),
          },
        });
        setUndoStack((stack) => [...stack, structuredClone(notes)].slice(-10));
        setNotes((current) => current.map((item) => item.id === note.id ? {
          ...item,
          title: mutation.memory.title,
          body: mutation.memory.memory.body,
          reason: mutation.memory.memory.reason,
          author: mutation.memory.memory.author,
          time: nowLabel(),
          version: mutation.memory.memory.version,
          nativeEvidence: mutation.memory.memory.evidence,
          previous: { title: item.title, body: item.body, kind: item.kind, author: item.author, reason: item.reason },
          retracted: false,
        } : item));
        pushActivity("메모를 이전 버전으로 복원했습니다", mutation.memory.title);
        setDetail(null);
        setToast("코어에 이전 버전을 새 버전으로 복원했습니다.");
      } catch (error) {
        setToast(`복원하지 못했습니다: ${String(error)}`);
      }
      return;
    }
    setUndoStack((stack) => [...stack, structuredClone(notes)].slice(-10));
    setNotes((current) => current.map((item) => item.id === note.id ? { ...item, title: note.previous!.title, body: note.previous!.body, kind: note.previous!.kind, author: note.previous!.author, reason: note.previous!.reason, time: nowLabel(), version: (item.version ?? 1) + 1, previous: { title: item.title, body: item.body, kind: item.kind, author: item.author, reason: item.reason } } : item));
    pushActivity("메모를 이전 버전으로 복원했습니다", note.previous.title); setDetail(null); setToast("이전 내용을 새 버전으로 복원했습니다.");
  }

  async function retractNote(note: Note) {
    if (note.nativeId && isNativeRuntime()) {
      try {
        const mutation = await invoke<NativeUiMemoryMutation>("ui_memory_retract", {
          request: {
            id: note.nativeId,
            expected_version: note.version ?? 1,
            idempotency_key: idempotencyKey("ui-retract"),
          },
        });
        setNotes((current) => current.map((item) => item.id === note.id ? { ...item, version: mutation.memory.memory.version, retracted: true } : item));
        setToast("코어에서 메모를 철회했습니다. 본문과 이력은 보존됩니다.");
      } catch (error) {
        setToast(`철회하지 못했습니다: ${String(error)}`);
      }
      return;
    }
    setNotes((current) => current.map((item) => item.id === note.id ? { ...item, retracted: true } : item));
    setToast(isNativeRuntime() ? "먼저 이 메모를 코어로 가져와야 철회할 수 있습니다." : "브라우저 데모에서 메모를 철회했습니다.");
  }

  async function importLocalNotes() {
    if (!isNativeRuntime()) {
      setToast("이 브라우저 화면은 데모 저장만 지원합니다. 네이티브 앱에서 가져오기를 실행하세요.");
      return;
    }
    let imported = 0;
    const importedById = new Map<number, NativeUiMemoryMutation>();
    for (const note of notes) {
      if (note.nativeId || note.retracted || !note.sources.length) continue;
      const evidence = note.nativeEvidence ?? note.sources.map((sourceId) => sourceRefFor(sources.find((source) => source.id === sourceId) ?? sources[0]));
      try {
        const mutation = await invoke<NativeUiMemoryMutation>("ui_memory_upsert", {
          request: {
            title: note.title,
            work: note.work,
            kind: note.kind,
            memory: {
              body: note.body,
              reason: note.reason,
              evidence,
              author: note.author,
              claim_type: note.kind === "후보" ? "inferred" : "explicit",
              idempotency_key: idempotencyKey(`ui-import-${note.id}`),
            },
          },
        });
        importedById.set(note.id, mutation);
        imported += 1;
      } catch (error) {
        setToast(`가져오기를 중단했습니다 (${imported}개 저장): ${String(error)}`);
        break;
      }
    }
    if (importedById.size) {
      setNotes((current) => current.map((note) => {
        const mutation = importedById.get(note.id);
        return mutation ? {
          ...note,
          nativeId: mutation.memory.id,
          nativeEvidence: mutation.memory.memory.evidence,
          version: mutation.memory.memory.version,
        } : note;
      }));
      setToast(`${imported}개 로컬 메모를 코어로 가져왔습니다. 기존 localStorage 데이터는 삭제하지 않았습니다.`);
    } else if (imported === 0) {
      setToast("가져올 메모가 없거나 근거가 선택되지 않았습니다.");
    }
  }

  async function clearCoreCache() {
    if (!isNativeRuntime()) {
      setToast("브라우저 데모에서는 코어 캐시를 삭제하지 않습니다.");
      return;
    }
    try {
      const result = await invoke<{ snapshots_removed: number }>("cache_clear");
      setToast(`캐시 ${result.snapshots_removed}개를 삭제했습니다. 원본 목록과 사용자 메모는 보존됩니다.`);
    } catch (error) {
      setToast(`캐시를 삭제하지 못했습니다: ${String(error)}`);
    }
  }

  async function backupCore() {
    const path = window.prompt("백업 파일의 새 경로를 입력하세요.");
    if (!path) return;
    if (!isNativeRuntime()) {
      setToast("브라우저 데모에서는 네이티브 DB 백업을 실행하지 않습니다.");
      return;
    }
    try {
      await invoke("core_backup", { path });
      setToast("코어 백업을 commit했습니다.");
    } catch (error) {
      setToast(`백업하지 못했습니다: ${String(error)}`);
    }
  }

  async function restoreCore() {
    const path = window.prompt("복원할 백업 파일의 경로를 입력하세요.");
    if (!path) return;
    if (!isNativeRuntime()) {
      setToast("브라우저 데모에서는 네이티브 DB 복원을 실행하지 않습니다.");
      return;
    }
    try {
      await invoke("core_restore", { path });
      setToast("코어 백업을 검증한 뒤 복원했습니다. 원래 localStorage 메모는 삭제하지 않았습니다.");
    } catch (error) {
      setToast(`복원하지 못했습니다: ${String(error)}`);
    }
  }

  function undoChange() {
    const previous = undoStack[undoStack.length - 1];
    if (!previous) return;
    setNotes(previous); setUndoStack((stack) => stack.slice(0, -1)); setToast("이전 메모 상태로 되돌렸습니다.");
  }

  async function refreshSource(source: Source) {
    if (!isNativeRuntime()) {
      setRefreshingSource(source.id); setRefreshMessage("브라우저 데모 확인 중 · 실제 계정이나 vault에는 접근하지 않습니다.");
      window.setTimeout(() => { setRefreshingSource(null); setRefreshMessage("데모 확인만 완료했습니다 · 마지막 예시 캐시를 유지합니다."); }, 700);
      return;
    }
    navigate("connections");
    setToast("연결 관리에서 확인할 계정과 경로를 선택하세요.");
  }

  function setSetting<K extends keyof AppSettings>(key: K, value: AppSettings[K]) { setSettings((current) => ({ ...current, [key]: value })); setToast("설정을 저장했습니다."); }

  function resizeInspector(nextWidth: number) {
    setInspectorWidth(clampInspectorWidth(nextWidth));
  }

  function handleSplitterPointerDown(event: ReactPointerEvent<HTMLDivElement>) {
    const splitter = event.currentTarget;
    splitter.setPointerCapture(event.pointerId);
    const updateWidth = (clientX: number) => {
      const workspace = splitter.parentElement?.getBoundingClientRect();
      resizeInspector((workspace?.right ?? window.innerWidth) - clientX);
    };
    const onMove = (moveEvent: PointerEvent) => updateWidth(moveEvent.clientX);
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp, { once: true });
    updateWidth(event.clientX);
  }

  function handleSplitterKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (event.key === "ArrowLeft") { event.preventDefault(); resizeInspector(inspectorWidth + 16); }
    if (event.key === "ArrowRight") { event.preventDefault(); resizeInspector(inspectorWidth - 16); }
  }

  function pageTitle() {
    if (page === "work") return `작업 묶음　/　${works[work]}`;
    return { workflow: "개인 작업 공간 / 업무와 할 일", home: "개인 작업 공간　/　최근 맥락", notes: "개인 작업 공간　/　모든 메모", activity: "개인 작업 공간　/　활동 기록", graph: "탐색　/　지식 그래프", search: "전체 검색", connections: "설정　/　연결 상태", settings: `설정　/　${setting}` }[page];
  }

  function renderPage() {
    if (page === "workflow") return <WorkflowPanel />;
    if (page === "work") return <WorkPage />;
    if (page === "graph") return renderGraphPage();
    if (page === "search") return <SearchPage />;
    if (page === "connections") return <ConnectionsPage />;
    if (page === "settings") return <SettingsPage />;
    const title = page === "home" ? "어디서 이어서 할까요?" : page === "notes" ? "모든 메모" : "활동 기록";
    return <OverviewPage title={title} />;
  }

  function WorkPage() {
    const list = activeWorkNotes.filter((note) => tab === "all" || (tab === "candidate" && note.kind === "후보"));
    return <div className="workspace"><div className="reading"><section className="section intro-section"><div className="container"><p className="eyebrow">작업 노트 / {String(work + 1).padStart(2, "0")}</p><div className="row-between intro-heading"><div><h1>{works[work]}</h1><p className="muted">{work === 0 ? "흩어진 진행 상황과 결정을 한곳에서 이어갑니다." : "다음 작업에서도 잊지 않을 결정과 질문을 모읍니다."}</p></div><button className="btn btn-primary" type="button" onClick={() => openEditor()}>＋ 메모 쓰기</button></div><div className="tag-row"><span className="tag">GitHub</span><span className="tag">Obsidian</span><span className="small muted">최근 기록 9월 18일</span></div></div></section><div className="tabs" role="tablist" aria-label="작업 내용">{([["all", "메모", noteCount], ["candidate", "검토할 후보", candidateCount], ["sources", "연결 자료", 2], ["activity", "활동", activities.length]] as const).map(([value, label, count]) => <button key={value} className={`tab ${tab === value ? "active" : ""}`} type="button" role="tab" aria-selected={tab === value} onClick={() => setTab(value)}>{label}<span>{count}</span></button>)}</div>{work === 0 && <div className="notice" role="status"><strong>일부 근거를 다시 확인해야 합니다</strong><br />GitHub 자료의 마지막 확인은 09:12입니다. <button type="button" onClick={() => refreshSource(sources[0])}>{refreshingSource === 0 ? "확인 중…" : "최신 상태 확인"}</button>{refreshMessage && <span className="status-inline">{refreshMessage}</span>}</div>}<section className="section memory-section"><div className="container">{tab === "sources" ? <SourceList onOpen={(source) => setDetail({ type: "source", source })} /> : tab === "activity" ? <ActivityList items={activities} /> : <><div className="subhead"><span>{tab === "candidate" ? "아직 확정하지 않은 내용" : "기억해 둘 내용"}</span><span className="muted small">최근 기록 순</span></div>{list.map((note) => <NoteCard key={note.id} note={note} selected={note.id === selected} onClick={() => selectNote(note.id)} />)}{list.length === 0 && <div className="empty">검토할 후보가 없습니다.</div>}</>}</div></section>{tab === "all" && <section className="section activity-section"><div className="container"><div className="subhead">최근 활동</div><ActivityList items={activities.slice(0, 3)} compact /></div></section>}<div className="footer-line">로컬 작업 공간 · 메모 변경은 이 기기에만 저장됩니다.</div></div><div className="splitter" role="separator" tabIndex={0} aria-label="근거 패널 너비 조절" aria-orientation="vertical" aria-valuemin={280} aria-valuemax={440} aria-valuenow={inspectorWidth} onPointerDown={handleSplitterPointerDown} onKeyDown={handleSplitterKeyDown} />{!inspectorHidden && <EvidencePanel />}</div>;
  }

  function selectGraphNode(nodeId: string) {
    setInspectorHidden(false);
    setGraphSelection({ type: "node", nodeId });
    const node = graphModel.nodes.find((item) => item.id === nodeId);
    if (node?.kind === "memory" && node.noteId !== undefined) setSelected(node.noteId);
  }

  function renderGraphPage() {
    const activeNodeSet = new Set(activation.activeNodeIds);
    const activeEdgeSet = new Set(activation.activeEdgeIds);
    const selectedGraphNode = graphSelection?.type === "node" ? graphModel.nodes.find((node) => node.id === graphSelection.nodeId) : undefined;
    const selectedGraphLink = graphSelection?.type === "edge" ? graphModel.links.find((link) => link.id === graphSelection.edgeId) : undefined;
    const filteredNodes = graphModel.nodes.filter((node) => {
      const matchesKind = graphKindFilter === "all" || node.kind === graphKindFilter;
      const matchesActivity = !graphActiveOnly || activeNodeSet.has(node.id);
      return matchesKind && matchesActivity;
    });
    const visibleNodeIds = new Set(filteredNodes.map((node) => node.id));
    const filteredLinks = graphModel.links.filter((link) => {
      const matchesNodes = visibleNodeIds.has(link.sourceId) && visibleNodeIds.has(link.targetId);
      return matchesNodes && (!graphActiveOnly || activeEdgeSet.has(link.id));
    });
    const stageProgress = activation.cursor < 0 ? 0 : activation.cursor + 1;
    const activationStatus = activation.status === "idle" ? "정지" : activation.status === "playing" ? `데모 재생 중 · ${stageProgress}/${activation.events.length}` : "데모 완료";

    return <div className="workspace graph-workspace"><div className="reading graph-reading"><section className="section intro-section"><div className="container"><p className="eyebrow">탐색 그래프 / {String(work + 1).padStart(2, "0")}</p><div className="row-between intro-heading"><div><h1>3차원 정보 지도</h1><p className="muted">목차에서 정보를 찾고, 지도의 연결을 따라 맥락을 탐색하세요.</p></div><span className="graph-status-badge" data-state={activation.status}>{activationStatus}</span></div><div className="tag-row"><span className="tag">현재 작업 · {works[work]}</span><span className="tag">실제 AI 추적 없음</span><span className="small muted">선택한 데모 재생만 활성화 상태를 바꿉니다.</span></div></div></section><section className="graph-toolbar" aria-label="그래프 보기 설정"><div className="graph-view-toggle" role="group" aria-label="그래프 보기"><button className={`btn ${graphMode === "3d" ? "btn-primary" : "btn-secondary"}`} type="button" aria-pressed={graphMode === "3d"} onClick={() => setGraphMode("3d")}>3D 탐색</button><button className={`btn ${graphMode === "list" ? "btn-primary" : "btn-secondary"}`} type="button" aria-pressed={graphMode === "list"} onClick={() => setGraphMode("list")}>목록 보기</button></div><label className="graph-filter">자료 유형<select value={graphKindFilter} onChange={(event) => setGraphKindFilter(event.target.value as GraphKindFilter)}><option value="all">모두</option><option value="source">자료</option><option value="memory">메모</option></select></label><label className="graph-check"><input type="checkbox" checked={graphActiveOnly} onChange={(event) => setGraphActiveOnly(event.target.checked)} />활성 자료만</label><div className="graph-toolbar-actions"><button className="btn btn-primary" type="button" disabled={activation.status === "playing" || activation.events.length === 0} onClick={() => dispatchActivation({ type: "start" })}>{activation.status === "playing" ? "재생 중…" : "데모 재생"}</button><button className="btn btn-ghost" type="button" disabled={activation.status === "idle"} onClick={() => dispatchActivation({ type: "reset" })}>리셋</button></div></section><div className="graph-notice" role="status"><strong>데모 경계</strong><span>예시 데이터로 검색 → 반환 → 인용을 재생합니다. 실제 AI 사용 기록은 아닙니다.</span></div><ol className="graph-timeline" hidden={activation.status === "idle"} aria-label="활성화 단계">{activation.events.map((event, index) => <li key={event.eventId} className={`${index <= activation.cursor ? "complete" : ""} ${index === activation.cursor ? "current" : ""}`}><span className="graph-timeline-marker">{index + 1}</span><span><strong>{stageLabel(event.stage)}</strong><small>{event.observer === "demo_fixture" ? "fixture · 실제 관측 아님" : event.observer}</small></span></li>)}</ol><div className="map-explorer"><MapContents key={graphModel.requestId} model={graphModel} selectedId={selectedGraphNode?.id} activeIds={activation.activeNodeIds} onSelect={(id) => { setGraphKindFilter("all"); setGraphActiveOnly(false); selectGraphNode(id); }} /><div className="graph-output">{graphMode === "3d" ? <GraphCanvasBoundary key={graphModel.requestId} fallback={<GraphFallback onList={() => setGraphMode("list")} />}><Suspense fallback={<div className="graph-fallback" role="status">3D 그래프를 불러오는 중…</div>}><GraphCanvas model={graphModel} kindFilter={graphKindFilter} activeOnly={graphActiveOnly} activation={activation} graphMotionReduced={graphMotionReduced} selectedNodeId={selectedGraphNode?.id} onNodeSelect={selectGraphNode} onLinkSelect={(edgeId) => setGraphSelection({ type: "edge", edgeId })} onClear={() => setGraphSelection(null)} /></Suspense></GraphCanvasBoundary> : renderAccessibleGraphList({ nodes: filteredNodes, links: filteredLinks, activation, selected: graphSelection, onNodeSelect: selectGraphNode, onLinkSelect: (edgeId) => setGraphSelection({ type: "edge", edgeId }) })}</div></div><div className="graph-footnote"><span>노드 {filteredNodes.length}개 · 연결 {filteredLinks.length}개</span><span>밝은 연결: 선택한 노드 주변 · 민트: 데모 이벤트</span></div></div><div className="splitter" role="separator" tabIndex={0} aria-label="그래프 근거 패널 너비 조절" aria-orientation="vertical" aria-valuemin={280} aria-valuemax={440} aria-valuenow={inspectorWidth} onPointerDown={handleSplitterPointerDown} onKeyDown={handleSplitterKeyDown} />{!inspectorHidden && <GraphEvidencePanel node={selectedGraphNode} link={selectedGraphLink} activation={activation} onClose={() => setInspectorHidden(true)} />}</div>;
  }

  function GraphFallback({ onList }: { onList: () => void }) {
    return <div className="graph-fallback" role="status"><strong>3D 캔버스를 사용할 수 없습니다.</strong><p>WebGL을 사용할 수 없는 환경에서도 아래 목록 대안으로 같은 자료와 근거에 접근할 수 있습니다.</p><button className="btn btn-secondary" type="button" onClick={onList}>목록 대안 열기</button></div>;
  }

  function renderAccessibleGraphList({ nodes, links, activation, selected, onNodeSelect, onLinkSelect }: { nodes: GraphNode[]; links: GraphLink[]; activation: ActivationState; selected: GraphSelection; onNodeSelect: (nodeId: string) => void; onLinkSelect: (edgeId: string) => void }) {
    return <section className="graph-list-view" aria-label="그래프 접근 가능한 목록"><div className="graph-list-heading"><div><h2>그래프 목록</h2><p className="small muted">키보드로 자료·메모를 선택하면 오른쪽 근거 패널에서 원문과 저장 이유를 확인할 수 있습니다.</p></div><span className="tag">{nodes.length}개 노드</span></div><ul className="graph-node-list">{nodes.map((node) => <li key={node.id}><button className={`graph-node-row ${selected?.type === "node" && selected.nodeId === node.id ? "selected" : ""}`} type="button" aria-pressed={selected?.type === "node" && selected.nodeId === node.id} onClick={() => onNodeSelect(node.id)}><span className={`graph-marker marker-${node.kind}`} aria-hidden="true">{node.kind === "source" ? "□" : node.kind === "memory" ? "●" : "◇"}</span><span className="graph-node-copy"><strong>{node.title}</strong><small>{graphNodeKindLabel(node.kind)} · {node.label}{node.stale ? " · 오래된 자료" : ""}</small></span><span className="graph-stage-badges">{(activation.nodeStages[node.id] ?? []).map((stage) => <span className="graph-stage-badge" key={stage}>{stageShortLabel(stage)}</span>)}</span></button></li>)}</ul>{links.length > 0 ? <><div className="subhead graph-list-subhead">연결</div><ul className="graph-edge-list">{links.map((link) => <li key={link.id}><button className={`graph-edge-row ${selected?.type === "edge" && selected.edgeId === link.id ? "selected" : ""}`} type="button" aria-pressed={selected?.type === "edge" && selected.edgeId === link.id} onClick={() => onLinkSelect(link.id)}><span className="graph-edge-line" aria-hidden="true" /><span><strong>{graphLinkKindLabel(link.kind)}</strong><small>{link.label}</small></span>{activation.activeEdgeIds.includes(link.id) && <span className="graph-stage-badge">활성</span>}</button></li>)}</ul></> : <div className="empty">현재 필터에 표시할 연결이 없습니다.</div>}</section>;
  }

  function OverviewPage({ title }: { title: string }) {
    return <div className="page"><section className="section overview-section"><div className="container"><p className="eyebrow">개인 작업 공간</p><h1>{title}</h1><p className="muted">{page === "home" ? "작업을 다시 시작하는 데 필요한 맥락이 여기 있습니다." : "로컬 메모와 출처, 저장 이유를 함께 확인하세요."}</p>{page === "home" ? <><div className="subhead">최근 작업 묶음</div><div className="overview-work-list">{works.map((name, index) => <button className="overview-work" type="button" key={name} onClick={() => chooseWork(index)}><span className="work-number">0{index + 1}</span><span><strong>{name}</strong><small>메모 {notes.filter((note) => note.work === index && !note.retracted).length}개 · GitHub, Obsidian</small></span><span className="arrow">→</span></button>)}</div></> : page === "notes" ? <div className="overview-note-list">{notes.filter((note) => !note.retracted).map((note) => <NoteCard key={note.id} note={note} selected={note.id === selected} onClick={() => selectNote(note.id)} />)}</div> : <ActivityList items={activities} />}</div></section></div>;
  }

  function SearchPage() {
    const normalized = query.trim().toLowerCase();
    const matchingNotes = notes.filter((note) => !note.retracted && (filter === "all" || filter === "notes") && `${note.title} ${note.body}`.toLowerCase().includes(normalized));
    const matchingSources = sources.filter((source) => filter !== "notes" && (filter === "all" || filter === source.provider) && `${source.title} ${source.body}`.toLowerCase().includes(normalized));
    return <div className="page"><section className="section overview-section"><div className="container"><h1>맥락 검색</h1><p className="muted">작업 묶음을 넘어 메모와 원본 자료를 찾습니다.</p><div className="searchbox"><label className="sr-only" htmlFor="global-query">검색어</label><div className="search-input-wrap"><Icon name="search" /><input id="global-query" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="배포, 테스트, 읽기 전용…" /></div><label className="sr-only" htmlFor="search-filter">자료 유형</label><select id="search-filter" value={filter} onChange={(event) => setFilter(event.target.value)}><option value="all">모든 자료</option><option value="notes">메모</option><option value="GitHub">GitHub</option><option value="Obsidian">Obsidian</option></select></div><div className="subhead">검색 결과 {matchingNotes.length + matchingSources.length}개</div>{matchingNotes.map((note) => <NoteCard key={`note-${note.id}`} note={note} selected={note.id === selected} onClick={() => selectNote(note.id)} />)}<SourceList sources={matchingSources} onOpen={(source) => setDetail({ type: "source", source })} />{matchingNotes.length + matchingSources.length === 0 && <div className="empty">일치하는 자료가 없습니다. 다른 검색어를 입력해 주세요.</div>}</div></section></div>;
  }

  function ConnectionsPage() { return <PluginsPanel />; }

  function SettingsPage() {
    return <div className="page"><section className="section overview-section"><div className="container"><h1>설정</h1><p className="muted">기억하는 방식과 연결 범위를 내 환경에 맞게.</p><div className="setting-layout"><nav className="setting-nav" aria-label="설정 분류">{settingCategories.map((category) => <button type="button" key={category} className={category === setting ? "active" : ""} aria-current={category === setting ? "page" : undefined} onClick={() => setSettingCategory(category)}>{category}</button>)}</nav><div className="setting-content"><h2>{setting}</h2>{renderSettingBody()}<p className="footnote">설정은 이 기기에 저장됩니다. OS 권한이나 외부 계정은 변경하지 않습니다.</p></div></div></div></section></div>;
  }

  function renderSettingBody() {
    if (setting === "메모리와 그래프") return <MemoryGraphPanel native={isNativeRuntime()} />;
    if (setting === "일반") return <div className="settings-list"><SettingRow title="로그인 시 자동 실행" description="실제 macOS 적용은 앱 구현에서 제공됩니다."><input type="checkbox" aria-label="자동 실행 예시 설정" checked={settings.autostart} onChange={(event) => setSetting("autostart", event.target.checked)} /></SettingRow><SettingRow title="창을 닫아도 백그라운드 유지" description="CLI와 MCP가 맥락을 조회할 수 있도록 유지합니다."><input type="checkbox" aria-label="백그라운드 유지 예시 설정" checked={settings.background} onChange={(event) => setSetting("background", event.target.checked)} /></SettingRow><SettingRow title="알림" description="새 메모와 확인이 필요한 연결을 알립니다."><input type="checkbox" aria-label="알림 예시 설정" checked={settings.notifications} onChange={(event) => setSetting("notifications", event.target.checked)} /></SettingRow></div>;
    if (setting === "모양") return <div className="settings-list"><SettingRow title="글자 크기" description="본문의 크기를 바로 확인합니다."><select value={settings.textSize} aria-label="글자 크기" onChange={(event) => setSetting("textSize", event.target.value as AppSettings["textSize"])}><option value="15">기본 · 15px</option><option value="16">크게 · 16px</option><option value="18">아주 크게 · 18px</option></select></SettingRow><SettingRow title="모션 감소" description="이동 효과 없이 즉시 전환합니다."><input type="checkbox" aria-label="모션 감소" checked={settings.reduceMotion} onChange={(event) => setSetting("reduceMotion", event.target.checked)} /></SettingRow><div className="card preview-card"><h3>메모 미리보기</h3><p>테스트 환경이 복구될 때까지 배포를 보류합니다.</p></div></div>;
    if (setting === "플러그인") return <div className="settings-list"><p>Obsidian 로컬 경로를 우선으로 GitHub와 Jira를 연결합니다. 여러 계정과 경로를 각각 관리할 수 있습니다.</p><button className="btn btn-secondary" type="button" onClick={() => navigate("connections")}>연결과 수집 범위 관리</button><div className="notice">제3자 마켓플레이스는 초기 범위에 포함하지 않습니다.</div></div>;
    if (setting === "에이전트 연결") return <AgentConnectionsPanel />;
    if (setting === "메모와 데이터") return <div className="settings-list"><SettingRow title="자동 메모 기준" description="명시한 결정·선호·제약을 기록하고, 추론은 후보로 구분합니다."><span className="tag">초기 기준</span></SettingRow><SettingRow title="비기록 범위" description="인증 비밀은 기록 대상에서 제외합니다."><input className="inline-input" aria-label="비기록 범위" value={settings.exclude} onChange={(event) => setSetting("exclude", event.target.value)} placeholder="예: 개인 일기" /></SettingRow><SettingRow title="메모 내보내기" description="이 기기에 저장된 메모를 JSON으로 내려받습니다."><button className="btn btn-secondary" type="button" onClick={exportNotes}>내보내기</button></SettingRow><SettingRow title="로컬 메모를 코어로 가져오기" description="명시적으로 실행할 때만 localStorage 메모를 네이티브 SQLite 코어에 복사합니다. 원본 localStorage는 삭제하지 않습니다."><button className="btn btn-secondary" type="button" onClick={() => { void importLocalNotes(); }}>가져오기</button></SettingRow><div className="notice">저장 성공은 코어의 commit 뒤에만 표시됩니다. 브라우저에서는 데모 저장과 네이티브 저장을 구분합니다.</div></div>;
    if (setting === "동기화") return <div className="settings-list"><SettingRow title="원격 조회 주기 (준비 중)" description="GitHub·Jira 자동 조회는 아직 적용되지 않습니다. Obsidian은 연결별 자동 갱신을 켜면 10초 간격으로 확인합니다."><select disabled value={settings.syncPeriod} aria-label="조회 주기" onChange={(event) => setSetting("syncPeriod", event.target.value as AppSettings["syncPeriod"])}><option>5분</option><option>15분</option><option>수동</option></select></SettingRow><SettingRow title="웹훅 릴레이" description="사용자가 별도로 켜고 권한을 부여해야 합니다."><span className="tag">연결 안 함</span></SettingRow><button className="btn btn-secondary" type="button" onClick={() => navigate("connections")}>연결별 마지막 성공 확인</button><div className="card"><h3>로컬 데이터 보호</h3><p className="small muted">연결 해제는 credential과 선택 범위를 끊고, 캐시 삭제는 별도의 명시 작업입니다. 사용자 메모는 백업·복원과 독립적으로 보존됩니다.</p><div className="actions"><button className="btn btn-secondary" type="button" onClick={() => { void backupCore(); }}>백업</button><button className="btn btn-secondary" type="button" onClick={() => { void restoreCore(); }}>복원</button><button className="btn btn-ghost" type="button" onClick={() => { void clearCoreCache(); }}>캐시만 삭제</button></div></div></div>;
    return <div className="settings-list"><p>디자인 및 로컬 기능 · 2026.09.18</p><div className="notice">초기 업데이트는 Homebrew 배포 경로를 사용할 계획입니다. 설치·업데이트 기능은 이 화면에서 실행하지 않습니다.</div><p className="small muted">진단 예시에는 토큰, 외부 문서 본문, 메모 본문을 포함하지 않습니다.</p><button className="btn btn-secondary" type="button" onClick={exportDiagnostics}>진단 예시 내보내기</button></div>;
  }

  function SettingRow({ title, description, children }: { title: string; description: string; children: ReactNode }) {
    return <div className="setting-row"><div><strong>{title}</strong><p>{description}</p></div><div className="setting-control">{children}</div></div>;
  }

  function GraphEvidencePanel({ node, link, activation, onClose }: { node?: GraphNode; link?: GraphLink; activation: ActivationState; onClose: () => void }) {
    if (link) {
      const from = graphModel.nodes.find((item) => item.id === link.sourceId);
      const to = graphModel.nodes.find((item) => item.id === link.targetId);
      return <aside className="evidence graph-evidence" aria-label="선택한 그래프 연결의 근거"><div className="row-between"><h2>연결 근거</h2><button className="tool panel-close" type="button" aria-label="그래프 근거 패널 닫기" onClick={onClose}><Icon name="close" /></button></div><div className="evidence-title">{graphLinkKindLabel(link.kind)}</div><div className="graph-inspector-path"><span>{from?.title ?? link.sourceId}</span><span aria-hidden="true">→</span><span>{to?.title ?? link.targetId}</span></div><div className="label">연결 설명</div><blockquote>{link.label}</blockquote><dl className="metadata"><dt>출처</dt><dd>{link.provenance === "explicit" ? "사용자가 선택한 evidence" : "데모 fixture · 실제 관측 아님"}</dd><dt>활성화</dt><dd>{activation.activeEdgeIds.includes(link.id) ? "선택한 데모 이벤트에서 지정됨" : "활성화되지 않음"}</dd><dt>간선 ID</dt><dd className="mono">{link.id}</dd></dl><p className="footnote">노드 양끝이 활성화됐다는 이유만으로 이 연결을 활성화하지 않습니다. 이벤트에 지정된 간선만 강조합니다.</p></aside>;
    }
    if (!node) return <aside className="evidence empty-evidence graph-evidence"><div className="row-between"><h2>그래프 근거</h2><button className="tool panel-close" type="button" aria-label="그래프 근거 패널 닫기" onClick={onClose}><Icon name="close" /></button></div><p>그래프에서 자료나 메모를 선택하면 실제 연결·원문·저장 이유를 확인할 수 있습니다.</p><p className="footnote">3D 캔버스가 보이지 않으면 목록 대안에서도 같은 선택과 패널을 사용할 수 있습니다.</p></aside>;

    if (node.kind === "request") {
      return <aside className="evidence graph-evidence" aria-label="데모 요청 정보"><div className="row-between"><h2>데모 요청</h2><button className="tool panel-close" type="button" aria-label="그래프 근거 패널 닫기" onClick={onClose}><Icon name="close" /></button></div><div className="evidence-title">{node.title}</div><div className="notice">실제 AI 요청이나 에이전트 수신을 나타내지 않습니다. 버튼을 눌렀을 때만 결정적 fixture 이벤트를 재생합니다.</div><div className="label">현재 단계</div><blockquote>{activation.currentStage ? stageLabel(activation.currentStage) : "아직 재생하지 않음"}</blockquote><dl className="metadata"><dt>요청 ID</dt><dd className="mono">{activation.requestId ?? node.id}</dd><dt>관측 주체</dt><dd>demo_fixture</dd><dt>진행</dt><dd>{activation.cursor < 0 ? "0" : activation.cursor + 1} / {activation.events.length}</dd></dl></aside>;
    }

    if (node.kind === "source" && node.sourceId !== undefined) {
      const source = sources.find((item) => item.id === node.sourceId);
      if (!source) return null;
      const linkedNotes = activeWorkNotes.filter((note) => note.sources.includes(source.id));
      return <aside className="evidence graph-evidence" aria-label="선택한 자료의 근거"><div className="row-between"><h2>자료 근거</h2><button className="tool panel-close" type="button" aria-label="그래프 근거 패널 닫기" onClick={onClose}><Icon name="close" /></button></div><div className="evidence-title">{source.title}</div><div className="source-head"><span className={`provider provider-${source.provider.toLowerCase()}`}>{source.provider[0]}</span>{source.provider}<span className="muted source-state">{source.stale ? "오래된 자료" : "캐시"}</span></div><div className="label">수집한 원문 발췌</div><blockquote>{source.body}</blockquote><dl className="metadata"><dt>참조</dt><dd>{source.ref}</dd><dt>마지막 확인</dt><dd className="mono">{source.time}</dd><dt>상태</dt><dd>{source.stale ? "stale · 재확인 필요" : "accessible · 예시 캐시"}</dd><dt>연결 메모</dt><dd>{linkedNotes.length}개</dd></dl><div className="actions"><button className="btn btn-secondary" type="button" onClick={() => setDetail({ type: "source", source })}>원문 보기</button></div>{linkedNotes.length > 0 && <><div className="label">이 자료를 근거로 삼은 메모</div><div className="graph-related-list">{linkedNotes.map((note) => <button key={note.id} type="button" onClick={() => selectGraphNode(noteGraphId(note))}><span>{note.kind}</span>{note.title}</button>)}</div></>}</aside>;
    }

    const note = node.noteId === undefined ? undefined : notes.find((item) => item.id === node.noteId);
    if (!note) return null;
    const linkedSources = note.nativeId ? [] : sources.filter((source) => note.sources.includes(source.id));
    const stages = activation.nodeStages[node.id] ?? [];
    return <aside className="evidence graph-evidence" aria-label="선택한 메모의 근거"><div className="row-between"><h2>메모 근거</h2><button className="tool panel-close" type="button" aria-label="그래프 근거 패널 닫기" onClick={onClose}><Icon name="close" /></button></div><div className="evidence-title">{note.title}</div>{stages.length > 0 && <div className="graph-stage-badges graph-stage-badges--panel">{stages.map((stage) => <span className="graph-stage-badge" key={stage}>{stageLabel(stage)}</span>)}</div>}<div className="label">메모 본문</div><blockquote>{note.body}</blockquote><div className="label">왜 기억했나요?</div><blockquote>{note.reason}</blockquote><dl className="metadata"><dt>작성 주체</dt><dd>{note.author}</dd><dt>기록 유형</dt><dd>{note.kind}</dd><dt>버전</dt><dd className="mono">{note.version ?? 1}</dd><dt>근거</dt><dd>{note.nativeId ? note.nativeEvidence?.length ?? 0 : linkedSources.length}개 · {works[note.work]}</dd></dl><div className="label">연결된 원본</div><div className="graph-related-list">{note.nativeId && graphModel.links.filter((edge) => edge.kind === "evidence" && edge.targetId === node.id).map((edge) => <button key={edge.id} type="button" onClick={() => selectGraphNode(edge.sourceId)}>{graphModel.nodes.find((item) => item.id === edge.sourceId)?.title}</button>)}{linkedSources.map((source) => <button key={source.id} type="button" onClick={() => selectGraphNode(sourceGraphId(source.id))}><span>{source.provider}</span>{source.title}</button>)}</div><div className="actions">{!note.retracted && <><button className="btn btn-secondary" type="button" onClick={() => openEditor(note.id)}>메모 수정</button><button className="btn btn-ghost" type="button" onClick={() => { void retractNote(note); }}>철회</button></>}<button className="btn btn-ghost" type="button" onClick={() => setDetail({ type: "history", note })}>변경 이력</button></div><p className="footnote">현재 Core에는 AI 추적 계약이 없어 실제 사용·프롬프트 포함을 표시하지 않습니다. 재생 단계는 fixture로만 구분됩니다.</p></aside>;
  }

  function EvidencePanel() {
    if (!selectedNote) return <aside className="evidence empty-evidence"><h2>연결된 근거</h2><p>메모를 선택하면 근거를 확인할 수 있습니다.</p></aside>;
    const linkedSources = sources.filter((source) => selectedNote.sources.includes(source.id));
    return <aside className="evidence" aria-label="선택한 메모의 근거"><div className="row-between"><h2>메모의 근거</h2><span className="small muted">{linkedSources.length}개 연결</span><button className="tool panel-close" type="button" aria-label="근거 패널 닫기" onClick={() => setInspectorHidden(true)}><Icon name="close" /></button></div><div className="evidence-title">{selectedNote.title}</div>{selectedNote.retracted && <div className="notice">철회된 메모 · 본문과 이력은 보존됩니다.</div>}<div className="label">왜 기억했나요?</div><blockquote>{selectedNote.reason}</blockquote><div className="label">연결된 원본</div><div className="source-stack">{linkedSources.map((source) => <SourceEvidence key={source.id} source={source} />)}</div><dl className="metadata"><dt>작성 주체</dt><dd>{selectedNote.author}</dd><dt>기록 유형</dt><dd>{selectedNote.kind}</dd><dt>저장 위치</dt><dd>이 기기 · {works[selectedNote.work]}</dd><dt>버전</dt><dd className="mono">{selectedNote.version ?? 1}</dd></dl><div className="actions">{!selectedNote.retracted && <><button className="btn btn-secondary" type="button" onClick={() => openEditor(selectedNote.id)}>메모 수정</button><button className="btn btn-ghost" type="button" onClick={() => { void retractNote(selectedNote); }}>철회</button></>}<button className="btn btn-ghost" type="button" onClick={() => setDetail({ type: "history", note: selectedNote })}>변경 이력</button></div><p className="footnote">원본의 상태가 바뀌어도 사용자 결정은 유지됩니다. 추론은 후보로 구분합니다.</p></aside>;
  }

  function SourceEvidence({ source }: { source: Source }) {
    return <article className="source evidence-source"><div className="source-head"><span className={`provider provider-${source.provider.toLowerCase()}`}>{source.provider[0]}</span>{source.provider}<span className="muted source-state">{source.stale ? "오래된 자료" : "캐시"}</span></div><strong>{source.title}</strong><p>{source.ref}<br />마지막 확인 · <span className="mono">{source.time}</span></p><button type="button" onClick={() => setDetail({ type: "source", source })}>수집한 원문 보기 ↗</button></article>;
  }

  function NoteCard({ note, selected: isSelected, onClick }: { note: Note; selected: boolean; onClick: () => void }) {
    return <button className={`note ${isSelected ? "selected" : ""}`} type="button" aria-pressed={isSelected} onClick={onClick}><div className="note-top"><span className="type">{note.kind}</span><span className="mono">{note.time}</span></div><h3>{note.title}</h3><p>{note.body}</p><div className="note-foot"><span>{note.author}</span><span>·</span><span>근거 {note.sources.length}개</span>{note.kind === "후보" && <span>· 검증 전</span>}</div></button>;
  }

  function SourceList({ sources: list = sources, onOpen }: { sources?: Source[]; onOpen: (source: Source) => void }) {
    return <div className="source-list">{list.map((source) => <button className="source-list-row" type="button" key={source.id} onClick={() => onOpen(source)}><span className={`provider provider-${source.provider.toLowerCase()}`}>{source.provider[0]}</span><span><strong>{source.title}</strong><small>{source.provider} · {source.time}</small></span><span className="source-arrow">↗</span></button>)}</div>;
  }

  function ActivityList({ items, compact = false }: { items: Activity[]; compact?: boolean }) {
    return <div className={`activity-list ${compact ? "compact" : ""}`}>{items.map((item) => <article className="log-row" key={item.id}><span className="meta">{item.time}</span><div><h3>{item.title}</h3><p>{item.body}</p></div></article>)}</div>;
  }

  function closeDetail() { setDetail(null); }

  function exportNotes() { downloadJson("aidebook-notes.json", notes); setToast("메모를 내보냈습니다."); }
  function exportDiagnostics() { downloadJson("aidebook-diagnostics.json", { type: "local-ui", version: status?.version ?? "0.1.0", externalConnections: false, exportedAt: new Date().toISOString() }); setToast("진단 예시를 내보냈습니다."); }
  function downloadJson(filename: string, value: unknown) { const blob = new Blob([JSON.stringify(value, null, 2)], { type: "application/json" }); const url = URL.createObjectURL(blob); const anchor = document.createElement("a"); anchor.href = url; anchor.download = filename; anchor.click(); window.setTimeout(() => URL.revokeObjectURL(url), 1000); }

  return <div className={`shell ${sidebarHidden ? "sidebar-hidden" : ""} ${sidebarMobileOpen ? "sidebar-mobile-open" : ""} ${inspectorHidden ? "inspector-hidden" : ""}`}><aside className="sidebar" aria-label="주요 메뉴"><div className="brand"><Icon name="brand" /><span>Aidebook</span></div><nav className="main-nav" aria-label="주요 메뉴"><NavButton active={page === "workflow"} icon="work" onClick={() => navigate("workflow")}>업무와 할 일</NavButton><NavButton active={page === "home"} icon="work" onClick={() => navigate("home")}>최근 맥락</NavButton><NavButton active={page === "notes"} icon="notes" onClick={() => navigate("notes")}>모든 메모<span className="count">{notes.filter((note) => !note.retracted).length}</span></NavButton><NavButton active={page === "activity"} icon="activity" onClick={() => navigate("activity")}>활동 기록</NavButton><NavButton active={page === "graph"} icon="graph" onClick={() => navigate("graph")}>그래프<span className="count">{activeWorkNotes.length}</span></NavButton></nav><div className="works"><p className="navlabel">작업 묶음</p><div>{works.map((name, index) => <button className={`navbtn work ${page === "work" && work === index ? "active" : ""}`} type="button" key={name} aria-current={page === "work" && work === index ? "page" : undefined} onClick={() => chooseWork(index)}><span className="dot" /><span>{name}</span><span className="count">{notes.filter((note) => note.work === index && !note.retracted).length}</span></button>)}</div></div><div className="sidebar-bottom"><button className={`navbtn ${page === "connections" ? "active" : ""}`} type="button" onClick={() => navigate("connections")}><Icon name="link" />연결 상태</button><button className={`navbtn ${page === "settings" ? "active" : ""}`} type="button" onClick={() => navigate("settings")}><Icon name="settings" />설정<span className="count">⌘ ,</span></button><div className="profile"><span className="avatar">나</span><div>개인 작업 공간<div className="small muted">이 기기에 보관</div></div></div></div></aside><main className="main" id="content"><header className="topbar"><button className="tool" type="button" aria-label="사이드바 접기 또는 펼치기" aria-expanded={!sidebarHidden} onClick={() => { if (window.innerWidth <= 760) setSidebarMobileOpen((open) => !open); else setSidebarHidden((hidden) => !hidden); }}><Icon name="panel-left" /></button><div className="crumb">{pageTitle()}</div><button className="search-launch" type="button" onClick={() => navigate("search")}><Icon name="search" />자료와 메모 검색 <kbd>⌘ K</kbd></button><span className="tag">{page === "workflow" ? "로컬 업무" : "예시 데이터"}</span><button className="tool inspector-toggle" type="button" aria-label="메모 근거 패널" aria-expanded={!inspectorHidden} onClick={() => setInspectorHidden((hidden) => !hidden)}><Icon name="panel-right" /></button></header><div id="view" tabIndex={-1}>{renderPage()}</div></main>{editor && <EditorModal />}{detail && <DetailModal />}{toast && <div className="toast" role="status">{toast}{undoStack.length > 0 && <button type="button" onClick={undoChange}>되돌리기</button>}</div>}</div>;

  function NavButton({ active, icon, onClick, children }: { active: boolean; icon: "work" | "notes" | "activity" | "graph"; onClick: () => void; children: ReactNode }) { return <button className={`navbtn ${active ? "active" : ""}`} type="button" aria-current={active ? "page" : undefined} onClick={onClick}><Icon name={icon} />{children}</button>; }

  function EditorModal() {
    if (!editor) return null;
    return <Modal title={editor.id === null ? "새 메모" : "메모 수정"} onClose={() => { if (!savingEditor) setEditor(null); }}><form onSubmit={saveEditor}><label className="field">제목<input ref={editorTitleRef} className="input" maxLength={100} required value={editor.title} onChange={(event) => setEditor({ ...editor, title: event.target.value })} /></label><label className="field">내용<textarea className="textarea" required value={editor.body} onChange={(event) => setEditor({ ...editor, body: event.target.value })} /></label><label className="field">저장 이유<textarea className="textarea" required value={editor.reason} onChange={(event) => setEditor({ ...editor, reason: event.target.value })} /></label><label className="field">메모 유형<select className="input" value={editor.kind} onChange={(event) => setEditor({ ...editor, kind: event.target.value as NoteKind })}>{noteKinds.map((kind) => <option key={kind}>{kind}</option>)}</select></label><fieldset className="evidence-picker"><legend>연결할 근거</legend>{sources.map((source) => <label key={source.id} className="check-row"><input type="checkbox" checked={editor.sources.includes(source.id)} onChange={(event) => setEditor({ ...editor, sources: event.target.checked ? [...editor.sources, source.id] : editor.sources.filter((id) => id !== source.id) })} />{source.provider} · {source.title}</label>)}</fieldset><p className="small muted">저장 시 선택한 근거·저장 이유·작성 주체·주장 유형을 코어 계약으로 전달합니다. 직접 작성한 메모는 동기화로 덮어쓰지 않습니다.</p><div className="actions"><button className="btn btn-secondary" type="button" disabled={savingEditor} onClick={() => setEditor(null)}>취소</button><button className="btn btn-primary" type="submit" disabled={savingEditor}>{savingEditor ? "저장 중…" : "메모 저장"}</button></div></form></Modal>;
  }

  function DetailModal() {
    if (!detail) return null;
    if (detail.type === "source") return <Modal title={`${detail.source.provider} 원문`} onClose={closeDetail}><div className="source-detail"><div className="source-head"><span className={`provider provider-${detail.source.provider.toLowerCase()}`}>{detail.source.provider[0]}</span>{detail.source.provider}<span className="tag">읽기 전용</span></div><h3>{detail.source.title}</h3><p className="small muted">{detail.source.ref} · 마지막 확인 {detail.source.time}</p><p>{detail.source.body}</p><div className="notice">원본 보기와 수집 범위는 외부 계정을 변경하지 않습니다.</div></div></Modal>;
    if (detail.type === "history") return <Modal title="메모 변경 이력" onClose={closeDetail}><p>현재 버전 {detail.note.version ?? 1} · {detail.note.author}</p><p>{detail.note.reason}</p>{detail.note.previous ? <><hr className="rule" /><p className="small muted">이전 버전</p><h3>{detail.note.previous.title}</h3><p>{detail.note.previous.body}</p><button className="btn btn-secondary" type="button" onClick={() => restoreNote(detail.note)}>이 버전으로 복원</button></> : <p className="muted">이 메모의 이전 수정 기록이 없습니다.</p>}</Modal>;
    const source = detail.source;
    return <Modal title={`${source.provider} 수집 범위`} onClose={closeDetail}><p>읽기 권한으로 선택한 자료만 가져옵니다.</p><label className="field">허용 범위<input className="input" value={scopes[`scope-${source.id}`] ?? (source.provider === "GitHub" ? "aidebook 저장소" : "업무/Aidebook 폴더")} onChange={(event) => setScopes((current) => ({ ...current, [`scope-${source.id}`]: event.target.value }))} /></label><label className="field">제외 규칙<input className="input" defaultValue="개인, 비공개" /></label><button className="btn btn-secondary" type="button" onClick={() => { closeDetail(); setToast("예시 수집 범위를 저장했습니다."); }}>범위 저장</button><p className="small muted">연결 해제와 수집 데이터 삭제는 별도로 처리합니다.</p></Modal>;
  }

  function Modal({ title, onClose, children }: { title: string; onClose: () => void; children: ReactNode }) { return <div className="modal-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}><section className="modal" role="dialog" aria-modal="true" aria-labelledby="modal-title"><div className="modal-heading"><h2 id="modal-title">{title}</h2><button className="tool" type="button" aria-label="닫기" onClick={onClose}><Icon name="close" /></button></div>{children}</section></div>; }
}

export default App;
