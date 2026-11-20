import assert from 'node:assert/strict';
import { test } from 'node:test';

import { readEvents } from '../src/api.ts';
import { keepButton, previewTitle, promptFor, type PreviewReport } from '../src/frame.ts';
import { entryByline, entryState, entryTitle, sheetsTouched, whoWords, type Entry, type JournalReport } from '../src/journal.ts';
import {
  ago,
  cardLines,
  onSheet,
  peekWords,
  placeCards,
  proposalTitle,
  proposedWords,
  sinceWords,
  staleWords,
  standing,
  tabCounts,
  type Proposal,
} from '../src/proposals.ts';

const SHEETS = ['DP-LD-101', 'DP-LD-102', 'DP-LD-201'];

const proposal = (over: Partial<Proposal> = {}): Proposal => ({
  proposal: 2,
  by: 'Assistant',
  at: '2026-11-19T20:00:00Z',
  base_entry: 4,
  changed_since: [],
  change_set: { operations: [{ op: 'move', target: { id: 'DP-LD-101/service' }, by: [0.1, 0, 0] }] },
  sheets: SHEETS,
  ...over,
});

const journal = (entries: Partial<Entry>[]): JournalReport => ({
  set: 'demo-plant',
  message: '',
  entries: entries.map((e, i) => ({ entry: i + 1, kind: 'change_set', at: '2026-11-19T20:00:00Z', ...e }) as Entry),
  can_undo: [],
});

const report = (sheets: { sheet: string; ok: boolean; code?: string; message?: string }[], findings: { sheet: string; rule: string; message: string }[] = []): PreviewReport => ({
  change: {
    landed: sheets.filter((s) => s.ok).length,
    failed: sheets.filter((s) => !s.ok).length,
    sheets: sheets.map((s) => ({ sheet: s.sheet, ok: s.ok, error: s.ok ? undefined : { code: s.code ?? 'locked', message: s.message ?? '' } })),
  },
  checks: { result: { findings } },
  objects: [],
});

test('a proposal is named by what it does, and each sheet it reaches counts it on its tab', () => {
  assert.equal(proposalTitle(proposal()), 'Move service');
  const other = proposal({ proposal: 3, sheets: ['DP-LD-102'], change_set: { operations: [{ op: 'set_attribute', tag: 'CHECKED', value: 'JK', target: { id: 'DP-LD-102/title-block' } }] } });
  assert.equal(proposalTitle(other), 'Set CHECKED = JK on title-block');
  const counts = tabCounts([proposal(), other]);
  assert.equal(counts.get('DP-LD-101'), 1);
  assert.equal(counts.get('DP-LD-102'), 2);
  assert.equal(counts.get('DP-LD-301'), undefined);
  assert.deepEqual(
    onSheet([proposal(), other], 'dp-ld-102').map((p) => p.proposal),
    [2, 3],
  );
  assert.equal(peekWords(2, 3), '2 proposals on this sheet, 3 in all');
  assert.equal(peekWords(0, 1), '1 proposal waiting, marked on their tabs');
});

test('a card says whether the set changed since, and what keeping it would do now, in the core’s words', () => {
  const j = journal([{}, {}, {}, {}, {}, { kind: 'undo', undid: [5] }]);
  const stale = proposal({ changed_since: [5, 6] });
  assert.equal(sinceWords([5, 6], j), 'change set 5 and undo 6');
  assert.equal(sinceWords([5], j), 'change set 5');

  const lands = report(SHEETS.map((sheet) => ({ sheet, ok: true })));
  assert.equal(standing(lands), 'ok');
  assert.deepEqual(cardLines(stale, lands, j), [
    { tone: 'stale', icon: '↻', text: 'Set changed since: change set 5 and undo 6 landed after it.' },
    { tone: 'ok', icon: '✓', text: 'All 3 would take it.' },
  ]);

  const fails = report(
    SHEETS.map((sheet) => ({ sheet, ok: true })),
    [{ sheet: 'DP-LD-201', rule: 'text_on_line', message: 'a line runs through `service`' }],
  );
  assert.equal(standing(fails), 'warn');
  assert.deepEqual(cardLines(proposal(), fails, j), [{ tone: 'warn', icon: '!', text: 'All 3 would take it, but DP-LD-201 would fail a check.' }]);

  // Refused: the sheet that refuses it, in the core's words, never one
  // only held back by it.
  const refused = report([
    { sheet: 'DP-LD-101', ok: false, code: 'shared_edit_failed', message: 'held back' },
    { sheet: 'DP-LD-102', ok: false, code: 'locked', message: 'layer `enclosure` is locked' },
    { sheet: 'DP-LD-201', ok: false, code: 'shared_edit_failed', message: 'held back' },
  ]);
  assert.equal(standing(refused), 'bad');
  assert.deepEqual(cardLines(proposal(), refused, j), [
    { tone: 'bad', icon: '✕', text: 'Refused now on DP-LD-102: layer enclosure is locked. Keep can’t land it.' },
  ]);
  assert.deepEqual(keepButton(refused), { label: 'Nothing can land', disabled: true });
  assert.deepEqual(keepButton(lands), { label: 'Keep', disabled: false });
  // On its way: only whether the set changed.
  assert.deepEqual(cardLines(proposal(), null, j), []);
});

test('the band of a proposal opened says who made it, when, and what landed since', () => {
  const now = Date.parse('2026-11-19T20:34:00Z');
  assert.equal(ago('2026-11-19T20:00:00Z', now), '34 min ago');
  assert.equal(ago('2026-11-19T20:33:50Z', now), 'just now');
  assert.equal(ago('2026-11-19T17:30:00Z', now), '3 h ago');
  assert.equal(proposedWords(proposal({ by: 'agent (MCP)' }), '08:00 PM', now), 'Proposed by agent (MCP) at 08:00 PM, 34 min ago. Nothing lands until you keep it.');
  assert.equal(staleWords(proposal(), null), null);
  assert.match(staleWords(proposal({ changed_since: [5] }), journal([{}, {}, {}, {}, {}])) ?? '', /^The set has changed since it was proposed: change set 5 landed after it\. Kept now, it applies to the set as it stands/);
  assert.deepEqual(previewTitle({ title: '', detail: '', path: '', sheets: SHEETS, edit: 'Move service' }, 2), { chip: 'Proposal 2', title: 'Move service', reach: 'on 3 sheets' });
  assert.deepEqual(previewTitle({ title: '', detail: '', path: '', sheets: ['DP-LD-104'], edit: 'Add circle' }, 4).reach, 'on DP-LD-104');
  const p = promptFor({ at: null, cannot: false, reaching: false, selected: null, previewing: true, proposal: true });
  assert.equal(p.command, 'PROPOSAL');
  assert.match(JSON.stringify(p.parts), /Esc closes it, and it keeps waiting/);
});

test('the journal reads a kept proposal as one entry, and a discard as a line with no undo (J1)', () => {
  const kept: Entry = {
    entry: 7,
    kind: 'change_set',
    at: '2026-11-19T20:44:00Z',
    change_set: { operations: [{ op: 'move', target: { id: 'DP-LD-101/service' } }] },
    landed_on: SHEETS,
    proposal: 2,
    proposed_by: 'Assistant',
    kept_by: 'drafter',
  };
  const discard: Entry = { entry: 8, kind: 'discard', at: '2026-11-19T20:45:00Z', proposal: 3, proposed_by: 'agent (HTTP API)', discarded_by: 'drafter' };
  assert.equal(whoWords('drafter'), 'you');
  assert.equal(`${entryTitle(kept)} · ${entryByline(kept)}`, 'Move service · proposed by Assistant · kept by you');
  assert.equal(`${entryTitle(discard)} · ${entryByline(discard)}`, 'Discarded proposal 3 · by you · proposed by agent (HTTP API)');
  const j = journal([]);
  j.entries = [kept, discard];
  assert.deepEqual(entryState(discard, j, new Set()), { cls: 'discard', label: 'Discarded' });
  assert.deepEqual(sheetsTouched(discard, j), [], 'a discard changes no sheet');
  assert.equal(entryByline({ ...kept, proposal: undefined }), '', 'a change set sent as one has no byline');
});

test('the event stream’s proposals come as their own event', () => {
  const { events } = readEvents('event: journal\ndata: {"ok":true}\n\nevent: proposals\ndata: {"ok":true,"result":{"proposals":[]}}\n\n');
  assert.deepEqual(
    events.map((e) => e.event),
    ['journal', 'proposals'],
  );
});

test('each card goes beside its change, clear of the others, and a leader joins them', () => {
  // Two changes near each other at the left: the cards go to their right,
  // the second moved down clear of the first.
  const placed = placeCards(
    [
      { w: 200, h: 80, box: [100, 100, 140, 120] },
      { w: 200, h: 80, box: [100, 130, 140, 150] },
    ],
    1000,
    700,
  );
  assert.equal(placed[0]?.x, 154);
  assert.equal(placed[1]?.x, 154);
  assert.ok((placed[1]?.y ?? 0) >= (placed[0]?.y ?? 0) + 80 + 6, 'clear of the first');
  // The leader runs from the card's left edge to the change's right.
  assert.deepEqual(placed[0]?.leader.slice(0, 1), [154]);
  assert.equal(placed[0]?.leader[2], 144);
  // No room at the right: the card goes to the left of its change.
  const left = placeCards([{ w: 200, h: 80, box: [800, 300, 900, 320] }], 1000, 700);
  assert.equal(left[0]?.x, 800 - 200 - 14);
  // Kept inside the drawing.
  const low = placeCards([{ w: 200, h: 80, box: [100, 690, 140, 699] }], 1000, 700);
  assert.ok((low[0]?.y ?? 0) + 80 <= 700);
});
