// The viewer, in the look the reviewer picked (variant A): the drawing
// fills the screen and follows the working revision live, the drawing
// set's sheets are tabs across the top, and the journal's sheet peeks up
// from the bottom, a panel on the right on a laptop. A toast names each
// change set as it lands and its objects are marked; the journal steps back
// through the set as it was; a tap picks an object for the inspector; and
// Undo last and Accept are always in reach.
//
// The page learns of every change from the server's event stream, which
// sends the journal each time it changes on disk, whichever process or
// interface changed it, and draws again the sheets the new entries touched.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { call, CommandError, run, watch, type StreamState } from './api.ts';
import { BottomSheet } from './BottomSheet.tsx';
import { Drawing, type Focus, type Marks } from './Drawing.tsx';
import {
  changedOn,
  entryTitle,
  firstSheet,
  lastUndoable,
  newest,
  notIssued,
  sheetsTouched,
  since,
  viaWords,
  type Entry,
  type JournalReport,
} from './journal.ts';
import { Inspector, JournalList, Peek } from './Panels.tsx';
import type { Summary } from './probe.ts';

export interface AppProps {
  /** The token for this tab, if the server needs one. */
  token: string | null;
  /** The drawing set, sheet description or DWG to show, as the server names it. */
  source: string;
  /** The sheet to show first; the set's first without one. */
  sheet: string | null;
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

interface Asking {
  report: AcceptReport;
  /** The newest entry the reviewer has seen, which the accept follows. */
  newestSeen: number;
}

const WIDE = '(min-width: 900px)';
const upper = (s: string) => s.toUpperCase();

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

export function App({ token, source, sheet }: AppProps) {
  const wide = useWide();
  const [wanted, setWanted] = useState<string | null>(sheet);
  const [at, setAt] = useState<number | null>(null);
  const [reply, setReply] = useState<string | null>(null);
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
  const [picked, setPicked] = useState<number | null>(null);
  const [inspecting, setInspecting] = useState(false);
  const [raise, setRaise] = useState(0);
  const [toast, setToast] = useState<Toast | null>(null);
  const [asking, setAsking] = useState<Asking | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [findings, setFindings] = useState<ReadonlyMap<string, Finding[]>>(new Map());
  const [versions, setVersions] = useState<Readonly<Record<string, number>>>({});
  const replies = useRef(new Map<string, string>());
  const seen = useRef<number | null>(null);
  const atNow = useRef(at);
  const keys = useRef(0);
  const tabs = useRef<HTMLElement>(null);
  atNow.current = at;

  const say = useCallback((tone: Tone, title: string, detail: string, entry?: number) => {
    keys.current += 1;
    setToast({ key: keys.current, tone, title, detail, entry });
  }, []);
  // A toast goes after a while.
  useEffect(() => {
    if (!toast) return;
    const gone = setTimeout(() => setToast((t) => (t?.key === toast.key ? null : t)), 6000);
    return () => clearTimeout(gone);
  }, [toast]);

  const sheetKey = upper(wanted ?? '');
  const cacheKey = `${at ?? 'live'}|${sheetKey}`;
  const version = at === null ? (versions[sheetKey] ?? 0) : 0;

  // Each sheet asked for is drawn by the server once, then kept, until an
  // entry of the journal touches it: then the set as it is now is asked
  // for again. A sheet as it was at an entry never changes.
  useEffect(() => {
    let current = true;
    const kept = replies.current.get(cacheKey);
    if (kept) {
      // A sheet drawn before is shown again at once: whatever another sheet
      // was saying, loading or failing, is not this one's to say. The
      // request for that other sheet, if it is still out, is let go.
      setReply(kept);
      setStatus({ kind: 'shown' });
      return;
    }
    if (at === null || !summary) {
      setStatus({ kind: 'loading', message: wanted ? `Drawing ${wanted}…` : 'Drawing the sheet…' });
    }
    const input: Record<string, string | number> = { source };
    if (wanted) input.sheet = wanted;
    if (at !== null) input.entry = at;
    run('display-list', input, token).then(
      (text) => {
        // Kept even when the reviewer has moved on, for when they come back.
        replies.current.set(cacheKey, text);
        if (!current) return;
        setReply(text);
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
    // The key holds the sheet and the entry; the version says it changed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cacheKey, version, source, token]);

  const loaded = useCallback(
    (shown: Summary) => {
      setSummary(shown);
      setStatus({ kind: 'shown' });
      if (reply) replies.current.set(`${shown.entry ?? 'live'}|${upper(shown.sheet)}`, reply);
      if (!wanted) setWanted(shown.sheet);
      // The address names the sheet shown, so a reload shows it again.
      const query = new URLSearchParams(window.location.search);
      query.set('sheet', shown.sheet);
      window.history.replaceState(null, '', `${window.location.pathname}?${query}`);
    },
    [reply, wanted],
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
    (entry: Entry, report: JournalReport, failedChecks: Finding[]) => {
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
          void report;
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
        seen.current = newest(report);
        if (before === null) {
          void check(null);
          return;
        }
        const fresh = since(report, before);
        if (!fresh.length) return;
        const touched = [...new Set(fresh.flatMap((e) => sheetsTouched(e, report)).map(upper))];
        for (const s of touched) replies.current.delete(`live|${s}`);
        setVersions((v) => {
          const next = { ...v };
          for (const s of touched) next[s] = (next[s] ?? 0) + 1;
          return next;
        });
        const last = fresh.at(-1) as Entry;
        const landed = [...fresh].reverse().find((e) => e.kind === 'change_set' && (e.landed_on?.length ?? 0) > 0);
        // A change set that lands is marked, and its marks pulse, unless the
        // reviewer is looking at the set as it was: they stay where they are.
        if (atNow.current === null && landed) {
          setFocused(landed.entry);
          setPulseSince(performance.now());
        }
        if (touched.length) {
          void check(touched).then((found) => toastFor(last, report, found));
        } else {
          toastFor(last, report, []);
        }
      },
      setStream,
    );
    return () => {
      stop();
      seen.current = null;
    };
  }, [source, token, check, toastFor]);

  // The objects the focused entry changed on the sheet shown, marked.
  const focusedEntry = journal?.entries.find((e) => e.entry === focused);
  const places = useMemo(() => {
    const byId = new Map<string, number>();
    (summary?.objects ?? []).forEach((o, i) => o.id && byId.set(upper(o.id), i));
    return byId;
  }, [summary]);
  const marks = useMemo<Marks>(() => {
    const showing = focusedEntry && (!focusedEntry.undone_by || at === focusedEntry.entry);
    const changed = showing && summary ? changedOn(focusedEntry, summary.sheet) : [];
    const found = changed.filter((o) => places.has(upper(o.id)));
    return { places: found.map((o) => places.get(upper(o.id)) as number), added: found.map((o) => o.how === 'added') };
  }, [focusedEntry, summary, places, at]);

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
    setInspecting(false);
    setWanted(target);
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

  const pick = (place: number | null) => {
    setPicked(place);
    setInspecting(place !== null);
    if (place !== null) setRaise((r) => r + 1);
  };

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
  const object = picked !== null ? summary?.objects[picked] : undefined;
  const line =
    at !== null
      ? `Showing entry ${at} as it was: read-only`
      : noJournal
        ? 'Not a drawing set: there is no journal to watch'
        : stream === 'open'
          ? 'Live: changes show as they land'
          : stream === 'refused'
            ? 'The server refused the live stream: open the viewer from the address serve printed'
            : 'Reaching the server…';

  const peek = (
    <Peek
      line={line}
      live={stream === 'open' && at === null && !noJournal}
      journal={journal}
      failing={failing}
      undoable={undoable}
      blocked={blocked}
      busy={busy}
      onShow={(n) => showEntry(n)}
      onUndo={() => void undo(undoable)}
      onAccept={() => void askAccept()}
    />
  );
  const body =
    inspecting && object && summary ? (
      <Inspector
        object={object}
        sheet={summary.sheet}
        journal={journal}
        fails={(findings.get(upper(summary.sheet)) ?? [])
          .filter((f) => object.id && f.object?.id && upper(f.object.id) === upper(object.id))
          .map((f) => f.message)}
        onBack={() => pick(null)}
        onShow={(n) => showEntry(n)}
      />
    ) : journal ? (
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

  const issuedSheets = asking?.report.sheets ?? [];
  const askedFindings = asking?.report.checks.findings ?? [];
  return (
    <div className="va">
      <header className="va-top">
        <div className="va-row">
          <div className="va-set">
            <b>{name}</b>
            {summary && <span>{plural(sheets.length, 'sheet')}</span>}
          </div>
          {journal && (
            <div className="va-status">
              <span className="pill">
                {pending.length ? (
                  <>
                    <span className="long">{'Working revision \u00b7 '}</span>
                    {pending.length} not issued
                  </>
                ) : lastAccept ? (
                  `Issued (entry ${lastAccept.entry})`
                ) : (
                  'Working revision, not issued'
                )}
              </span>
              {journal.inconsistent && <span className="pill warn">Inconsistent set</span>}
              {failCount > 0 && <span className="pill bad">{plural(failCount, 'failed check')}</span>}
            </div>
          )}
        </div>
        <nav className="va-tabs" ref={tabs} aria-label="Sheets">
          {sheets.map((s) => {
            const dot = marked.has(upper(s)) ? (
              <i className="dot warn">!</i>
            ) : failing.has(upper(s)) ? (
              <i className="dot bad" />
            ) : changedSheets.has(upper(s)) ? (
              <i className="dot info" />
            ) : null;
            return (
              <button
                key={s}
                type="button"
                className={`tab${s === summary?.sheet ? ' on' : ''}`}
                aria-current={s === summary?.sheet ? 'page' : undefined}
                onClick={() => {
                  setPicked(null);
                  setInspecting(false);
                  setWanted(s);
                }}
              >
                <b>{s}</b>
                {dot}
              </button>
            );
          })}
        </nav>
      </header>
      <div className="va-main">
        <div className="va-vp">
          <Drawing
            reply={reply}
            coveredBelow={covered}
            compact={!wide}
            marks={marks}
            picked={picked}
            pulseSince={pulseSince}
            focus={focus}
            onLoaded={loaded}
            onError={failed}
            onUnavailable={setUnavailable}
            onPick={pick}
          />
          <div className="va-overlays">
            {at !== null && (
              <div className="va-banner" role="status">
                <div>
                  <b>Entry {at}, as it was</b>
                  <small>Read-only: rebuilt from the journal&rsquo;s undo states</small>
                </div>
                <button type="button" className="btn sm primary" onClick={backToLive}>
                  Back to live
                </button>
              </div>
            )}
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
        <BottomSheet wide={wide} onCovers={setCovered} peek={peek} body={body} raise={raise} />
      </div>
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
