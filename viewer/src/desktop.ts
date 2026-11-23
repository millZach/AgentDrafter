// Desktop shell bridge & pure state helpers for Issue #158 (Tauri v2 shell).
//
// In a Tauri desktop window, commands reach the native Rust shell through
// `window.__TAURI_INTERNALS__.invoke`. In unit and Chromium browser tests,
// `window.__AGENTDRAFTER_DESKTOP__` can inject a test bridge. In plain
// `agentdrafter serve` browser mode, no bridge is present and every existing
// browser flow behaves identically to before.

import type { Entry, JournalReport } from './journal.ts';

/** What kind of drawing source is open (Zach's A1, 2026-11-22). */
export type SourceKind = 'set' | 'dwg' | 'sheet';

/** Parses an explicit `?kind=` query parameter if valid. */
export function parseSourceKind(value: string | null | undefined): SourceKind | null {
  if (value === 'set' || value === 'dwg' || value === 'sheet') return value;
  return null;
}

/** A folder or file classified by the desktop shell. */
export interface InspectedPath {
  kind: SourceKind;
  path: string;
  name: string;
  detail: string;
}

/** One entry in the persisted Recent drawings & sets list. */
export interface RecentEntry {
  path: string;
  kind: SourceKind;
  name: string;
  opened_at_ms: number;
  subtitle?: string;
}

/** One group of records the DWG library or core could not decode/read. */
export interface DecodeRecord {
  kind: string;
  count: number;
  handles: string[];
}

/** Reply from `agentdrafter read <dwg> --no-mirror` (#70). */
export interface ReadReport {
  source: string;
  sheet: string;
  message: string;
  dwg_version: string;
  /** SHA-256 fingerprint of the original DWG bytes read. */
  sha256: string;
  units: string | null;
  decoded: boolean;
  not_decoded: DecodeRecord[];
  skipped: number;
  recovered_errors: number;
  warnings?: number;
  diagnostics_truncated?: boolean;
  read_past_damage?: string;
  objects: Record<string, number>;
  not_read: DecodeRecord[];
  tables_not_read: Record<string, number>;
  mirror?: string;
}

/** Native desktop shell commands invoked by the page. */
export interface DesktopBridge {
  pickDrawingSetFolder(): Promise<InspectedPath | null>;
  pickDrawingFile(filter?: 'dwg' | 'yaml'): Promise<InspectedPath | null>;
  inspectPath(path: string): Promise<InspectedPath>;
  resolveSiblingPath?(base: string, sibling: string): Promise<string>;
  listRecents(): Promise<RecentEntry[]>;
  recordRecent(entry: { path: string; kind: SourceKind; name: string; subtitle?: string }): Promise<RecentEntry[]>;
  removeRecent(path: string): Promise<RecentEntry[]>;
  openInNewWindow(source: string | null, title?: string, kind?: SourceKind | null): Promise<string>;
  setWindowTitle(title: string): Promise<void>;
}

interface TauriInternals {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
}

declare global {
  interface Window {
    __TAURI_INTERNALS__?: TauriInternals;
    __AGENTDRAFTER_DESKTOP__?: DesktopBridge;
  }
}

/** Local storage key used as a fallback recents list when no native bridge is present. */
export const RECENTS_STORAGE_KEY = 'agentdrafter-desktop-recents';

/** Maximum number of recent items kept. */
export const MAX_RECENTS = 20;

/** Returns the active desktop shell bridge, or null in plain browser mode. */
export function getDesktopBridge(): DesktopBridge | null {
  if (typeof window === 'undefined') return null;
  if (window.__AGENTDRAFTER_DESKTOP__) return window.__AGENTDRAFTER_DESKTOP__;
  const tauri = window.__TAURI_INTERNALS__;
  if (tauri && typeof tauri.invoke === 'function') {
    return {
      pickDrawingSetFolder: () => tauri.invoke<InspectedPath | null>('pick_drawing_set_folder'),
      pickDrawingFile: (filter) => tauri.invoke<InspectedPath | null>('pick_drawing_file', { filter: filter ?? null }),
      inspectPath: (path) => tauri.invoke<InspectedPath>('inspect_path', { path }),
      resolveSiblingPath: (base, sibling) => tauri.invoke<string>('resolve_sibling_path', { base, sibling }),
      listRecents: () => tauri.invoke<RecentEntry[]>('list_recents'),
      recordRecent: ({ path, kind, name, subtitle }) =>
        tauri.invoke<RecentEntry[]>('record_recent', { path, kind, name, subtitle: subtitle ?? null }),
      removeRecent: (path) => tauri.invoke<RecentEntry[]>('remove_recent', { path }),
      openInNewWindow: (source, title, kind) =>
        tauri.invoke<string>('open_in_new_window', { source, kind: kind ?? null, title: title ?? null }),
      setWindowTitle: (title) => tauri.invoke<void>('set_window_title', { title }),
    };
  }
  return null;
}

/**
 * Whether `path` uses Windows path syntax (`C:\…`, `C:/…`, or UNC `\\server\…`).
 * Any POSIX path (including relative POSIX paths containing `/` such as
 * `\\yard/\\part.dwg` or bare POSIX filenames starting with `\\`) never treats
 * literal `\` characters as directory separators.
 */
export function isWindowsPath(path: string): boolean {
  if (/^[A-Za-z]:[\\/]/.test(path)) return true;
  return path.startsWith('\\\\') && !path.includes('/') && path.indexOf('\\', 2) > 2;
}

function stripTrailingSeparators(path: string): string {
  const trimmed = path.trim();
  if (isWindowsPath(trimmed)) return trimmed.replace(/[\\/]+$/, '');
  return trimmed.replace(/\/+$/, '');
}

/**
 * Fallback suffix-based source classifier when no explicit `kind` was provided
 * by `inspect_path`, the native picker, Recents, or `?kind=set|dwg|sheet`.
 */
export function classifySource(path: string, explicitKind?: SourceKind | null): SourceKind {
  if (explicitKind) return explicitKind;
  const trimmed = stripTrailingSeparators(path);
  const lower = trimmed.toLowerCase();
  if (lower.endsWith('.dwg')) return 'dwg';
  const base = sourceDisplayName(trimmed).toLowerCase();
  if (base === 'set.yaml') return 'set';
  if (lower.endsWith('.yaml') || lower.endsWith('.yml')) return 'sheet';
  return 'set';
}

function displayNameWithMode(path: string, windows: boolean): string {
  const trimmed = windows ? path.trim().replace(/[\\/]+$/, '') : path.trim().replace(/\/+$/, '');
  if (!trimmed || trimmed === '.') return trimmed || '.';
  const parts = windows ? trimmed.split(/[\\/]/) : trimmed.split('/');
  return parts[parts.length - 1] || trimmed;
}

/**
 * Human-readable name for a source path (file or folder basename across POSIX
 * and Windows, preserving literal backslashes inside POSIX filenames).
 */
export function sourceDisplayName(path: string): string {
  return displayNameWithMode(path, isWindowsPath(path));
}

/** Whether `path` is absolute on POSIX (`/…`) or Windows (`C:\…`, `C:/…`, `\\server\…`). */
export function isAbsolutePath(path: string): boolean {
  return path.startsWith('/') || isWindowsPath(path);
}

/**
 * Parent directory of `path`, using Windows separators only for Windows paths
 * so POSIX paths containing literal `\` stay intact.
 */
export function sourceParentDir(path: string): string {
  const trimmed = stripTrailingSeparators(path);
  if (isWindowsPath(trimmed)) {
    const slash = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
    if (slash < 0) return '';
    if (slash === 2 && /^[A-Za-z]:[\\/]/.test(trimmed)) return trimmed.slice(0, 3);
    return trimmed.slice(0, slash);
  }
  const slash = trimmed.lastIndexOf('/');
  if (slash < 0) return '';
  if (slash === 0) return '/';
  return trimmed.slice(0, slash);
}

/**
 * Resolves a path `relOrAbs` (such as a revision DWG filename from the journal)
 * beside `originalPath`.
 *
 * When `originalPath` is a POSIX path (not a Windows drive/UNC path), only a
 * leading `/` counts as absolute so valid POSIX filenames starting with `\\`
 * (e.g. `\\weird.rev1.dwg`) are resolved beside `originalPath` rather than
 * misclassified as Windows UNC paths.
 */
export function resolveBeside(originalPath: string, relOrAbs: string): string {
  if (!relOrAbs) return originalPath;
  const winOrig = isWindowsPath(originalPath);
  if (winOrig ? isAbsolutePath(relOrAbs) : relOrAbs.startsWith('/')) return relOrAbs;
  const parent = sourceParentDir(originalPath);
  if (!parent) return relOrAbs;
  if (winOrig) {
    const sep = originalPath.includes('\\') && !originalPath.includes('/') ? '\\' : '/';
    const cleanedRel = sep === '\\' ? relOrAbs.replace(/\//g, '\\') : relOrAbs;
    return parent.endsWith('/') || parent.endsWith('\\') ? `${parent}${cleanedRel}` : `${parent}${sep}${cleanedRel}`;
  }
  return parent.endsWith('/') ? `${parent}${relOrAbs}` : `${parent}/${relOrAbs}`;
}

/** Default filename the core chooses for the first revision beside `originalName`. */
export function nextRevisionHint(originalName: string): string {
  const base = sourceDisplayName(originalName);
  const stem = base.replace(/\.dwg$/i, '');
  return `${stem}.rev1.dwg`;
}

/** Active revision metadata resolved from an original DWG's journal. */
export interface ActiveRevision {
  path: string;
  file: string;
  entry: number;
  dwgVersion: string;
  sha256: string;
  originalSha256: string;
}

function matchesOriginalSha(recorded: string | undefined, expected: string | null | undefined): boolean {
  if (!expected) return true;
  if (!recorded) return false;
  return recorded.toLowerCase() === expected.toLowerCase();
}

function matchesWorkingRevisionStem(revisionDwg: string, origPathOrName: string, winOrig: boolean): boolean {
  if (!revisionDwg) return false;
  if (winOrig) {
    if (revisionDwg.includes('/') || revisionDwg.includes('\\') || isAbsolutePath(revisionDwg)) return false;
  } else if (revisionDwg.includes('/')) {
    return false;
  }
  if (!revisionDwg.endsWith('.dwg')) return false;
  const withoutExt = revisionDwg.slice(0, -4);
  const revIdx = withoutExt.lastIndexOf('.rev');
  if (revIdx < 0) return false;
  const stem = withoutExt.slice(0, revIdx);
  const num = withoutExt.slice(revIdx + 4);
  if (!/^[1-9]\d*$/.test(num)) return false;
  const origStem = displayNameWithMode(origPathOrName, winOrig).replace(/\.dwg$/i, '');
  return winOrig ? stem.toLowerCase() === origStem.toLowerCase() : stem === origStem;
}

function isWorkingRevisionMeta(
  meta: NonNullable<Entry['original_revision']> | undefined,
  originalPath: string,
  winOrig: boolean,
): meta is NonNullable<Entry['original_revision']> {
  if (!meta?.revision_dwg || meta.standalone_output) return false;
  return (
    matchesWorkingRevisionStem(meta.revision_dwg, originalPath, winOrig) ||
    (!!meta.original_file && matchesWorkingRevisionStem(meta.revision_dwg, meta.original_file, winOrig))
  );
}

/**
 * The `original_sha256` recorded on the newest active (finished, non-undone,
 * non-rolled-back) original-DWG revision entry in `journal`, or null if none.
 */
export function newestActiveOriginalSha(journal: JournalReport | null): string | null {
  if (!journal) return null;
  for (let i = journal.entries.length - 1; i >= 0; i -= 1) {
    const e = journal.entries[i] as Entry | undefined;
    if (!e || e.kind !== 'change_set' || e.undone_by || e.rolled_back) continue;
    const sha = e.original_revision?.original_sha256;
    if (sha) return sha;
  }
  return null;
}

/**
 * Latest finished, non-undone, non-standalone working revision DWG recorded in
 * an original DWG's journal. Inspects ONLY the newest active non-standalone
 * working revision entry so an A -> B -> A disk swap never skips past B's newer
 * entry back to A's older revision (staying 100% aligned with the core).
 */
export function latestActiveRevision(
  journal: JournalReport | null,
  originalPath: string,
  expectedOriginalSha256?: string | null,
): ActiveRevision | null {
  if (!journal) return null;
  const winOrig = isWindowsPath(originalPath);
  for (let i = journal.entries.length - 1; i >= 0; i -= 1) {
    const e = journal.entries[i] as Entry | undefined;
    if (!e || e.kind !== 'change_set' || e.undone_by || e.rolled_back) continue;
    const meta = e.original_revision;
    if (!isWorkingRevisionMeta(meta, originalPath, winOrig)) continue;
    if (!matchesOriginalSha(meta.original_sha256, expectedOriginalSha256)) return null;
    return {
      path: resolveBeside(originalPath, meta.revision_dwg),
      file: displayNameWithMode(meta.revision_dwg, winOrig),
      entry: e.entry,
      dwgVersion: meta.dwg_version,
      sha256: meta.revision_sha256,
      originalSha256: meta.original_sha256,
    };
  }
  return null;
}

/**
 * The working revision DWG that was active just after `entryNumber` in an
 * original DWG's journal, or null when stepping back to before any working
 * revision (or when the newest active working revision at `entryNumber` was
 * made against a different original SHA-256). Standalone `--output` entries
 * never enter working-revision selection.
 */
export function revisionAtEntry(
  journal: JournalReport | null,
  entryNumber: number,
  originalPath: string,
  expectedOriginalSha256?: string | null,
): { path: string; file: string; sha256: string } | null {
  if (!journal) return null;
  const winOrig = isWindowsPath(originalPath);
  // Walk backwards from entryNumber to find the newest non-undone, non-standalone working revision at that point.
  const undoneByThen = new Set<number>();
  for (const e of journal.entries) {
    if (e.entry > entryNumber) break;
    if (e.kind === 'undo') {
      for (const n of e.undid ?? []) undoneByThen.add(n);
    }
  }
  for (let i = journal.entries.length - 1; i >= 0; i -= 1) {
    const e = journal.entries[i];
    if (!e || e.entry > entryNumber) continue;
    if (e.kind !== 'change_set' || e.rolled_back || undoneByThen.has(e.entry) || e.undone_by) continue;
    const meta = e.original_revision;
    if (!isWorkingRevisionMeta(meta, originalPath, winOrig)) continue;
    if (!matchesOriginalSha(meta.original_sha256, expectedOriginalSha256)) return null;
    return {
      path: resolveBeside(originalPath, meta.revision_dwg),
      file: displayNameWithMode(meta.revision_dwg, winOrig),
      sha256: meta.revision_sha256,
    };
  }
  return null;
}

/**
 * Rewrites `sheet` and `${shownSheet}/handle:` prefixes inside a change set back to
 * `origSheet` when the page is currently displaying a revision DWG beside the original
 * (e.g. displaying `DP-P-101.rev1.dwg` while sending `change` against `DP-P-101.dwg`).
 */
export function normalizeOriginalChangeSet<T>(value: T, origSheet: string, shownSheet: string): T {
  if (!origSheet || !shownSheet || origSheet.toUpperCase() === shownSheet.toUpperCase()) return value;
  const shownUpper = shownSheet.toUpperCase();
  const handlePrefixUpper = `${shownUpper}/HANDLE:`;
  const rewrite = (node: unknown): unknown => {
    if (Array.isArray(node)) return node.map(rewrite);
    if (node && typeof node === 'object') {
      const out: Record<string, unknown> = {};
      for (const [k, v] of Object.entries(node as Record<string, unknown>)) {
        if (k === 'sheet' && typeof v === 'string' && v.toUpperCase() === shownUpper) {
          out[k] = origSheet;
        } else if (k === 'id' && typeof v === 'string' && v.toUpperCase().startsWith(handlePrefixUpper)) {
          out[k] = `${origSheet}${v.slice(shownSheet.length)}`;
        } else {
          out[k] = rewrite(v);
        }
      }
      return out;
    }
    return node;
  };
  return rewrite(value) as T;
}

/**
 * Matches a journal `ObjectChanged.id` against a drawn object's `id`, matching
 * either exact ID (case-insensitive) or identical `/handle:<HEX>` suffix so
 * revision DWGs highlight edited handles from the original's journal.
 */
export function matchOriginalObjectId(changedId: string, objectId: string | undefined): boolean {
  if (!objectId) return false;
  const a = changedId.toUpperCase();
  const b = objectId.toUpperCase();
  if (a === b) return true;
  const ah = a.indexOf('/HANDLE:');
  const bh = b.indexOf('/HANDLE:');
  return ah >= 0 && bh >= 0 && a.slice(ah) === b.slice(bh);
}

/**
 * Content-bound consent key for ADR-0004 `allow_loss` approval: ties approval
 * strictly to BOTH the path AND the original DWG's exact SHA-256 fingerprint so
 * replacing the file at the same path always revokes prior consent.
 */
export function allowLossKey(source: string, sha256: string | null | undefined): string | null {
  if (!source || !sha256) return null;
  return `${source}::${sha256.toLowerCase()}`;
}

/** Editing guard state for the open drawing (ADR-0004). */
export type EditGuard = 'editable' | 'view-only' | 'allowed-loss' | 'sheet-readonly' | 'loading';

/**
 * Computes whether the open drawing can be edited right now:
 * - Drawing sets are always editable (live).
 * - Single YAML sheets are read-only (`sheet-readonly`).
 * - Plant DWGs fail closed (`loading` while `read` is in flight, `view-only` when
 *   `decoded: false` without explicit content-bound session `allowLoss` or if `read` failed).
 */
export function dwgEditGuard(
  kind: SourceKind,
  readReport: ReadReport | null,
  readPending: boolean,
  allowLoss: boolean,
): EditGuard {
  if (kind === 'set') return 'editable';
  if (kind === 'sheet') return 'sheet-readonly';
  if (!readReport) return readPending ? 'loading' : 'view-only';
  if (readReport.decoded) return 'editable';
  return allowLoss ? 'allowed-loss' : 'view-only';
}

/**
 * Whether Undo is permitted on the open drawing (Blocker #7):
 * view-only / loading undecoded plant DWGs lock Undo too so reopening an
 * undecoded original with an unissued revision cannot delete its revision
 * before explicit ADR-0004 acknowledgement.
 */
export function canUndoDrawing(kind: SourceKind, desktop: boolean, guard: EditGuard): boolean {
  if (!desktop) return true;
  if (kind === 'sheet') return false;
  if (kind !== 'dwg') return true;
  return guard === 'editable' || guard === 'allowed-loss';
}

/** Short human-readable summary of why a plant DWG has `decoded: false`. */
export function undecodedSummary(report: ReadReport): string {
  const parts: string[] = [];
  if (report.not_decoded.length) {
    const total = report.not_decoded.reduce((n, r) => n + r.count, 0);
    const first = report.not_decoded[0];
    if (report.not_decoded.length === 1 && first) {
      const handles = first.handles.length ? ` (${first.handles.slice(0, 4).join(', ')}${first.handles.length > 4 ? ', …' : ''})` : '';
      parts.push(`${first.count} undecoded ${first.kind} record${first.count === 1 ? '' : 's'}${handles}`);
    } else {
      parts.push(`${total} undecoded records across ${report.not_decoded.length} kinds`);
    }
  }
  if (report.skipped > 0) parts.push(`${report.skipped} skipped`);
  if (report.recovered_errors > 0) parts.push(`${report.recovered_errors} recovered error${report.recovered_errors === 1 ? '' : 's'}`);
  if (report.diagnostics_truncated) parts.push('diagnostics truncated');
  if (report.read_past_damage) parts.push('read past damage');
  return parts.join('; ') || 'incomplete decode';
}

/** Parsed initial desktop/browser URL state. */
export interface DesktopUrlState {
  /** Whether the desktop shell chrome (Start screen, Open…, switcher) is active. */
  desktop: boolean;
  /** Whether to show the Start screen instead of opening a drawing immediately. */
  start: boolean;
  /** The drawing source to open (null when on the Start screen). */
  source: string | null;
  /** Explicit validated source kind (`?kind=set|dwg|sheet`), or null if unspecified. */
  kind: SourceKind | null;
  sheet: string | null;
  layout: string | null;
}

/**
 * Parses URL query parameters while preserving 100% of existing `agentdrafter serve`
 * browser behavior when neither `?desktop=1`, `?start=1`, nor a desktop bridge is present.
 */
export function parseDesktopUrl(search: string, hasBridge: boolean): DesktopUrlState {
  const query = new URLSearchParams(search);
  const desktop = hasBridge || query.get('desktop') === '1' || query.get('start') === '1';
  const rawSource = query.get('source');
  const start = desktop && (query.get('start') === '1' || !rawSource);
  return {
    desktop,
    start,
    source: start ? null : rawSource || '.',
    kind: start ? null : parseSourceKind(query.get('kind')),
    sheet: start ? null : query.get('sheet') || null,
    layout: start ? null : query.get('layout') || null,
  };
}

/** Builds a clean query string (without leading `?`) reflecting current desktop state. */
export function buildDesktopSearch(state: DesktopUrlState): string {
  const query = new URLSearchParams();
  if (state.desktop) query.set('desktop', '1');
  if (state.start || !state.source) {
    if (state.desktop) query.set('start', '1');
  } else {
    if (state.source !== '.' || state.desktop) query.set('source', state.source);
    if (state.desktop && state.kind) query.set('kind', state.kind);
    if (state.sheet) query.set('sheet', state.sheet);
    if (state.layout) query.set('layout', state.layout);
  }
  return query.toString();
}

/** Deduplicates, prepends, and caps a recents list (used for local fallback and tests). */
export function mergeRecent(entries: readonly RecentEntry[], item: RecentEntry, max = MAX_RECENTS): RecentEntry[] {
  const filtered = entries.filter((e) => e.path !== item.path);
  return [item, ...filtered].slice(0, max);
}

/** Removes `path` from a recents list. */
export function removeRecentLocal(entries: readonly RecentEntry[], path: string): RecentEntry[] {
  return entries.filter((e) => e.path !== path);
}

/** Loads fallback recents from localStorage (browser test mode when no native bridge exists). */
export function loadLocalRecents(storage: Pick<Storage, 'getItem'> | null): RecentEntry[] {
  if (!storage) return [];
  try {
    const raw = storage.getItem(RECENTS_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as unknown;
    return Array.isArray(parsed) ? (parsed as RecentEntry[]).slice(0, MAX_RECENTS) : [];
  } catch {
    return [];
  }
}

/** Saves fallback recents to localStorage. */
export function saveLocalRecents(storage: Pick<Storage, 'setItem'> | null, entries: readonly RecentEntry[]): void {
  if (!storage) return;
  try {
    storage.setItem(RECENTS_STORAGE_KEY, JSON.stringify(entries.slice(0, MAX_RECENTS)));
  } catch {
    // Ignore quota/private-mode storage errors.
  }
}

/** Badge text for a source kind (`Set`, `DWG`, `Sheet`). */
export function kindBadge(kind: SourceKind): string {
  switch (kind) {
    case 'set':
      return 'Set';
    case 'dwg':
      return 'DWG';
    case 'sheet':
      return 'Sheet';
  }
}
