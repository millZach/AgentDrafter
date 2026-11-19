// The viewer, in the drafting app's frame (#150, the look Zach picked): a
// title bar with Undo last and Accept, a ribbon with the drawing tools
// (#152) and the Layer box, a tab for each of the drawing set's sheets, the
// drawing as dark model space following the working revision live, the
// command bar under it, a status bar, and the journal in a panel on the
// right beside the assistant's place. On a phone the ribbon folds behind
// Tools, the status bar goes, and the journal is a sheet that peeks up from
// the bottom with Undo last and Accept. A toast names each change set as it
// lands and its objects are marked; the journal steps back through the set
// as it was; a tap selects an object, whose quick properties open beside
// it. A drawing tool runs in the renderer: the page passes it the clicks,
// taps and typing, shows its prompt, asks before an edit to a template's
// object, and sends the change set it gives through `change`, as an agent
// would, so it lands in the journal, where Undo and Accept take it; the
// sheet is asked for again as soon as `change` replies.
//
// The page learns of every change from the server's event stream, which
// sends the journal each time it changes on disk, whichever process or
// interface changed it, and draws again the sheets the new entries touched.

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';

import { call, CommandError, run, watch, type StreamState } from './api.ts';
import { BottomSheet } from './BottomSheet.tsx';
import { Drawing, type DrawingHandle, type Focus, type Mark } from './Drawing.tsx';
import {
  changedOn,
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
  type Entry,
  type JournalReport,
} from './journal.ts';
import {
  colorIndex,
  coordinates,
  currentLayerOf,
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
} from './frame.ts';
import { CommandBar, Ribbon, SnapMenu, StatusBar, TemplateDialog, TextBox } from './Frame.tsx';
import { screen_color } from './wasm/agentdrafter_viewer.js';
import { Assistant, JournalList, Peek, QuickProperties } from './Panels.tsx';
import type { Summary } from './probe.ts';

export interface AppProps {
  /** The token for this tab, if the server needs one. */
  token: string | null;
  /** The drawing set, sheet description or DWG to show, as the server names it. */
  source: string;
  /** The sheet to show first; the set's first without one. */
  sheet: string | null;
  /** The layout of that sheet to show first, when it is drawn in several; its first without one. */
  layout: string | null;
}

type Status =
  | { kind: 'loading'; message: string }
  | { kind: 'shown' }
  | { kind: 'failed'; message: string }
  | { kind: 'cannot'; message: string };

type Tone = 'ok' | 'bad' | 'warn' | 'info';

interface Toast {
  key: number;
  tone: Tone;
  title: string;
  detail: string;
  /** The entry its Show button shows. */
  entry?: number;
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
}

/** The object the reviewer picked: by its ID, so it is followed as the sheet changes; by its place when it has none. */
interface Picked {
  id: string | null;
  place: number;
  /** The drawing it was picked on, which an object with no ID is known in alone. */
  summary: Summary;
}

/** What `change` replies, as the page reads it. */
interface ChangeReply {
  journal_entry?: number;
  sheets?: { sheet: string; ok: boolean }[];
  message?: string;
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
const listed = (items: (string | number)[]) =>
  items.length <= 1 ? items.join('') : `${items.slice(0, -1).join(', ')} and ${items.at(-1)}`;

export function App({ token, source, sheet, layout }: AppProps) {
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
  // An edit to a template's object, waiting for the drafter's yes.
  const [templateAsk, setTemplateAsk] = useState<ToolSend | null>(null);
  // A tool asked for while the set as it was is shown: it starts once the set as it is is drawn.
  const [pendingTool, setPendingTool] = useState<{ word: string; picked: number[] } | null>(null);
  // The change sets this page sent whose sheets it has already asked for again.
  const ownEntries = useRef(new Set<number>());
  // The newest change set this page sent that landed, which Undo last
  // takes until the journal the page holds has caught up with it.
  const [ownLatest, setOwnLatest] = useState<number | null>(null);
  // The sheet's layers, for the Layer box, and the current layer kept for this set.
  const [layers, setLayers] = useState<LayerEntry[]>([]);
  const [keptLayer, setKeptLayer] = useState<string | null>(() => readCurrentLayer(keeps, source));
  const [side, setSide] = useState<'journal' | 'assistant'>('journal');
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
  // A toast goes after a while.
  useEffect(() => {
    if (!toast) return;
    const gone = setTimeout(() => setToast((t) => (t?.key === toast.key ? null : t)), 6000);
    return () => clearTimeout(gone);
  }, [toast]);

  // The drawing tools run in the renderer, which the drawing holds.
  const renderer = () => drawing.current?.viewer() ?? null;
  const refreshTool = useCallback(() => {
    const v = drawing.current?.viewer();
    setToolState(v ? (JSON.parse(v.tool_state(!wide)) as ToolState) : null);
    drawing.current?.redraw();
  }, [wide]);
  useEffect(() => refreshTool(), [refreshTool]);

  const sheetKey = upper(wanted ?? '');
  const layoutKey = upper(wantedLayout ?? '');
  // The set as it is comes with its object snaps, for the cursor to catch,
  // while Object snap is on; the set as it was, read-only, without. A
  // sheet drawn without them is asked for again when it is turned on.
  const withSnaps = at === null && toggles.osnap;
  const cacheKey = `${at ?? 'live'}|${sheetKey}|${layoutKey}|${withSnaps ? 'snaps' : ''}`;
  const version = at === null ? (versions[sheetKey || ANY] ?? 0) : 0;

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

  // Each sheet asked for is drawn by the server once, then kept, until an
  // entry of the journal touches it: then the set as it is now is asked
  // for again. A sheet as it was at an entry never changes.
  useEffect(() => {
    let current = true;
    // With Object snap off, a drawing kept with its snaps does as well.
    const kept = replies.current.get(cacheKey) ?? (withSnaps ? undefined : replies.current.get(`${cacheKey}snaps`));
    if (kept) {
      // A sheet drawn before is shown again at once: whatever another sheet
      // was saying, loading or failing, is not this one's to say. The
      // request for that other sheet, if it is still out, is let go.
      setReply({ text: kept, at, sheetKey, generation: generationOf(at, sheetKey), snaps: withSnaps });
      setStatus({ kind: 'shown' });
      return;
    }
    if (at === null || !summary) {
      setStatus({ kind: 'loading', message: wanted ? `Drawing ${wanted}…` : 'Drawing the sheet…' });
    }
    const input: Record<string, string | number | boolean> = { source };
    if (wanted) input.sheet = wanted;
    if (wantedLayout) input.layout = wantedLayout;
    if (at !== null) input.entry = at;
    if (withSnaps) input.snaps = true;
    // The set as it is says where each object comes from, which the tools
    // and quick properties need.
    if (at === null) input.origins = true;
    const generation = generationOf(at, sheetKey);
    run('display-list', input, token).then(
      (text) => {
        // Kept even when the reviewer has moved on, for when they come
        // back, unless the sheet has changed since it was asked for: then
        // it is the old drawing, and the sheet is asked for again.
        if (generationOf(at, sheetKey) !== generation) return;
        replies.current.set(cacheKey, text);
        if (!current) return;
        setReply({ text, at, sheetKey, generation, snaps: withSnaps });
      },
      (e) => {
        if (!current) return;
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
    // it changed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cacheKey, version, epoch, source, token]);

  const loaded = useCallback(
    (shown: Summary) => {
      setSummary(shown);
      setStatus({ kind: 'shown' });
      // Kept by its sheet's own name too, while it is still current.
      if (reply && generationOf(reply.at, reply.sheetKey) === reply.generation) {
        replies.current.set(
          `${shown.entry ?? 'live'}|${upper(shown.sheet)}|${upper(shown.layout ?? '')}|${reply.snaps ? 'snaps' : ''}`,
          reply.text,
        );
      }
      // The tool running follows the sheet drawn again, or ends on another.
      refreshTool();
      if (!wanted) setWanted(shown.sheet);
      if (!wantedLayout && shown.layout) setWantedLayout(shown.layout);
      // The address names the sheet and the layout shown, so a reload shows them again.
      const query = new URLSearchParams(window.location.search);
      query.set('sheet', shown.sheet);
      if (shown.layout) query.set('layout', shown.layout);
      else query.delete('layout');
      window.history.replaceState(null, '', `${window.location.pathname}?${query}`);
    },
    [reply, wanted, wantedLayout, refreshTool],
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

  // The checker's findings, sheet by sheet: all of them once, then those
  // of the sheets each new entry touched.
  const check = useCallback(
    async (sheets: string[] | null): Promise<Finding[]> => {
      try {
        const input: Record<string, unknown> = { source };
        if (sheets) input.sheets = sheets;
        const result = await call<{ sheets: string[]; findings: Finding[] }>('check', input, token);
        setFindings((before) => {
          const next = new Map(sheets ? before : []);
          for (const s of result.sheets) next.set(upper(s), []);
          for (const f of result.findings) next.get(upper(f.sheet))?.push(f);
          return next;
        });
        return result.findings;
      } catch {
        return [];
      }
    },
    [source, token],
  );

  // What a new entry says, as a toast.
  const toastFor = useCallback(
    (entry: Entry, failedChecks: Finding[]) => {
      const n = entry.entry;
      switch (entry.kind) {
        case 'failed_attempt':
          return say('bad', `Change set ${n} failed. Nothing landed.`, entry.error?.message ?? entryTitle(entry), n);
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
              `Change set ${n} landed on ${listed(landed)}. ${plural(mine.length, 'check')} failed`,
              mine[0]?.message ?? '',
              n,
            );
          }
          return say('ok', `Change set ${n} landed on ${listed(landed)}`, entryTitle(entry), n);
        }
      }
    },
    [say],
  );

  // The journal, from the server's event stream.
  useEffect(() => {
    const stop = watch(
      source,
      token,
      (text) => {
        const replied = JSON.parse(text) as { ok: boolean; result?: JournalReport; error?: { message: string } };
        if (!replied.ok || !replied.result) {
          setNoJournal(replied.error?.message ?? 'there is no journal');
          setJournal(null);
          return;
        }
        const report = replied.result;
        setNoJournal(null);
        setJournal(report);
        const before = seen.current;
        // A journal cut back, as a commit rolled back cuts it, or whose
        // entries seen were taken away or replaced, is not the one the
        // drawings came from: every drawing kept is dropped, and drawn
        // again.
        const replaced = before !== null && journalReplaced(seenEntries.current, report);
        seen.current = newest(report);
        seenEntries.current = recordedEntries(report);
        if (before === null) {
          const n = report.entries.length;
          log(`Journal read: ${n} entr${n === 1 ? 'y' : 'ies'}.`);
        }
        if (before === null || replaced) {
          if (replaced) {
            replies.current.clear();
            epochNow.current += 1;
            setEpoch(epochNow.current);
          }
          void check(null);
          return;
        }
        const fresh = since(report, before);
        if (!fresh.length) return;
        const touched = [...new Set(fresh.flatMap((e) => sheetsTouched(e, report)).map(upper))];
        // A change set this page sent had its sheets asked for again as
        // soon as `change` replied: they are not asked for twice.
        const others = [
          ...new Set(fresh.filter((e) => !ownEntries.current.has(e.entry)).flatMap((e) => sheetsTouched(e, report)).map(upper)),
        ];
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
          void check(touched).then((found) => toastFor(last, found));
        } else {
          toastFor(last, []);
        }
      },
      setStream,
    );
    return () => {
      stop();
      seen.current = null;
    };
  }, [source, token, check, toastFor, log, drawAgain]);

  // The objects the focused entry changed on the sheet shown, marked.
  const focusedEntry = journal?.entries.find((e) => e.entry === focused);
  const places = useMemo(() => {
    const byId = new Map<string, number>();
    (summary?.objects ?? []).forEach((o, i) => o.id && byId.set(upper(o.id), i));
    return byId;
  }, [summary]);
  const marks = useMemo<Mark[]>(() => {
    const showing = focusedEntry && (!focusedEntry.undone_by || at === focusedEntry.entry);
    const changed = showing && summary ? changedOn(focusedEntry, summary.sheet) : [];
    return changed
      .filter((o) => places.has(upper(o.id)))
      .map((o) => ({ place: places.get(upper(o.id)) as number, added: o.how === 'added' }));
  }, [focusedEntry, summary, places, at]);

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
    setAt(asItWas ? number : null);
    setFocused(number);
    setPulseSince(null);
    setPicked(null);
    endTool();
    setSide('journal');
    setWanted(target);
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
    setBusy('undo');
    try {
      await call('undo', { set: source, change_set: changeSet }, token);
    } catch (e) {
      say('warn', `Change set ${changeSet} was not undone`, why(e, token));
    } finally {
      setBusy(null);
    }
  };

  // Accept asks first: the server says what it would issue, and any failed
  // checks, without writing; the reviewer confirms.
  const askAccept = async () => {
    if (!journal) return;
    const newestSeen = newest(journal);
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
    if (!summary) return;
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
  const toolDid = (text: string) => {
    if (text) {
      const o = JSON.parse(text) as ToolOutcome;
      for (const line of o.log) log(line);
      if (o.send) {
        if (o.send.template) setTemplateAsk(o.send);
        else void sendChange(o.send);
      }
    }
    refreshTool();
  };
  // Sends a tool's change set through `change`, as an agent would, and
  // asks for the sheets it landed on as soon as `change` replies, without
  // waiting for the journal.
  const sendChange = async (s: ToolSend) => {
    const landed = (reply: ChangeReply | undefined) => {
      const sheets = (reply?.sheets ?? []).filter((x) => x.ok).map((x) => upper(x.sheet));
      const entry = reply?.journal_entry;
      if (entry !== undefined) {
        // The journal already told of it: its sheets were asked for then.
        if (seen.current !== null && seen.current >= entry) return;
        ownEntries.current.add(entry);
        if (sheets.length) setOwnLatest((n) => Math.max(n ?? 0, entry));
      }
      drawAgain(sheets);
    };
    // Undo last and Accept wait while it is on its way: the change set
    // they would take may be about to land.
    setBusy('change');
    try {
      const reply = await call<ChangeReply>('change', { set: source, change_set: s.change_set }, token);
      landed(reply);
      log(`${s.what}: change set ${reply.journal_entry ?? ''} landed`.replace('  ', ' '));
    } catch (e) {
      if (e instanceof CommandError && e.result) landed(e.result as ChangeReply);
      say('warn', `${s.what}: not landed`, why(e, token));
    } finally {
      setBusy(null);
    }
  };
  const startTool = (word: string, fromSelection = true) => {
    setToolsOpen(false);
    setTaken(null);
    setSnapMenu(null);
    const picked = fromSelection && pickedPlace !== null ? [pickedPlace] : [];
    if (at !== null) {
      // The set as it was is read-only: the tool starts on the set as it is.
      backToLive();
      setPendingTool({ word, picked });
      return;
    }
    const v = renderer();
    if (!v) return;
    const text = v.tool_start(word, Uint32Array.from(picked));
    const state = JSON.parse(v.tool_state(!wide)) as ToolState;
    if (state.tool) setPicked(null);
    if (state.tool === 'text' && !wide) setFieldShown(false);
    toolDid(text);
    if (state.tool && wide) field.current?.focus({ preventScroll: true });
  };
  const ribbonTool = (tool: Tool) => {
    // The running tool's own button ends it.
    if (toolState?.tool === tool.key) toolDid(renderer()?.tool_escape() ?? '');
    else startTool(tool.key);
  };
  const endTool = () => {
    const v = renderer();
    if (v && toolState?.tool) toolDid(v.tool_escape());
  };
  // Enter, Space or a right-click, with what the command bar's field holds.
  const enter = (text: string) => {
    const v = renderer();
    if (!v) return;
    if (!toolState?.tool && text.trim()) {
      // A tool's alias starts it, Move and Erase on what is selected.
      if (at !== null) {
        backToLive();
        setPendingTool({ word: text.trim(), picked: pickedPlace !== null ? [pickedPlace] : [] });
        return;
      }
      startTool(text.trim());
      return;
    }
    toolDid(v.tool_input(text));
  };
  useEffect(() => {
    if (!pendingTool || !summary || summary.entry !== null || at !== null) return;
    const v = renderer();
    if (!v) return;
    setPendingTool(null);
    toolDid(v.tool_start(pendingTool.word, Uint32Array.from(pendingTool.picked)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pendingTool, summary, at]);

  // AutoCAD's keys: F3 Object snap, F7 Grid, F8 Ortho; Escape steps back
  // out of whatever is open, one thing at a time.
  const escape = useRef<() => void>(() => {});
  escape.current = () => {
    if (templateAsk) cancelTemplate();
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
    if (asking || templateAsk) return;
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.key === 'Delete' && !toolState?.tool && pickedPlace !== null) {
      e.preventDefault();
      startTool('erase');
      return;
    }
    if (target?.closest?.('button') && (e.key === 'Enter' || e.key === ' ')) return;
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      enter('');
      return;
    }
    if (e.key.length === 1 && e.key !== ' ') {
      // Typing anywhere goes to the command bar, as in AutoCAD.
      e.preventDefault();
      setFieldShown(true);
      const f = field.current;
      if (!f) return;
      f.hidden = false;
      f.value += e.key;
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
  // The template dialog: Cancel lands nothing; Change all sends the change set.
  const cancelTemplate = () => {
    const was = templateAsk;
    setTemplateAsk(null);
    if (was) log(`${was.what}: cancelled, nothing changed`);
  };
  const confirmTemplate = () => {
    const was = templateAsk;
    setTemplateAsk(null);
    if (was) void sendChange(was);
  };

  // The sheet's layers, for the Layer box: asked for each sheet, and again
  // when the box is opened, which takes in a layer a change set added. Not
  // after every change set, which would add a request to each one landing.
  const [layersAsked, setLayersAsked] = useState(0);
  const hasTemplate = !!summary?.template;
  useEffect(() => {
    if (!summary || summary.entry !== null || !hasTemplate) {
      setLayers([]);
      return;
    }
    let current = true;
    call<{ layers: { sheet: string; layers: LayerEntry[] }[] }>(
      'query',
      { source, sheet: summary.sheet, query: { kind: 'layers' } },
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
  }, [summary?.sheet, summary?.entry, hasTemplate, layersAsked, source, token]);
  const currentLayer = currentLayerOf(keptLayer, layers);
  const layerEntry = layers.find((l) => l.name === currentLayer);
  const chooseLayer = (name: string) => {
    setKeptLayer(name);
    writeCurrentLayer(keeps, source, name);
    log(`Current layer: ${name}`);
  };
  const toolLayer = useMemo(() => ({ name: currentLayer, color: colorIndex(layerEntry?.color) }), [currentLayer, layerEntry?.color]);
  // The prompt shows the current layer a tool draws on.
  useEffect(() => refreshTool(), [toolLayer, refreshTool]);

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
  // that wraps grows over the drawing without moving the fitted sheet.
  useLayoutEffect(() => {
    const vp = viewport.current;
    const b = bar.current;
    if (!vp || !b) return;
    const measure = () => {
      const covered = vp.getBoundingClientRect().bottom - b.getBoundingClientRect().bottom + BAR_LINE;
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
    return () => resized.disconnect();
  }, [peekHeight, wide]);

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
  const blocked = !!journal && (!!journal.inconsistent || (!!lastAccept && pending.length === 0));

  const sheets = summary?.sheets ?? [];
  const name = summary?.name ?? '';
  const object = pickedPlace !== null ? summary?.objects[pickedPlace] : undefined;
  const { state: liveState, line } = liveLine(at, !!noJournal, stream);
  const prompt = promptFor({
    tool: toolState,
    at,
    cannot: status.kind === 'cannot',
    reaching: !noJournal && stream !== 'open',
    selected: object ?? null,
    taken: taken?.taken ?? null,
    touch: !wide,
  });
  const act = (a: Act) => {
    if (a === 'clear') pick(null);
    else if (a === 'copy') void copySnap();
    else if (a === 'older') step(-1);
    else if (a === 'newer') step(1);
    else if (a === 'live') backToLive();
    else if (a === 'move' || a === 'erase') startTool(a);
    else if (a.startsWith('tool:')) {
      const v = renderer();
      if (!v) return;
      // The field's words go with Done, as Enter takes them.
      const typedText = field.current?.value ?? '';
      if (a === 'tool:done' && toolState?.typing_text && typedText) {
        if (field.current) field.current.value = '';
        toolDid(v.tool_input(typedText));
        toolDid(v.tool_input(''));
        return;
      }
      toolDid(v.tool_option(a.slice(5)));
    }
  };

  const undoLabel = `Undo last${undoable ? ` (#${undoable})` : ''}`;
  const acceptLabel = busy === 'accept' ? 'Checking…' : 'Accept…';
  const peek = wide ? null : (
    <Peek
      line={line}
      live={liveState}
      journal={journal}
      failing={failing}
      undoable={undoable}
      blocked={blocked}
      busy={busy}
      stepping={at !== null}
      onShow={(n) => showEntry(n)}
      onUndo={() => void undo(undoable)}
      onAccept={() => void askAccept()}
    />
  );
  const journalBody = journal ? (
    <JournalList
      journal={journal}
      failing={failing}
      at={at}
      focused={focused}
      busy={busy}
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
  // The journal now, and beside it the assistant's place, which build 8 fills.
  const body = (
    <>
      <div className="sidetabs" role="tablist">
        <button
          type="button"
          role="tab"
          className={`stab${side === 'journal' ? ' on' : ''}`}
          aria-selected={side === 'journal'}
          onClick={() => setSide('journal')}
        >
          Journal
        </button>
        <button
          type="button"
          role="tab"
          className={`stab${side === 'assistant' ? ' on' : ''}`}
          aria-selected={side === 'assistant'}
          onClick={() => {
            setSide('assistant');
            setRaise((r) => r + 1);
          }}
        >
          Assistant <span className="soon">Soon</span>
        </button>
      </div>
      <div className="sidebody">{side === 'journal' ? journalBody : <Assistant />}</div>
    </>
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
  return (
    <div
      className={`va${toolsOpen ? ' tools-open' : ''}`}
      style={{ ['--peek' as string]: `${peekHeight}px` }}
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
            <button
              type="button"
              className="qa"
              data-act="undo"
              disabled={!undoable || busy !== null || at !== null}
              title={at !== null ? 'Off while you look back at an entry' : undoable ? `Undo change set ${undoable}` : 'Nothing an undo can take'}
              onClick={() => void undo(undoable)}
            >
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                <path d="M3 6h6.5a3.5 3.5 0 010 7H6M3 6l3-3M3 6l3 3" />
              </svg>
              {undoLabel}
            </button>
            <span className="file">
              {name}
              {summary ? ` / ${summary.sheet}` : ''}
            </span>
          </>
        ) : (
          <button
            type="button"
            className="qa toolsbtn"
            aria-expanded={toolsOpen}
            aria-controls="ribbon"
            onClick={() => setToolsOpen((o) => !o)}
          >
            Tools &#9662;
          </button>
        )}
        <div className="va-status">{pills}</div>
        {wide && (
          <button
            type="button"
            className={`btn primary${blocked ? ' blocked' : ''}`}
            data-act="accept"
            disabled={!journal || busy !== null || at !== null}
            onClick={() => void askAccept()}
          >
            {acceptLabel}
          </button>
        )}
      </header>
      <div id="ribbon" className={`ribbonwrap${wide || toolsOpen ? ' open' : ''}`}>
        <Ribbon
          folded={!wide}
          toggles={toggles}
          onToggle={toggle}
          onTool={ribbonTool}
          running={toolState?.tool ?? null}
          layers={layers}
          current={currentLayer}
          onLayer={chooseLayer}
          onLayersOpen={() => setLayersAsked((n) => n + 1)}
          swatch={(l) => screen_color(colorIndex(l?.color))}
          selected={object ? { name: object.layer, color: pickedColor } : null}
          onSnaps={() => openSnapMenu(null)}
        />
      </div>
      </div>
      <nav className="va-tabs sheettabs" ref={tabs} aria-label="Sheets">
        {sheets.map((s) => {
          const dot = marked.has(upper(s)) ? (
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
              className={`tab${s === summary?.sheet ? ' on' : ''}`}
              aria-current={s === summary?.sheet ? 'page' : undefined}
              onClick={() => {
                setPicked(null);
                setWanted(s);
                setWantedLayout(null);
              }}
            >
              <b>{s}</b>
              {dot}
            </button>
          );
        })}
      </nav>
      {!wide && layoutTabs}
      <div className="va-main">
        <div
          className={`va-vp${at !== null ? ' past' : ''}`}
          ref={viewport}
          onPointerDownCapture={() => toolsOpen && setToolsOpen(false)}
        >
          <Drawing
            reply={reply?.text ?? null}
            coveredBelow={covered}
            compact={!wide}
            grid={toggles.grid}
            card={card}
            marks={marks}
            picked={pickedPlace}
            pulseSince={pulseSince}
            focus={focus}
            onLoaded={loaded}
            onError={failed}
            onUnavailable={setUnavailable}
            onPick={pick}
            onCursor={cursorAt}
            snapMask={runningMask(toggles.osnap && at === null, running)}
            onTakeSnap={takeSnap}
            onNoSnap={noSnap}
            space={`${sheetCovers}:${barHeight}`}
            onPickedColor={setPickedColor}
            tool={toolState}
            onToolOutcome={toolDid}
            onToolEnter={() => enter('')}
            ortho={toggles.ortho}
            layer={toolLayer}
            handle={drawing}
          />
          <div className="va-overlays">
            {toast && (
              <div key={toast.key} className={`va-toast in ${toast.tone}`} role="status">
                <span className={`dot ${toast.tone}`} />
                <div>
                  <b>{toast.title}</b>
                  <small>{toast.detail}</small>
                </div>
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
              onCancel={endTool}
            />
          )}
          {object && summary && (
            <section className="qp" ref={card} data-ui aria-label={`Quick properties of ${nameOf(object)}`}>
              <QuickProperties
                object={object}
                color={pickedColor}
                journal={journal}
                origin={originWords(object.from, summary.template)}
                fails={(findings.get(upper(summary.sheet)) ?? [])
                  .filter((f) => object.id && f.object?.id && upper(f.object.id) === upper(object.id))
                  .map((f) => f.message)}
                onClose={() => pick(null)}
                onShow={(n) => showEntry(n)}
              />
            </section>
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
            typeOffered={!toolState?.typing_text && (!!toolState?.tool || !object)}
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
            warning={toolState?.warning ?? null}
            note={
              toolState?.typing_text
                ? 'Enter: next line · Enter again or Esc: done'
                : toolState?.tool
                  ? 'Enter or right-click: done · Esc: end · F8: Ortho'
                  : 'F3: snaps · F8: Ortho · Tab: the next snap here'
            }
          />
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
        <BottomSheet wide={wide} onCovers={setPeekHeight} onSettled={setSheetCovers} peek={peek} body={body} raise={raise} />
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
      {templateAsk?.template && (
        <TemplateDialog edit={templateAsk.template} sheet={summary?.sheet ?? ''} onCancel={cancelTemplate} onConfirm={confirmTemplate} />
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
    </div>
  );
}
