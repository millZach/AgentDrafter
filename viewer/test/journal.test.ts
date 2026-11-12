import assert from 'node:assert/strict';
import { test } from 'node:test';

import { eventsRequest, readEvents } from '../src/api.ts';
import {
  changedBy,
  changedOn,
  entryState,
  entryTitle,
  journalReplaced,
  lastUndoable,
  newest,
  notIssued,
  recordedEntries,
  sheetsTouched,
  since,
  undoneWith,
  type JournalReport,
} from '../src/journal.ts';
import { focusOn, pulseStrength, PULSE_MS } from '../src/view.ts';

test('a stream of events is read whole event by whole event, the rest kept for later', () => {
  const { events, rest } = readEvents('event: journal\ndata: {"ok":true}\n\n: still watching\n\nevent: journal\ndata: {"o');
  assert.deepEqual(events, [{ event: 'journal', data: '{"ok":true}' }]);
  assert.equal(rest, 'event: journal\ndata: {"o');
  const more = readEvents(`${rest}k":false}\r\n\r\n`);
  assert.deepEqual(more.events, [{ event: 'journal', data: '{"ok":false}' }]);
  assert.equal(more.rest, '');
  // Data over several lines joins with line breaks; an unnamed event is a message.
  assert.deepEqual(readEvents('data: a\ndata: b\n\n').events, [{ event: 'message', data: 'a\nb' }]);
});

test('the event stream is asked for as a command is, its token as a bearer, never in the address', () => {
  const { url, init } = eventsRequest('demo-plant', 'ab'.repeat(32));
  assert.equal(url, '/events');
  assert.equal(init.method, 'POST');
  assert.equal(init.body, JSON.stringify({ set: 'demo-plant' }));
  const headers = init.headers as Record<string, string>;
  assert.equal(headers.Authorization, `Bearer ${'ab'.repeat(32)}`);
  assert.equal(headers['Content-Type'], 'application/json');
  assert.equal(eventsRequest('.', null).url, '/events');
  assert.equal((eventsRequest('.', null).init.headers as Record<string, string>).Authorization, undefined);
});

/** A made-up journal: a template edit, a change set that failed on a sheet, an undo of it, a failed attempt, and an accept, then a change set after it. */
const report: JournalReport = {
  set: 'demo-plant',
  message: '',
  can_undo: [6],
  entries: [
    {
      entry: 1,
      kind: 'change_set',
      at: '2026-11-11T20:05:29Z',
      change_set: { operations: [{ op: 'move', target: { id: 'DP-LD-101/border' }, by: [0.1, 0, 0] }] },
      landed_on: ['DP-LD-101', 'DP-LD-102'],
      objects: [
        { id: 'DP-LD-101/border', how: 'changed' },
        { id: 'DP-LD-102/border', how: 'changed' },
      ],
      undo_state: 'kept',
      issued_by: 5,
    },
    {
      entry: 2,
      kind: 'change_set',
      at: '2026-11-11T20:06:00Z',
      change_set: {
        operations: [
          { op: 'add_circle', sheet: 'DP-LD-101', id: 'tank' },
          { op: 'set_attribute', target: { name: 'title-block', sheet: 'DP-LD-103' }, tag: 'REV', value: 'A' },
          { op: 'delete', target: { id: 'DP-LD-101/note' } },
        ],
      },
      landed_on: ['DP-LD-101'],
      failed_on: [{ sheet: 'DP-LD-103', code: 'unknown_name', message: 'no such tag' }],
      objects: [
        { id: 'DP-LD-101/tank', how: 'added' },
        { id: 'DP-LD-101/note', how: 'deleted' },
      ],
      undo_state: 'undone',
      undone_by: 3,
    },
    { entry: 3, kind: 'undo', at: '2026-11-11T20:07:00Z', undid: [2] },
    { entry: 4, kind: 'failed_attempt', at: '2026-11-11T20:08:00Z', change_set: { operations: [{ op: 'nope' }] }, error: { code: 'unknown_name', message: 'no operation `nope`' } },
    { entry: 5, kind: 'accept', at: '2026-11-11T20:09:00Z', via: 'mcp', issued: [1] },
    {
      entry: 6,
      kind: 'change_set',
      at: '2026-11-11T20:10:00Z',
      change_set: { operations: [{ op: 'move', target: { id: 'DP-LD-102/device' }, by: [0.5, 0, 0] }] },
      landed_on: ['DP-LD-102'],
      objects: [{ id: 'DP-LD-102/device', how: 'changed' }],
      undo_state: 'kept',
    },
  ],
};

test('each entry is named, and says what became of it', () => {
  assert.equal(entryTitle(report.entries[0]!), 'move border');
  assert.equal(entryTitle(report.entries[1]!), 'add_circle on DP-LD-101, set REV = A on title-block and 1 more');
  assert.equal(entryTitle(report.entries[2]!), 'Undo of change set 2');
  assert.equal(entryTitle(report.entries[4]!), 'Accept: issued change set 1');
  const state = (n: number, failing: string[] = []) => entryState(report.entries[n - 1]!, report, new Set(failing)).label;
  assert.equal(state(1), 'Issued');
  assert.equal(state(2), 'Undone');
  assert.equal(state(3), 'Undo');
  assert.equal(state(4), 'Failed attempt');
  assert.equal(state(5), 'Accept, the MCP server');
  assert.equal(state(6), 'Landed');
  assert.equal(state(6, ['DP-LD-102']), 'Failed check');
  const marked = { ...report, inconsistent: { message: '', failures: [{ sheet: 'DP-LD-102', code: 'x', message: '', landed_on: [] }] } };
  const failedHere = { ...report.entries[5]!, failed_on: [{ sheet: 'DP-LD-102', code: 'x', message: '' }] };
  assert.equal(entryState(failedHere, marked, new Set()).label, 'Inconsistent');
});

test('Undo last takes the newest change set an undo can take; accept issues those not issued', () => {
  assert.equal(lastUndoable(report), 6);
  assert.equal(lastUndoable({ ...report, can_undo: [] }), null);
  assert.deepEqual(undoneWith({ ...report, can_undo: [4, 6, 7] }, 4), [6, 7]);
  assert.deepEqual(
    notIssued(report).map((e) => e.entry),
    [6],
    'not the issued one, nor the undone one',
  );
  assert.equal(newest(report), 6);
  assert.equal(newest({ ...report, entries: [] }), 0);
  assert.deepEqual(
    since(report, 4).map((e) => e.entry),
    [5, 6],
  );
});

test('the sheets an entry touched are drawn again, and its changes on a sheet are marked', () => {
  assert.deepEqual(sheetsTouched(report.entries[0]!, report), ['DP-LD-101', 'DP-LD-102']);
  assert.deepEqual(sheetsTouched(report.entries[2]!, report), ['DP-LD-101'], 'an undo touches what it undid');
  assert.deepEqual(sheetsTouched(report.entries[3]!, report), [], 'a failed attempt changed nothing');
  assert.deepEqual(sheetsTouched(report.entries[4]!, report), [], 'nor did an accept');
  assert.deepEqual(changedOn(report.entries[1], 'dp-ld-101'), [{ id: 'DP-LD-101/tank', how: 'added' }], 'a deleted object is not marked');
  assert.deepEqual(changedOn(report.entries[0], 'DP-LD-103'), []);
  assert.deepEqual(changedOn(report.entries[2], 'DP-LD-101'), [], 'an undo marks nothing');
  assert.deepEqual(
    changedBy(report, 'dp-ld-101/border').map((e) => e.entry),
    [1],
  );
});

test('a journal cut back, or whose entries seen were taken away or rewritten, is not the journal seen', () => {
  const seen = recordedEntries(report);
  const only = (numbers: number[]): JournalReport => ({ ...report, entries: report.entries.filter((e) => numbers.includes(e.entry)) });
  assert.equal(journalReplaced(seen, report), false, 'the same journal');
  const grown = { ...report, entries: [...report.entries, { entry: 7, kind: 'undo' as const, at: '2026-11-11T20:11:00Z', undid: [6] }] };
  assert.equal(journalReplaced(seen, grown), false, 'a journal that only grew');
  // What has become of an entry since is not what it recorded.
  const later = { ...report, entries: report.entries.map((e) => (e.entry === 6 ? { ...e, undo_state: 'undone' as const, undone_by: 7 } : e)) };
  assert.equal(journalReplaced(seen, later), false, 'an entry undone since is the same entry');
  assert.equal(journalReplaced(seen, only([1, 2, 3, 4, 5])), true, 'cut back: the newest entry is gone');
  assert.equal(journalReplaced(seen, only([2, 3, 4, 5, 6])), true, 'the first entry taken away: the newest is still 6');
  const rewritten = { ...report, entries: report.entries.map((e) => (e.entry === 3 ? { ...e, at: '2026-11-12T08:00:00Z' } : e)) };
  assert.equal(journalReplaced(seen, rewritten), true, 'an entry of a number seen rewritten');
});

test('a change set\'s marks pulse three times as it lands, then hold', () => {
  assert.equal(pulseStrength(0), 1);
  assert.ok(Math.abs(pulseStrength(600) - 0.25) < 1e-9, 'a quarter at the middle of a pulse');
  assert.equal(pulseStrength(1200), 1);
  assert.equal(pulseStrength(PULSE_MS), 1);
  assert.equal(pulseStrength(Number.NaN), 1);
  for (let ms = 0; ms < PULSE_MS; ms += 50) {
    const s = pulseStrength(ms);
    assert.ok(s >= 0.25 - 1e-9 && s <= 1, `${ms}: ${s}`);
  }
});

test('showing a change set\'s objects zooms in on them, no less than the fit and no more than 3.2 times it', () => {
  const area = { x: 0, y: 0, width: 1000, height: 600 };
  const fitted = { left: -1, top: 12, scale: 50 };
  // A small box: zoomed in as far as allowed, its middle at the area's middle.
  const small = focusOn([5, 5, 5.1, 5.1], area, fitted);
  assert.equal(small.scale, 160);
  assert.ok(Math.abs((5.05 - small.left) * small.scale - 500) < 1e-9);
  assert.ok(Math.abs((small.top - 5.05) * small.scale - 300) < 1e-9);
  // A box larger than the fit: no less than the fit.
  assert.equal(focusOn([-50, -50, 50, 50], area, fitted).scale, 50);
});
