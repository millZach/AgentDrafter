// The journal as the page reads it: the `journal` command's reply, which
// the server's event stream sends each time the set's journal changes, and
// what the page makes of it: what each entry is called, which sheets it
// touched, what it changed on a sheet, and what Undo last and Accept would
// take. Plain functions of the reply, tested without a browser.

export interface Operation {
  op?: string;
  target?: { id?: string; name?: string; sheet?: string };
  sheet?: string;
  template?: string;
  tag?: string;
  value?: string;
  [key: string]: unknown;
}

export interface SheetFailed {
  sheet: string;
  code: string;
  message: string;
}

export interface ObjectChanged {
  id: string;
  how: 'added' | 'changed' | 'deleted';
}

export interface CheckFailed {
  sheet: string;
  rule: string;
  id?: string;
  message: string;
}

export type EntryKind = 'change_set' | 'failed_attempt' | 'undo' | 'accept' | 'discard';

export interface Entry {
  entry: number;
  kind: EntryKind;
  at: string;
  change_set?: { operations?: Operation[]; fixes?: string[] };
  landed_on?: string[];
  failed_on?: SheetFailed[];
  error?: { code: string; message: string };
  undid?: number[];
  objects?: ObjectChanged[];
  via?: string;
  issued?: number[];
  failed_checks?: CheckFailed[];
  files?: { path: string; before: string | null; after: string | null }[];
  /** The proposal a change set or a failed attempt kept, or a discard ended (#155). */
  proposal?: number;
  /** How a commit that never finished was rolled back: it changed nothing. */
  rolled_back?: unknown;
  proposed_by?: string;
  kept_by?: string;
  discarded_by?: string;
  undo_state?: 'kept' | 'pruned' | 'undone';
  undone_by?: number;
  pruned_by?: number;
  issued_by?: number;
}

export interface Failure {
  sheet: string;
  code: string;
  message: string;
  landed_on: string[];
}

export interface JournalReport {
  set: string;
  message: string;
  entries: Entry[];
  can_undo: number[];
  inconsistent?: { message: string; failures: Failure[] };
}

/** The interfaces an accept came through, as the reviewer reads them. */
const VIA: Record<string, string> = {
  viewer: 'the viewer',
  cli: 'the command line',
  http: 'the HTTP API',
  mcp: 'the MCP server',
  python: 'the Python library',
};

export const viaWords = (via: string | undefined): string => (via ? (VIA[via] ?? via) : 'an unknown way');

/** What an operation does to which object, short, such as `move device`. */
export function operationWords(op: Operation): string {
  const name = op.op ?? '?';
  const target = op.target?.name ?? op.target?.id?.split('/').slice(1).join('/') ?? op.target?.id;
  if (name === 'set_attribute') return `set ${op.tag ?? '?'} = ${op.value ?? ''}${target ? ` on ${target}` : ''}`;
  // A copy names its objects and, when it makes several, how many (#66).
  if (name === 'copy') {
    const several = Array.isArray(op.targets) && op.targets.length ? `${op.targets.length} objects` : target;
    const copies = Array.isArray(op.offsets) && op.offsets.length > 1 ? `, ${op.offsets.length} copies` : '';
    return `copy${several ? ` ${several}` : ''}${copies}`;
  }
  if (target) return `${name} ${target}`;
  if (op.sheet) return `${name} on ${op.sheet}`;
  if (op.template) return `${name} in ${op.template}`;
  return name;
}

/** The name the page gives the drafter when it keeps or discards a proposal, which it shows as `you`. */
export const DRAFTER = 'drafter';

/** Who proposed, kept or discarded, as the drafter reads it: `you` for the drafter's own. */
export const whoWords = (by: string | undefined): string => (!by ? 'someone' : by === DRAFTER ? 'you' : by);

/** What a change set's operations do, in a few words: `move service, set CHECKED = JK on title-block`. */
export function operationsTitle(operations: Operation[] | undefined, fixes?: string[]): string {
  const ops = operations ?? [];
  const first = ops.slice(0, 2).map(operationWords).join(', ');
  const more = ops.length > 2 ? ` and ${ops.length - 2} more` : '';
  const fixed = fixes?.length ? `fixes ${fixes.join(', ')}` : '';
  return [first + more, fixed].filter(Boolean).join('; ') || 'An empty change set';
}

/** The same words with a capital, as a proposal is named: `Move service`. */
export const capital = (text: string): string => text.charAt(0).toUpperCase() + text.slice(1);

/**
 * Who a kept proposal or a discard came from, as its line in the journal
 * says it beside its title (Zach's J1, 2026-11-19): `proposed by Assistant
 * · kept by you`, or `by you · proposed by Assistant`. Empty for any other.
 */
export function entryByline(entry: Entry): string {
  if (entry.proposal === undefined) return '';
  if (entry.kind === 'discard') return `by ${whoWords(entry.discarded_by)} · proposed by ${whoWords(entry.proposed_by)}`;
  // A keep the set refused is an attempt: it kept nothing.
  if (entry.kind === 'failed_attempt') return `proposed by ${whoWords(entry.proposed_by)} · keep attempted by ${whoWords(entry.kept_by)} · refused`;
  return `proposed by ${whoWords(entry.proposed_by)} · kept by ${whoWords(entry.kept_by)}`;
}

/** Whether proposal `n` has ended: a change set kept it, or a discard ended it, and was not rolled back. */
export function proposalEnded(report: JournalReport, n: number): boolean {
  return report.entries.some((e) => e.proposal === n && (e.kind === 'change_set' || e.kind === 'discard') && !e.rolled_back);
}

/** What an entry is, in a few words, for its line in the journal and its toast. */
export function entryTitle(entry: Entry): string {
  switch (entry.kind) {
    case 'discard':
      return `Discarded proposal ${entry.proposal ?? '?'}`;
    case 'undo':
      return `Undo of change set${(entry.undid?.length ?? 0) > 1 ? 's' : ''} ${(entry.undid ?? []).join(', ')}`;
    case 'accept':
      return entry.issued?.length
        ? `Accept: issued change set${entry.issued.length > 1 ? 's' : ''} ${entry.issued.join(', ')}`
        : 'Accept: issued the set as it stood';
    default: {
      const title = operationsTitle(entry.change_set?.operations, entry.change_set?.fixes);
      // A kept proposal is named as its card named it.
      return entry.proposal !== undefined ? capital(title) : title;
    }
  }
}

export type StateClass = 'landed' | 'failed' | 'undone' | 'issued' | 'inconsistent' | 'undo' | 'accept' | 'check' | 'discard';

/** An entry's badge: what has become of it. */
export function entryState(entry: Entry, report: JournalReport, failing: ReadonlySet<string>): { cls: StateClass; label: string } {
  if (entry.kind === 'failed_attempt') return { cls: 'failed', label: 'Failed attempt' };
  if (entry.kind === 'undo') return { cls: 'undo', label: 'Undo' };
  if (entry.kind === 'accept') return { cls: 'accept', label: `Accept, ${viaWords(entry.via)}` };
  if (entry.kind === 'discard') return { cls: 'discard', label: 'Discarded' };
  if (entry.undone_by) return { cls: 'undone', label: 'Undone' };
  if (entry.issued_by) return { cls: 'issued', label: 'Issued' };
  const marked = report.inconsistent?.failures ?? [];
  const failedHere = (entry.failed_on ?? []).some((f) => marked.some((m) => m.sheet.toUpperCase() === f.sheet.toUpperCase()));
  if (failedHere) return { cls: 'inconsistent', label: 'Inconsistent' };
  const checks = (entry.landed_on ?? []).filter((s) => failing.has(s.toUpperCase())).length;
  if (checks) return { cls: 'check', label: 'Failed check' };
  return { cls: 'landed', label: 'Landed' };
}

/** The change sets still in place that no accept has issued, the oldest first. */
export function notIssued(report: JournalReport): Entry[] {
  return report.entries.filter((e) => e.kind === 'change_set' && !e.undone_by && !e.issued_by);
}

/** The change set Undo last undoes: the newest an undo can take, or none. */
export function lastUndoable(report: JournalReport): number | null {
  const can = report.can_undo;
  return can.length ? (can[can.length - 1] ?? null) : null;
}

/** The change sets an undo of `change_set` takes with it: every later one an undo can take. */
export function undoneWith(report: JournalReport, changeSet: number): number[] {
  return report.can_undo.filter((n) => n > changeSet);
}

/** The newest entry's number, 0 for an empty journal. */
export const newest = (report: JournalReport): number => report.entries.at(-1)?.entry ?? 0;

/**
 * The sheets whose drawings `entry` changed: those a change set landed on,
 * and for an undo those of the change sets it undid. A failed attempt and
 * an accept change no file a sheet is drawn from.
 */
export function sheetsTouched(entry: Entry, report: JournalReport): string[] {
  if (entry.kind === 'change_set') return entry.landed_on ?? [];
  if (entry.kind === 'undo') {
    const sheets = new Set<string>();
    for (const n of entry.undid ?? []) {
      for (const s of report.entries.find((e) => e.entry === n)?.landed_on ?? []) sheets.add(s);
    }
    return [...sheets];
  }
  return [];
}

/** What an entry recorded, apart from what has become of it since, to tell it from another of its number. */
export const recorded = (e: Entry): string =>
  JSON.stringify([e.kind, e.at, e.change_set ?? null, e.files ?? null, e.undid ?? null, e.issued ?? null]);

/** What each entry of the journal recorded, by its number. */
export const recordedEntries = (report: JournalReport): Map<number, string> =>
  new Map(report.entries.map((e) => [e.entry, recorded(e)]));

/**
 * The keys of the drawings kept as the set is now, `live|SHEET|LAYOUT`,
 * that the sheets `touched` (in capitals) make stale: each touched sheet's,
 * whatever its layout, and the one kept for a sheet not yet named.
 */
export function staleDrawings(keys: Iterable<string>, touched: readonly string[]): string[] {
  if (!touched.length) return [];
  return [...keys].filter((key) => {
    const [at, sheet] = key.split('|');
    // A preview's drawing (`pv…`) is the change set applied to the set as
    // it was asked for: any change to the set may change what it would do
    // on every sheet, its verdicts and checks among it.
    if (at?.startsWith('pv')) return true;
    return at === 'live' && (sheet === '' || touched.includes(sheet ?? ''));
  });
}

/**
 * Whether `report` is not the journal the entries in `seen` came from, but
 * one cut back or rewritten: an entry seen is gone, as when a commit is
 * rolled back or the journal's first lines are taken away, or an entry of
 * a number seen now records something else. A journal that only grew is
 * the same journal.
 */
export function journalReplaced(seen: ReadonlyMap<number, string>, report: JournalReport): boolean {
  const now = recordedEntries(report);
  for (const [entry, was] of seen) {
    if (now.get(entry) !== was) return true;
  }
  return false;
}

/** The entries that came after `seen`, the oldest first. */
export const since = (report: JournalReport, seen: number): Entry[] => report.entries.filter((e) => e.entry > seen);

/** What `entry` changed on `sheet`: each object's ID and how, deleted ones left out. */
export function changedOn(entry: Entry | undefined, sheet: string): ObjectChanged[] {
  if (!entry || entry.kind !== 'change_set') return [];
  const prefix = `${sheet.toUpperCase()}/`;
  return (entry.objects ?? []).filter((o) => o.how !== 'deleted' && o.id.toUpperCase().startsWith(prefix));
}

/** The first sheet `entry` changed, to show it on: where it landed, or failed. */
export function firstSheet(entry: Entry, report: JournalReport): string | null {
  return sheetsTouched(entry, report)[0] ?? entry.failed_on?.[0]?.sheet ?? null;
}

/** The entries whose change sets changed the object with `id`, the newest first. */
export function changedBy(report: JournalReport, id: string): Entry[] {
  const wanted = id.toUpperCase();
  return report.entries.filter((e) => e.kind === 'change_set' && (e.objects ?? []).some((o) => o.id.toUpperCase() === wanted)).reverse();
}

/** The time an entry was added, as a clock reads it in the page's own zone. */
export function clock(at: string): string {
  const when = new Date(at);
  if (Number.isNaN(when.getTime())) return at;
  return when.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
}
