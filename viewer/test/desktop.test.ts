import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  allowLossKey,
  buildDesktopSearch,
  canUndoDrawing,
  classifySource,
  dwgEditGuard,
  isAbsolutePath,
  kindBadge,
  latestActiveRevision,
  loadLocalRecents,
  matchOriginalObjectId,
  mergeRecent,
  nextRevisionHint,
  normalizeOriginalChangeSet,
  parseDesktopUrl,
  removeRecentLocal,
  resolveBeside,
  revisionAtEntry,
  saveLocalRecents,
  sourceDisplayName,
  sourceParentDir,
  undecodedSummary,
  type ReadReport,
  type RecentEntry,
} from '../src/desktop.ts';
import type { JournalReport } from '../src/journal.ts';

function sampleReadReport(overrides: Partial<ReadReport> = {}): ReadReport {
  return {
    source: '/tmp/DP-E-204.dwg',
    sheet: 'DP-E-204',
    message: 'read DP-E-204.dwg',
    dwg_version: 'AC1032',
    sha256: 'deadbeef0123456789abcdef',
    units: 'in',
    decoded: true,
    not_decoded: [],
    skipped: 0,
    recovered_errors: 0,
    objects: { line: 4 },
    not_read: [],
    tables_not_read: {},
    ...overrides,
  };
}

test('classifies drawing set folders, plant DWGs, and single YAML sheets across POSIX and Windows paths', () => {
  assert.equal(classifySource('/home/drafter/demo-plant'), 'set');
  assert.equal(classifySource('C:\\Users\\drafter\\demo-plant\\'), 'set');
  assert.equal(classifySource('/home/drafter/demo-plant/set.yaml'), 'set');
  assert.equal(classifySource('/home/drafter/DP-P-101.dwg'), 'dwg');
  assert.equal(classifySource('C:\\Drawings\\DP-P-101.DWG'), 'dwg');
  assert.equal(classifySource('/home/drafter/geometry.yaml'), 'sheet');
  assert.equal(classifySource('examples/one-line.yml'), 'sheet');
  // Explicit kind from native picker / URL overrides extension guessing (dotted folder names).
  assert.equal(classifySource('/home/drafter/project.yaml', 'set'), 'set');
  assert.equal(classifySource('/home/drafter/archive.dwg', 'set'), 'set');

  assert.equal(sourceDisplayName('/home/drafter/demo-plant/'), 'demo-plant');
  assert.equal(sourceDisplayName('C:\\Drawings\\DP-P-101.dwg'), 'DP-P-101.dwg');
  // POSIX path containing a literal backslash keeps the backslash intact.
  assert.equal(sourceDisplayName('/tmp/a\\b.dwg'), 'a\\b.dwg');
  assert.equal(sourceParentDir('/tmp/dir\\1/a\\b.dwg'), '/tmp/dir\\1');
  assert.equal(resolveBeside('/tmp/dir\\1/a\\b.dwg', 'a\\b.rev1.dwg'), '/tmp/dir\\1/a\\b.rev1.dwg');
  assert.equal(sourceDisplayName('.'), '.');
  assert.equal(kindBadge('set'), 'Set');
  assert.equal(kindBadge('dwg'), 'DWG');
  assert.equal(kindBadge('sheet'), 'Sheet');
  assert.equal(nextRevisionHint('DP-P-101.dwg'), 'DP-P-101.rev1.dwg');
});

test('resolves revision DWG paths beside POSIX and Windows originals', () => {
  assert.equal(isAbsolutePath('/tmp/a.dwg'), true);
  assert.equal(isAbsolutePath('C:\\Drawings\\a.dwg'), true);
  assert.equal(isAbsolutePath('\\\\server\\share\\a.dwg'), true);
  assert.equal(isAbsolutePath('a.rev1.dwg'), false);

  assert.equal(sourceParentDir('/tmp/plant/P-101.dwg'), '/tmp/plant');
  assert.equal(sourceParentDir('C:\\Plant\\P-101.dwg'), 'C:\\Plant');
  assert.equal(sourceParentDir('C:\\P-101.dwg'), 'C:\\');
  assert.equal(resolveBeside('/tmp/plant/P-101.dwg', 'P-101.rev1.dwg'), '/tmp/plant/P-101.rev1.dwg');
  assert.equal(resolveBeside('C:\\Plant\\P-101.dwg', 'P-101.rev1.dwg'), 'C:\\Plant\\P-101.rev1.dwg');
  assert.equal(resolveBeside('/tmp/plant/P-101.dwg', '/other/custom.dwg'), '/other/custom.dwg');
  // Valid POSIX filename starting with `\\` is resolved beside the POSIX original, NOT treated as a UNC path.
  assert.equal(resolveBeside('/demo/\\\\weird.dwg', '\\\\weird.rev1.dwg'), '/demo/\\\\weird.rev1.dwg');
  assert.equal(resolveBeside('\\\\server\\share\\P-101.dwg', '\\\\other\\share\\custom.dwg'), '\\\\other\\share\\custom.dwg');
});

test('parses desktop and plain browser URLs without changing browser defaults', () => {
  // Plain browser mode (no bridge, no ?desktop=1, no ?start=1) keeps exact existing default.
  assert.deepEqual(parseDesktopUrl('', false), {
    desktop: false,
    start: false,
    source: '.',
    kind: null,
    sheet: null,
    layout: null,
  });
  assert.deepEqual(parseDesktopUrl('?source=examples/demo-plant&sheet=DP-LD-102', false), {
    desktop: false,
    start: false,
    source: 'examples/demo-plant',
    kind: null,
    sheet: 'DP-LD-102',
    layout: null,
  });

  // Desktop mode with no source opens on the Start screen.
  assert.deepEqual(parseDesktopUrl('?desktop=1', false), {
    desktop: true,
    start: true,
    source: null,
    kind: null,
    sheet: null,
    layout: null,
  });
  assert.deepEqual(parseDesktopUrl('', true), {
    desktop: true,
    start: true,
    source: null,
    kind: null,
    sheet: null,
    layout: null,
  });
  // Desktop mode with a source and explicit kind opens it directly.
  assert.deepEqual(parseDesktopUrl('?desktop=1&source=%2Ftmp%2Fproject.yaml&kind=set', true), {
    desktop: true,
    start: false,
    source: '/tmp/project.yaml',
    kind: 'set',
    sheet: null,
    layout: null,
  });

  assert.equal(
    buildDesktopSearch({ desktop: true, start: true, source: null, kind: null, sheet: null, layout: null }),
    'desktop=1&start=1',
  );
  assert.equal(
    buildDesktopSearch({ desktop: true, start: false, source: '/tmp/project.yaml', kind: 'set', sheet: 'DP-LD-101', layout: null }),
    'desktop=1&source=%2Ftmp%2Fproject.yaml&kind=set&sheet=DP-LD-101',
  );
});

test('ADR-0004 edit guard locks undecoded plant DWGs until explicit allowLoss', () => {
  assert.equal(dwgEditGuard('set', null, false, false), 'editable');
  assert.equal(dwgEditGuard('sheet', null, false, false), 'sheet-readonly');
  // Fail-closed while read is in flight or failed.
  assert.equal(dwgEditGuard('dwg', null, true, false), 'loading');
  assert.equal(dwgEditGuard('dwg', null, false, false), 'view-only');
  // Clean DWG is immediately editable.
  assert.equal(dwgEditGuard('dwg', sampleReadReport({ decoded: true }), false, false), 'editable');
  // Undecoded DWG starts view-only, unlocks only when allowLoss is granted.
  const undecoded = sampleReadReport({
    decoded: false,
    not_decoded: [{ kind: 'MADE_UP_DEVICE', count: 2, handles: ['2A7', '2A9'] }],
    skipped: 1,
    recovered_errors: 2,
    diagnostics_truncated: true,
    read_past_damage: 'truncated section',
  });
  assert.equal(dwgEditGuard('dwg', undecoded, false, false), 'view-only');
  assert.equal(dwgEditGuard('dwg', undecoded, false, true), 'allowed-loss');

  // Content-bound allow-loss key changes when the file at the same path is replaced.
  assert.notEqual(allowLossKey('/tmp/a.dwg', 'sha-a'), allowLossKey('/tmp/a.dwg', 'sha-b'));
  assert.equal(allowLossKey('/tmp/a.dwg', 'sha-a'), allowLossKey('/tmp/a.dwg', 'sha-a'));

  // Undo is locked while a plant DWG is loading or view-only, unlocked once editable or allowed-loss.
  assert.equal(canUndoDrawing('dwg', true, 'loading'), false);
  assert.equal(canUndoDrawing('dwg', true, 'view-only'), false);
  assert.equal(canUndoDrawing('dwg', true, 'editable'), true);
  assert.equal(canUndoDrawing('dwg', true, 'allowed-loss'), true);
  assert.equal(canUndoDrawing('sheet', true, 'sheet-readonly'), false);
  assert.equal(canUndoDrawing('set', true, 'editable'), true);
  assert.equal(canUndoDrawing('set', false, 'editable'), true);

  const summary = undecodedSummary(undecoded);
  assert.ok(summary.includes('2 undecoded MADE_UP_DEVICE records (2A7, 2A9)'), summary);
  assert.ok(summary.includes('1 skipped'), summary);
  assert.ok(summary.includes('2 recovered errors'), summary);
  assert.ok(summary.includes('diagnostics truncated'), summary);
  assert.ok(summary.includes('read past damage'), summary);
});

test('resolves active and stepped revisions from an original DWG journal and normalizes handle IDs', () => {
  const journal: JournalReport = {
    set: '/plant/P-101.dwg',
    message: '2 entries',
    can_undo: [1],
    entries: [
      {
        entry: 1,
        kind: 'change_set',
        at: '2026-11-22T18:00:00Z',
        landed_on: ['P-101'],
        original_revision: {
          original_file: 'P-101.dwg',
          original_sha256: 'aaa',
          dwg_version: 'AC1027',
          revision_dwg: 'P-101.rev1.dwg',
          revision_sha256: 'bbb',
        },
      },
      {
        entry: 2,
        kind: 'change_set',
        at: '2026-11-22T18:05:00Z',
        landed_on: ['P-101'],
        undone_by: 3,
        original_revision: {
          original_file: 'P-101.dwg',
          original_sha256: 'aaa',
          dwg_version: 'AC1027',
          revision_dwg: 'P-101.rev2.dwg',
          revision_sha256: 'ccc',
        },
      },
      {
        entry: 3,
        kind: 'undo',
        at: '2026-11-22T18:06:00Z',
        undid: [2],
      },
      {
        entry: 4,
        kind: 'change_set',
        at: '2026-11-22T18:07:00Z',
        landed_on: ['P-101'],
        original_revision: {
          original_file: 'P-101.dwg',
          original_sha256: 'aaa',
          dwg_version: 'AC1027',
          revision_dwg: 'alternate.dwg',
          revision_sha256: 'ddd',
          standalone_output: true,
        },
      },
    ],
  };

  // Skips standalone `--output` entry 4 and matches original_sha256 'aaa' -> entry 1.
  assert.deepEqual(latestActiveRevision(journal, '/plant/P-101.dwg', 'aaa'), {
    path: '/plant/P-101.rev1.dwg',
    file: 'P-101.rev1.dwg',
    entry: 1,
    dwgVersion: 'AC1027',
    sha256: 'bbb',
    originalSha256: 'aaa',
  });
  // If original file on disk was replaced by a different file ('zzz'), NEVER show A's revision!
  assert.equal(latestActiveRevision(journal, '/plant/P-101.dwg', 'zzz'), null);
  assert.equal(revisionAtEntry(journal, 1, '/plant/P-101.dwg', 'zzz'), null);
  assert.deepEqual(revisionAtEntry(journal, 1, '/plant/P-101.dwg', 'aaa'), {
    path: '/plant/P-101.rev1.dwg',
    file: 'P-101.rev1.dwg',
    sha256: 'bbb',
  });

  // Change set generated while displaying P-101.rev1.dwg rewrites sheet & handle prefix back to P-101.
  const cs = {
    operations: [
      { op: 'add_line', sheet: 'P-101.rev1', from: [0, 0, 0], to: [1, 1, 0] },
      { op: 'move', target: { id: 'P-101.rev1/handle:2F' }, by: [1, 0, 0] },
    ],
  };
  const normalized = normalizeOriginalChangeSet(cs, 'P-101', 'P-101.rev1');
  assert.deepEqual(normalized, {
    operations: [
      { op: 'add_line', sheet: 'P-101', from: [0, 0, 0], to: [1, 1, 0] },
      { op: 'move', target: { id: 'P-101/handle:2F' }, by: [1, 0, 0] },
    ],
  });

  assert.equal(matchOriginalObjectId('P-101/handle:2F', 'P-101.rev1/handle:2F'), true);
  assert.equal(matchOriginalObjectId('P-101/handle:2F', 'P-101.rev1/handle:3A'), false);
  assert.equal(matchOriginalObjectId('geometry/line1', 'geometry/line1'), true);
});

test('merges, caps, removes, and persists fallback local recents cleanly', () => {
  const mk = (i: number): RecentEntry => ({
    path: `/sets/set-${i}`,
    kind: 'set',
    name: `set-${i}`,
    opened_at_ms: i,
  });
  let list: RecentEntry[] = [];
  for (let i = 0; i < 25; i += 1) list = mergeRecent(list, mk(i));
  assert.equal(list.length, 20);
  assert.equal(list[0]?.path, '/sets/set-24');

  list = mergeRecent(list, { ...mk(10), subtitle: 'reopened' });
  assert.equal(list.length, 20);
  assert.equal(list[0]?.path, '/sets/set-10');
  assert.equal(list.filter((e) => e.path === '/sets/set-10').length, 1);

  list = removeRecentLocal(list, '/sets/set-10');
  assert.equal(list.length, 19);

  const mem = new Map<string, string>();
  const store = {
    getItem: (k: string) => mem.get(k) ?? null,
    setItem: (k: string, v: string) => {
      mem.set(k, v);
    },
  };
  saveLocalRecents(store, list);
  assert.deepEqual(loadLocalRecents(store), list);
});
