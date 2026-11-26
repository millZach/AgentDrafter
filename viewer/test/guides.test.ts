import assert from 'node:assert/strict';
import { test } from 'node:test';

import { guideMade, type Part } from '../src/assistant.ts';
import {
  cardWords,
  clickNote,
  dots,
  endedNote,
  endsStepTool,
  givenToast,
  landed,
  lastWords,
  layerChosen,
  marked,
  nowWords,
  pulseOf,
  skipStep,
  startRun,
  stepDone,
  stopRun,
  summaryOf,
  synced,
  toastAfterStart,
  toolNote,
  undoAllBlocked,
  undoItBlocked,
  type Guide,
  type GuideStep,
} from '../src/guides.ts';
import { entryByline, type Entry } from '../src/journal.ts';

// The prototype's guide on the demo plant's DP-LD-101, as `guides` lists it.
const guide: Guide = {
  guide: 1,
  by: 'Assistant',
  at: '2026-11-21T22:22:20Z',
  base_entry: 1,
  title: 'Draw FT-101’s + wire to TB1-1',
  sheet: 'DP-LD-101',
  steps: [
    { step: 1, say: 'Make wire the current layer.', title: 'Make wire the current layer', setting: { current_layer: 'wire' }, points: [] },
    {
      step: 2,
      say: 'Draw the wire with Line.',
      title: 'Draw the wire',
      tool: 'line',
      points: [
        { label: 'FT-101’s + terminal', at: [3.2, 6.75, 0], snap: { id: 'DP-LD-101/device', snap: 'center', which: 'objects[5]' } },
        { label: 'TB1-1', at: [7.2, 6.75, 0], snap: { id: 'DP-LD-101/terminal-1', snap: 'midpoint' } },
      ],
      operation: { op: 'add_line' },
    },
    {
      step: 3,
      say: 'Number the wire 101+.',
      title: 'Number it 101+',
      tool: 'text',
      needs: [2],
      points: [{ label: 'the wire’s middle', waits_on: 2 }],
    },
    {
      step: 4,
      say: 'Move the number 0.04 in up.',
      title: 'Move the number above the wire',
      tool: 'move',
      needs: [2, 3],
      points: [{ label: '101+', waits_on: 3 }],
      operation: { op: 'move', by: [0, 0.04, 0] },
    },
  ],
};
const step = (n: number): GuideStep => guide.steps[n - 1] as GuideStep;

test('a guide starts at its first step, with nothing done', () => {
  const run = startRun(guide);
  assert.equal(run.at, 0);
  assert.deepEqual(run.status, ['todo', 'todo', 'todo', 'todo']);
  assert.deepEqual(dots(run), ['now', '', '', '']);
  assert.equal(run.end, null);
});

test('a setting step is done by the layer it names, in any case, and says so when another is picked', () => {
  assert.deepEqual(layerChosen(step(1), 'WIRE'), { done: true, note: null });
  const other = layerChosen(step(1), 'notes');
  assert.equal(other.done, false);
  assert.equal(other.note?.words, 'That made notes current; the step wants wire. Pick wire in the Layer box.');
});

test('the banner says what to do now, from what the renderer says the step wants', () => {
  assert.equal(nowWords(step(1), null, false), 'Pick wire in the ribbon’s Layer box.');
  assert.equal(nowWords(step(1), null, true), 'Open Tools, and pick wire in the Layer box.');
  assert.equal(nowWords(step(2), { phase: 'start' }, false), 'Start Line: type L and Enter, or click Line in the ribbon.');
  assert.equal(nowWords(step(2), { phase: 'start' }, true), 'Open Tools and tap Line: it pulses.');
  assert.equal(nowWords(step(2), { phase: 'click', point: 0 }, false), 'Click the ring: FT-101’s + terminal, where the Centre snap shows.');
  assert.equal(nowWords(step(2), { phase: 'click', point: 1 }, true), 'Tap the ring: TB1-1, where the Midpoint snap shows.');
  assert.equal(nowWords(step(2), { phase: 'finish' }, false), 'Press Enter, or click Done, to end the line.');
  assert.equal(nowWords(step(4), { phase: 'finish' }, false), 'Type @0,0.04 and press Enter.');
  assert.equal(nowWords(step(3), { phase: 'click', point: 0 }, false), 'The ring waits on step 2.');
  const typed = { ...step(3), operation: { op: 'add_text', contents: '101+' } };
  assert.equal(nowWords(typed, { phase: 'type' }, false), 'Type 101+, then press Enter twice.');
});

test('a change set that completes the step moves the guide on; one that does not offers to undo it, and the guide waits', () => {
  let run = stepDone(startRun(guide), guide);
  assert.equal(run.at, 1);
  assert.deepEqual(lastWords(run), { text: '✓ Step 1 done', tone: 'ok' });
  const wrong = { guide: 1, step: 2, of: 4, completes: false, message: 'Its `to` went to the midpoint of `DP-LD-101/terminal-2`, [7.2, 6.25, 0]; the step wants the midpoint of `DP-LD-101/terminal-1`, [7.2, 6.75, 0].' };
  run = landed(run, guide, wrong, 7);
  assert.equal(run.at, 1, 'the guide waits');
  assert.equal(run.note?.tone, 'warn');
  assert.ok(run.note?.words.endsWith('Undo it?'));
  assert.deepEqual(run.note?.act, { kind: 'undo', label: 'Undo it', entry: 7 });
  run = landed(run, guide, { ...wrong, completes: true, message: 'That completes step 2 of 4.' }, 9);
  assert.equal(run.at, 2);
  assert.equal(run.note, null);
  assert.deepEqual(run.entries, [7, 9]);
  assert.deepEqual(lastWords(run), { text: '✓ Step 2 done · change set 9', tone: 'ok' });
  // A check of another guide's step, or another step, does not move it.
  assert.equal(landed(run, guide, { ...wrong, step: 4, completes: true }, 10).at, 2);
});

test('done for the drafter is said so', () => {
  const run = stepDone({ ...startRun(guide), forMe: true }, guide, 3);
  assert.equal(run.status[0], 'forme');
  assert.equal(lastWords(run)?.text, '✓ Step 1 done for you · change set 3');
});

test('skipping a step that later steps need skips them too, and says why', () => {
  let run = stepDone(startRun(guide), guide);
  run = skipStep(run, guide);
  assert.deepEqual(run.status, ['done', 'skipped', 'needs', 'needs']);
  assert.equal(run.end, 'done');
  const s = summaryOf(run, guide, []);
  assert.equal(s.head, 'Finished: 1 of 4 steps done, 3 not done');
  assert.deepEqual(
    s.items.map((i) => i.small),
    ['', 'skipped', 'skipped: it needs what step 2 makes', 'skipped: it needs what step 2 makes'],
  );
  assert.equal(s.line, 'Nothing landed.');
  assert.equal(s.undoFrom, null);
});

test('a point or a pick the step does not want, and another tool, get a gentle note with what takes them back', () => {
  const note = clickNote({ words: 'That went to the midpoint of terminal-2; the step wants TB1-1.', take_back: 'point' });
  assert.equal(note.words, 'That went to the midpoint of terminal-2; the step wants TB1-1. Undo it?');
  assert.deepEqual(note.act, { kind: 'take_back', label: 'Undo it', what: 'point', place: 0 });
  const pick = clickNote({ words: 'That picked terminal-2; the step wants 101+.', take_back: 'pick', place: 27 });
  assert.deepEqual(pick.act, { kind: 'take_back', label: 'Take it out', what: 'pick', place: 27 });
  assert.equal(toolNote(step(2), 'line', '0'), null);
  assert.equal(toolNote(step(2), 'rectangle', 'wire')?.words, 'That started Rectangle; this step wants Line. End it?');
  assert.deepEqual(toolNote(step(2), 'rectangle', 'wire')?.act, { kind: 'end_tool', label: 'End Rectangle' });
  assert.equal(toolNote(step(1), 'line', '0')?.words, 'Line would draw on layer 0; this step wants wire current first. End Line and pick wire?');
  assert.equal(endedNote(step(2))?.words, 'Line ended with nothing drawn, so the step waits. Start Line again to go on, or skip the step.');
  assert.equal(endedNote(step(2))?.tone, 'info');
});

test('the summary offers one undo of the guide’s steps, from the first, saying how many later go with it', () => {
  let run = stepDone(startRun(guide), guide);
  run = landed(run, guide, { guide: 1, step: 2, of: 4, completes: true, message: '' }, 5);
  run = stopRun(run);
  let s = summaryOf(run, guide, [3, 5, 6]);
  assert.equal(s.head, 'Stopped at step 3 of 4');
  assert.equal(s.undoFrom, 5);
  assert.equal(s.undoLabel, 'Undo the guide’s steps and 1 later');
  assert.equal(s.line, 'What you drew is in the journal: change set 5, each undone as any edit.');
  s = summaryOf({ ...run, undone: 7 }, guide, []);
  assert.equal(s.line, 'Undone: entry 7 took back change set 5.');
  assert.equal(s.undoFrom, null);
});

test('a tool’s change set is marked with the step only on the guide’s sheet, on an operation step', () => {
  const cs = { operations: [{ op: 'add_line' }] };
  const run = startRun(guide);
  assert.equal(marked(cs, run, guide, 'DP-LD-101'), cs, 'a setting step is no change set’s');
  const two = stepDone(run, guide);
  assert.deepEqual(marked(cs, two, guide, 'dp-ld-101'), { ...cs, guide: { guide: 1, step: 2 } });
  assert.equal(marked(cs, two, guide, 'DP-LD-102'), cs);
  assert.equal(marked(cs, stopRun(two), guide, 'DP-LD-101'), cs);
  assert.equal(marked(cs, null, guide, 'DP-LD-101'), cs);
});

test('the journal marks a change set sent for a step', () => {
  const entry = { entry: 9, kind: 'change_set', at: '', guide: { guide: 1, step: 2, of: 4 } } as Entry;
  assert.equal(entryByline(entry), 'guide step 2 of 4');
});

test('the chat’s card belongs to a guide the assistant gave on the set shown', () => {
  const part = {
    kind: 'command',
    command: 'guide',
    input: { set: 'examples/demo-plant' },
    reply: { ok: true, result: { guide: 3, set: 'examples/demo-plant' } },
    ok: true,
  } as Part;
  assert.equal(guideMade(part, 'examples/demo-plant'), 3);
  assert.equal(guideMade(part, 'examples/demo-set'), null);
  assert.equal(guideMade({ ...part, command: 'propose' } as Part, 'examples/demo-plant'), null);
  assert.equal(cardWords('run', stepDone(startRun(guide), guide), 4), 'Running: step 2 of 4.');
  assert.equal(cardWords('end', stopRun(startRun(guide)), 4), 'Stopped.');
});

test('a late reply for another guide’s step leaves this run as it was', () => {
  const run = stepDone(startRun(guide), guide);
  const other = { guide: 2, step: 2, of: 4, completes: true, message: 'That completes step 2 of 4.' };
  assert.equal(landed(run, guide, other, 12), run);
});

test('stopping and restarting the same guide while an edit is pending ignores the stale reply', () => {
  const first = stepDone(startRun(guide), guide);
  const restarted = stepDone(startRun(guide), guide);
  assert.notEqual(first.runId, restarted.runId);
  const stale = { guide: 1, step: 2, of: 4, completes: true, message: 'That completes step 2 of 4.' };
  const after = landed(restarted, guide, stale, 12, first.runId);
  assert.equal(after, restarted);
  assert.equal(after.at, 1);
  assert.deepEqual(after.entries, []);
});

test('Undo it is not pressed through edits that landed after the change set', () => {
  assert.equal(undoItBlocked(7, [5, 7]), null);
  const note = undoItBlocked(7, [5, 7, 8, 9]);
  assert.equal(note?.words, 'Undo it would also take back 2 later edits, so nothing was undone. Undo from the journal if you mean to.');
  assert.equal(note?.act, undefined, 'nothing to press');
  assert.equal(undoItBlocked(7, [7, 8])?.words.includes('1 later edit,'), true);
});

test('Undo the guide’s steps is checked again against the journal when pressed', () => {
  let run = stepDone(startRun(guide), guide);
  run = landed(run, guide, { guide: 1, step: 2, of: 4, completes: true, message: '' }, 5);
  run = stopRun(run);
  const shown = summaryOf(run, guide, [3, 5, 6]);
  assert.equal(undoAllBlocked(run, guide, [3, 5, 6], 5, shown.undoLabel), null);
  assert.equal(
    undoAllBlocked(run, guide, [3, 5, 6, 9], 5, shown.undoLabel),
    'The journal changed since: the button now says “Undo the guide’s steps and 2 later”.',
  );
  assert.equal(undoAllBlocked(run, guide, [3], 5, shown.undoLabel), 'Its steps can no longer be undone.');
});

test('a step whose change set is undone, from anywhere, is to do again, and the guide steps back', () => {
  let run = stepDone(startRun(guide), guide);
  run = landed(run, guide, { guide: 1, step: 2, of: 4, completes: true, message: '' }, 7);
  assert.deepEqual(run.doneBy, [null, 7, null, null]);
  assert.equal(run.at, 2);
  const listing = (completedBy?: number): Guide => ({
    ...guide,
    steps: guide.steps.map((s) => (s.step === 2 ? { ...s, completed_by: completedBy } : s)),
  });
  // Still in place: as it was.
  assert.equal(synced(run, listing(7), 7), run);
  // A listing from before it landed says nothing of it.
  assert.equal(synced(run, listing(undefined), 6), run);
  // Undone: step 2 is to do again, and the guide is back at it.
  const back = synced(run, listing(undefined), 8);
  assert.equal(back.at, 1);
  assert.deepEqual(back.status, ['done', 'todo', 'todo', 'todo']);
  assert.deepEqual(back.doneBy, [null, null, null, null]);
  assert.equal(back.last, null);
  // An ended guide keeps its summary.
  assert.equal(synced(stopRun(run), listing(undefined), 8).at, 2);
});

test('Move and Copy are worded by the tool’s own phase: pick, Enter, base, then where it goes', () => {
  assert.equal(nowWords(step(4), { phase: 'pick', point: 0 }, false), 'Click 101+, in the ring, to pick it.');
  assert.equal(nowWords(step(4), { phase: 'end_pick' }, false), 'Press Enter, or click Done: what the step picks is picked.');
  assert.equal(nowWords(step(4), { phase: 'base' }, false), 'Click any point, as the base point.');
  assert.equal(nowWords(step(4), { phase: 'finish' }, false), 'Type @0,0.04 and press Enter.');
  const copy: GuideStep = {
    step: 5,
    say: 'Copy it up.',
    title: 'Copy it up',
    tool: 'copy',
    points: [{ label: '101+', object: 'DP-LD-101/text-1' }, { label: 'base', at: [5, 6, 0] }, { label: 'up', at: [5, 7, 0] }],
    operation: { op: 'copy', by: [0, 1, 0] },
  };
  assert.equal(nowWords(copy, { phase: 'finish' }, false), 'Press Enter, or click Done, to place the copy.');
  const erase: GuideStep = { step: 6, say: 'Erase it.', title: 'Erase it', tool: 'erase', points: [{ label: '101+', object: 'DP-LD-101/text-1' }] };
  assert.equal(nowWords(erase, { phase: 'end_pick' }, true), 'Tap Done to erase it.');
});

test('guides given between two looks each get a Start', () => {
  const who = (by: string) => by;
  assert.equal(givenToast([], who), null);
  const one = givenToast([guide], who);
  assert.equal(one?.title, 'Assistant gives a guide: Draw FT-101’s + wire to TB1-1');
  assert.deepEqual(one?.guides, [1]);
  const two = givenToast([guide, { ...guide, guide: 2, title: 'Number the wire' }], who);
  assert.equal(two?.title, '2 guides given: Draw FT-101’s + wire to TB1-1; Number the wire');
  assert.deepEqual(two?.guides, [1, 2]);
  assert.equal(two?.detail, 'Guide 1: 4 steps on DP-LD-101 · Guide 2: 4 steps on DP-LD-101, for you to do on your drawing.');
});

test('Skip and Stop end only the step’s own tool; one the drafter started keeps its work', () => {
  assert.equal(endsStepTool(step(2), 'line', 'DP-LD-101', 'DP-LD-101'), true);
  assert.equal(endsStepTool(step(2), 'line', 'dp-ld-101', 'DP-LD-101'), true, 'a sheet named in any case');
  assert.equal(endsStepTool(step(2), 'copy', 'DP-LD-101', 'DP-LD-101'), false, 'a Copy the drafter started keeps its copies');
  assert.equal(endsStepTool(step(2), null, 'DP-LD-101', 'DP-LD-101'), false);
  assert.equal(endsStepTool(step(1), 'line', 'DP-LD-101', 'DP-LD-101'), false, 'a setting step has no tool of its own');
});

test('Stop on another sheet keeps the line the drafter is drawing there', () => {
  // The guide runs on DP-LD-101; the drafter is drawing a Line of their own on DP-LD-102.
  assert.equal(endsStepTool(step(2), 'line', 'DP-LD-102', 'DP-LD-101'), false);
  assert.equal(endsStepTool(step(2), 'line', null, 'DP-LD-101'), false, 'no sheet shown, nothing of the step’s running');
});

test('a step with no rings asks for its tool’s points, with nothing pulsing, then Done', () => {
  const free: GuideStep = { step: 2, say: 'Draw the wire.', title: 'Draw the wire', tool: 'line', points: [], operation: { op: 'add_line' } };
  assert.equal(nowWords(free, { phase: 'free' }, false), 'Click a point for the line, as the step says.');
  assert.equal(nowWords(free, { phase: 'free' }, true), 'Tap a point for the line, as the step says.');
  assert.equal(pulseOf(free, { phase: 'free' }, false), undefined, 'Done waits for the line’s two points');
  assert.equal(pulseOf(free, { phase: 'finish' }, false), 'done');
});

test('a Copy step whose copy goes by a typed displacement asks for it, then for Done', () => {
  const copy: GuideStep = {
    step: 5,
    say: 'Copy it up.',
    title: 'Copy it up',
    tool: 'copy',
    points: [{ label: '101+', object: 'DP-LD-101/text-1' }, { label: 'base', at: [5, 6, 0] }],
    operation: { op: 'copy', by: [0, 1, 0] },
  };
  assert.equal(nowWords(copy, { phase: 'place' }, false), 'Type @0,1 and press Enter.');
  assert.equal(nowWords(copy, { phase: 'finish' }, false), 'Press Enter, or click Done, to place the copy.');
  assert.equal(pulseOf(copy, { phase: 'place' }, false), undefined);
  assert.equal(pulseOf(copy, { phase: 'finish' }, false), 'done');
  assert.equal(nowWords(step(4), { phase: 'place' }, false), 'Type @0,0.04 and press Enter.');
});

test('what pulses, for every phase the renderer reports', () => {
  const erase: GuideStep = { step: 6, say: 'Erase it.', title: 'Erase it', tool: 'erase', points: [{ label: '101+', object: 'DP-LD-101/text-1' }] };
  assert.equal(pulseOf(step(1), null, false), 'layer');
  assert.equal(pulseOf(step(2), { phase: 'start' }, false), 'line');
  assert.equal(pulseOf(step(2), null, false), 'line');
  assert.equal(pulseOf(erase, { phase: 'pick', point: 0 }, false), undefined);
  assert.equal(pulseOf(erase, { phase: 'end_pick' }, false), 'done', 'Erase: Done once picked');
  assert.equal(pulseOf(step(4), { phase: 'end_pick' }, false), 'done');
  assert.equal(pulseOf(step(4), { phase: 'base' }, false), undefined);
  assert.equal(pulseOf(step(4), { phase: 'place' }, false), undefined);
  assert.equal(pulseOf(step(4), { phase: 'finish' }, false), undefined, 'Move lands on its last point');
  assert.equal(pulseOf(step(2), { phase: 'click', point: 0 }, false), undefined);
  assert.equal(pulseOf(step(2), { phase: 'finish' }, false), 'done');
  assert.equal(pulseOf(step(3), { phase: 'type' }, false), undefined);
  assert.equal(pulseOf(step(2), { phase: 'finish' }, true), undefined, 'nothing while a note waits');
  assert.equal(pulseOf(null, null, false), undefined);
});

test('starting one guide of a toast leaves the other’s Start', () => {
  type Toast = { key: number; title: string; guides?: number[] };
  const toast: Toast = { key: 1, title: '2 guides given', guides: [1, 2] };
  const proposal: Toast = { key: 3, title: 'Proposal' };
  assert.deepEqual(toastAfterStart(toast, 1), { key: 1, title: '2 guides given', guides: [2] });
  assert.equal(toastAfterStart({ ...toast, guides: [2] }, 2), null);
  assert.equal(toastAfterStart(proposal, 1), null);
  assert.equal(toastAfterStart<Toast>(null, 1), null);
});
