// The viewer, in the drafting app's frame (#150, the look Zach picked): a
// title bar with Undo last and Accept, a ribbon with the drawing tools
// (#152) and the Layer box, a tab for each of the drawing set's sheets, the
// drawing as dark model space following the working revision live, the
// command bar under it, a status bar, and the assistant's panel on the
// right, its Chat beside the Journal (#156). On a phone the ribbon folds
// behind Tools, the status bar goes, and the panel is a sheet that peeks up
// from the bottom with the way into the chat, Undo last and Accept. A toast names each change set as it
// lands and its objects are marked; the journal steps back through the set
// as it was; a tap selects an object, whose quick properties open beside
// it. A drawing tool runs in the renderer: the page passes it the clicks,
// taps and typing, shows its prompt, and sends the change set it gives
// through `change`, as an agent would, so it lands in the journal, where
// Undo and Accept take it; the sheet is asked for again as soon as `change`
// replies. An edit to a template's object is drawn first, before it lands,
// on each sheet it reaches (#154), and lands only on Change all. Words
// typed in the command bar that are no tool's go to the assistant, which
// is the drafter's own agent, started by `serve`; the page tells the core
// what the drafter is looking at, with `look-at`, for it to read.
//
// The page learns of every change from the server's event stream, which
// sends the journal each time it changes on disk, whichever process or
// interface changed it, and draws again the sheets the new entries touched.

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';

import { assistantCall, call, CommandError, run, watch, watchAssistant, type StreamState } from './api.ts';
import { ArrayPanel } from './ArrayPanel.tsx';
import { AssistantPanel, type ChatCardView, type Side } from './Assistant.tsx';
import {
  agentName,
  askHint,
  lookingAtOf,
  lookKey,
  notSentWords,
  peekAskWords,
  spaceIsEnter,
  typedRoute,
  unavailableAgents,
  chatCardActs,
  type AgentsReport,
  type Conversation,
} from './assistant.ts';
import { BottomSheet } from './BottomSheet.tsx';
import { Drawing, type DrawingHandle, type Focus, type GhostSpec, type Mark, type PreviewTag } from './Drawing.tsx';
import {
  changedOn,
  clock,
  DRAFTER,
  entryByline,
  entryTitle,
  firstSheet,
  journalReplaced,
  recordedEntries,
  lastUndoable,
  newest,
  notIssued,
  sheetsTouched,
  since,
  staleDrawings,
  viaWords,
  whoWords,
  type Entry,
  type JournalReport,
} from './journal.ts';
import {
  colorIndex,
  coordinates,
  currentLayerOf,
  droppedOnLoad,
  layersWhileAsked,
  DEFAULT_RUNNING,
  originWords,
  readCurrentLayer,
  writeCurrentLayer,
  type LayerEntry,
  type ToolOutcome,
  type ToolSend,
  type ToolState,
  liveLine,
  paperWords,
  promptFor,
  readSnapSettings,
  runningMask,
  snapLabel,
  snappedPoint,
  takenOf,
  toggleFor,
  barNote,
  barWarning,
  dismissToastOnToolStart,
  switchingAway,
  toastDuration,
  canToolUndo,
  isUndoKey,
  rightClickText,
  toggledWords,
  typeWords,
  nameOf,
  writeSnapSettings,
  type Act,
  type Hovered,
  type SnapKind,
  type Taken,
  type Tool,
  type Toggle,
  editsHere,
  previewStep,
  previewTitle,
  onPreviewDrawing,
  journalSnapshotValid,
  previewIsCurrent,
  previewRedrawnOn,
  reportGeneration,
  drawnFor,
  layoutOn,
  PANELS,
  previewVerdicts,
  type PreviewReport,
  type TemplateEdit,
  type Committed,
  type PropEdited,
  type PropField,
} from './frame.ts';
import { CommandBar, PreviewBand, Ribbon, SnapMenu, StatusBar, TextBox, TextEditBox, TextOnDrawing, VerdictMark } from './Frame.tsx';
import { is_tool_word, screen_color } from './wasm/agentdrafter_viewer.js';
import { JournalList, Peek, QuickProperties } from './Panels.tsx';
import { ProposalCard } from './Proposals.tsx';
import { GuideBanner, GuideCard } from './Guide.tsx';
import {
  clickNote,
  endedNote,
  layerChosen,
  endsStepTool,
  givenToast,
  landed as guideLanded,
  previewConfirmSend,
  pulseOf,
  toastAfterStart,
  marked as guideMarked,
  synced as guideSynced,
  undoAllBlocked,
  undoItBlocked,
  skipStep,
  startRun,
  stepDone,
  stopRun,
  toolNote,
  type Guide,
  type GuideChecked,
  type GuideRun,
  type GuidesReport,
  type Phase,
  type RendererNote,
} from './guides.ts';
import {
  cardLines,
  onSheet,
  peekWords,
  proposalTitle,
  proposedWords,
  staleWords,
  standing,
  tabCounts,
  type CardLine,
  type Proposal,
  type ProposalsReport,
  type Standing,
} from './proposals.ts';
import {
  allowLossKey,
  beginDwgRead,
  buildDesktopSearch,
  canUndoDrawing,
  classifySource,
  completeDwgRead,
  createDwgReadTracker,
  dwgEditGuard,
  getDesktopBridge,
  kindBadge,
  latestActiveRevision,
  loadLocalRecents,
  matchOriginalObjectId,
  mergeRecent,
  newestActiveOriginalRevision,
  normalizeOriginalChangeSet,
  noteJournalOriginalSha,
  removeRecentLocal,
  resetDwgReadJournalState,
  resolveBeside,
  revisionAtEntry,
  saveLocalRecents,
  sourceDisplayName,
  type ReadReport,
  type RecentEntry,
  type SourceKind,
} from './desktop.ts';
import { DecodeReportModal, DwgBanner, StartScreen, SwitcherModal } from './DesktopShell.tsx';
import type { Summary } from './probe.ts';

export interface AppProps {
  /** The token for this tab, if the server needs one. */
  token: string | null;
  /** The drawing set, sheet description or DWG to show, as the server names it. */
  source: string;
  /** Explicit validated source kind (`set` | `dwg` | `sheet`) from picker/URL, if known. */
  kind?: SourceKind | null;
  /** The sheet to show first; the set's first without one. */
  sheet: string | null;
  /** The layout of that sheet to show first, when it is drawn in several; its first without one. */
  layout: string | null;
  /** Whether the desktop shell chrome (Start screen, Open…, switcher) is active (#158). */
  desktop?: boolean;
  /** Whether to show the Start screen instead of opening `source` immediately. */
  start?: boolean;
}

type Status =
  | { kind: 'loading'; message: string }
  | { kind: 'shown' }
  | { kind: 'failed'; message: string }
  | { kind: 'cannot'; message: string };

type Tone = 'ok' | 'bad' | 'warn' | 'info' | 'prop';

interface Toast {
  key: number;
  tone: Tone;
  title: string;
  detail: string;
  /** The entry its Show button shows. */
  entry?: number;
  /** The proposal its Show button opens. */
  proposal?: number;
  /** The guides its Start buttons start, one each (#157). */
  guides?: number[];
}

interface Finding {
  sheet: string;
  rule: string;
  message: string;
  object?: { id: string };
}

interface AcceptReport {
  issued: number[];
  sheets: string[];
  follows?: number;
  checks: { findings: Finding[] };
}

/** A `display-list` reply, with the request it answered and the generation it was asked in. */
interface Drawn {
  text: string;
  /** The entry and the sheet asked for, as the cache keys them. */
  at: number | null;
  sheetKey: string;
  generation: string;
  /** Whether it came with its object snaps. */
  snaps: boolean;
  /** Whether it is a preview's: the sheet as a change set would leave it, never kept as the set as it is. */
  preview: boolean;
  /** The preview it was asked for, by its key: a proposal opened is shown only from its own drawing. */
  pvKey: number | null;
}

/** A change set to a template's objects, drawn before it lands (#154). */
interface Previewing {
  send: ToolSend & { template: TemplateEdit };
  /** The sheet it was made on, and its layout, which Cancel and Change all go back to. */
  home: string;
  homeLayout: string | null;
  /** Moves on to ask for its drawings again, as Check again does. */
  key: number;
  /** The object selected when it was made, as from quick properties: selected again once it ends. */
  picked: Picked | null;
  /** A proposal opened (#155): Keep sends it with `keep`, and closing it leaves it waiting. */
  proposal?: Proposal;
  /** The guide run this template preview belongs to (#264), so Change all ticks its step. */
  guideRunId?: number;
}

/** A waiting proposal drawn on a sheet as it would leave it (#155): the reply's text, its preview, or why it could not be drawn. */
interface GhostReply {
  text: string;
  report: PreviewReport | null;
  error: string | null;
}

/** The object the reviewer picked: by its ID, so it is followed as the sheet changes; by its place when it has none. */
interface Picked {
  id: string | null;
  place: number;
  /** The drawing it was picked on, which an object with no ID is known in alone. */
  summary: Summary;
}

/** What `change` replies, as the page reads it (drawing set or original DWG revision). */
interface ChangeReply {
  journal_entry?: number;
  sheets?: { sheet: string; ok: boolean }[];
  sheet?: string;
  revision_dwg?: string;
  revision?: { file: string; sha256: string; dwg_version: string };
  message?: string;
  /** For a change set sent for a guide's step, the core's check of it against the step (#157). */
  guide?: GuideChecked;
}

interface Asking {
  report: AcceptReport;
  /** The newest entry the reviewer has seen, which the accept follows. */
  newestSeen: number;
}

const WIDE = '(min-width: 900px)';

/** The command bar's height with a prompt of one line, in CSS pixels, which the fitted sheet stays above. */
const BAR_LINE = 32;

/** How many lines of what happened the command bar keeps; a laptop shows the last two over the drawing. */
const HISTORY = 40;
const upper = (s: string) => s.toUpperCase();

/** The version that counts every change that touched a sheet, for a sheet not yet named. */
const ANY = '*';


function useWide(): boolean {
  const [wide, setWide] = useState(() => window.matchMedia(WIDE).matches);
  useEffect(() => {
    const media = window.matchMedia(WIDE);
    const changed = () => setWide(media.matches);
    media.addEventListener('change', changed);
    return () => media.removeEventListener('change', changed);
  }, []);
  return wide;
}

/** What to say when the server refused or failed. */
function why(e: unknown, token: string | null): string {
  if (e instanceof CommandError && e.status === 401) {
    return token
      ? 'The server did not take this tab’s token: it makes a new one each time it starts. Open the viewer again from the address agentdrafter serve printed when it started.'
      : 'This server listens beyond the local machine, so the viewer needs the token it printed when it started. Open the viewer from the address agentdrafter serve printed, which ends in #token=…';
  }
  return e instanceof Error ? e.message : String(e);
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
/** Who made a proposal, to start a toast with: `Assistant`, `Agent (MCP)`. */
const capitalWho = (by: string): string => {
  const who = whoWords(by);
  return who.charAt(0).toUpperCase() + who.slice(1);
};

const listed = (items: (string | number)[]) =>
  items.length <= 1 ? items.join('') : `${items.slice(0, -1).join(', ')} and ${items.at(-1)}`;

/** Whether a word typed in the command bar is a drawing tool's alias or name: the renderer's to say. */
const toolWord = (word: string): boolean => {
  try {
    return is_tool_word(word);
  } catch {
    return false;
  }
};

/** A tool's name on the ribbon, for the command bar's hint: `Move` for MOVE. */
const toolName = (word: string): string | null =>
  PANELS.flatMap((p) => p.tools).find((t) => [t.label, t.command, t.alias, t.key].some((x) => upper(x) === upper(word)))?.label ?? null;

interface WorkspaceProps {
  token: string | null;
  source: string;
  sourceKind: SourceKind;
  sheet: string | null;
  layout: string | null;
  desktop: boolean;
  isLossAllowed(sha256: string | null): boolean;
  onAllowLoss(sha256: string): void;
  onRevokeLoss(sha256: string): void;
  onOpenSwitcher(): void;
}

/**
 * Top-level App shell: in plain `agentdrafter serve` browser mode it renders
 * the drawing workspace immediately with zero extra chrome; in the Tauri
 * desktop shell (or `?desktop=1` / `?start=1`) it adds the Start screen,
 * native OS folder/file pickers, Recents, title-bar switcher, multi-window
 * launcher, and content-bound per-session ADR-0004 allow-loss scope.
 */
export function App({ token, source, kind = null, sheet, layout, desktop = false, start = false }: AppProps) {
  const bridge = useMemo(() => getDesktopBridge(), []);
  const isDesktop = Boolean(desktop || start || bridge);
  const storage = useMemo(() => {
    try {
      return window.localStorage;
    } catch {
      return null;
    }
  }, []);

  const [onStart, setOnStart] = useState(Boolean(isDesktop && start));
  const [activeSource, setActiveSource] = useState(source);
  const [activeKind, setActiveKind] = useState<SourceKind | null>(kind);
  const [initialSheet, setInitialSheet] = useState<string | null>(sheet);
  const [initialLayout, setInitialLayout] = useState<string | null>(layout);
  const [recents, setRecents] = useState<RecentEntry[]>(() => (bridge ? [] : loadLocalRecents(storage)));
  const [switcherOpen, setSwitcherOpen] = useState(false);
  const [desktopError, setDesktopError] = useState<string | null>(null);
  const [desktopBusy, setDesktopBusy] = useState(false);
  const [allowLossKeys, setAllowLossKeys] = useState<ReadonlySet<string>>(() => new Set());

  const syncTitle = useCallback(
    (name: string | null) => {
      document.title = name ? `AgentDrafter — ${name}` : 'AgentDrafter';
      if (bridge && name) {
        void bridge.setWindowTitle(name).catch(() => {});
      }
    },
    [bridge],
  );

  const recordOpened = useCallback(
    async (path: string, k: SourceKind, name: string, subtitle?: string) => {
      if (bridge) {
        try {
          const updated = await bridge.recordRecent({ path, kind: k, name, subtitle });
          setRecents(updated);
          return;
        } catch {
          // Fall back to local state if bridge fails.
        }
      }
      setRecents((prev) => {
        const next = mergeRecent(prev, { path, kind: k, name, opened_at_ms: Date.now(), subtitle });
        saveLocalRecents(storage, next);
        return next;
      });
    },
    [bridge, storage],
  );

  const removeRecent = useCallback(
    async (path: string) => {
      if (bridge) {
        try {
          const updated = await bridge.removeRecent(path);
          setRecents(updated);
          return;
        } catch {
          // Fall back to local removal.
        }
      }
      setRecents((prev) => {
        const next = removeRecentLocal(prev, path);
        saveLocalRecents(storage, next);
        return next;
      });
    },
    [bridge, storage],
  );

  // Load persisted recents when desktop mode starts.
  useEffect(() => {
    if (!isDesktop || !bridge) return;
    let current = true;
    bridge
      .listRecents()
      .then((list) => current && setRecents(list))
      .catch(() => {});
    return () => {
      current = false;
    };
  }, [isDesktop, bridge]);

  // If opened directly via URL without `?kind=`, ask the native shell to inspect
  // the path so folders named `project.yaml` holding `set.yaml` are never misclassified.
  useEffect(() => {
    if (!isDesktop || onStart || activeKind || !bridge || !activeSource || activeSource === '.') return;
    let current = true;
    bridge
      .inspectPath(activeSource)
      .then((info) => {
        if (!current) return;
        setActiveKind(info.kind);
      })
      .catch(() => {});
    return () => {
      current = false;
    };
  }, [isDesktop, onStart, activeKind, bridge, activeSource]);

  const resolvedKind = classifySource(activeSource, activeKind);

  // Keep native window title and recents in sync when opening directly via URL/CLI arg.
  useEffect(() => {
    if (!isDesktop) return;
    if (onStart) {
      syncTitle('Start');
      return;
    }
    const name = sourceDisplayName(activeSource);
    syncTitle(name);
    if (activeSource && activeSource !== '.') {
      void recordOpened(activeSource, resolvedKind, name);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isDesktop, onStart, activeSource, resolvedKind]);

  const openSource = useCallback(
    (path: string, k?: SourceKind, name?: string, subtitle?: string) => {
      const resolved = classifySource(path, k);
      const n = name ?? sourceDisplayName(path);
      setActiveSource(path);
      setActiveKind(resolved);
      setInitialSheet(null);
      setInitialLayout(null);
      setOnStart(false);
      setSwitcherOpen(false);
      setDesktopError(null);
      const qs = buildDesktopSearch({ desktop: true, start: false, source: path, kind: resolved, sheet: null, layout: null });
      window.history.replaceState(null, '', `${window.location.pathname}${qs ? `?${qs}` : ''}`);
      syncTitle(n);
      void recordOpened(path, resolved, n, subtitle);
    },
    [recordOpened, syncTitle],
  );

  const goToStart = useCallback(() => {
    setOnStart(true);
    setSwitcherOpen(false);
    setDesktopError(null);
    const qs = buildDesktopSearch({ desktop: true, start: true, source: null, kind: null, sheet: null, layout: null });
    window.history.replaceState(null, '', `${window.location.pathname}${qs ? `?${qs}` : ''}`);
    syncTitle('Start');
  }, [syncTitle]);

  const openInNewWindow = useCallback(
    async (path: string | null, title?: string, k?: SourceKind | null) => {
      setSwitcherOpen(false);
      setDesktopError(null);
      if (bridge) {
        try {
          await bridge.openInNewWindow(path, title, k ?? null);
          return;
        } catch (e) {
          setDesktopError(e instanceof Error ? e.message : String(e));
          return;
        }
      }
      const qs = buildDesktopSearch({
        desktop: true,
        start: !path,
        source: path,
        kind: k ?? null,
        sheet: null,
        layout: null,
      });
      const url = `${window.location.pathname}${qs ? `?${qs}` : ''}${token ? `#token=${token}` : ''}`;
      window.open(url, '_blank', 'noopener');
    },
    [bridge, token],
  );

  const pickSet = useCallback(
    async (newWindow = false) => {
      if (!bridge) {
        setDesktopError('Native OS folder picker is available in the AgentDrafter desktop app.');
        return;
      }
      setDesktopBusy(true);
      setDesktopError(null);
      try {
        const picked = await bridge.pickDrawingSetFolder();
        if (!picked) return;
        if (newWindow) await openInNewWindow(picked.path, picked.name, picked.kind);
        else openSource(picked.path, picked.kind, picked.name, picked.detail);
      } catch (e) {
        setDesktopError(e instanceof Error ? e.message : String(e));
      } finally {
        setDesktopBusy(false);
      }
    },
    [bridge, openInNewWindow, openSource],
  );

  const pickFile = useCallback(
    async (filter?: 'dwg' | 'yaml', newWindow = false) => {
      if (!bridge) {
        setDesktopError('Native OS file picker is available in the AgentDrafter desktop app.');
        return;
      }
      setDesktopBusy(true);
      setDesktopError(null);
      try {
        const picked = await bridge.pickDrawingFile(filter);
        if (!picked) return;
        if (newWindow) await openInNewWindow(picked.path, picked.name, picked.kind);
        else openSource(picked.path, picked.kind, picked.name, picked.detail);
      } catch (e) {
        setDesktopError(e instanceof Error ? e.message : String(e));
      } finally {
        setDesktopBusy(false);
      }
    },
    [bridge, openInNewWindow, openSource],
  );

  // Desktop keyboard shortcuts: Ctrl+O (Open / Switcher), Ctrl+Shift+O (New window switcher), Esc (close switcher).
  useEffect(() => {
    if (!isDesktop) return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'o') {
        e.preventDefault();
        e.stopPropagation();
        if (e.shiftKey) {
          setDesktopError(null);
          setSwitcherOpen(true);
        } else if (onStart) {
          void pickSet(false);
        } else {
          setDesktopError(null);
          setSwitcherOpen(true);
        }
      } else if (e.key === 'Escape' && switcherOpen) {
        e.preventDefault();
        e.stopPropagation();
        setSwitcherOpen(false);
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [isDesktop, onStart, pickSet, switcherOpen]);

  const isLossAllowed = useCallback(
    (sha256: string | null): boolean => {
      const k = allowLossKey(activeSource, sha256);
      return k !== null && allowLossKeys.has(k);
    },
    [activeSource, allowLossKeys],
  );

  const grantLoss = useCallback(
    (sha256: string) => {
      const k = allowLossKey(activeSource, sha256);
      if (!k) return;
      setAllowLossKeys((prev) => {
        const next = new Set(prev);
        next.add(k);
        return next;
      });
    },
    [activeSource],
  );

  const revokeLoss = useCallback(
    (sha256: string) => {
      const k = allowLossKey(activeSource, sha256);
      if (!k) return;
      setAllowLossKeys((prev) => {
        if (!prev.has(k)) return prev;
        const next = new Set(prev);
        next.delete(k);
        return next;
      });
    },
    [activeSource],
  );

  if (isDesktop && onStart) {
    return (
      <>
        <StartScreen
          recents={recents}
          error={desktopError}
          busy={desktopBusy}
          onDismissError={() => setDesktopError(null)}
          onPickSet={() => void pickSet(false)}
          onPickDwg={() => void pickFile('dwg', false)}
          onPickSheet={() => void pickFile('yaml', false)}
          onOpenRecent={(entry, newWin) => {
            if (newWin) void openInNewWindow(entry.path, entry.name, entry.kind);
            else openSource(entry.path, entry.kind, entry.name, entry.subtitle);
          }}
          onRemoveRecent={(path) => void removeRecent(path)}
        />
        {switcherOpen && (
          <SwitcherModal
            currentPath=""
            recents={recents}
            error={desktopError}
            busy={desktopBusy}
            onClose={() => setSwitcherOpen(false)}
            onGoStart={() => setSwitcherOpen(false)}
            onPickSet={(newWin) => void pickSet(newWin)}
            onPickFile={(filter, newWin) => void pickFile(filter, newWin)}
            onOpenRecent={(entry, newWin) => {
              if (newWin) void openInNewWindow(entry.path, entry.name, entry.kind);
              else openSource(entry.path, entry.kind, entry.name, entry.subtitle);
            }}
            onRemoveRecent={(path) => void removeRecent(path)}
          />
        )}
      </>
    );
  }

  return (
    <>
      <DrawingWorkspace
        key={`${activeSource}|${resolvedKind}`}
        token={token}
        source={activeSource}
        sourceKind={resolvedKind}
        sheet={initialSheet}
        layout={initialLayout}
        desktop={isDesktop}
        isLossAllowed={isLossAllowed}
        onAllowLoss={grantLoss}
        onRevokeLoss={revokeLoss}
        onOpenSwitcher={() => {
          setDesktopError(null);
          setSwitcherOpen(true);
        }}
      />
      {isDesktop && switcherOpen && (
        <SwitcherModal
          currentPath={activeSource}
          recents={recents}
          error={desktopError}
          busy={desktopBusy}
          onClose={() => setSwitcherOpen(false)}
          onGoStart={goToStart}
          onPickSet={(newWin) => void pickSet(newWin)}
          onPickFile={(filter, newWin) => void pickFile(filter, newWin)}
          onOpenRecent={(entry, newWin) => {
            if (newWin) void openInNewWindow(entry.path, entry.name, entry.kind);
            else openSource(entry.path, entry.kind, entry.name, entry.subtitle);
          }}
          onRemoveRecent={(path) => void removeRecent(path)}
        />
      )}
    </>
  );
}

function DrawingWorkspace({
  token,
  source,
  sourceKind,
  sheet,
  layout,
  desktop,
  isLossAllowed,
  onAllowLoss,
  onRevokeLoss,
  onOpenSwitcher,
}: WorkspaceProps) {
  // Desktop-only plant DWG & single-sheet workflows are gated strictly on `desktop`
  // so plain `agentdrafter serve` browser access keeps its exact pre-desktop behavior.
  const isDwg = desktop && sourceKind === 'dwg';
  const isSheet = desktop && sourceKind === 'sheet';
  const [readReport, setReadReport] = useState<ReadReport | null>(null);
  const [readPending, setReadPending] = useState(isDwg);
  const [readError, setReadError] = useState<string | null>(null);
  const [decodeModalOpen, setDecodeModalOpen] = useState(false);
  const [plotting, setPlotting] = useState(false);
  const [ownRev, setOwnRev] = useState<{ path: string; entry: number; sha256: string; originalSha256: string } | null>(null);
  const dwgReadTrackerRef = useRef(createDwgReadTracker());
  const workspaceActiveRef = useRef(true);
  const revokeLossRef = useRef(onRevokeLoss);
  revokeLossRef.current = onRevokeLoss;

  const triggerDwgRead = useCallback(
    (markPending: boolean) => {
      const tracker = dwgReadTrackerRef.current;
      const { gen, startedForEntry } = beginDwgRead(tracker);
      if (markPending) {
        setReadPending(true);
        setReadError(null);
      }
      void call<ReadReport>('read', { source, no_mirror: true }, token).then(
        (fresh) => {
          if (!workspaceActiveRef.current) return;
          const done = completeDwgRead(tracker, gen, startedForEntry, fresh.sha256);
          if (!done.accept) return;
          if (done.changed && done.prevSha) {
            revokeLossRef.current(done.prevSha);
            setOwnRev(null);
          }
          setReadError(null);
          setReadReport((prev) =>
            prev &&
            prev.sha256 === fresh.sha256 &&
            prev.decoded === fresh.decoded &&
            prev.warnings === fresh.warnings &&
            prev.read_past_damage === fresh.read_past_damage
              ? prev
              : fresh,
          );
          setReadPending(false);
          if (done.followUp) triggerDwgRead(false);
        },
        (e) => {
          if (!workspaceActiveRef.current) return;
          const done = completeDwgRead(tracker, gen, startedForEntry, null);
          if (!done.accept) return;
          if (done.prevSha) revokeLossRef.current(done.prevSha);
          setOwnRev(null);
          setReadReport(null);
          setReadError(why(e, token));
          setReadPending(false);
          if (done.followUp) triggerDwgRead(false);
        },
      );
    },
    [source, token],
  );

  useEffect(() => {
    workspaceActiveRef.current = true;
    const tracker = dwgReadTrackerRef.current;
    if (!isDwg) {
      tracker.gen += 1;
      tracker.inFlight = false;
      tracker.lastReadSha = null;
      resetDwgReadJournalState(tracker);
      setReadReport(null);
      setReadPending(false);
      setReadError(null);
      return;
    }
    triggerDwgRead(true);
    return () => {
      workspaceActiveRef.current = false;
      tracker.gen += 1;
    };
  }, [isDwg, triggerDwgRead]);

  const allowLoss = isDwg && isLossAllowed(readReport?.sha256 ?? null);
  const editGuard = desktop ? dwgEditGuard(sourceKind, readReport, readPending, allowLoss) : 'editable';
  const dwgLocked = isDwg && (editGuard === 'view-only' || editGuard === 'loading');
  const undoLocked = !canUndoDrawing(sourceKind, desktop, editGuard);
  const editingDisabled = dwgLocked || isSheet;
  const wide = useWide();
  const [wanted, setWanted] = useState<string | null>(sheet);
  // The layout asked for, of a sheet drawn in several; its first without one.
  const [wantedLayout, setWantedLayout] = useState<string | null>(layout);
  const [at, setAt] = useState<number | null>(null);
  const [reply, setReply] = useState<Drawn | null>(null);
  const [summary, setSummary] = useState<Summary | null>(null);
  const [status, setStatus] = useState<Status>({ kind: 'loading', message: 'Drawing the sheet…' });
  const [covered, setCovered] = useState(0);
  const [unavailable, setUnavailable] = useState<string | null>(null);
  const [journal, setJournal] = useState<JournalReport | null>(null);
  const [noJournal, setNoJournal] = useState<string | null>(null);
  const [stream, setStream] = useState<StreamState>('reconnecting');
  const [focused, setFocused] = useState<number | null>(null);
  const [pulseSince, setPulseSince] = useState<number | null>(null);
  const [focus, setFocus] = useState<Focus | null>(null);
  const [focusWanted, setFocusWanted] = useState<{ key: number; sheet: string; ids: string[] } | null>(null);
  const [picked, setPicked] = useState<Picked | null>(null);
  const [pickedColor, setPickedColor] = useState('');
  const [raise, setRaise] = useState(0);
  // The frame's own: the status bar's toggles, a phone's folded ribbon, a
  // tool still to come just tapped, the side panel's tab, and what happened.
  // Object snap (F3) and the running snaps are remembered in this browser
  // (localStorage), as AutoCAD keeps OSMODE; Grid and Ortho start as the
  // frame starts them.
  const keeps = useMemo(() => {
    try {
      return window.localStorage;
    } catch {
      return null;
    }
  }, []);
  const [snapsKept] = useState(() => readSnapSettings(keeps));
  const [toggles, setToggles] = useState<Record<Toggle, boolean>>({ grid: true, osnap: snapsKept.on, ortho: false });
  const [running, setRunning] = useState<ReadonlySet<SnapKind>>(() => new Set(snapsKept.running));
  useEffect(() => writeSnapSettings(keeps, { on: toggles.osnap, running: [...running] }), [keeps, toggles.osnap, running]);
  // The Object snap menu: above the status bar's Object snap on a laptop
  // (its place), a card on a phone ('card'), or shut.
  const [snapMenu, setSnapMenu] = useState<{ left: number; bottom: number } | 'card' | null>(null);
  // A snap taken with no tool running: its point read out, its snapped
  // point, the sheet and layout drawn it was taken on, and every object it
  // names, by its ID: its own, and an intersection's other.
  const [taken, setTaken] = useState<{
    taken: Taken;
    point: { snap: Record<string, unknown> } | null;
    summary: Summary;
    ids: (string | null)[];
  } | null>(null);
  // The read-out goes when what it names does: another sheet or layout
  // drawn, or an object it names gone from this one. An object with no ID
  // is known only by its place, which any new drawing may give another.
  useEffect(() => {
    setTaken((t) => {
      if (!t) return t;
      // Another sheet or layout asked for goes at once, before it is drawn.
      const was = t.summary;
      if (wanted !== null && upper(wanted) !== upper(was.sheet)) return null;
      if (wantedLayout !== null && upper(wantedLayout) !== upper(was.layout ?? '')) return null;
      if (was === summary) return t;
      if (!summary || upper(summary.sheet) !== upper(was.sheet) || upper(summary.layout ?? '') !== upper(was.layout ?? '')) return null;
      return t.ids.every((id) => id !== null && summary.objects.some((o) => o.id === id)) ? t : null;
    });
  }, [summary, wanted, wantedLayout]);
  const [toolsOpen, setToolsOpen] = useState(false);
  // The drawing tool running, as the renderer says it, and what reaches it.
  const [toolState, setToolState] = useState<ToolState | null>(null);
  const drawing = useRef<DrawingHandle | null>(null);
  const field = useRef<HTMLInputElement>(null);
  // A phone's field shows when ⌨ Type asks for it.
  const [fieldShown, setFieldShown] = useState(false);
  // An edit to a template's object, drawn before it lands, waiting for the
  // drafter's Change all or Cancel; and the core's preview of it as the
  // latest reply gave it, which every sheet's reply carries.
  const [preview, setPreview] = useState<Previewing | null>(null);
  const [previewReport, setPreviewReport] = useState<PreviewReport | null>(null);
  // The generation of the set the report was drawn on: a change landing
  // since makes it old until the preview is drawn again.
  const [previewGeneration, setPreviewGeneration] = useState('');
  // Whether a valid journal snapshot has come: a preview opened before it
  // (or after a failed response) is drawn again on the next valid snapshot,
  // and is not current until then (#192).
  const [journalRead, setJournalRead] = useState(false);
  const journalReadNow = useRef(false);
  const previewOpen = useRef(false);
  previewOpen.current = !!preview;
  const previewKeys = useRef(0);
  // The proposals waiting (#155), from the server's event stream, and the
  // numbers of those seen, so a new one is said in a toast.
  const [proposals, setProposals] = useState<Proposal[]>([]);
  const proposalsSeen = useRef<Set<number> | null>(null);
  // The guides given on the set (#157), from the event stream, each worked
  // out on the set as it stands, and those seen, so a new one is said in a
  // toast; the one running, if any; and the room its banner takes.
  const [guides, setGuides] = useState<Guide[]>([]);
  const guidesSeen = useRef<Set<number> | null>(null);
  const [guideRun, setRun] = useState<GuideRun | null>(null);
  const runNow = useRef<GuideRun | null>(null);
  runNow.current = guideRun;
  const [guideClosed, setGuideClosed] = useState(true);
  const [guideBusy, setGuideBusy] = useState(false);
  const guideBanner = useRef<HTMLDivElement>(null);
  const [bannerHeight, setBannerHeight] = useState(0);
  // Each waiting proposal drawn on the sheet shown as it would leave it,
  // by `proposal|SHEET|LAYOUT|generation`, and those asked for.
  const [ghostReplies, setGhostReplies] = useState<ReadonlyMap<string, GhostReply>>(new Map());
  const ghostAsked = useRef(new Set<string>());
  const ghostGeneration = useRef('');
  // On a phone, the proposal whose chip is opened into its card.
  const [chipOpen, setChipOpen] = useState<number | null>(null);
  // While a keep or a discard is on its way.
  const [proposalBusy, setProposalBusy] = useState(false);
  // Why the core refuses a proposal opened whole, by the preview's key.
  const [previewRefusal, setPreviewRefusal] = useState<{ key: number; message: string } | null>(null);
  // Moves on to ask for a proposal's drawing again, as after a keep the set refused.
  const [ghostNonce, setGhostNonce] = useState(0);
  // The time, for how long ago each proposal was made.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const tick = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(tick);
  }, []);
  // A text whose contents are edited on the drawing, or in a phone's box,
  // by its place; and a count that moves on when an edit in quick
  // properties is cancelled, so its fields show their values again.
  const [editing, setEditing] = useState<{ place: number; touch: boolean } | null>(null);
  const [propsReset, setPropsReset] = useState(0);
  const editor = useRef<HTMLDivElement>(null);
  // A tool asked for while the set as it was is shown: it starts once the set as it is is drawn.
  // The objects it takes go by their IDs: their places on the set as it was are not theirs on the set as it is.
  const [pendingTool, setPendingTool] = useState<{ word: string; ids: string[] } | null>(null);
  // The change sets this page sent whose sheets it has already asked for again.
  const ownEntries = useRef(new Set<number>());
  // The entries whose sheets the journal's event asked for again, so a
  // change set's reply that comes after does not ask twice.
  const journalDrew = useRef(new Set<number>());
  // How many change sets this page sent have not replied yet: Undo last
  // and Accept wait for every one. They are sent one after another, in
  // the order the tools gave them.
  const [sending, setSending] = useState(0);
  const sendQueue = useRef<Promise<void>>(Promise.resolve());
  // The newest change set this page sent that landed, which Undo last
  // takes until the journal the page holds has caught up with it.
  const [ownLatest, setOwnLatest] = useState<number | null>(null);
  // The sheet's layers, for the Layer box, and the current layer kept for this set.
  // Null while they are being asked for.
  const [layers, setLayers] = useState<LayerEntry[] | null>(null);
  const [keptLayer, setKeptLayer] = useState<string | null>(() => readCurrentLayer(keeps, source));
  // The side panel's tab: the assistant's chat first, as the prototype shows it.
  const [side, setSide] = useState<Side>('chat');
  // The assistant (#156): the agents `serve` found, the conversation its
  // stream sends, and what the next message leaves off; why it cannot be
  // reached, when it cannot.
  const [agents, setAgents] = useState<AgentsReport | null>(null);
  const [agentsDown, setAgentsDown] = useState<string | null>(null);
  const [conversation, setConversation] = useState<Conversation | null>(null);
  const [conversationDown, setConversationDown] = useState<string | null>(null);
  const [assistantStream, setAssistantStream] = useState<StreamState>('reconnecting');
  const [chipOff, setChipOff] = useState(false);
  const [lookingAgain, setLookingAgain] = useState(false);
  // What the command bar's field holds, for its hint.
  const [typed, setTyped] = useState('');
  // A phone's sheet, snapped where the page asks: full for the chat's input, down to its peek for a proposal opened.
  const [snapAsked, setSnapAsked] = useState<{ key: number; snap: 'peek' | 'half' | 'full' } | null>(null);
  const snapKeys = useRef(0);
  const snapSheet = (snap: 'peek' | 'half' | 'full') => {
    snapKeys.current += 1;
    setSnapAsked({ key: snapKeys.current, snap });
  };
  const [history, setHistory] = useState<string[]>([]);
  const [peekHeight, setPeekHeight] = useState(0);
  // The room quick properties have on a phone changes with how far the
  // journal is raised and how tall the command bar is: the selected object
  // is brought out from under the card again when it does.
  const [sheetCovers, setSheetCovers] = useState(0);
  const [barHeight, setBarHeight] = useState(0);
  const viewport = useRef<HTMLDivElement>(null);
  const bar = useRef<HTMLDivElement>(null);
  const card = useRef<HTMLElement>(null);
  // Array's panel on a laptop, which the drawing places beside the copies (#66).
  const arrayBox = useRef<HTMLDivElement>(null);
  const coords = useRef<HTMLSpanElement>(null);
  const [toast, setToast] = useState<Toast | null>(null);
  const [asking, setAsking] = useState<Asking | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [findings, setFindings] = useState<ReadonlyMap<string, Finding[]>>(new Map());
  const [versions, setVersions] = useState<Readonly<Record<string, number>>>({});
  const [epoch, setEpoch] = useState(0);
  const replies = useRef(new Map<string, string>());
  const seen = useRef<number | null>(null);
  // What each entry seen was, as recorded, to notice one replaced.
  const seenEntries = useRef(new Map<number, string>());
  // The generations, read when a reply comes: each sheet's version, the
  // count of every change that touched a sheet (ANY), and the epoch, which
  // a journal cut back or replaced moves on.
  const versionsNow = useRef(versions);
  const epochNow = useRef(epoch);
  versionsNow.current = versions;
  epochNow.current = epoch;
  const atNow = useRef(at);
  const keys = useRef(0);
  const tabs = useRef<HTMLElement>(null);
  atNow.current = at;

  // A line of what happened, for the command bar's history.
  const log = useCallback((line: string) => {
    setHistory((h) => [...h.slice(-(HISTORY - 1)), line]);
  }, []);
  const say = useCallback(
    (tone: Tone, title: string, detail: string, entry?: number) => {
      keys.current += 1;
      setToast({ key: keys.current, tone, title, detail, entry });
      log(title);
    },
    [log],
  );
  // A toast goes after a while: routine landing toasts clear in 3 s, while
  // warnings, errors, assistant proposals and guides stay for 6 s (#257).
  useEffect(() => {
    if (!toast) return;
    const gone = setTimeout(() => setToast((t) => (t?.key === toast.key ? null : t)), toastDuration(toast));
    return () => clearTimeout(gone);
  }, [toast]);

  // The drawing tools run in the renderer, which the drawing holds.
  const renderer = () => drawing.current?.viewer() ?? null;
  // The tool running as last read from the renderer, which a sheet drawn
  // again is compared with.
  const toolRead = useRef<ToolState | null>(null);
  const refreshTool = useCallback((): ToolState | null => {
    const v = drawing.current?.viewer();
    const state = v ? (JSON.parse(v.tool_state(!wide)) as ToolState) : null;
    toolRead.current = state;
    setToolState(state);
    drawing.current?.redraw();
    return state;
  }, [wide]);
  useEffect(() => {
    refreshTool();
  }, [refreshTool]);

  const originalSha = readReport?.sha256 ?? null;
  const originalShaRef = useRef<string | null>(originalSha);
  originalShaRef.current = originalSha;
  // Only select a journal revision made against the exact file currently on
  // disk (matching `original_sha256`), never showing A's revision after A was
  // replaced on disk by B.
  const activeRev = isDwg && originalSha ? latestActiveRevision(journal, source, originalSha) : null;
  const steppedRev = isDwg && at !== null && originalSha ? revisionAtEntry(journal, at, source, originalSha) : null;
  // When our own edit landed and wrote a revision newer than what the journal
  // snapshot currently shows, display it right away without waiting for SSE.
  const ownRevFresh =
    ownRev && (!originalSha || ownRev.originalSha256 === originalSha) && (!journal || newest(journal) < ownRev.entry)
      ? ownRev
      : null;
  const displaySource = isDwg
    ? at !== null
      ? (steppedRev?.path ?? source)
      : (ownRevFresh?.path ?? activeRev?.path ?? source)
    : source;

  useEffect(() => {
    if (isDwg && ownRev && journal && newest(journal) >= ownRev.entry) {
      setOwnRev(null);
    }
  }, [isDwg, ownRev, journal]);

  const sheetKey = upper(wanted ?? '');
  const layoutKey = upper(wantedLayout ?? '');
  // The set as it is comes with its object snaps, for the cursor to catch,
  // while Object snap is on; the set as it was, read-only, without. A
  // sheet drawn without them is asked for again when it is turned on.
  // A preview is drawn without them: nothing is picked or snapped there.
  const withSnaps = at === null && toggles.osnap && !preview;
  // Keep `live`/`pv`/`entry` FIRST so `staleDrawings` invalidates live DWG entries
  // cleanly, and include the revision's entry + SHA-256 so an undo followed by a
  // new edit reusing `.rev1.dwg` never hits stale cached geometry (Blocker #5).
  const dwgRevTag = isDwg
    ? `|dwg:${displaySource}@${
        at !== null
          ? (steppedRev?.sha256 ?? readReport?.sha256 ?? 'orig')
          : ownRevFresh
            ? `${ownRevFresh.entry}:${ownRevFresh.sha256}`
            : activeRev
              ? `${activeRev.entry}:${activeRev.sha256}`
              : (readReport?.sha256 ?? 'orig')
      }`
    : '';
  const cacheKey = `${preview ? `pv${preview.key}` : (at ?? 'live')}|${sheetKey}|${layoutKey}|${withSnaps ? 'snaps' : ''}${dwgRevTag}`;
  // A preview is the change set applied to the whole set: any change to
  // any sheet may change it.
  const version = preview ? (versions[ANY] ?? 0) : at === null ? (versions[sheetKey || ANY] ?? 0) : 0;
  const generationKey = preview ? '' : sheetKey;

  /**
   * The generation of what the server draws for `sheetKey` at `entry`, now:
   * a sheet as it is changes with each entry that touches it, and with any
   * when the sheet is not named yet; a sheet as it was changes only when
   * the journal is cut back or replaced. A reply is kept only while its
   * generation is still the one it was asked in.
   */
  const generationOf = (entry: number | null, key: string): string =>
    entry === null
      ? `${epochNow.current}:${key || ANY}:${versionsNow.current[key || ANY] ?? 0}`
      : `${epochNow.current}`;
  // Whether the drawing shown, or the one on its way, is a preview's: while
  // it is, even once the preview has ended, nothing is picked, snapped or
  // started on it, only on the set as it is once that is drawn.
  const onPreview = onPreviewDrawing(!!preview, reply, summary);
  // Whether the preview's report is of the set as it is now: a change
  // landing, or an undo, makes it old until the preview is drawn again,
  // and Change all waits for that.
  const previewCurrent = previewIsCurrent({
    previewing: !!preview,
    reported: !!previewReport,
    journalRead,
    reply,
    reportedAt: previewGeneration,
    now: generationOf(null, ''),
  });

  // Each sheet asked for is drawn by the server once, then kept, until an
  // entry of the journal touches it: then the set as it is now is asked
  // for again. A sheet as it was at an entry never changes.
  useEffect(() => {
    if (isDwg && readPending) return;
    let current = true;
    // With Object snap off, a drawing kept with its snaps does as well.
    const kept = replies.current.get(cacheKey) ?? (withSnaps ? undefined : replies.current.get(`${cacheKey}snaps`));
    if (kept) {
      // A sheet drawn before is shown again at once: whatever another sheet
      // was saying, loading or failing, is not this one's to say. The
      // request for that other sheet, if it is still out, is let go.
      setReply({ text: kept, at, sheetKey, generation: generationOf(at, generationKey), snaps: withSnaps, preview: !!preview, pvKey: preview?.key ?? null });
      setStatus({ kind: 'shown' });
      return;
    }
    if (at === null || !summary) {
      setStatus({ kind: 'loading', message: wanted ? `Drawing ${wanted}…` : 'Drawing the sheet…' });
    }
    const input: Record<string, string | number | boolean> = { source: displaySource };
    if (wanted && !isDwg) input.sheet = wanted;
    if (wantedLayout) input.layout = wantedLayout;
    if (at !== null && !isDwg) input.entry = at;
    if (withSnaps) input.snaps = true;
    // When inspecting a damaged original that `read` read past damage, ask
    // `display-list` to draw the recoverable objects read-only (Blocker #9).
    if (isDwg && displaySource === source && readReport?.read_past_damage) {
      input.read_past_damage = true;
    }
    // The set as it is says where each object comes from, which the tools
    // and quick properties need.
    if (at === null) input.origins = true;
    // And with each object's values, which quick properties edit, and
    // where each is written; a preview's are not edited.
    if (at === null && !preview) input.values = true;
    // A preview: the sheet as the change set would leave it.
    const previewed = preview;
    if (previewed && at === null) (input as Record<string, unknown>).change_set = previewed.send.change_set;
    // A proposal is shown as a keep would land it: on every sheet it reaches or on none.
    if (previewed?.proposal && at === null) (input as Record<string, unknown>).all_or_none = true;
    const generation = generationOf(at, generationKey);
    run('display-list', input, token).then(
      (text) => {
        // Kept even when the reviewer has moved on, for when they come
        // back, unless the sheet has changed since it was asked for: then
        // it is the old drawing, and the sheet is asked for again.
        if (generationOf(at, generationKey) !== generation) return;
        replies.current.set(cacheKey, text);
        if (!current) return;
        setReply({ text, at, sheetKey, generation, snaps: withSnaps, preview: !!previewed, pvKey: previewed?.key ?? null });
      },
      (e) => {
        if (!current) return;
        if (previewed?.proposal) {
          // A proposal the core now refuses whole, such as one naming an
          // object since deleted, cannot be drawn: its band stays open and
          // says why in the core's words, with Keep off and Discard there.
          setPreviewRefusal({ key: previewed.key, message: why(e, token) });
          // The sheet as it is stays shown: nothing is on its way.
          setStatus({ kind: 'shown' });
          return;
        }
        if (previewed) {
          // A change set the core refuses whole cannot be drawn: it says
          // why, as a change set that does not land does, and nothing lands.
          endPreview(previewed.home, previewed.homeLayout, previewed.picked);
          setPropsReset((n) => n + 1);
          say('warn', `${previewed.send.what}: not landed`, why(e, token));
          return;
        }
        if (at !== null && e instanceof CommandError && e.status !== 401) {
          setStatus({ kind: 'cannot', message: e.message });
        } else {
          setStatus({ kind: 'failed', message: why(e, token) });
        }
      },
    );
    return () => {
      current = false;
    };
    // The key holds the sheet and the entry; the version and the epoch say
    // it changed. Re-run once `read` completes so `read_past_damage` is applied.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cacheKey, version, epoch, source, displaySource, token, readPending, readReport?.read_past_damage]);

  // The text the summary shown was read from: a reply of the same text is
  // not drawn again, and its summary is that one's.
  const summaryText = useRef<string | null>(null);
  const loaded = useCallback(
    (shown: Summary, text: string) => {
      // The text the renderer loaded, as it says: a context restored loads
      // it again through the callback it was given at mount.
      summaryText.current = text;
      setSummary(shown);
      setStatus({ kind: 'shown' });
      // Kept by its sheet's own name too, while it is still current; a
      // preview's drawing is never the set as it is.
      if (reply && !reply.preview && !isDwg && generationOf(reply.at, reply.sheetKey) === reply.generation) {
        replies.current.set(
          `${shown.entry ?? 'live'}|${upper(shown.sheet)}|${upper(shown.layout ?? '')}|${reply.snaps ? 'snaps' : ''}`,
          reply.text,
        );
      }
      // The tool running follows the sheet drawn again, or is dropped on
      // another sheet or layout: then what was typed for it goes too, as Esc
      // drops it, so it is not entered into whatever comes next.
      const was = toolRead.current?.tool ?? null;
      const now = refreshTool()?.tool ?? null;
      if (droppedOnLoad(was, now) && field.current) field.current.value = '';
      if (!wanted) setWanted(shown.sheet);
      if (!wantedLayout && shown.layout) setWantedLayout(shown.layout);
      // The address names the sheet and the layout shown, so a reload shows them again.
      const query = new URLSearchParams(window.location.search);
      query.delete('start');
      if (!isDwg) query.set('sheet', shown.sheet);
      else query.delete('sheet');
      if (shown.layout) query.set('layout', shown.layout);
      else query.delete('layout');
      const qs = query.toString();
      window.history.replaceState(null, '', `${window.location.pathname}${qs ? `?${qs}` : ''}`);
    },
    [reply, wanted, wantedLayout, refreshTool, isDwg],
  );
  const failed = useCallback((message: string) => setStatus({ kind: 'failed', message }), []);

  /**
   * Drops every drawing kept of the sheets `touched` and moves their
   * versions on, so the sheet shown is asked for again and a reply already
   * on its way is seen as old when it comes.
   */
  const drawAgain = useCallback((touched: string[]) => {
    if (!touched.length) return;
    // Every drawing kept of a touched sheet goes, each of its layouts
    // among them, and so does the one kept for a sheet not yet named.
    for (const key of staleDrawings(replies.current.keys(), touched)) replies.current.delete(key);
    const next = { ...versionsNow.current };
    for (const s of [...touched, ANY]) next[s] = (next[s] ?? 0) + 1;
    versionsNow.current = next;
    setVersions(next);
  }, []);
  // An open preview's drawings go, and its version moves on: it is not
  // current until it is drawn again.
  const redrawPreview = useCallback(() => {
    for (const key of [...replies.current.keys()]) if (key.startsWith('pv')) replies.current.delete(key);
    const next = { ...versionsNow.current, [ANY]: (versionsNow.current[ANY] ?? 0) + 1 };
    versionsNow.current = next;
    setVersions(next);
  }, []);

  // The checker's findings, sheet by sheet: all of them once, then those
  // of the sheets each new entry touched. Skip strict `check` on damaged
  // originals that `read` read past damage (strict check would 422).
  const damagedOriginal = isDwg && !!readReport?.read_past_damage;
  const readDamageRef = useRef({ pending: readPending, damaged: damagedOriginal });
  readDamageRef.current = { pending: readPending, damaged: damagedOriginal };
  const checkSourceRef = useRef(displaySource);
  checkSourceRef.current = displaySource;
  const check = useCallback(
    async (sheets: string[] | null, explicitSource?: string): Promise<Finding[]> => {
      const target = explicitSource ?? checkSourceRef.current;
      if (isDwg && target === source && (readDamageRef.current.pending || readDamageRef.current.damaged)) {
        return [];
      }
      try {
        const input: Record<string, unknown> = { source: target };
        if (sheets && sourceKind === 'set') input.sheets = sheets;
        const result = await call<{ sheets: string[]; findings: Finding[] }>('check', input, token);
        setFindings((before) => {
          const next = new Map(sheets && sourceKind === 'set' ? before : []);
          for (const s of result.sheets) next.set(upper(s), []);
          for (const f of result.findings) next.get(upper(f.sheet))?.push(f);
          return next;
        });
        return result.findings;
      } catch {
        return [];
      }
    },
    [isDwg, source, sourceKind, token],
  );
  // Once `read` finishes for a non-damaged plant DWG, run its initial check.
  useEffect(() => {
    if (!isDwg || readPending || damagedOriginal || !journal) return;
    void check(null, latestActiveRevision(journal, source, originalSha)?.path ?? source);
  }, [isDwg, readPending, damagedOriginal, journal, source, originalSha, check]);

  // What a new entry says, as a toast.
  const toastFor = useCallback(
    (entry: Entry, failedChecks: Finding[]) => {
      const n = entry.entry;
      switch (entry.kind) {
        case 'failed_attempt':
          if (entry.proposal !== undefined) {
            return say(
              'bad',
              `Proposal ${entry.proposal} was not kept. Nothing landed.`,
              `${entry.error?.message ?? entry.failed_on?.[0]?.message ?? entryTitle(entry)} It is still waiting.`,
              n,
            );
          }
          return say('bad', `Change set ${n} failed. Nothing landed.`, entry.error?.message ?? entryTitle(entry), n);
        case 'discard':
          return say(
            'info',
            `Discarded proposal ${entry.proposal ?? '?'}`,
            `Proposed by ${whoWords(entry.proposed_by)}, discarded by ${whoWords(entry.discarded_by)}. Nothing changed, and nothing undoes a discard.`,
          );
        case 'undo':
          return say(
            'info',
            `Undid change set${(entry.undid?.length ?? 0) > 1 ? 's' : ''} ${listed(entry.undid ?? [])}`,
            `The working revision is back to how it was before change set ${entry.undid?.at(-1) ?? '?'}.`,
          );
        case 'accept':
          return say(
            'ok',
            `Accepted through ${viaWords(entry.via)}`,
            entry.issued?.length
              ? `Issued change set${entry.issued.length > 1 ? 's' : ''} ${listed(entry.issued)}: ${entry.issued.length > 1 ? 'they' : 'it'} can no longer be undone.`
              : 'Issued the set as it stood.',
          );
        default: {
          const landed = entry.landed_on ?? [];
          const failedOn = (entry.failed_on ?? []).map((f) => f.sheet);
          if (failedOn.length) {
            return say(
              'warn',
              `Change set ${n} landed on ${listed(landed)}, failed on ${listed(failedOn)}`,
              `The set is inconsistent until ${listed(failedOn)} ${failedOn.length > 1 ? 'are' : 'is'} fixed or #${n} is undone.`,
              n,
            );
          }
          const mine = failedChecks.filter((f) => landed.some((s) => upper(s) === upper(f.sheet)));
          if (mine.length) {
            return say(
              'bad',
              `${entry.proposal !== undefined ? `Proposal ${entry.proposal} kept: change set` : 'Change set'} ${n} landed on ${listed(landed)}. ${plural(mine.length, 'check')} failed`,
              mine[0]?.message ?? '',
              n,
            );
          }
          if (entry.proposal !== undefined) {
            return say('ok', `Proposal ${entry.proposal} kept: change set ${n} landed on ${listed(landed)}`, `${entryTitle(entry)} · ${entryByline(entry)}`, n);
          }
          return say('ok', `Change set ${n} landed on ${listed(landed)}`, entryTitle(entry), n);
        }
      }
    },
    [say],
  );

  // The journal, from the server's event stream (drawing sets and original DWGs;
  // single YAML sheets have no set journal).
  useEffect(() => {
    if (isSheet) {
      journalReadNow.current = true;
      setJournalRead(true);
      setJournal(null);
      setNoJournal('single sheet descriptions have no journal');
      setStream('open');
      void check(null);
      return;
    }
    journalReadNow.current = false;
    setJournalRead(false);
    const stop = watch(
      source,
      token,
      (text) => {
        const replied = JSON.parse(text) as { ok: boolean; result?: JournalReport; error?: { message: string } };
        // Only a valid snapshot counts as the journal having been read: a
        // failed response keeps any open preview stale and Change all off (#192).
        if (!journalSnapshotValid(replied) || !replied.result) {
          journalReadNow.current = false;
          setJournalRead(false);
          setNoJournal(replied.error?.message ?? 'there is no journal');
          setJournal(null);
          return;
        }
        const wasRead = journalReadNow.current;
        journalReadNow.current = true;
        setJournalRead(true);
        const report = replied.result;
        setNoJournal(null);
        setJournal(report);
        const before = seen.current;
        // A journal cut back, as a commit rolled back cuts it, or whose
        // entries seen were taken away or replaced, is not the one the
        // drawings came from: every drawing kept is dropped, and drawn
        // again.
        const replaced = before !== null && journalReplaced(seenEntries.current, report);
        // Track the newest active `(entry, original_sha256)` on every journal SSE
        // update (even while the initial `read` is in flight) and trigger a
        // guarded `read --no-mirror` refresh whenever a newer entry differs from
        // the last read. Reset entry watermarks first if the journal was replaced.
        if (isDwg) {
          if (replaced) resetDwgReadJournalState(dwgReadTrackerRef.current);
          const newestOrig = newestActiveOriginalRevision(report);
          if (noteJournalOriginalSha(dwgReadTrackerRef.current, newestOrig?.entry ?? null, newestOrig?.sha ?? null)) {
            triggerDwgRead(false);
          }
        }
        seen.current = newest(report);
        seenEntries.current = recordedEntries(report);
        if (before === null) {
          const n = report.entries.length;
          log(`Journal read: ${n} entr${n === 1 ? 'y' : 'ies'}.`);
        }
        // An open preview is drawn again on the first valid snapshot (and
        // after a failed response), which may hold a change set that landed
        // before the page read the journal.
        if (previewRedrawnOn(previewOpen.current, !wasRead)) redrawPreview();
        const checkTarget = isDwg ? (latestActiveRevision(report, source, originalShaRef.current)?.path ?? source) : source;
        if (before === null || replaced) {
          if (replaced) {
            replies.current.clear();
            epochNow.current += 1;
            setEpoch(epochNow.current);
          }
          void check(null, checkTarget);
          return;
        }
        const fresh = since(report, before);
        if (!fresh.length) return;
        const touched = [...new Set(fresh.flatMap((e) => sheetsTouched(e, report)).map(upper))];
        // A change set this page sent had its sheets asked for again as
        // soon as `change` replied: they are not asked for twice.
        const theirs = fresh.filter((e) => !ownEntries.current.has(e.entry));
        for (const e of theirs) journalDrew.current.add(e.entry);
        const others = [...new Set(theirs.flatMap((e) => sheetsTouched(e, report)).map(upper))];
        drawAgain(others);
        const last = fresh.at(-1) as Entry;
        const landed = [...fresh].reverse().find((e) => e.kind === 'change_set' && (e.landed_on?.length ?? 0) > 0);
        // A change set that lands is marked, and its marks pulse, unless the
        // reviewer is looking at the set as it was: they stay where they are.
        if (atNow.current === null && landed) {
          setFocused(landed.entry);
          setPulseSince(performance.now());
        }
        if (touched.length) {
          void check(touched, checkTarget).then((found) => toastFor(last, found));
        } else {
          toastFor(last, []);
        }
      },
      setStream,
      fetch,
      (text) => {
        // The proposals waiting, as each is made, kept or discarded.
        const replied = JSON.parse(text) as { ok: boolean; result?: ProposalsReport };
        if (!replied.ok || !replied.result) return;
        const list = replied.result.proposals;
        setProposals(list);
        const known = proposalsSeen.current;
        proposalsSeen.current = new Set(list.map((p) => p.proposal));
        // The first snapshot says nothing; a proposal made since does.
        const made = known ? list.filter((p) => !known.has(p.proposal)).at(-1) : undefined;
        if (!made) return;
        const title = `${capitalWho(made.by)} proposes: ${proposalTitle(made)}`;
        keys.current += 1;
        setToast({
          key: keys.current,
          tone: 'prop',
          title,
          detail: `Proposal ${made.proposal}, on ${listed(made.sheets)}. Nothing changes until you keep it.`,
          proposal: made.proposal,
        });
        log(title);
      },
      (text) => {
        // The guides given, each worked out on the set as it stands.
        const replied = JSON.parse(text) as { ok: boolean; result?: GuidesReport };
        if (!replied.ok || !replied.result) return;
        const list = replied.result.guides;
        setGuides(list);
        // A guide running steps back when a step's change set is undone,
        // from any interface: the listing says which steps stay done.
        const newestEntry = replied.result.newest_entry;
        if (newestEntry !== undefined) {
          setRun((r) => {
            const g = r ? list.find((x) => x.guide === r.guide) : undefined;
            const next = r && g ? guideSynced(r, g, newestEntry) : r;
            if (next) runNow.current = next;
            return next;
          });
        }
        const known = guidesSeen.current;
        guidesSeen.current = new Set(list.map((g) => g.guide));
        // Every guide given since the last listing gets its Start, however
        // many came between two looks.
        const offered = givenToast(known ? list.filter((g) => !known.has(g.guide)) : [], capitalWho);
        if (!offered) return;
        keys.current += 1;
        setToast({ key: keys.current, tone: 'prop', title: offered.title, detail: offered.detail, guides: offered.guides });
        log(offered.title);
      },
    );
    return () => {
      stop();
      seen.current = null;
      proposalsSeen.current = null;
      guidesSeen.current = null;
    };
  }, [source, isDwg, isSheet, token, check, toastFor, log, drawAgain, redrawPreview, triggerDwgRead]);

  // The objects the focused entry changed on the sheet shown, marked.
  const focusedEntry = journal?.entries.find((e) => e.entry === focused);
  const places = useMemo(() => {
    const byId = new Map<string, number>();
    (summary?.objects ?? []).forEach((o, i) => o.id && byId.set(upper(o.id), i));
    return byId;
  }, [summary]);
  const marks = useMemo<Mark[]>(() => {
    const showing = focusedEntry && (!focusedEntry.undone_by || at === focusedEntry.entry);
    if (!showing || !summary) return [];
    if (isDwg) {
      const origSheet = readReport?.sheet ?? summary.sheet;
      const changed = changedOn(focusedEntry, origSheet);
      const out: Mark[] = [];
      for (const o of changed) {
        const idx = summary.objects.findIndex((obj) => matchOriginalObjectId(o.id, obj.id));
        if (idx >= 0) out.push({ place: idx, added: o.how === 'added' });
      }
      return out;
    }
    const changed = changedOn(focusedEntry, summary.sheet);
    return changed
      .filter((o) => places.has(upper(o.id)))
      .map((o) => ({ place: places.get(upper(o.id)) as number, added: o.how === 'added' }));
  }, [focusedEntry, summary, places, at, isDwg, readReport]);

  // The picked object, followed by its ID as the sheet is drawn again; one
  // gone from the sheet is no longer picked.
  const pickedPlace =
    picked === null || !summary
      ? null
      : picked.id !== null
        ? (places.get(upper(picked.id)) ?? null)
        : picked.summary === summary
          ? picked.place
          : null;
  useEffect(() => {
    if (picked !== null && summary && pickedPlace === null) {
      setPicked(null);
    }
  }, [picked, summary, pickedPlace]);

  // A part of a sheet asked for, once that sheet is drawn.
  useEffect(() => {
    if (!focusWanted || !summary || upper(summary.sheet) !== upper(focusWanted.sheet)) return;
    if ((summary.entry ?? null) !== at) return;
    setFocus({
      key: focusWanted.key,
      sheet: focusWanted.sheet,
      places: focusWanted.ids.filter((id) => places.has(upper(id))).map((id) => places.get(upper(id)) as number),
    });
    setFocusWanted(null);
  }, [focusWanted, summary, places, at]);

  // The tab shown stays in sight on a phone's narrow strip.
  useEffect(() => {
    tabs.current?.querySelector('.tab.on')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }, [summary]);

  /** Shows `entry`: the set as it was just after it, on a sheet it touched, its changes marked. */
  const showEntry = (number: number, asItWas = true) => {
    if (!journal) return;
    const entry = journal.entries.find((e) => e.entry === number);
    if (!entry) return;
    const touched = [...sheetsTouched(entry, journal), ...(entry.failed_on ?? []).map((f) => f.sheet)];
    const current = summary?.sheet ?? wanted ?? '';
    const target = touched.some((s) => upper(s) === upper(current)) ? current : (firstSheet(entry, journal) ?? current);
    // A preview ends first, landing nothing: an entry is drawn as it was, never with a change set.
    if (preview) {
      endPreview('', null);
      setPropsReset((n) => n + 1);
      log(`${preview.send.what}: cancelled, nothing changed`);
    }
    // Placed copies (and drawn lines/text) land first; a template's asks in Change all and stays (#246).
    if (leaveTool()) return;
    setAt(asItWas ? number : null);
    setFocused(number);
    setPulseSince(null);
    setPicked(null);
    setSide('journal');
    setWanted(target);
    // Another sheet may not have the layout shown: it opens on its first.
    setWantedLayout(layoutOn(current, target, wantedLayout));
    if (asItWas) log(`Showing entry ${number} as it was: read-only`);
    keys.current += 1;
    setFocusWanted({ key: keys.current, sheet: target, ids: changedOn(entry, target).map((o) => o.id) });
    setRaise((r) => r + 1);
  };
  const step = (by: number) => {
    if (!journal?.entries.length) return;
    const entries = journal.entries;
    const from = focused === null ? -1 : entries.findIndex((e) => e.entry === focused);
    const start = from < 0 ? entries.length : from;
    const next = Math.min(entries.length - 1, Math.max(0, start + by));
    const entry = entries[next];
    if (entry) showEntry(entry.entry);
  };
  const backToLive = () => {
    setAt(null);
    setPulseSince(null);
    setPicked(null);
    log('Back to live');
  };

  const undo = async (changeSet: number | null) => {
    if (!changeSet) return;
    if (undoLocked) {
      log('View-only (ADR-0004): inspect the decode report to allow revising or undoing this original DWG');
      return;
    }
    setBusy('undo');
    if (isDwg) setOwnRev(null);
    try {
      await call('undo', { set: source, change_set: changeSet }, token);
      if (ownLatest !== null && changeSet <= ownLatest) setOwnLatest(null);
    } catch (e) {
      say('warn', `Change set ${changeSet} was not undone`, why(e, token));
    } finally {
      setBusy(null);
    }
  };

  // Accept asks first: the server says what it would issue, and any failed
  // checks, without writing; the reviewer confirms.
  const askAccept = async () => {
    if (!journal || editingDisabled) return;
    const newestSeen = Math.max(newest(journal), ownLatest ?? 0);
    setBusy('accept');
    try {
      const report = await call<AcceptReport>('accept', { set: source, dry_run: true, newest_seen: newestSeen }, token);
      setAsking({ report, newestSeen });
    } catch (e) {
      if (e instanceof CommandError && e.code === 'checks_failed' && e.result) {
        setAsking({ report: e.result as AcceptReport, newestSeen });
      } else {
        say('warn', 'Can’t accept yet', why(e, token));
      }
    } finally {
      setBusy(null);
    }
  };
  const confirmAccept = async () => {
    const was = asking;
    setAsking(null);
    if (!was) return;
    setBusy('accept');
    try {
      await call(
        'accept',
        {
          set: source,
          newest_seen: was.newestSeen,
          despite_failed_checks: was.report.checks.findings.length > 0,
        },
        token,
      );
    } catch (e) {
      say('warn', 'Not accepted', why(e, token));
    } finally {
      setBusy(null);
    }
  };

  // A tap selects the object the renderer picked, or nothing; its quick
  // properties open beside it.
  const pick = (place: number | null) => {
    const o = place === null || !summary ? undefined : summary.objects[place];
    setTaken(null);
    setPicked(place === null || !summary ? null : { id: o?.id ?? null, place, summary });
    if (o) {
      log(`Selected: ${typeWords(o)}${o.name ? ` ${o.name}` : ''}${o.id ? ` (${o.id})` : ''}`);
    }
  };

  const toggle = useCallback(
    (which: Toggle) => {
      setToggles((t) => {
        const next = { ...t, [which]: !t[which] };
        log(toggledWords(which, next[which]));
        return next;
      });
    },
    [log],
  );

  // A snap clicked, or lifted from under a held finger, with no tool
  // running: its point read out, as AutoCAD's ID command does, and its
  // object selected.
  const takeSnap = (h: Hovered) => {
    if (!summary || arrayUp) return;
    const o = summary.objects[h.object];
    const read = takenOf(h, summary.objects, summary.units);
    setPicked(o ? { id: o.id ?? null, place: h.object, summary } : null);
    const ids = [o?.id ?? null, ...(h.with === null ? [] : [summary.objects[h.with]?.id ?? null])];
    setTaken({ taken: read, point: snappedPoint(h, summary.objects), summary, ids });
    log(`${read.label}${read.rest}`);
  };
  const noSnap = (p: [number, number]) => {
    log(`No snap under your finger: ${coordinates(p[0], p[1], summary?.units ?? null)}`);
  };
  const copySnap = async () => {
    const point = taken?.point;
    if (!point) return;
    const text = JSON.stringify(point);
    // The Clipboard API needs a secure context: a page opened over the
    // tailnet by its address, plain http, has none, so the older copy of a
    // selected field is tried there, which a click still allows.
    const older = () => {
      const field = document.createElement('textarea');
      field.value = text;
      field.setAttribute('readonly', '');
      field.style.position = 'fixed';
      field.style.opacity = '0';
      document.body.append(field);
      field.select();
      const done = document.execCommand('copy');
      field.remove();
      return done;
    };
    try {
      if (navigator.clipboard) await navigator.clipboard.writeText(text);
      else if (!older()) throw new Error('no clipboard');
      log(`Copied ${text}`);
    } catch {
      log(`Could not copy; the snapped point: ${text}`);
    }
  };

  // The Object snap menu's choices.
  const snapOn = (on: boolean) => {
    setToggles((t) => (t.osnap === on ? t : { ...t, osnap: on }));
  };
  const flipSnap = (k: SnapKind) => {
    setRunning((before) => {
      const next = new Set(before);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      log(`${snapLabel(k)} ${next.has(k) ? 'on' : 'off'}`);
      return next;
    });
    // Ticking a snap while snapping is off turns it on.
    if (!running.has(k) && !toggles.osnap) {
      snapOn(true);
      log('Object snap on');
    }
  };
  const openSnapMenu = (anchor: HTMLElement | null) => {
    if (!anchor || !wide) {
      setToolsOpen(false);
      setSnapMenu('card');
      return;
    }
    if (snapMenu) {
      setSnapMenu(null);
      return;
    }
    const r = anchor.getBoundingClientRect();
    setSnapMenu({ left: Math.max(8, Math.min(window.innerWidth - 338, r.left - 120)), bottom: window.innerHeight - r.top + 6 });
  };
  // The menu shuts on a press anywhere else.
  useEffect(() => {
    if (!snapMenu) return;
    const away = (e: PointerEvent) => {
      if (!(e.target as Element | null)?.closest?.('.osmenu, .osmenubtn, [data-snaps]')) setSnapMenu(null);
    };
    window.addEventListener('pointerdown', away, true);
    return () => window.removeEventListener('pointerdown', away, true);
  }, [snapMenu]);

  // The drawing tools, which run in the renderer: the page passes them
  // what the drafter does and shows what they say.
  // What came of a tool's step: its lines in the history, and its change
  // set sent, or asked about first when it edits a template's object.
  const toolDid = (text: string): boolean => {
    let asking = false;
    if (text) {
      const o = JSON.parse(text) as ToolOutcome & { guide?: RendererNote };
      for (const line of o.log) log(line);
      // A point or a pick a guide's step does not want: a gentle note,
      // which offers to take it back; nothing is taken back unasked.
      if (o.guide) {
        const note = clickNote(o.guide);
        setRun((r) => (r && !r.end ? { ...r, note } : r));
        log(`Guide: ${o.guide.words}`);
      } else {
        // Another point or pick since: the note's Undo it no longer
        // takes back the last thing taken, so it goes.
        setRun((r) => (r?.note?.act?.kind === 'take_back' ? { ...r, note: null } : r));
      }
      // Each in order: one to a template's object is asked about first,
      // carrying the guide mark through the preview when a guide runs (#264).
      for (const s of o.sends ?? []) {
        if (s.template) {
          const cs = guideMarked(s.change_set, runNow.current, runGuide(), summary?.sheet ?? null);
          const guideRunId = cs !== s.change_set ? runNow.current?.runId : undefined;
          startPreview(cs !== s.change_set ? { ...s, change_set: cs } : s, guideRunId);
          asking = true;
        } else {
          void sendGuided(s);
        }
      }
    }
    refreshTool();
    return asking;
  };
  // Sends a tool's change set through `change`, as an agent would, and
  // asks for the sheets it landed on as soon as `change` replies, without
  // waiting for the journal.
  // Gives whether it landed on any sheet.
  const sendOne = async (s: ToolSend, keep?: number, onReply?: (reply: ChangeReply) => void): Promise<boolean> => {
    const landed = (reply: ChangeReply | undefined) => {
      const sheets = (reply?.sheets ?? []).filter((x) => x.ok).map((x) => upper(x.sheet));
      const entry = reply?.journal_entry;
      if (isDwg && reply?.revision_dwg) {
        const revPath = resolveBeside(source, reply.revision_dwg);
        const revSha = reply.revision?.sha256 ?? `${entry ?? Date.now()}`;
        if (entry !== undefined) {
          setOwnRev({ path: revPath, entry, sha256: revSha, originalSha256: readReport?.sha256 ?? '' });
          setOwnLatest((n) => Math.max(n ?? 0, entry));
        }
        const revSheet = upper(reply.sheet ?? summary?.sheet ?? '');
        if (revSheet) drawAgain([revSheet]);
        return;
      }
      if (entry !== undefined) {
        if (sheets.length) setOwnLatest((n) => Math.max(n ?? 0, entry));
        // The journal's event drew its sheets already. A first reading of
        // the journal draws none, so only an event that did counts.
        if (journalDrew.current.has(entry)) return;
        ownEntries.current.add(entry);
      }
      drawAgain(sheets);
    };
    try {
      // Blocker #6 (ADR-0004): before passing `allow_loss: true`, re-verify the
      // original DWG's SHA-256 on disk so replacing the file at the same path
      // always revokes prior consent and forces fresh acknowledgement.
      if (isDwg && allowLoss && readReport?.sha256) {
        const { gen, startedForEntry } = beginDwgRead(dwgReadTrackerRef.current);
        let fresh: ReadReport;
        try {
          fresh = await call<ReadReport>('read', { source, no_mirror: true }, token);
        } catch (readErr) {
          completeDwgRead(dwgReadTrackerRef.current, gen, startedForEntry, null);
          throw readErr;
        }
        const done = completeDwgRead(dwgReadTrackerRef.current, gen, startedForEntry, fresh.sha256);
        if (fresh.sha256 !== readReport.sha256) {
          onRevokeLoss(readReport.sha256);
          setOwnRev(null);
          if (done.accept) setReadReport(fresh);
          if (done.followUp) triggerDwgRead(false);
          say(
            'warn',
            'Original DWG changed on disk',
            'Re-inspect the decode report and confirm before revising this file.',
          );
          return false;
        }
        if (done.followUp) triggerDwgRead(false);
      }
      const prepared =
        isDwg && readReport && summary
          ? normalizeOriginalChangeSet(s.change_set, readReport.sheet, summary.sheet)
          : s.change_set;
      const changeInput: Record<string, unknown> = { set: source, change_set: prepared };
      if (isDwg && readReport?.sha256) changeInput.expected_original_sha256 = readReport.sha256;
      if (isDwg && allowLoss) changeInput.allow_loss = true;
      // A proposal kept (#155) lands through `keep`, which records who
      // proposed it and who kept it; its reply is `change`'s.
      const reply =
        keep === undefined
          ? await call<ChangeReply>('change', changeInput, token)
          : await call<ChangeReply>('keep', { set: source, proposal: keep, by: DRAFTER }, token);
      landed(reply);
      log(
        reply.revision_dwg
          ? `${s.what}: saved revision ${sourceDisplayName(reply.revision_dwg)} (original untouched)`
          : `${s.what}${keep === undefined ? '' : ' kept'}: change set ${reply.journal_entry ?? ''} landed`.replace('  ', ' '),
      );
      onReply?.(reply);
      return true;
    } catch (e) {
      if (isDwg && e instanceof CommandError && e.code === 'changed_by_hand') {
        const { gen, startedForEntry } = beginDwgRead(dwgReadTrackerRef.current);
        try {
          const fresh = await call<ReadReport>('read', { source, no_mirror: true }, token);
          const done = completeDwgRead(dwgReadTrackerRef.current, gen, startedForEntry, fresh.sha256);
          if (done.accept && readReport?.sha256 && fresh.sha256 !== readReport.sha256) {
            onRevokeLoss(readReport.sha256);
            setOwnRev(null);
            setReadReport(fresh);
          }
          if (done.followUp) triggerDwgRead(false);
        } catch {
          completeDwgRead(dwgReadTrackerRef.current, gen, startedForEntry, null);
          // Ignore follow-up read error; the original change error is shown below.
        }
      }
      if (e instanceof CommandError && e.result) landed(e.result as ChangeReply);
      say('warn', `${s.what}: not landed`, why(e, token));
      return !!(e instanceof CommandError && (e.result as ChangeReply | undefined)?.sheets?.some((x) => x.ok));
    }
  };
  // Undo last and Accept wait while any is on its way: a change set they
  // would take may be about to land.
  const sendChange = (s: ToolSend, keep?: number, onReply?: (reply: ChangeReply) => void) => {
    setSending((n) => n + 1);
    const sent = sendQueue.current.then(() => sendOne(s, keep, onReply)).finally(() => setSending((n) => n - 1));
    sendQueue.current = sent.then(
      () => {},
      () => {},
    );
    return sent;
  };
  const startTool = (word: string, fromSelection = true) => {
    if (editingDisabled) {
      log(
        dwgLocked
          ? 'View-only (ADR-0004): inspect the decode report to allow revising this original DWG'
          : 'Single sheet view is read-only',
      );
      return;
    }
    if (onPreview) {
      log(preview ? 'Change all or Cancel the preview first' : 'Drawing the sheet as it is first');
      return;
    }
    setToolsOpen(false);
    setTaken(null);
    setSnapMenu(null);
    if (dismissToastOnToolStart(toast)) setToast(null);
    const picked = fromSelection && pickedPlace !== null ? [pickedPlace] : [];
    if (at !== null) {
      // The set as it was is read-only: the tool starts on the set as it
      // is, on the selected object found there by its ID.
      backToLive();
      setPendingTool({ word, ids: fromSelection && picked.length && object?.id ? [object.id] : [] });
      return;
    }
    const v = renderer();
    if (!v) return;
    const text = v.tool_start(word, Uint32Array.from(picked));
    const state = JSON.parse(v.tool_state(!wide)) as ToolState;
    if (state.tool) setPicked(null);
    guideToolStarted(state.tool ?? null);
    if (state.tool === 'text' && !wide) setFieldShown(false);
    toolDid(text);
    if (state.tool && wide) field.current?.focus({ preventScroll: true });
  };
  const ribbonTool = (tool: Tool) => {
    // The running tool's own button ends it.
    if (toolState?.tool === tool.key) toolDid(renderer()?.tool_escape() ?? '');
    else startTool(tool.key);
  };
  // Array's panel (#66): up while an array is set up, its values the
  // renderer's. A click on the drawing then selects nothing, but takes a
  // polar array's centre while it is picked.
  const arrayUp = !!toolState?.array && !toolState.array.picking;
  // Another sheet or layout asked for: the array set up on the one drawn
  // goes, with nothing sent, the moment the tab is asked for, not when the
  // sheet comes; until then nothing of it can land (#66, review).
  const arrayStale = () =>
    !!toolState?.array &&
    switchingAway(
      wanted,
      wantedLayout,
      summary ? { sheet: summary.sheet, layout: summary.layout ?? null, layouts: summary.layouts } : null,
    );
  useEffect(() => {
    if (arrayStale()) leaveTool();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wanted, wantedLayout]);
  // A tab clicked ends the running tool at once, in the renderer: Copy's
  // copies placed (and Line/Text) land as Esc lands them, Array drops with
  // nothing sent (#246). Returns true when a template edit opens the
  // Change all band, so the page stays on the sheet shown.
  const leaveToolFor = (sheet: string, layout: string | null): boolean => {
    const shown = summary ? { sheet: summary.sheet, layout: summary.layout ?? null, layouts: summary.layouts } : null;
    if (!switchingAway(sheet, layout, shown)) return false;
    return leaveTool();
  };
  const arrayAct = (a: string) => {
    if (arrayStale()) {
      leaveTool();
      return;
    }
    toolDid(renderer()?.tool_option(a) ?? '');
    if (a === 'rectangular' || a === 'polar' || a === 'path') {
      requestAnimationFrame(() => drawing.current?.frameArray(true));
    } else if (toolRead.current?.array) {
      requestAnimationFrame(() => drawing.current?.frameArray(false));
    }
  };
  const arrayValue = (key: string, text: string) => {
    if (arrayStale()) return;
    renderer()?.tool_array_value(key, text);
    refreshTool();
    requestAnimationFrame(() => drawing.current?.frameArray(false));
  };
  const arrayKey = toolState?.array ? `${toolState.array.kind}|${toolState.array.picking}` : '';
  useEffect(() => {
    // The view comes to the objects and their copies as the panel opens,
    // and as a polar array's centre is taken.
    if (arrayKey) requestAnimationFrame(() => drawing.current?.frameArray(true));
  }, [arrayKey]);
  const arrayPanel = (touch: boolean) =>
    toolState?.array && !preview ? (
      <ArrayPanel panel={toolState.array} touch={touch} onAct={arrayAct} onValue={arrayValue} boxRef={touch ? undefined : arrayBox} />
    ) : null;
  // Inside a running tool that offers Undo, Undo, U and Ctrl+Z (⌘Z) take the last point or copy back (#245).
  // Outside a tool, Ctrl+Z does nothing: Undo last stays a button, since there is no Redo.
  const toolCanUndo = canToolUndo(toolState);
  const undoTool = () => toolDid(renderer()?.tool_option('undo') ?? '');
  // The page leaves what the tool works on: another sheet or layout, an
  // entry of the journal, or a proposal opened (#246). Placed copies (and
  // drawn lines/text) land as Esc lands them; Array drops with nothing sent.
  // Returns true if landing opened a template preview ("Change all N sheets"),
  // so the caller stays on the sheet shown.
  const leaveTool = (): boolean => {
    const v = renderer();
    if (!v || !toolRead.current?.tool) return false;
    const askingTemplate = toolDid(v.tool_leave());
    if (field.current) field.current.value = '';
    setTyped('');
    return askingTemplate;
  };
  const endTool = () => {
    const v = renderer();
    if (!v || !toolState?.tool) return;
    const was = toolState.tool;
    const text = v.tool_escape();
    toolDid(text);
    // The step's own tool ended with nothing drawn: the step waits.
    const sent = text ? ((JSON.parse(text) as ToolOutcome).sends ?? []).length : 0;
    const step = guideStepHere();
    if (step && !sent && step.tool === was) setRun((r) => (r ? { ...r, note: endedNote(step) } : r));
  };
  // Enter, Space or a right-click, with what the command bar's field holds.
  const enter = (text: string) => {
    const v = renderer();
    if (!v) return;
    const as = typedRoute(text, !!toolState?.tool, onPreview, toolWord);
    if (as === 'refused') {
      log(preview ? 'Change all or Cancel the preview first' : 'Drawing the sheet as it is first');
      return;
    }
    if (as === 'assistant') {
      // Words that are no tool's go to the assistant, in its chat.
      askFromBar(text.trim());
      return;
    }
    if (as === 'start') {
      // A tool's alias starts it, Move and Erase on what is selected.
      startTool(text.trim());
      return;
    }
    if (arrayStale()) {
      leaveTool();
      return;
    }
    const wasArray = !!toolState?.array;
    toolDid(v.tool_input(text));
    // Typed spacing/counts in the command bar (e.g. `@0,-10`) reframe the view
    // when ghosts move off screen, matching edits made in the panel (#259).
    if (wasArray && toolRead.current?.array) {
      requestAnimationFrame(() => drawing.current?.frameArray(false));
    }
  };
  // Enter or Space away from the field: what it holds is entered.
  const enterTyped = () => {
    const f = field.current;
    const text = f?.value ?? '';
    if (f) f.value = '';
    setTyped('');
    enter(text);
  };
  // Right-click on the drawing while a tool runs: Done, clearing any
  // half-typed point draft so it never stalls finishing Copy (#248).
  const rightClickFinish = () => {
    const f = field.current;
    const text = f?.value ?? '';
    if (f) f.value = '';
    setTyped('');
    enter(rightClickText(text, !!toolState?.typing_text));
  };
  useEffect(() => {
    if (!pendingTool || !summary || summary.entry !== null || at !== null) return;
    const v = renderer();
    if (!v) return;
    setPendingTool(null);
    const live = pendingTool.ids.map((id) => places.get(upper(id)));
    pendingTool.ids.forEach((id, i) => {
      if (live[i] === undefined) log(`${id} is not on the sheet as it is: the tool starts without it`);
    });
    const found = live.filter((p): p is number => p !== undefined);
    toolDid(v.tool_start(pendingTool.word, Uint32Array.from(found)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pendingTool, summary, at]);

  // AutoCAD's keys: F3 Object snap, F7 Grid, F8 Ortho; Escape steps back
  // out of whatever is open, one thing at a time.
  const escape = useRef<() => void>(() => {});
  escape.current = () => {
    // Esc drops what is typed, as AutoCAD's does.
    if (field.current) field.current.value = '';
    setTyped('');
    if (decodeModalOpen) setDecodeModalOpen(false);
    else if (editing) setEditing(null);
    else if (preview) cancelPreview();
    else if (asking) setAsking(null);
    else if (snapMenu) setSnapMenu(null);
    else if (toolsOpen) setToolsOpen(false);
    else if (toolState?.tool) endTool();
    else if (toolState?.error) toolDid(renderer()?.tool_option('ok') ?? '');
    else if (picked) pick(null);
    else if (at !== null) backToLive();
  };
  // AutoCAD's keys on the page: typing goes to the command bar, Enter or
  // Space is Enter, Delete erases what is selected.
  const keys2 = useRef<(e: KeyboardEvent) => void>(() => {});
  keys2.current = (e: KeyboardEvent) => {
    const target = e.target as Element | null;
    if (target?.closest?.('input, select, textarea')) return;
    if (asking || decodeModalOpen) return;
    if (preview) {
      // The arrow keys step through the sheets it reaches; nothing else is typed.
      if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
        e.preventDefault();
        stepPreview(e.key === 'ArrowLeft' ? -1 : 1);
      }
      return;
    }
    if (isUndoKey(e) && toolCanUndo) {
      // Ctrl+Z takes back the running tool's last point or copy, as U does (#245).
      e.preventDefault();
      undoTool();
      return;
    }
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.key === 'Delete' && !toolState?.tool && pickedPlace !== null) {
      e.preventDefault();
      startTool('erase');
      return;
    }
    if (target?.closest?.('button') && (e.key === 'Enter' || e.key === ' ')) return;
    // Space is Enter inside a tool, with nothing typed, or after a short
    // alias; after other words it is a space, so a sentence can be typed
    // for the assistant.
    if (e.key === 'Enter' || (e.key === ' ' && spaceIsEnter(field.current?.value ?? '', !!toolState?.tool, toolWord))) {
      e.preventDefault();
      enterTyped();
      return;
    }
    if (e.key.length === 1) {
      // Typing anywhere goes to the command bar, as in AutoCAD.
      e.preventDefault();
      setFieldShown(true);
      const f = field.current;
      if (!f) return;
      f.hidden = false;
      f.value += e.key;
      setTyped(f.value);
      f.focus({ preventScroll: true });
      if (toolState?.typing_text) renderer()?.tool_typing(f.value);
      drawing.current?.redraw();
    }
  };
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      const which = toggleFor(e.key);
      if (which) {
        e.preventDefault();
        toggle(which);
      } else if (e.key === 'Escape') {
        escape.current();
      } else {
        keys2.current(e);
      }
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [toggle]);
  // A change set to a template's objects is drawn before it lands (#154):
  // the sheet shown as it would leave it, then each sheet it reaches, one
  // by one. Cancel lands nothing; Change all sends it, and it lands on all
  // of them or on none.
  function startPreview(s: ToolSend, guideRunId?: number) {
    if (!s.template) return;
    previewKeys.current += 1;
    setPicked(null);
    setTaken(null);
    setEditing(null);
    setSnapMenu(null);
    setToolsOpen(false);
    setPreviewReport(null);
    const home = summary?.sheet ?? wanted ?? '';
    const homeLayout = summary?.layout ?? wantedLayout ?? null;
    setPreview({ send: { ...s, template: s.template }, home, homeLayout, key: previewKeys.current, picked, guideRunId });
    // The command bar's field lets go, so the arrow keys step through the sheets.
    field.current?.blur();
    log(`Preview: ${s.template.edit} ${previewTitle(s.template).reach}. Nothing has landed.`);
  }
  // Its drawings go, and the sheet it was made on comes back, with the
  // object selected when it was made, as quick properties had it, once
  // that sheet is drawn as it is.
  const restorePick = useRef<{ picked: Picked; home: string } | null>(null);
  function endPreview(home: string, layout: string | null, was: Picked | null = null) {
    restorePick.current = was ? { picked: was, home } : null;
    setPreview(null);
    setPreviewRefusal(null);
    setPreviewReport(null);
    for (const key of [...replies.current.keys()]) if (key.startsWith('pv')) replies.current.delete(key);
    if (home && (upper(home) !== upper(wanted ?? '') || upper(layout ?? '') !== upper(wantedLayout ?? ''))) {
      setWanted(home);
      setWantedLayout(layout);
    }
  }
  const cancelPreview = () => {
    const was = preview;
    if (!was) return;
    endPreview(was.home, was.homeLayout, was.picked);
    setPropsReset((n) => n + 1);
    // A proposal closed is not ended: only Discard ends it.
    log(was.proposal ? `Proposal ${was.proposal.proposal}: closed; it keeps waiting` : `${was.send.what}: cancelled, nothing changed`);
  };
  const confirmPreview = () => {
    const was = preview;
    if (!was || !previewCurrent || !previewDrawn) return;
    if (was.proposal) {
      keepProposal(was.proposal);
      return;
    }
    endPreview(was.home, was.homeLayout, was.picked);
    const { changeSet, runId } = previewConfirmSend(
      was.send.change_set as ToolSend['change_set'] & { guide?: { guide: number; step: number } },
      was.guideRunId,
      runNow.current,
    );
    const toSend = changeSet === was.send.change_set ? was.send : { ...was.send, change_set: changeSet };
    const onReply = runId !== undefined ? (reply: ChangeReply) => guideReplied(reply, runId) : undefined;
    // An edit in quick properties that did not land shows its value again.
    void sendChange(toSend, undefined, onReply).then((ok) => ok || setPropsReset((n) => n + 1));
  };
  // A sheet whose DWG could not be written may be written now: ask again.
  const recheckPreview = () => {
    if (!preview) return;
    previewKeys.current += 1;
    for (const key of [...replies.current.keys()]) if (key.startsWith('pv')) replies.current.delete(key);
    setPreview({ ...preview, key: previewKeys.current });
    log('Asked again whether each sheet would take it');
  };
  // A proposal opened (#155): shown in the band's PROPOSAL form, the sheet
  // shown as it would leave it, on the sheet it was opened on when it
  // reaches it, or its first.
  function openProposal(p: Proposal, on?: string) {
    // Placed copies (and drawn lines/text) land first; a template's asks in Change all and stays (#246).
    if (leaveTool()) return;
    // From the set as it was, as from a toast's Show: back to live first,
    // since a proposal is drawn on the set as it is.
    if (at !== null) backToLive();
    setPreviewRefusal(null);
    setChipOpen(null);
    setToast(null);
    const title = proposalTitle(p);
    const shown = summary?.sheet ?? wanted ?? '';
    const reaches = (s: string | undefined) => !!s && p.sheets.some((x) => upper(x) === upper(s));
    const go = reaches(on) ? (on as string) : reaches(shown) ? shown : (p.sheets[0] ?? shown);
    previewKeys.current += 1;
    setPicked(null);
    setTaken(null);
    setEditing(null);
    setSnapMenu(null);
    setToolsOpen(false);
    setPreviewReport(null);
    const home = preview?.home ?? shown;
    const homeLayout = preview ? preview.homeLayout : (summary?.layout ?? wantedLayout ?? null);
    setPreview({
      send: {
        change_set: p.change_set as ToolSend['change_set'],
        what: `Proposal ${p.proposal}`,
        sheets: p.sheets,
        template: { title, detail: '', path: '', sheets: p.sheets, edit: title },
      },
      home,
      homeLayout,
      key: previewKeys.current,
      picked: null,
      proposal: p,
    });
    if (upper(go) !== upper(shown)) {
      setWanted(go);
      setWantedLayout(null);
    }
    field.current?.blur();
    log(`Proposal ${p.proposal}, by ${whoWords(p.by)}: ${title}. Nothing has landed.`);
  }
  // Guides (#157). A guide runs on the page: its step's words in the
  // banner, the tool's button pulsing, the renderer's ring on the point its
  // next click wants, and each change set the drafter's tools send while
  // it runs marked with the step, so the core checks it and the journal
  // marks it. Nothing of the drafter's is undone unless they ask.
  function runGuide(): Guide | null {
    const r = runNow.current;
    return r ? (guides.find((g) => g.guide === r.guide) ?? null) : null;
  }
  /** The step running, on its own sheet as it is shown; null otherwise. */
  function guideStepHere() {
    const r = runNow.current;
    const g = runGuide();
    if (!r || !g || r.end || at !== null || !summary || upper(summary.sheet) !== upper(g.sheet)) return null;
    return g.steps[r.at] ?? null;
  }
  /** A tool started while a step runs: another than the step's gets a note. */
  function guideToolStarted(tool: string | null) {
    const step = guideStepHere();
    if (!step || !tool) return;
    const note = toolNote(step, tool, currentLayer);
    setRun((r) => (r ? { ...r, note } : r));
    if (note) log(`Guide: ${note.words}`);
  }
  /** A tool's change set, sent marked with the step running, when one runs on its sheet. */
  function sendGuided(s: ToolSend) {
    const sentRunId = runNow.current?.runId;
    const cs = guideMarked(s.change_set, runNow.current, runGuide(), summary?.sheet ?? null);
    if (cs === s.change_set) return sendChange(s);
    return sendChange({ ...s, change_set: cs }, undefined, (reply) => guideReplied(reply, sentRunId));
  }
  /** The core's check of a change set sent for a step: the guide moves on, or the note offers to undo it. */
  function guideReplied(reply: ChangeReply, sentRunId?: number) {
    if (sentRunId !== undefined && runNow.current?.runId !== sentRunId) return;
    const g = runGuide();
    const checked = reply.guide;
    if (!checked || !g) return;
    setRun((r) => (r ? guideLanded(r, g, checked, reply.journal_entry, sentRunId) : r));
    log(`Guide: ${checked.message}`);
  }
  function startGuide(n: number) {
    const g = guides.find((x) => x.guide === n);
    if (!g) return;
    if (preview) cancelPreview();
    if (leaveTool()) return;
    if (at !== null) backToLive();
    setPicked(null);
    // Its own Start goes; another guide's, given with it, stays.
    setToast((t) => toastAfterStart(t, n));
    const fresh = startRun(g);
    runNow.current = fresh;
    setRun(fresh);
    setGuideClosed(false);
    if (!summary || upper(summary.sheet) !== upper(g.sheet)) {
      setWanted(g.sheet);
      setWantedLayout(null);
    }
    if (!wide) snapSheet('peek');
    // The view comes to the part of the sheet the guide's points are in.
    const ats = g.steps.flatMap((s) => s.points.flatMap((p) => (p.at ? [p.at] : [])));
    if (ats.length) {
      const xs = ats.map((p) => p[0]);
      const ys = ats.map((p) => p[1]);
      // Room round them, as wide as the sheet's unit allows, and the banner above.
      const across = Math.max(...xs) - Math.min(...xs);
      const pad = Math.max(across * 0.35, 1e-6);
      keys.current += 1;
      setFocus({
        key: keys.current,
        sheet: g.sheet,
        places: [],
        box: [Math.min(...xs) - pad, Math.min(...ys) - pad, Math.max(...xs) + pad, Math.max(...ys) + pad * 1.6],
      });
    }
    log(`Guide started: ${g.title}`);
  }
  function skipGuideStep() {
    const r = runNow.current;
    const g = runGuide();
    if (!r || !g) return;
    // What the step's own tool had not finished on the guide's sheet is
    // dropped, not landed; a tool the drafter started for themselves, on
    // this sheet or another, keeps what it holds.
    const v = renderer();
    if (v && endsStepTool(g.steps[r.at], toolState?.tool, summary?.sheet, g.sheet)) toolDid(v.tool_drop());
    setRun(skipStep(r, g));
    log(`Guide: step ${r.at + 1} skipped`);
  }
  function stopGuide() {
    const r = runNow.current;
    if (!r) return;
    // What the step's own tool had not finished on the guide's sheet is
    // dropped, not landed; a tool the drafter started for themselves, on
    // this sheet or another, keeps what it holds.
    const v = renderer();
    const g = runGuide();
    if (v && endsStepTool(g?.steps[r.at], toolState?.tool, summary?.sheet, g?.sheet)) toolDid(v.tool_drop());
    setRun(stopRun(r));
    setGuideClosed(false);
    log(`Guide: stopped at step ${r.at + 1}`);
  }
  function closeGuide() {
    const r = runNow.current;
    setGuideClosed(true);
    if (r) {
      void call('end-guide', { set: source, guide: r.guide, by: DRAFTER }, token).catch(() => {});
    }
    log('Guide closed');
  }
  /** Do it for me: the step's operation lands as the drafter's would, marked with the step. */
  function doForMe() {
    const r = runNow.current;
    const g = runGuide();
    const step = guideStepHere();
    if (!r || !g || !step) return;
    // The tool's unfinished points are dropped, so only the step lands.
    const v = renderer();
    if (v && toolState?.tool) toolDid(v.tool_drop());
    const forMe = { ...r, forMe: true, note: null };
    runNow.current = forMe;
    setRun(forMe);
    if (step.setting) {
      // As the sheet names the layer, so the Layer box shows it.
      const want = step.setting.current_layer;
      chooseLayer(layers?.find((l) => upper(l.name) === upper(want))?.name ?? want);
      return;
    }
    if (!step.operation) return;
    // On the current layer and in the space shown, as the tools send theirs.
    const filled = v?.guide_operation(JSON.stringify(step.operation));
    const operation = filled ? (JSON.parse(filled) as Record<string, unknown>) : step.operation;
    setGuideBusy(true);
    const sentRunId = forMe.runId;
    void sendChange(
      {
        change_set: { operations: [operation], guide: { guide: r.guide, step: step.step } } as ToolSend['change_set'],
        what: `Guide step ${step.step}, done for you`,
        sheets: [g.sheet],
      },
      undefined,
      (reply) => guideReplied(reply, sentRunId),
    ).finally(() => setGuideBusy(false));
  }
  /** The note's button: undo the change set, take the point or the pick back, or end the other tool. */
  function guideNoteAct() {
    const act = runNow.current?.note?.act;
    if (!act) return;
    setRun((r) => (r ? { ...r, note: null } : r));
    if (act.kind === 'undo') {
      void undoIt(act.entry);
    } else if (act.kind === 'take_back') {
      const took = renderer()?.guide_take_back(act.what, act.place);
      refreshTool();
      log(took ? `Guide: the ${act.what} taken back, as you asked` : `Guide: the ${act.what} was no longer the last taken, so nothing was taken back`);
    } else {
      leaveTool();
    }
  }
  /** The change sets an undo can take now and the journal's newest entry, read fresh. */
  async function canUndoNow(): Promise<{ canUndo: number[]; newest: number }> {
    const fresh = await call<{ can_undo?: number[]; entries?: { entry: number }[] }>('journal', { set: source }, token);
    return {
      canUndo: fresh.can_undo ?? [],
      newest: fresh.entries?.at(-1)?.entry ?? 0,
    };
  }
  /**
   * Undo it, of a change set sent for a step that does not complete it:
   * an undo takes every later change set with it, so one with later edits
   * in place is not undone, and the note says so.
   */
  async function undoIt(entry: number) {
    setGuideBusy(true);
    try {
      const { canUndo, newest } = await canUndoNow();
      const blocked = undoItBlocked(entry, canUndo);
      if (blocked) {
        setRun((r) => (r ? { ...r, note: blocked } : r));
        log(`Guide: ${blocked.words}`);
        return;
      }
      await call('undo', { set: source, change_set: entry, newest_seen: newest }, token);
      log(`Guide: change set ${entry} undone, as you asked`);
    } catch (e) {
      say('warn', `Change set ${entry} was not undone`, why(e, token));
    } finally {
      setGuideBusy(false);
    }
  }
  /** Undo the guide's steps: one undo, of the first and every later one, while the button still says what it takes. */
  async function undoGuide(from: number, label: string) {
    const r = runNow.current;
    const g = runGuide();
    if (!r || !g) return;
    setGuideBusy(true);
    try {
      const { canUndo, newest } = await canUndoNow();
      const blocked = undoAllBlocked(r, g, canUndo, from, label);
      if (blocked) {
        say('warn', 'The guide’s steps were not undone', blocked);
        return;
      }
      const reply = await call<{ journal_entry?: number }>('undo', { set: source, change_set: from, newest_seen: newest }, token);
      setRun((r) => (r ? { ...r, undone: reply.journal_entry ?? 0 } : r));
      log(`Guide: its steps undone, from change set ${from}`);
    } catch (e) {
      say('warn', 'The guide’s steps were not undone', why(e, token));
    } finally {
      setGuideBusy(false);
    }
  }

  // Keep lands it as a change set, on the set as it stands; Discard ends
  // it, with a line in the journal. Either ends it if it is open.
  function keepProposal(p: Proposal) {
    // Keep and Discard act on the set as it is, never on the set as it was.
    if (at !== null) {
      log('Go back to live to keep or discard a proposal');
      return;
    }
    if (preview?.proposal) endPreview(preview.home, preview.homeLayout, preview.picked);
    setChipOpen(null);
    setProposalBusy(true);
    void sendChange({ change_set: p.change_set as ToolSend['change_set'], what: `Proposal ${p.proposal}`, sheets: p.sheets }, p.proposal)
      .then((landed) => (landed ? endedHere(p.proposal) : redrawProposal(p.proposal)))
      .finally(() => setProposalBusy(false));
  }
  // Kept or discarded: its card goes on the command's reply, not waiting
  // for the stream. Refused: what it was drawn as is old, so it is asked
  // for again, and its card says how it stands now.
  function endedHere(n: number) {
    setProposals((ps) => ps.filter((x) => x.proposal !== n));
  }
  function redrawProposal(n: number) {
    for (const key of [...ghostAsked.current]) if (key.startsWith(`${n}|`)) ghostAsked.current.delete(key);
    setGhostReplies((m) => new Map([...m].filter(([key]) => !key.startsWith(`${n}|`))));
    setGhostNonce((k) => k + 1);
  }
  function discardProposal(p: Proposal) {
    if (at !== null) {
      log('Go back to live to keep or discard a proposal');
      return;
    }
    if (preview?.proposal) endPreview(preview.home, preview.homeLayout, preview.picked);
    setChipOpen(null);
    setProposalBusy(true);
    call('discard', { set: source, proposal: p.proposal, by: DRAFTER }, token)
      .then(
        () => {
          endedHere(p.proposal);
          log(`Proposal ${p.proposal} discarded: nothing changed`);
        },
        (e) => say('warn', `Proposal ${p.proposal} was not discarded`, why(e, token)),
      )
      .finally(() => setProposalBusy(false));
  }
  const stepPreview = (by: number) => {
    if (!preview) return;
    const next = previewStep(preview.send.template.sheets, summary?.sheet ?? wanted ?? preview.home, by);
    setWanted(next);
    setWantedLayout(null);
  };
  useEffect(() => {
    const r = restorePick.current;
    if (!r || preview || !summary || summary.preview != null || summary.entry !== null) return;
    if (upper(summary.sheet) !== upper(r.home)) return;
    restorePick.current = null;
    // By its ID, followed onto the sheet as it is; gone, if it is gone.
    setPicked({ ...r.picked, summary });
  }, [summary, preview]);
  // The core's preview, from each reply of it; and the view comes to what
  // it marks on each sheet once, as it is first drawn.
  const previewFocused = useRef('');
  // Only the drawing asked for with this preview, once the renderer has
  // loaded it, says what this one would do: one opened while another's
  // drawing is still shown says nothing of it.
  const previewDrawn = !!preview && drawnFor(preview.key, reply, summaryText.current) && summary?.preview != null;
  useEffect(() => {
    if (!preview || !summary?.preview || !previewDrawn) return;
    setPreviewReport(summary.preview as PreviewReport);
    // Drawn again on the set as it now is: a reply of the same text as the
    // one shown is not drawn again, but it is as current.
    setPreviewGeneration((had) => reportGeneration(reply, summaryText.current, had));
    const k = `${preview.key}|${upper(summary.sheet)}`;
    if (previewFocused.current === k) return;
    previewFocused.current = k;
    keys.current += 1;
    setFocus({ key: keys.current, sheet: summary.sheet, places: [], preview: true });
  }, [preview, summary, reply, previewDrawn]);
  // Its words over the drawing: by each cloud, the check it would fail;
  // round a sheet that would refuse it, or be held back, the paper framed.
  const previewTags: PreviewTag[] = [];
  if (preview && previewReport && summary?.preview && previewDrawn) {
    for (const c of (summary.preview as PreviewReport).clouds ?? []) {
      previewTags.push({ text: `Would fail: ${c.rule}`, tone: 'bad', at: [c.bounds.min[0], c.bounds.min[1], c.bounds.max[0], c.bounds.max[1]] });
    }
    const v = previewVerdicts(previewReport).get(upper(summary.sheet));
    if (v?.state === 'refused') previewTags.push({ text: `Refused on ${summary.sheet}`, tone: 'bad', at: null });
    if (v?.state === 'held') previewTags.push({ text: `Held back on ${summary.sheet}`, tone: 'held', at: null });
  }
  const previewMarks = preview && previewReport ? previewVerdicts(previewReport) : null;

  // A proposal opened that has ended meanwhile, kept or discarded through
  // any interface, is closed: there is nothing more to keep.
  useEffect(() => {
    const open = preview?.proposal;
    if (!open || proposalsSeen.current === null) return;
    if (proposals.some((p) => p.proposal === open.proposal)) return;
    endPreview(preview.home, preview.homeLayout, preview.picked);
    log(`Proposal ${open.proposal} has ended: it is no longer waiting`);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [proposals]);
  // The proposal opened, as the stream says it now: what has landed since.
  const openProposalNow = preview?.proposal ? (proposals.find((p) => p.proposal === preview.proposal?.proposal) ?? preview.proposal) : null;

  // The proposals waiting on the sheet shown as it is (#155), each drawn
  // dashed with its card at the change, from `display-list` with its change
  // set, as a preview is drawn: asked for once for each state of the set.
  const ghostsOn = at === null && !preview && !onPreview && !!summary && summary.entry === null && summary.preview == null;
  const here = ghostsOn && summary ? onSheet(proposals, summary.sheet) : [];
  const ghostAt = `${epoch}:${versions[ANY] ?? 0}`;
  ghostGeneration.current = ghostAt;
  const ghostKey = (p: Proposal) => `${p.proposal}|${upper(summary?.sheet ?? '')}|${upper(summary?.layout ?? '')}|${ghostAt}`;
  const wantedGhosts = here.map(ghostKey).join(' ');
  useEffect(() => {
    if (!ghostsOn || !summary) return;
    // Only this state of the set's are kept.
    const keep = (m: ReadonlyMap<string, GhostReply>) => new Map([...m].filter(([k]) => k.endsWith(`|${ghostGeneration.current}`)));
    for (const p of here) {
      const key = ghostKey(p);
      if (ghostAsked.current.has(key)) continue;
      ghostAsked.current.add(key);
      const input: Record<string, unknown> = { source, sheet: summary.sheet, change_set: p.change_set, all_or_none: true };
      if (summary.layout) input.layout = summary.layout;
      const done = (got: GhostReply) => {
        if (!key.endsWith(`|${ghostGeneration.current}`)) return;
        setGhostReplies((m) => keep(m).set(key, got));
      };
      run('display-list', input, token).then(
        (text) => {
          let report: PreviewReport | null = null;
          try {
            report = (JSON.parse(text) as { result?: { preview?: PreviewReport } }).result?.preview ?? null;
          } catch {
            // Not read: drawn as it cannot be.
          }
          done({ text, report, error: report ? null : 'it cannot be drawn' });
        },
        (e) => done({ text: '', report: null, error: why(e, token) }),
      );
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ghostsOn, wantedGhosts, source, token, ghostNonce]);
  const ghostSpecs: GhostSpec[] = [];
  const cardViews: { p: Proposal; title: string; standing: Standing | null; lines: CardLine[] }[] = here.map((p) => {
    const got = ghostReplies.get(ghostKey(p));
    const report = got?.report ?? null;
    const st: Standing | null = got ? (report ? standing(report) : 'bad') : null;
    if (got?.text && report) ghostSpecs.push({ proposal: p.proposal, reply: got.text, refused: st === 'bad', key: ghostKey(p) });
    const lines = cardLines(p, report, journal);
    // One the core refuses whole cannot be drawn: its card says why, in its words.
    if (got && !report) lines.push({ tone: 'bad', icon: '✕', text: `Refused now: ${(got.error ?? '').replace(/`/g, '')}. Keep can’t land it.` });
    return { p, title: proposalTitle(p), standing: st, lines };
  });
  const proposalCards =
    cardViews.length > 0
      ? cardViews.map((c) => (
          <ProposalCard
            key={c.p.proposal}
            proposal={c.p}
            title={c.title}
            lines={c.lines}
            standing={c.standing}
            now={now}
            compact={!wide}
            open={chipOpen === c.p.proposal}
            onOpenChip={(o) => setChipOpen(o ? c.p.proposal : null)}
            onShow={() => openProposal(c.p, summary?.sheet)}
            onKeep={() => keepProposal(c.p)}
            onDiscard={() => discardProposal(c.p)}
            busy={proposalBusy || busy !== null || sending > 0}
          />
        ))
      : null;
  const cardsKey = `${cardViews.map((c) => `${c.p.proposal}:${c.standing}:${c.lines.length}`).join(',')}|${chipOpen}|${wide}`;
  const tabProposals = tabCounts(proposals);

  // The sheet's layers, for the Layer box: asked for each sheet, and again
  // when the box is opened, which takes in a layer a change set added. Not
  // after every change set, which would add a request to each one landing.
  const [layersAsked, setLayersAsked] = useState(0);
  const hasTemplate = !!summary?.template;
  // The sheet the layers held are of: asked again for the same sheet, as
  // when the box is reopened, they stay until the reply replaces them.
  const layersOf = useRef('');
  useEffect(() => {
    if (
      !summary ||
      summary.entry !== null ||
      (!hasTemplate && !isDwg) ||
      (isDwg && (readPending || (displaySource === source && damagedOriginal)))
    ) {
      // None held for a sheet as it is: the next ask starts from nothing.
      layersOf.current = '';
      setLayers([]);
      return;
    }
    // Another sheet's are still coming: the layer kept is the current one
    // until they come. The same sheet's stay as they are meanwhile.
    const of = `${displaySource}|${upper(summary.sheet)}`;
    const was = layersOf.current;
    layersOf.current = of;
    setLayers((held) => layersWhileAsked(was, of, held));
    let current = true;
    call<{ layers: { sheet: string; layers: LayerEntry[] }[] }>(
      'query',
      { source: displaySource, ...(isDwg ? {} : { sheet: summary.sheet }), query: { kind: 'layers' } },
      token,
    ).then(
      (r) => current && setLayers(r.layers[0]?.layers ?? []),
      () => current && setLayers([]),
    );
    return () => {
      current = false;
    };
    // The summary itself changes with each drawing: only what is asked by matters.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [summary?.sheet, summary?.entry, hasTemplate, isDwg, readPending, damagedOriginal, layersAsked, source, displaySource, token]);
  const currentLayer = currentLayerOf(keptLayer, layers);
  const layerEntry = layers?.find((l) => l.name === currentLayer);
  const chooseLayer = (name: string) => {
    setKeptLayer(name);
    writeCurrentLayer(keeps, source, name);
    log(`Current layer: ${name}`);
    // A setting step that makes a layer current is done when it is the one
    // the step wants, and says so when it is not.
    const step = guideStepHere();
    const r = runNow.current;
    const g = runGuide();
    if (!step || !r || !g || !step.setting) return;
    const chosen = layerChosen(step, name);
    if (chosen.done) {
      setRun(stepDone(r, g));
      log(`Guide: step ${r.at + 1} done`);
    } else setRun({ ...r, note: chosen.note });
  };
  const toolLayer = useMemo(() => ({ name: currentLayer, color: colorIndex(layerEntry?.color) }), [currentLayer, layerEntry?.color]);
  // The prompt shows the current layer a tool draws on.
  useEffect(() => {
    refreshTool();
  }, [toolLayer, refreshTool]);

  // The cursor's place, written straight into the status bar, so a moving
  // mouse redraws nothing else.
  const unitsNow = useRef<string | null>(null);
  unitsNow.current = summary?.units ?? null;
  // A snap held shows its point exactly, with its name.
  const cursorAt = useCallback((p: [number, number] | null, snapped: Hovered | null) => {
    const el = coords.current;
    if (!el) return;
    if (!p) {
      el.textContent = '\u00a0';
      return;
    }
    if (!snapped) {
      el.textContent = coordinates(p[0], p[1], unitsNow.current);
      return;
    }
    const label = document.createElement('span');
    label.className = 'snapw';
    label.textContent = snapLabel(snapped.snap);
    el.replaceChildren(coordinates(snapped.at[0], snapped.at[1], unitsNow.current), label);
  }, []);

  // What the command bar and a phone's peek cover of the drawing: from the
  // command bar's top down, the bar taken as one line tall, so a prompt
  // that wraps grows over the drawing without moving the fitted sheet. A
  // preview's band covers from its own top, so what it shows is never
  // under it; on a phone the band is the bar.
  const previewShown = !!preview;
  useLayoutEffect(() => {
    const vp = viewport.current;
    const b = bar.current;
    if (!vp || !b) return;
    // On a phone, Array's panel docks above the bar, and covers from its top.
    const band = vp.querySelector('[data-preview]') ?? (wide ? null : vp.querySelector('[data-arraypanel]'));
    const measure = () => {
      const bottom = vp.getBoundingClientRect().bottom;
      const covered = band ? bottom - band.getBoundingClientRect().top : bottom - b.getBoundingClientRect().bottom + BAR_LINE;
      setCovered(Math.max(0, Math.round(covered)));
      // The bar's real height, a wrapped prompt's among it, which quick
      // properties on a phone keep clear of.
      vp.style.setProperty('--cmd', `${b.offsetHeight}px`);
      setBarHeight(b.offsetHeight);
    };
    measure();
    const resized = new ResizeObserver(measure);
    resized.observe(vp);
    resized.observe(b);
    if (band) resized.observe(band);
    return () => resized.disconnect();
  }, [peekHeight, wide, previewShown, !!toolState?.array]);

  // What the header and the tabs say.
  const pending = journal ? notIssued(journal) : [];
  const lastAccept = journal ? [...journal.entries].reverse().find((e) => e.kind === 'accept') : undefined;
  const marked = new Set((journal?.inconsistent?.failures ?? []).map((f) => upper(f.sheet)));
  const failing = new Set([...findings].filter(([, f]) => f.length).map(([s]) => s));
  const failCount = [...findings.values()].reduce((n, f) => n + f.length, 0);
  const changedSheets = new Set(pending.flatMap((e) => e.landed_on ?? []).map(upper));
  // A drafter's own edit, landed but not yet in the journal the page holds,
  // is the newest an undo can take: Undo last takes it, not the one before
  // it, which would undo both.
  const undoable = ownLatest !== null && (!journal || newest(journal) < ownLatest) ? ownLatest : journal ? lastUndoable(journal) : null;
  // Undo last and Accept wait while anything is busy, or any change set this page sent has not replied.
  const busyNow = busy ?? (sending > 0 ? 'change' : null);
  const blocked = !!journal && (!!journal.inconsistent || (!!lastAccept && pending.length === 0) || (isDwg && pending.length === 0));

  const sheets = summary?.sheets ?? [];
  const name = summary?.name ?? '';
  const object = pickedPlace !== null ? summary?.objects[pickedPlace] : undefined;
  // Quick properties' fields, as the renderer gives them from the sheet it
  // holds: what each shows, whether it can be edited, and where its value
  // is written.
  // They edit only while the drawing on the canvas is the view asked for
  // and the set as it is: a step back to an entry whose drawing is still
  // coming leaves the live drawing there, which is not edited then.
  const editable =
    !editingDisabled &&
    editsHere(
      { at, sheet: isDwg ? null : wanted, layout: wantedLayout },
      summary ? { at: summary.entry, sheet: summary.sheet, layout: summary.layout, layouts: summary.layouts } : null,
    );
  const fieldsOf = (place: number | null): PropField[] => {
    const v = renderer();
    if (!v || place === null || !summary) return [];
    const given = JSON.parse(v.props_fields(place)) as PropField[];
    return editable ? given : given.map((f) => ({ ...f, editable: false, note: undefined }));
  };
  const fields = fieldsOf(pickedPlace);
  // An edit in quick properties, on the drawing or in the Layer box: the
  // renderer gives its change set, which is sent as a tool's is, asked
  // about first when it edits the template.
  const editProp = (place: number, key: string, value: string): Committed => {
    const v = renderer();
    if (!v) return { kind: 'same' };
    if (!editable) {
      const why = 'The drawing shown is not the set as it is: nothing is edited until it is.';
      log(why);
      return { kind: 'error', message: why };
    }
    const done = JSON.parse(v.props_edit(place, key, value)) as PropEdited;
    if (done.error) {
      log(done.error);
      return { kind: 'error', message: done.error };
    }
    if (!done.send) return { kind: 'same' };
    if (done.send.template) startPreview(done.send);
    else void sendChange(done.send).then((ok) => ok || setPropsReset((n) => n + 1));
    return { kind: 'sent' };
  };
  const commitProp = (key: string, value: string): Committed => (pickedPlace === null ? { kind: 'same' } : editProp(pickedPlace, key, value));
  // Why the renderer would refuse a field's value, sending nothing: for a paste.
  const refuseProp = (key: string, value: string): string | null => {
    const v = renderer();
    if (!v || pickedPlace === null) return null;
    return (JSON.parse(v.props_edit(pickedPlace, key, value)) as PropEdited).error ?? null;
  };
  // A double click on a text, or a tap on the text already selected on a
  // phone, edits its contents on the drawing.
  const openText = (place: number, touch: boolean): boolean => {
    const contents = fieldsOf(place).find((f) => f.key === 'contents');
    if (!contents?.editable) return false;
    if (place !== pickedPlace) pick(place);
    setEditing({ place, touch });
    const o = summary?.objects[place];
    if (o) log(`Editing ${nameOf(o)}’s text on the drawing`);
    drawing.current?.redraw();
    return true;
  };
  const editingField = editing !== null && editing.place === pickedPlace ? fields.find((f) => f.key === 'contents') : undefined;
  // Another object picked, or another sheet, ends the text's editing.
  useEffect(() => {
    if (editing && (pickedPlace !== editing.place || at !== null)) setEditing(null);
  }, [editing, pickedPlace, at]);
  const textDone = (text: string) => {
    const place = editing?.place ?? null;
    setEditing(null);
    if (place === null) return;
    const done = editProp(place, 'contents', text);
    if (done.kind === 'error') say('warn', 'The text was not changed', done.message);
  };
  const textCancelled = () => {
    setEditing(null);
    log('Text editing cancelled, nothing changed');
  };
  // Why the renderer would refuse a text's contents, sending nothing: for
  // line breaks pasted into one line, which the browser would strip.
  const refuseText = (text: string): string | null => {
    const v = renderer();
    if (!v || editing === null) return null;
    return (JSON.parse(v.props_edit(editing.place, 'contents', text)) as PropEdited).error ?? null;
  };
  const { state: liveState, line } = liveLine(at, !!noJournal, stream);
  const prompt = promptFor({
    tool: toolState,
    at,
    cannot: status.kind === 'cannot',
    reaching: !noJournal && stream !== 'open',
    selected: object ?? null,
    taken: taken?.taken ?? null,
    touch: !wide,
    editing: editingField && object ? object : null,
    previewing: !!preview,
    proposal: !!preview?.proposal,
  });
  const act = (a: Act) => {
    if (a === 'clear') pick(null);
    else if (a === 'copy') void copySnap();
    else if (a === 'older') step(-1);
    else if (a === 'newer') step(1);
    else if (a === 'live') backToLive();
    else if (a === 'move' || a === 'erase') startTool(a);
    else if (a === 'copy-objects') startTool('copy');
    else if (a === 'array-objects') startTool('array');
    else if (a.startsWith('tool:')) {
      const v = renderer();
      if (!v) return;
      // The field's words go with Done, as Enter takes them.
      const typedText = field.current?.value ?? '';
      if (a === 'tool:done' && toolState?.typing_text && typedText) {
        if (field.current) field.current.value = '';
        setTyped('');
        toolDid(v.tool_input(typedText));
        toolDid(v.tool_input(''));
        return;
      }
      if (a === 'tool:done' || a === 'tool:cancel') {
        if (field.current) field.current.value = '';
        setTyped('');
      }
      toolDid(v.tool_option(a.slice(5)));
    }
  };

  // The assistant (#156). The agents `serve` found on the PATH when it
  // started, asked for once; Look again asks it to look again.
  useEffect(() => {
    let current = true;
    assistantCall<AgentsReport>('', null, token).then(
      (report) => {
        if (!current) return;
        setAgents(report);
        setAgentsDown(null);
      },
      (e) => current && setAgentsDown(`The assistant can’t be reached: ${why(e, token)}`),
    );
    return () => {
      current = false;
    };
  }, [token]);
  // The conversation, from its stream: each event holds the whole of it.
  // The assistant works on a drawing set, which has a journal.
  const isSet = sourceKind === 'set' && !!journal;
  useEffect(() => {
    if (!isSet) return;
    return watchAssistant(
      source,
      token,
      (text) => {
        const replied = JSON.parse(text) as { ok: boolean; result?: Conversation; error?: { message?: string } };
        if (!replied.ok || !replied.result) {
          setConversationDown(`The assistant can’t be reached here: ${replied.error?.message ?? 'the server said nothing of it'}`);
          return;
        }
        setConversationDown(null);
        setConversation(replied.result);
      },
      setAssistantStream,
    );
  }, [source, token, isSet]);
  const unreachable =
    agentsDown ??
    (sourceKind !== 'set' || noJournal ? 'The assistant works on a drawing set’s folder, and this is not one.' : null) ??
    conversationDown ??
    (assistantStream === 'refused' ? 'The assistant can’t be reached here: it works on a drawing set, in the viewer opened from the address agentdrafter serve printed.' : null);
  const runningAgent = agentName(agents, conversation?.agent ?? null);
  // What the drafter is looking at, which the next message carries: the
  // sheet shown, its layout, and the object selected, by its ID.
  const look = lookingAtOf(summary?.sheet ?? null, summary?.layout ?? null, object?.id ? { id: object.id, name: nameOf(object) } : null);
  // The same, recorded in the core with `look-at` for the agent to read
  // through `looking-at`: on a drawing set's sheet as it is, once it is
  // drawn and each time it changes, a moment after. A failure says nothing.
  const recordable = isSet && at === null && !preview && !onPreview && !!summary && summary.entry === null && summary.preview == null;
  const lookToRecord = recordable ? look : null;
  const lookToRecordKey = lookKey(lookToRecord);
  const lookRecorded = useRef('');
  useEffect(() => {
    if (!lookToRecord || lookToRecordKey === lookRecorded.current) return;
    const soon = setTimeout(() => {
      lookRecorded.current = lookToRecordKey;
      call('look-at', { set: source, sheet: lookToRecord.sheet, layout: lookToRecord.layout, selected: lookToRecord.selected }, token).catch((e: unknown) => {
        lookRecorded.current = '';
        console.warn(`What you are looking at was not recorded: ${why(e, token)}`);
      });
    }, 300);
    return () => clearTimeout(soon);
    // The key says what it holds.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lookToRecordKey, source, token]);
  /**
   * Sends the drafter's message to the assistant, carrying what they look
   * at unless × left it off; false when it did not go.
   */
  const ask = async (text: string, via: 'chat' | 'command_bar'): Promise<boolean> => {
    if (!runningAgent) {
      setSide('chat');
      setRaise((r) => r + 1);
      say('warn', 'No agent to ask', 'Install Claude Code or Codex first: the panel says how.');
      return false;
    }
    if (conversation?.busy) {
      say('info', `${runningAgent} is still working`, 'It answers one message at a time.');
      return false;
    }
    try {
      await assistantCall('messages', { set: source, text, via, looking_at: chipOff ? null : look }, token);
      setChipOff(false);
      // On a phone the sheet comes down to half, its keyboard gone, so the
      // reply and the drawing both show.
      if (!wide) snapSheet('half');
      return true;
    } catch (e) {
      say('warn', 'Not sent to the assistant', notSentWords(e instanceof CommandError ? e.code : '', why(e, token), runningAgent));
      return false;
    }
  };
  // Words from the command bar: the chat opens, raised to half on a phone, and they go.
  function askFromBar(text: string) {
    setSide('chat');
    setRaise((r) => r + 1);
    log(`To the assistant: “${text}”`);
    void ask(text, 'command_bar').then((sent) => {
      // What did not go stays in the bar, as the chat keeps its draft.
      const f = field.current;
      if (!sent && f && !f.value) {
        f.value = text;
        setTyped(text);
      }
      // Taken: on a phone the bar's field lets go, as the chat's input
      // does, so the keyboard goes down and the reply shows.
      if (sent && !wide) {
        f?.blur();
        setFieldShown(false);
      }
    });
  }
  const pickAgent = (agent: string) => {
    assistantCall('agent', { set: source, agent }, token).catch((e: unknown) =>
      say('warn', 'The agent was not changed', notSentWords(e instanceof CommandError ? e.code : '', why(e, token), runningAgent)),
    );
  };
  const lookAgain = () => {
    setLookingAgain(true);
    assistantCall<AgentsReport>('find', {}, token)
      .then(
        (report) => {
          setAgents(report);
          setAgentsDown(null);
        },
        (e: unknown) => say('warn', 'Could not look again', why(e, token)),
      )
      .finally(() => setLookingAgain(false));
  };
  // A proposal's card in the chat says how it stands as its card on the
  // drawing does, from the same preview, when it reaches the sheet shown.
  const chatCard = (p: Proposal): ChatCardView => {
    const c = cardViews.find((v) => v.p.proposal === p.proposal);
    const stepping = at !== null;
    if (c) {
      const checking = c.standing === null;
      return { standing: c.standing, lines: c.lines, checking, acts: chatCardActs({ stepping, drawn: true, checking, refused: c.standing === 'bad' }) };
    }
    return { standing: null, lines: cardLines(p, null, journal), checking: false, acts: chatCardActs({ stepping, drawn: false, checking: false, refused: false }) };
  };

  const plotPdf = async () => {
    setPlotting(true);
    try {
      const plotted = await call<{ output?: string; message?: string }>('plot', { source: displaySource, overwrite: true }, token);
      say('ok', 'Plotted PDF', plotted.output ? `Wrote ${plotted.output}` : (plotted.message ?? 'Exact-size PDF written.'));
    } catch (e) {
      say('warn', 'Could not plot PDF', why(e, token));
    } finally {
      setPlotting(false);
    }
  };

  const undoLabel = `Undo last${undoable ? ` (#${undoable})` : ''}`;
  const acceptLabel = busy === 'accept' ? 'Checking…' : 'Accept…';
  const acceptDisabled = editingDisabled || !journal || busyNow !== null || at !== null;
  const peek = wide ? null : (
    <Peek
      line={line}
      live={liveState}
      journal={journal}
      failing={failing}
      undoable={undoable}
      blocked={blocked || editingDisabled}
      busy={busyNow}
      stepping={at !== null || editingDisabled || undoLocked}
      onShow={(n) => showEntry(n)}
      onUndo={() => void undo(undoable)}
      onAccept={() => void askAccept()}
      ask={{
        words: peekAskWords(runningAgent, !!conversation?.busy, !!agents, unavailableAgents(agents).length > 0),
        agent: runningAgent,
        none: !!agents && !runningAgent,
        onAsk: () => {
          setSide('chat');
          setRaise((r) => r + 1);
        },
      }}
      proposals={
        proposals.length && at === null && !preview
          ? {
              count: proposals.length,
              words: peekWords(here.length, proposals.length),
              onOpen: () => {
                const first = here[0] ?? proposals[0];
                if (first) openProposal(first, summary?.sheet);
              },
            }
          : null
      }
    />
  );
  const journalBody = journal ? (
    <JournalList
      journal={journal}
      failing={failing}
      at={at}
      focused={focused}
      busy={busyNow}
      undoDisabled={undoLocked}
      onStep={step}
      onShow={(n) => showEntry(n)}
      onUndo={(n) => void undo(n)}
    />
  ) : (
    <>
      <div className="va-jhead">
        <b>Journal</b>
      </div>
      <p className="muted">{noJournal ? `There is no journal here: ${noJournal}.` : 'Reading the journal…'}</p>
    </>
  );
  // The assistant's panel: its Chat, and the Journal beside it.
  const body = (
    <AssistantPanel
      source={source}
      agents={agents}
      unreachable={unreachable}
      conversation={conversation}
      look={look}
      chipOff={chipOff}
      onChipOff={setChipOff}
      side={side}
      onSide={setSide}
      journal={journal}
      journalBody={journalBody}
      proposals={proposals}
      cardView={chatCard}
      proposalBusy={proposalBusy || busy !== null || sending > 0}
      onOpenProposal={(p) => {
        // On a phone the sheet goes down to its peek, so the band and the drawing show.
        if (!wide) snapSheet('peek');
        openProposal(p, summary?.sheet);
      }}
      onKeep={keepProposal}
      onDiscard={discardProposal}
      onSend={(text) => ask(text, 'chat')}
      onPickAgent={pickAgent}
      onLookAgain={lookAgain}
      lookingAgain={lookingAgain}
      onInputFocus={() => {
        if (!wide) snapSheet('full');
      }}
      guideCard={(n) => (
        <GuideCard
          guide={guides.find((g) => g.guide === n) ?? null}
          number={n}
          run={guideRun}
          onStart={() => startGuide(n)}
          onStop={stopGuide}
        />
      )}
    />
  );

  const layoutTabs =
    (summary?.layouts.length ?? 0) > 1 ? (
      // A sheet drawn in several layouts shows each as a tab of its own, as
      // AutoCAD's layout tabs are: at the left of the status bar on a
      // laptop, under the sheets' tabs on a phone, which has no status bar.
      <nav className="va-tabs va-layouts" aria-label="Layouts">
        {summary?.layouts.map((l) => (
          <button
            key={l}
            type="button"
            className={`tab${l === summary?.layout ? ' on' : ''}`}
            aria-current={l === summary?.layout ? 'page' : undefined}
            onClick={() => {
              if (leaveToolFor(summary?.sheet ?? '', l)) return;
              setPicked(null);
              setWantedLayout(l);
            }}
          >
            <b>{l}</b>
          </button>
        ))}
      </nav>
    ) : null;

  const pills =
    journal && at !== null ? (
      // Stepping back: the chip says which entry is shown, and goes back to live.
      <button type="button" className="pill past" data-act="live" onClick={backToLive}>
        Entry {at} of {newest(journal)} &middot; Live &rsaquo;
      </button>
    ) : isDwg && readReport ? (
      <>
        <span className="pill dk-pill-ro" data-dwg-version>
          {readReport.dwg_version} &middot; original read-only
        </span>
        {!readReport.decoded ? (
          <button
            type="button"
            className="pill warn dk-pill-warnbtn"
            data-act="dwg-loss-pill"
            onClick={() => setDecodeModalOpen(true)}
          >
            {allowLoss ? 'allow-loss' : 'view-only'}
          </button>
        ) : lastAccept && pending.length === 0 ? (
          <span className="pill dk-pill-ok">Issued (entry {lastAccept.entry})</span>
        ) : pending.length > 0 ? (
          <span className="pill">{pending.length} not issued</span>
        ) : (
          <span className="pill dk-pill-ok">decoded whole</span>
        )}
        {failCount > 0 && <span className="pill bad">{plural(failCount, 'failed check')}</span>}
      </>
    ) : (
      journal && (
        <>
          <span className="pill">
            {pending.length ? (
              <>
                <span className="long">{'Working revision · '}</span>
                {pending.length} not issued
              </>
            ) : lastAccept ? (
              `Issued (entry ${lastAccept.entry})`
            ) : (
              'Working revision, not issued'
            )}
          </span>
          {journal.inconsistent && (
            <span className="pill warn">
              Inconsistent<span className="long"> set</span>
            </span>
          )}
          {failCount > 0 && <span className="pill bad">{plural(failCount, 'failed check')}</span>}
        </>
      )
    );

  const issuedSheets = asking?.report.sheets ?? [];
  const askedFindings = asking?.report.checks.findings ?? [];
  // The guide running, its step on its own sheet as it is, and what the
  // step wants next, as the renderer says it (#157).
  const runningGuide = guideRun ? (guides.find((g) => g.guide === guideRun.guide) ?? null) : null;
  const bannerShown = !!guideRun && !!runningGuide && !guideClosed && at === null && !preview;
  const guideElsewhere = !!runningGuide && !!summary && upper(summary.sheet) !== upper(runningGuide.sheet);
  const stepNow = bannerShown && !guideRun?.end && !guideElsewhere && runningGuide ? (runningGuide.steps[guideRun!.at] ?? null) : null;
  const phaseText = stepNow && !stepNow.setting ? (renderer()?.guide_phase() ?? '') : '';
  const phase: Phase | null = phaseText ? (JSON.parse(phaseText) as Phase) : null;
  const gpulse = pulseOf(stepNow, phase, guideRun?.note?.tone === 'warn');
  // The renderer is told the step running, as `guides` works it out on the
  // set as it stands, so it can say what the step wants and ring its point;
  // and the drawing, how far down the banner reaches.
  const stepJson = stepNow && !stepNow.setting ? JSON.stringify(stepNow) : '';
  useEffect(() => {
    const v = renderer();
    if (!v) return;
    v.guide_step(stepJson);
    refreshTool();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stepJson, summary, refreshTool]);
  useLayoutEffect(() => {
    const h = bannerShown ? (guideBanner.current?.offsetHeight ?? 0) + 8 : 0;
    if (h !== bannerHeight) setBannerHeight(h);
  });
  return (
    <div
      className={`va${toolsOpen ? ' tools-open' : ''}${desktop ? ' dk-mode' : ''}`}
      style={{ ['--peek' as string]: `${peekHeight}px`, ['--gtop' as string]: `${bannerHeight}px` }}
      data-gpulse={gpulse}
    >
      {/* The title bar, and under it the ribbon, which a phone folds behind Tools and opens over the drawing, under the title bar however tall its chips make it. */}
      <div className="topbar">
      <header className="titlebar">
        <svg className="mark" viewBox="0 0 20 20" aria-hidden="true">
          <path d="M3 17V4l13 13z" fill="none" stroke="#4aa3ff" strokeWidth="2" strokeLinejoin="round" />
          <path d="M7 14v-3l3 3z" fill="#4aa3ff" />
        </svg>
        {wide ? (
          <>
            <span className="appname">AgentDrafter</span>
            {desktop && (
              <button
                type="button"
                className="qa dk-openbtn"
                data-act="desktop-open"
                title="Switch drawing or open another (Ctrl+O)"
                onClick={onOpenSwitcher}
              >
                Open&#8230;
              </button>
            )}
            <button
              type="button"
              className="qa"
              data-act="undo"
              disabled={!undoable || busyNow !== null || at !== null || undoLocked}
              title={
                undoLocked
                  ? 'View-only (ADR-0004): inspect the decode report to allow revising or undoing'
                  : at !== null
                    ? 'Off while you look back at an entry'
                    : undoable
                      ? `Undo change set ${undoable}`
                      : 'Nothing an undo can take'
              }
              onClick={() => void undo(undoable)}
            >
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                <path d="M3 6h6.5a3.5 3.5 0 010 7H6M3 6l3-3M3 6l3 3" />
              </svg>
              {undoLabel}
            </button>
            {desktop ? (
              <div className="dk-switcher">
                <button
                  type="button"
                  className="dk-switchbtn"
                  data-act="desktop-switcher"
                  title="Switch open drawing or return to Start (Ctrl+O)"
                  onClick={onOpenSwitcher}
                >
                  <span className={`dk-kbadge ${sourceKind}`}>{kindBadge(sourceKind)}</span>
                  <span className="sname">{name || sourceDisplayName(source)}</span>
                  <span className="spath">{source}</span>
                  <span className="caret" aria-hidden="true">
                    &#9662;
                  </span>
                </button>
              </div>
            ) : (
              <span className="file">
                {name}
                {summary ? ` / ${summary.sheet}` : ''}
              </span>
            )}
          </>
        ) : (
          <>
            {desktop && (
              <button
                type="button"
                className="qa dk-openbtn"
                data-act="desktop-open"
                title="Switch drawing or open another"
                onClick={onOpenSwitcher}
              >
                Open&#8230;
              </button>
            )}
            <button
              type="button"
              className="qa toolsbtn"
              aria-expanded={toolsOpen}
              aria-controls="ribbon"
              onClick={() => setToolsOpen((o) => !o)}
            >
              Tools &#9662;
            </button>
            {desktop && (
              <div className="dk-switcher">
                <button
                  type="button"
                  className="dk-switchbtn"
                  data-act="desktop-switcher"
                  title="Switch open drawing or return to Start"
                  onClick={onOpenSwitcher}
                >
                  <span className={`dk-kbadge ${sourceKind}`}>{kindBadge(sourceKind)}</span>
                  <span className="sname">{name || sourceDisplayName(source)}</span>
                  <span className="caret" aria-hidden="true">
                    &#9662;
                  </span>
                </button>
              </div>
            )}
          </>
        )}
        <div className="va-status">{pills}</div>
        {wide && (
          <button
            type="button"
            className={`btn primary${blocked ? ' blocked' : ''}`}
            data-act="accept"
            disabled={acceptDisabled}
            onClick={() => void askAccept()}
          >
            {acceptLabel}
          </button>
        )}
      </header>
      <div id="ribbon" className={`ribbonwrap${wide || toolsOpen ? ' open' : ''}`}>
        <Ribbon
          folded={!wide}
          disabled={editingDisabled}
          toggles={toggles}
          onToggle={toggle}
          onTool={ribbonTool}
          running={toolState?.tool ?? null}
          layers={layers ?? []}
          current={currentLayer}
          onLayer={chooseLayer}
          onLayersOpen={() => setLayersAsked((n) => n + 1)}
          swatch={(l) => screen_color(colorIndex(l?.color))}
          selected={
            object
              ? {
                  name: object.layer,
                  color: pickedColor,
                  movable: !!fields.find((f) => f.key === 'layer')?.editable,
                  label: nameOf(object),
                }
              : null
          }
          onMoveTo={(name) => {
            if (pickedPlace === null) return;
            const done = editProp(pickedPlace, 'layer', name);
            if (done.kind === 'error') say('warn', 'The layer was not changed', done.message);
          }}
          onSnaps={() => openSnapMenu(null)}
        />
      </div>
      </div>
      <nav className="va-tabs sheettabs" ref={tabs} aria-label="Sheets">
        {sheets.map((s) => {
          // In a preview, each sheet it reaches says what would become of
          // it, and a sheet it does not reach is greyed and stays shut.
          const reaches = !preview || preview.send.template.sheets.some((x) => upper(x) === upper(s));
          const verdict = previewMarks?.get(upper(s));
          const dot = preview ? (
            verdict ? <VerdictMark state={verdict.state} /> : null
          ) : marked.has(upper(s)) ? (
            <i className="dot warn" title="Inconsistent: a change set failed here">
              !
            </i>
          ) : failing.has(upper(s)) ? (
            <i className="dot bad" title="Fails a check" />
          ) : changedSheets.has(upper(s)) ? (
            <i className="dot info" title="Has change sets not issued" />
          ) : null;
          return (
            <button
              key={s}
              type="button"
              className={`tab${s === summary?.sheet ? ' on' : ''}${reaches ? '' : ' pvoff'}`}
              aria-current={s === summary?.sheet ? 'page' : undefined}
              aria-disabled={reaches ? undefined : true}
              title={reaches ? undefined : 'The change does not reach this sheet'}
              onClick={() => {
                if (!reaches) return;
                if (leaveToolFor(s, null)) return;
                setPicked(null);
                setWanted(s);
                setWantedLayout(null);
              }}
            >
              <b>{s}</b>
              {dot}
              {!preview && (tabProposals.get(upper(s)) ?? 0) > 0 && (
                <span className="pcount" data-pcount={tabProposals.get(upper(s))} title={`${tabProposals.get(upper(s))} waiting proposals reach it`}>
                  {tabProposals.get(upper(s))}
                </span>
              )}
            </button>
          );
        })}
      </nav>
      {!wide && layoutTabs}
      {(isDwg || (isSheet && desktop)) && (
        <DwgBanner
          kind={sourceKind}
          source={source}
          guard={editGuard}
          readReport={readReport}
          readError={readError}
          latestRevisionFile={activeRev?.file ?? (ownRevFresh ? sourceDisplayName(ownRevFresh.path) : null)}
          plotting={plotting}
          onInspectDecode={() => setDecodeModalOpen(true)}
          onPlotPdf={() => void plotPdf()}
        />
      )}
      <div className="va-main">
        <div
          className={`va-vp${at !== null ? ' past' : ''}`}
          ref={viewport}
          onPointerDownCapture={() => toolsOpen && setToolsOpen(false)}
        >
          {bannerShown && guideRun && runningGuide && (
            <div className="gwrap" ref={guideBanner}>
              <GuideBanner
                guide={runningGuide}
                run={guideRun}
                phase={phase}
                phone={!wide}
                elsewhere={guideElsewhere}
                canUndo={journal?.can_undo ?? []}
                canDoForMe={!!stepNow && (!!stepNow.setting || !!stepNow.operation)}
                busy={guideBusy || sending > 0}
                onDoForMe={doForMe}
                onSkip={skipGuideStep}
                onStop={stopGuide}
                onNoteAct={guideNoteAct}
                onBack={() => {
                  if (leaveToolFor(runningGuide.sheet, null)) return;
                  setWanted(runningGuide.sheet);
                  setWantedLayout(null);
                }}
                onUndoAll={(from, label) => void undoGuide(from, label)}
                onClose={closeGuide}
              />
            </div>
          )}
          <Drawing
            desktop={desktop}
            guide={stepNow && guideRun ? { step: guideRun.at + 1, top: bannerHeight } : null}
            guideSee={stepNow && guideRun ? `${guideRun.guide}:${guideRun.at}:${summary?.sheet ?? ''}:${stepNow.points.map((p) => (p.at ? p.at.join(',') : (p.object ?? ''))).join(';')}` : ''}
            reply={reply?.text ?? null}
            coveredBelow={covered}
            compact={!wide}
            grid={toggles.grid}
            card={card}
            marks={onPreview ? [] : marks}
            picked={onPreview ? null : pickedPlace}
            pulseSince={pulseSince}
            focus={focus}
            onLoaded={loaded}
            onError={failed}
            onUnavailable={setUnavailable}
            onPick={onPreview || arrayUp ? () => {} : pick}
            arrayPanel={arrayBox}
            onCursor={cursorAt}
            snapMask={runningMask(toggles.osnap && at === null && !onPreview, running)}
            onTakeSnap={takeSnap}
            onNoSnap={noSnap}
            space={`${sheetCovers}:${barHeight}`}
            onPickedColor={setPickedColor}
            tool={toolState}
            onToolOutcome={toolDid}
            onToolEnter={rightClickFinish}
            ortho={toggles.ortho}
            layer={toolLayer}
            handle={drawing}
            editor={editor}
            onOpen={onPreview || arrayUp ? () => false : openText}
            previewTags={previewTags}
            ghosts={ghostsOn ? ghostSpecs : []}
            cards={ghostsOn ? proposalCards : null}
            cardsKey={ghostsOn ? cardsKey : ''}
          />
          {preview && summary?.preview != null && (
            <div className={`pvhead${preview.proposal ? ' prop' : ''}`} data-pvhead>
              {summary.sheet} {preview.proposal ? `as proposal ${preview.proposal.proposal} would leave it` : 'as it would be'} &middot; nothing has landed
            </div>
          )}
          <div className="va-overlays">
            {toast && (
              <div key={toast.key} className={`va-toast in ${toast.tone}`} role="status">
                <span className={`dot ${toast.tone}`} />
                <div>
                  <b>{toast.title}</b>
                  <small>{toast.detail}</small>
                </div>
                {toast.proposal !== undefined && proposals.some((p) => p.proposal === toast.proposal) && (
                  <button
                    type="button"
                    className="btn sm"
                    data-toast="show-proposal"
                    onClick={() => {
                      const p = proposals.find((x) => x.proposal === toast.proposal);
                      setToast(null);
                      if (p) openProposal(p);
                    }}
                  >
                    Show
                  </button>
                )}
                {(toast.guides ?? [])
                  .filter((n) => guides.some((g) => g.guide === n))
                  .map((n, _, all) => (
                    <button key={n} type="button" className="btn sm guide" data-toast="start-guide" data-guide-n={n} onClick={() => startGuide(n)}>
                      {all.length > 1 ? `Start guide ${n}` : 'Start'}
                    </button>
                  ))}
                {toast.entry !== undefined && journal?.entries.some((e) => e.entry === toast.entry && e.kind === 'change_set') && (
                  <button
                    type="button"
                    className="btn sm"
                    onClick={() => {
                      const n = toast.entry as number;
                      setToast(null);
                      const entry = journal.entries.find((e) => e.entry === n);
                      showEntry(n, !!entry?.undone_by);
                    }}
                  >
                    Show
                  </button>
                )}
              </div>
            )}
          </div>
          {toolState?.typing_text && !wide && (
            <TextBox
              height={renderer()?.tool_text_height() ?? 0.12}
              unit={summary?.units ?? ''}
              onTyping={(t) => {
                renderer()?.tool_typing(t);
                drawing.current?.redraw();
              }}
              onAdd={(t, h) => toolDid(renderer()?.tool_text_box(t, h) ?? '')}
              onCancel={() => toolDid(renderer()?.tool_option('cancel') ?? '')}
            />
          )}
          {/* A phone's text box takes the card's place while a text is edited in it. */}
          {object && summary && !(editingField && editing && (editing.touch || !wide)) && (
            <section className="qp" ref={card} key={`${summary.sheet}|${object.id ?? pickedPlace}`} data-ui aria-label={`Quick properties of ${nameOf(object)}`}>
              <QuickProperties
                object={object}
                color={pickedColor}
                journal={journal}
                origin={originWords(object.from, summary.template)}
                fails={(findings.get(upper(summary.sheet)) ?? [])
                  .filter((f) => object.id && f.object?.id && upper(f.object.id) === upper(object.id))
                  .map((f) => f.message)}
                fields={fields}
                layers={(layers ?? []).map((l) => l.name)}
                onCommit={commitProp}
                onRefuse={refuseProp}
                resetKey={propsReset}
                onClose={() => pick(null)}
                onShow={(n) => showEntry(n)}
              />
            </section>
          )}
          {editingField && editing && !editing.touch && wide && (
            <TextOnDrawing
              key={`${editing.place}:${editingField.value}`}
              ref={editor}
              value={editingField.value}
              multiline={editingField.multiline}
              note={editingField.note ?? null}
              onDone={textDone}
              onCancel={textCancelled}
              refuse={refuseText}
            />
          )}
          {editingField && editing && (editing.touch || !wide) && (
            <TextEditBox
              key={`${editing.place}:${editingField.value}`}
              value={editingField.value}
              multiline={editingField.multiline}
              note={editingField.note ?? null}
              onDone={textDone}
              onCancel={textCancelled}
              refuse={refuseText}
            />
          )}
          <CommandBar
            prompt={prompt}
            history={wide ? history.slice(-2) : []}
            onAct={act}
            bar={bar}
            field={field}
            placeholder={toolState?.placeholder ?? ''}
            touch={!wide}
            fieldShown={fieldShown && !(toolState?.typing_text && !wide)}
            typeOffered={!preview && !toolState?.typing_text && (!!toolState?.tool || !object)}
            onShowField={() => {
              setFieldShown(true);
              setTimeout(() => field.current?.focus(), 0);
            }}
            onEnter={(text) => {
              enter(text);
              if (!wide && !text) setFieldShown(false);
            }}
            onEscape={() => escape.current()}
            onTyping={(text) => {
              if (toolState?.typing_text) {
                renderer()?.tool_typing(text);
                drawing.current?.redraw();
              }
            }}
            typingText={!!toolState?.typing_text}
            toolRunning={!!toolState?.tool}
            hint={askHint(typed, !!toolState?.tool, onPreview, toolWord, toolName)}
            spaceEnters={(text) => spaceIsEnter(text, !!toolState?.tool, toolWord)}
            onTyped={setTyped}
            warning={barWarning({ tool: toolState, previewing: !!preview })}
            band={
              preview ? (
                <PreviewBand
                  edit={preview.send.template}
                  sheet={summary?.sheet ?? preview.home}
                  report={previewReport}
                  refreshing={!!previewReport && !previewCurrent}
                  onStep={stepPreview}
                  onCancel={cancelPreview}
                  onGo={confirmPreview}
                  onCheckAgain={recheckPreview}
                  proposal={
                    openProposalNow
                      ? {
                          number: openProposalNow.proposal,
                          proposed: proposedWords(openProposalNow, clock(openProposalNow.at).replace(/:\d\d(?=\s|$)/, ''), now),
                          stale: staleWords(openProposalNow, journal),
                          refused: previewRefusal?.key === preview.key ? previewRefusal.message : null,
                          onDiscard: () => discardProposal(openProposalNow),
                        }
                      : undefined
                  }
                />
              ) : null
            }
            note={barNote(toolState)}
            onUndoKey={toolCanUndo ? undoTool : undefined}
            panel={wide ? null : arrayPanel(true)}
          />
          {wide && arrayPanel(false)}
          {unavailable ? (
            <div className="va-note failed" role="alert">
              The viewer cannot draw here: {unavailable}.
            </div>
          ) : status.kind === 'cannot' ? (
            <div className="va-cannot" role="alert">
              <div className="va-note failed">
                <b>Entry {at} cannot be shown.</b> {status.message}
              </div>
            </div>
          ) : (
            status.kind !== 'shown' && (
              <div className={`va-note ${status.kind}`} role={status.kind === 'failed' ? 'alert' : 'status'}>
                {status.message}
              </div>
            )
          )}
        </div>
        <BottomSheet wide={wide} onCovers={setPeekHeight} onSettled={setSheetCovers} peek={peek} body={body} raise={raise} snapTo={snapAsked} />
      </div>
      {wide && (
        <StatusBar
          layouts={layoutTabs}
          coords={coords}
          paper={summary ? paperWords(summary) : ''}
          toggles={toggles}
          onToggle={toggle}
          live={liveState}
          line={line}
          onSnapMenu={openSnapMenu}
          snapMenuOpen={snapMenu !== null}
        />
      )}
      {snapMenu && (
        <SnapMenu
          on={toggles.osnap}
          running={running}
          place={snapMenu === 'card' ? null : snapMenu}
          onToggle={() => toggle('osnap')}
          onSnap={flipSnap}
          onDefaults={() => {
            setRunning(new Set(DEFAULT_RUNNING));
            log('Object snaps back to the defaults');
          }}
          onClear={() => {
            setRunning(new Set());
            log('No object snaps ticked');
          }}
        />
      )}
      {asking && (
        <div className="va-modal" role="dialog" aria-modal="true" aria-label="Accept the working revision">
          <div className="dialog">
            <h3>Accept the working revision?</h3>
            <p>
              {asking.report.issued.length
                ? `This issues ${plural(asking.report.issued.length, 'change set')} (${listed(asking.report.issued.map((n) => `#${n}`))}) on ${listed(issuedSheets)}. An issued change set cannot be undone.`
                : asking.report.follows
                  ? `No change set has landed since the accept in entry ${asking.report.follows}; this issues the set as it stands.`
                  : 'No change set has landed; this issues the set as it stands.'}
            </p>
            {askedFindings.length ? (
              <div className="err">
                <p className="err">
                  {plural(askedFindings.length, 'check is', 'checks are')} failing. Accepting issues the set with{' '}
                  {askedFindings.length === 1 ? 'it' : 'them'}:
                </p>
                <ul className="findings">
                  {askedFindings.map((f, i) => (
                    <li key={i}>
                      <b>{f.sheet}</b> {f.message}
                    </li>
                  ))}
                </ul>
              </div>
            ) : (
              <p className="ok">All checks pass.</p>
            )}
            <div className="dialog-acts">
              <button type="button" className="btn" data-cancel onClick={() => setAsking(null)}>
                Cancel
              </button>
              <button type="button" className="btn primary" data-ok onClick={() => void confirmAccept()}>
                Accept and issue
              </button>
            </div>
          </div>
        </div>
      )}
      {decodeModalOpen && readReport && (
        <DecodeReportModal
          source={source}
          report={readReport}
          alreadyAllowed={allowLoss}
          onClose={() => setDecodeModalOpen(false)}
          onAllowLoss={() => {
            onAllowLoss(readReport.sha256);
            setDecodeModalOpen(false);
            say(
              'warn',
              `Enabled revising ${sourceDisplayName(source)} with loss`,
              `Revisions save beside the untouched original in ${readReport.dwg_version}.`,
            );
          }}
        />
      )}
    </div>
  );
}
