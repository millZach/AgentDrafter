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

export type EntryKind = 'change_set' | 'failed_attempt' | 'undo' | 'accept';

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
  if (target) return `${name} ${target}`;
  if (op.sheet) return `${name} on ${op.sheet}`;
  if (op.template) return `${name} in ${op.template}`;
  return name;
}

/** What an entry is, in a few words, for its line in the journal and its toast. */
export function entryTitle(entry: Entry): string {
  switch (entry.kind) {
    case 'undo':
      return `Undo of change set${(entry.undid?.length ?? 0) > 1 ? 's' : ''} ${(entry.undid ?? []).join(', ')}`;
    case 'accept':
      return entry.issued?.length
        ? `Accept: issued change set${entry.issued.length > 1 ? 's' : ''} ${entry.issued.join(', ')}`
        : 'Accept: issued the set as it stood';
    default: {
      const ops = entry.change_set?.operations ?? [];
      const first = ops.slice(0, 2).map(operationWords).join(', ');
      const more = ops.length > 2 ? ` and ${ops.length - 2} more` : '';
      const fixes = entry.change_set?.fixes?.length ? `fixes ${entry.change_set.fixes.join(', ')}` : '';
      return [first + more, fixes].filter(Boolean).join('; ') || 'An empty change set';
    }
  }
}

export type StateClass = 'landed' | 'failed' | 'undone' | 'issued' | 'inconsistent' | 'undo' | 'accept' | 'check';

/** An entry's badge: what has become of it. */
export function entryState(entry: Entry, report: JournalReport, failing: ReadonlySet<string>): { cls: StateClass; label: string } {
  if (entry.kind === 'failed_attempt') return { cls: 'failed', label: 'Failed attempt' };
  if (entry.kind === 'undo') return { cls: 'undo', label: 'Undo' };
  if (entry.kind === 'accept') return { cls: 'accept', label: `Accept, ${viaWords(entry.via)}` };
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
