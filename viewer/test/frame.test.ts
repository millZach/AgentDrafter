import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  coordinates,
  liveLine,
  nameOf,
  PANELS,
  paperWords,
  placeBeside,
  promptFor,
  SOON,
  toggledWords,
  toggleFor,
  typeWords,
  unitWords,
  type Part,
  type PromptState,
} from '../src/frame.ts';

const words = (parts: Part[]) => parts.map((p) => ('text' in p ? p.text : `<${p.option}>`)).join('');
const none: PromptState = { soon: null, at: null, cannot: false, reaching: false, selected: null };

test('the ribbon shows Line, Circle, Rectangle and Text in Draw, Move and Erase in Modify, with AutoCAD aliases', () => {
  assert.deepEqual(
    PANELS.map((p) => [p.title, p.tools.map((t) => `${t.label} ${t.command} ${t.alias}`)]),
    [
      ['Draw', ['Line LINE L', 'Circle CIRCLE C', 'Rectangle RECTANG REC', 'Text TEXT T']],
      ['Modify', ['Move MOVE M', 'Erase ERASE E']],
    ],
  );
  for (const tool of PANELS.flatMap((p) => p.tools)) assert.ok(tool.tip.length > 20, `${tool.label} has a tip in plain words`);
});

test('the command bar prompts to select, and names what is selected with Clear', () => {
  assert.equal(words(promptFor(none).parts), 'Select an object to see its properties');
  const selected = promptFor({ ...none, selected: { id: 'DP-LD-101/device', name: 'FT-101', type: 'insert', block: 'TRANSMITTER' } });
  assert.equal(selected.command, null);
  assert.equal(words(selected.parts), 'FT-101 selected [ <Clear> ]');
  assert.deepEqual(
    selected.parts.filter((p) => 'act' in p).map((p) => ('act' in p ? p.act : '')),
    ['clear'],
  );
});

test('a tool still to come names its command and says it is coming, until OK', () => {
  const line = PANELS[0]?.tools[0];
  assert.ok(line);
  const p = promptFor({ ...none, soon: line, selected: { type: 'line' } });
  assert.equal(p.command, 'LINE');
  assert.equal(words(p.parts), `(L) ${SOON} [ <OK> ]`);
});

test('stepping back offers Older, Newer and Live, and says when the entry cannot be shown', () => {
  assert.equal(words(promptFor({ ...none, at: 6 }).parts), 'Showing entry 6 as it was, read-only [ <Older> <Newer> <Live> ]');
  assert.equal(words(promptFor({ ...none, at: 1, cannot: true }).parts), 'Entry 1 cannot be shown [ <Older> <Newer> <Live> ]');
  // Stepping back comes before a selection, and reaching the server before it too.
  assert.match(words(promptFor({ ...none, at: 2, selected: { type: 'line' } }).parts), /^Showing entry 2/);
  assert.match(words(promptFor({ ...none, reaching: true, selected: { type: 'line' } }).parts), /^Reaching the server/);
});

test('the connection says live, reaching, looking back, or no journal', () => {
  assert.deepEqual(liveLine(null, false, 'open'), { state: 'open', line: 'Live: changes show as they land' });
  assert.equal(liveLine(null, false, 'reconnecting').state, 'down');
  assert.match(liveLine(null, false, 'refused').line, /refused the live stream/);
  assert.deepEqual(liveLine(3, false, 'open'), { state: 'past', line: 'Showing entry 3 as it was: read-only' });
  assert.equal(liveLine(null, true, 'open').state, 'none');
});

test('F3, F7 and F8 turn Object snap, Grid and Ortho, as in AutoCAD', () => {
  assert.equal(toggleFor('F3'), 'osnap');
  assert.equal(toggleFor('F7'), 'grid');
  assert.equal(toggleFor('F8'), 'ortho');
  assert.equal(toggleFor('F9'), null);
  assert.equal(toggledWords('grid', false), 'Grid off');
  assert.equal(toggledWords('ortho', true), 'Ortho on: it acts once drawing tools come');
});

test('units, the paper and the cursor in words', () => {
  assert.deepEqual(unitWords(25.4), { name: 'inches', short: 'in' });
  assert.deepEqual(unitWords(1), { name: 'millimetres', short: 'mm' });
  assert.deepEqual(unitWords(304.8), { name: 'feet', short: 'ft' });
  assert.deepEqual(unitWords(3.7), { name: 'drawing units', short: '' });
  assert.equal(paperWords('ANSI B landscape', 25.4), 'ANSI B landscape, inches');
  assert.equal(paperWords(null, 1), 'Extents, millimetres');
  assert.equal(coordinates(1.72, 6.5, 25.4), '1.720, 6.500 in');
  assert.equal(coordinates(-3, 0, 3.7), '-3.000, 0.000');
});

test('an object is called by its name, its ID without the sheet, or its type', () => {
  assert.equal(nameOf({ id: 'DP-LD-101/device', name: 'FT-101', type: 'insert' }), 'FT-101');
  assert.equal(nameOf({ id: 'DP-LD-101/border', type: 'polyline' }), 'border');
  assert.equal(nameOf({ type: 'line' }), 'line');
  assert.equal(typeWords({ type: 'insert', block: 'TRANSMITTER' }), 'Insert of TRANSMITTER');
  assert.equal(typeWords({ type: 'circle' }), 'Circle');
});

test('quick properties sit to the right of the object, or to its left, kept in the room', () => {
  const room = [0, 0, 1000, 700] as const;
  const card = { width: 268, height: 300 };
  // Room to the right: 16 px past its right edge, level with its top.
  assert.deepEqual(placeBeside([100, 200, 160, 260], card, room), [176, 200]);
  // None to the right: to its left.
  assert.deepEqual(placeBeside([800, 200, 900, 260], card, room), [800 - 16 - 268, 200]);
  // Low on the screen: lifted so it stays in the room.
  assert.deepEqual(placeBeside([100, 600, 160, 650], card, room), [176, 700 - 8 - 300]);
  // Wider than the room either side: kept inside it.
  const [x] = placeBeside([10, 10, 990, 50], card, room);
  assert.ok(x >= 8 && x + 268 <= 992, `kept inside: ${x}`);
});
