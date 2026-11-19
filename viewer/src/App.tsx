// The viewer, in the drafting app's frame (#150, the look Zach picked): a
// title bar with Undo last and Accept, a ribbon whose drawing tools are
// shown before they work, a tab for each of the drawing set's sheets, the
// drawing as dark model space following the working revision live, the
// command bar under it, a status bar, and the journal in a panel on the
// right beside the assistant's place. On a phone the ribbon folds behind
// Tools, the status bar goes, and the journal is a sheet that peeks up from
// the bottom with Undo last and Accept. A toast names each change set as it
// lands and its objects are marked; the journal steps back through the set
// as it was; a tap selects an object, whose quick properties open beside
// it. Nothing edits yet.
//
// The page learns of every change from the server's event stream, which
// sends the journal each time it changes on disk, whichever process or
// interface changed it, and draws again the sheets the new entries touched.

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';

import { call, CommandError, run, watch, type StreamState } from './api.ts';
import { BottomSheet } from './BottomSheet.tsx';
import { Drawing, type Focus, type Mark } from './Drawing.tsx';
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
  coordinates,
  DEFAULT_RUNNING,
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
import { CommandBar, Ribbon, SnapMenu, StatusBar } from './Frame.tsx';
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
  // point, and the sheet drawn and the object it was taken on, by its ID.
  const [taken, setTaken] = useState<{
    taken: Taken;
    point: { snap: Record<string, unknown> } | null;
    summary: Summary;
    id: string | null;
  } | null>(null);
  // The read-out goes when what it names does: another sheet drawn, or its
  // object gone from this one. An object with no ID is known only by its
  // place, which any new drawing of the sheet may give another.
  useEffect(() => {
    setTaken((t) => {
      if (!t) return t;
      // Another sheet asked for goes at once, before it is drawn.
      if (wanted !== null && upper(wanted) !== upper(t.summary.sheet)) return null;
      if (t.summary === summary) return t;
      if (!summary || upper(summary.sheet) !== upper(t.summary.sheet) || t.id === null) return null;
      return summary.objects.some((o) => o.id === t.id) ? t : null;
    });
  }, [summary, wanted]);
  const [toolsOpen, setToolsOpen] = useState(false);
  const [soon, setSoon] = useState<Tool | null>(null);
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
      if (!wanted) setWanted(shown.sheet);
      if (!wantedLayout && shown.layout) setWantedLayout(shown.layout);
      // The address names the sheet and the layout shown, so a reload shows them again.
      const query = new URLSearchParams(window.location.search);
      query.set('sheet', shown.sheet);
      if (shown.layout) query.set('layout', shown.layout);
      else query.delete('layout');
      window.history.replaceState(null, '', `${window.location.pathname}?${query}`);
    },
    [reply, wanted, wantedLayout],
  );
  const failed = useCallback((message: string) => setStatus({ kind: 'failed', message }), []);

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
        // Every drawing kept of a touched sheet goes, each of its layouts
        // among them, and so does the one kept for a sheet not yet named.
        for (const key of staleDrawings(replies.current.keys(), touched)) replies.current.delete(key);
        // The versions move on now, so a reply already on its way is seen
        // as old when it comes.
        const next = { ...versionsNow.current };
        for (const s of [...touched, ...(touched.length ? [ANY] : [])]) next[s] = (next[s] ?? 0) + 1;
        versionsNow.current = next;
        setVersions(next);
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
  }, [source, token, check, toastFor, log]);

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
    setSoon(null);
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
      setSoon(null);
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
    setSoon(null);
    setPicked(o ? { id: o.id ?? null, place: h.object, summary } : null);
    setTaken({ taken: read, point: snappedPoint(h, summary.objects), summary, id: o?.id ?? null });
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

  // A tool still to come, tapped: it says so in the command bar, for a while.
  const soonTool = (tool: Tool) => {
    setToolsOpen(false);
    setSoon(tool);
  };
  useEffect(() => {
    if (!soon) return;
    const gone = setTimeout(() => setSoon(null), 6000);
    return () => clearTimeout(gone);
  }, [soon]);

  // AutoCAD's keys: F3 Object snap, F7 Grid, F8 Ortho; Escape steps back
  // out of whatever is open, one thing at a time.
  const escape = useRef<() => void>(() => {});
  escape.current = () => {
    if (asking) setAsking(null);
    else if (snapMenu) setSnapMenu(null);
    else if (toolsOpen) setToolsOpen(false);
    else if (soon) setSoon(null);
    else if (picked) pick(null);
    else if (at !== null) backToLive();
  };
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      const which = toggleFor(e.key);
      if (which) {
        e.preventDefault();
        toggle(which);
      } else if (e.key === 'Escape') {
        escape.current();
      }
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [toggle]);

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
  const undoable = journal ? lastUndoable(journal) : null;
  const blocked = !!journal && (!!journal.inconsistent || (!!lastAccept && pending.length === 0));

  const sheets = summary?.sheets ?? [];
  const name = summary?.name ?? '';
  const object = pickedPlace !== null ? summary?.objects[pickedPlace] : undefined;
  const { state: liveState, line } = liveLine(at, !!noJournal, stream);
  const prompt = promptFor({
    soon,
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
    else if (a === 'ok') setSoon(null);
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
          onSoon={soonTool}
          layer={object ? { name: object.layer, color: pickedColor } : null}
          flashing={soon?.key ?? null}
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
          {object && summary && (
            <section className="qp" ref={card} data-ui aria-label={`Quick properties of ${nameOf(object)}`}>
              <QuickProperties
                object={object}
                color={pickedColor}
                journal={journal}
                fails={(findings.get(upper(summary.sheet)) ?? [])
                  .filter((f) => object.id && f.object?.id && upper(f.object.id) === upper(object.id))
                  .map((f) => f.message)}
                onClose={() => pick(null)}
                onShow={(n) => showEntry(n)}
              />
            </section>
          )}
          <CommandBar prompt={prompt} history={wide ? history.slice(-2) : []} onAct={act} bar={bar} />
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
