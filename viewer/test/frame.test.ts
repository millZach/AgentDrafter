import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  CORE_ORDER,
  coordinates,
  DEFAULT_RUNNING,
  freeBeside,
  readSnapSettings,
  runningMask,
  SNAPS,
  SNAPS_KEY,
  snappedPoint,
  snapTag,
  snapWords,
  takenOf,
  writeSnapSettings,
  type Hovered,
  newlyHidden,
  liveLine,
  nameOf,
  PANELS,
  paperWords,
  placeBeside,
  promptFor,
  colorIndex,
  currentLayerOf,
  layersWhileAsked,
  LAYER_KEY,
  originWords,
  readCurrentLayer,
  templateWords,
  toolOf,
  writeCurrentLayer,
  type ToolState,
  toggledWords,
  toggleFor,
  typeWords,
  unitWords,
  type Part,
  type PromptState,
} from '../src/frame.ts';

const words = (parts: Part[]) =>
  parts
    .map((p) => ('text' in p ? p.text : 'snap' in p ? p.snap : 'layer' in p ? `{${p.layer}} ` : 'error' in p ? `!${p.error}` : `<${p.option}>`))
    .join('');
const none: PromptState = { tool: null, at: null, cannot: false, reaching: false, selected: null };

/** A tool's state as the renderer gives it, with nothing taken. */
const toolState = (over: Partial<ToolState>): ToolState => ({
  tool: null,
  command: null,
  layer: null,
  prompt: '',
  options: [],
  error: null,
  warning: null,
  placeholder: '',
  typing_text: false,
  takes_points: false,
  selecting: false,
  picked: [],
  points: [],
  text: null,
  ...over,
});

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

test('the command bar prompts for a command or a selection, and offers Move, Erase and Clear on what is selected', () => {
  assert.equal(words(promptFor(none).parts), 'Type a command, such as L for Line, or select an object, or click a snap to read its point');
  assert.equal(words(promptFor({ ...none, touch: true }).parts), 'Pick a tool in Tools, or tap an object to see it; touch and hold to read a point');
  const selected = promptFor({ ...none, selected: { id: 'DP-LD-101/device', name: 'FT-101', type: 'insert', block: 'TRANSMITTER' } });
  assert.equal(selected.command, null);
  assert.equal(words(selected.parts), 'FT-101 selected [ <Move> <Erase> <Clear> ]');
  assert.deepEqual(
    selected.parts.filter((p) => 'act' in p).map((p) => ('act' in p ? p.act : '')),
    ['move', 'erase', 'clear'],
  );
});

test('a running tool prompts in the renderer’s words, on its layer, with its options to click and what went wrong', () => {
  const line = toolState({
    tool: 'line',
    command: 'LINE',
    layer: 'wire',
    prompt: 'Click the next point, or type @dx,dy or a length',
    options: [
      { label: 'Undo', act: 'undo', main: false },
      { label: 'Done', act: 'done', main: true },
    ],
    takes_points: true,
  });
  const p = promptFor({ ...none, tool: line, selected: { type: 'line' } });
  assert.equal(p.command, 'LINE');
  assert.equal(words(p.parts), '{on wire} Click the next point, or type @dx,dy or a length [ <Undo> · <Done> ]');
  assert.deepEqual(
    p.parts.filter((x) => 'act' in x).map((x) => ('act' in x ? [x.act, x.main] : [])),
    [
      ['tool:undo', false],
      ['tool:done', true],
    ],
  );
  const wrong = promptFor({ ...none, tool: { ...line, error: '“x” is not a point.' } });
  assert.match(words(wrong.parts), /!“x” is not a point\.$/);
  // Move draws nothing new, so names no layer.
  assert.doesNotMatch(words(promptFor({ ...none, tool: { ...line, tool: 'move', command: 'MOVE', layer: null } }).parts), /\{on/);
  // A word typed that is not a tool says so, with OK, while no tool runs.
  const said = promptFor({ ...none, tool: toolState({ error: '“hello” is not a drawing tool' }) });
  assert.equal(words(said.parts), '!“hello” is not a drawing tool [ <OK> ]');
  // The set as it was shows no tool.
  assert.match(words(promptFor({ ...none, at: 3, tool: line }).parts), /^Showing entry 3/);
});

test('the template dialog names its template and asks to change all its sheets', () => {
  const edit = {
    title: 'This is part of the template.',
    detail: 'Moving it changes 7 sheets.',
    path: 'templates/loop-2-wire.yaml',
    sheets: ['DP-LD-101', 'DP-LD-102', 'DP-LD-104', 'DP-LD-201', 'DP-LD-202', 'DP-LD-301', 'DP-LD-302'],
  };
  assert.deepEqual(templateWords(edit), {
    title: 'This is part of the template.',
    body: 'Moving it changes 7 sheets. The template loop-2-wire.yaml changes, and so does every sheet built from it:',
    button: 'Change all 7 sheets',
  });
  assert.equal(templateWords({ ...edit, sheets: ['DP-LD-103'] }).button, 'Change the template');
});

test('quick properties say where an object comes from', () => {
  const template = { path: 'templates/loop-2-wire.yaml', sheets: ['DP-LD-101', 'DP-LD-102'] };
  assert.equal(originWords('template', template), 'The template (2 sheets)');
  assert.equal(originWords('template', { ...template, sheets: ['DP-LD-103'] }), 'The template (this sheet only)');
  assert.equal(originWords('row', template), 'This sheet’s row');
  assert.equal(originWords(undefined, null), null);
});

test('the current layer is kept for each set, and is layer 0 where the sheet has not got it', () => {
  const kept = new Map<string, string>();
  const keeps = { getItem: (k: string) => kept.get(k) ?? null, setItem: (k: string, v: string) => void kept.set(k, v) };
  assert.equal(readCurrentLayer(keeps, 'examples/demo-plant'), null);
  writeCurrentLayer(keeps, 'examples/demo-plant', 'wire');
  writeCurrentLayer(keeps, 'examples/demo-set', 'notes');
  assert.equal(readCurrentLayer(keeps, 'examples/demo-plant'), 'wire');
  assert.equal(readCurrentLayer(keeps, 'examples/demo-set'), 'notes');
  assert.ok(kept.has(LAYER_KEY));
  const layers = [
    { name: '0', color: 7 },
    { name: 'wire', color: 7 },
  ];
  assert.equal(currentLayerOf('wire', layers), 'wire');
  assert.equal(currentLayerOf('notes', layers), '0');
  assert.equal(currentLayerOf(null, layers), '0');
  // While the layers are still coming, the one kept: a circle drawn then goes on it, not on 0.
  assert.equal(currentLayerOf('wire', null), 'wire');
  assert.equal(currentLayerOf(null, null), '0');
  // Reopening the box asks again for the same sheet's layers: they stay
  // meanwhile, so `wire`, which sheet B has not got, does not come back.
  const sheetB = [{ name: '0', color: 7 }];
  assert.equal(currentLayerOf('wire', layersWhileAsked('set|B', 'set|B', sheetB)), '0');
  // Another sheet's are still coming: the layer kept, until they come.
  assert.equal(layersWhileAsked('set|A', 'set|B', sheetB), null);
  kept.set(LAYER_KEY, 'not json');
  assert.equal(readCurrentLayer(keeps, 'examples/demo-plant'), null);
  assert.equal(colorIndex(1), 1);
  assert.equal(colorIndex('#ff0000'), 7);
  assert.equal(colorIndex(undefined), 7);
});

test('every tool is found by its name', () => {
  assert.equal(toolOf('rectangle')?.alias, 'REC');
  assert.equal(toolOf('nothing'), undefined);
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
  assert.equal(toggledWords('osnap', true), 'Object snap on');
  assert.equal(toggledWords('ortho', true), 'Ortho on');
});

test('units, the paper and the cursor in words', () => {
  assert.deepEqual(unitWords('in'), { name: 'inches', short: 'in' });
  assert.deepEqual(unitWords('mm'), { name: 'millimetres', short: 'mm' });
  assert.deepEqual(unitWords('ft'), { name: 'feet', short: 'ft' });
  // No unit from the core, whether stated none (INSUNITS 0) or one it does not carry (miles):
  // no word, no suffix, and none guessed from the scale.
  assert.deepEqual(unitWords(null), { name: '', short: '' });
  const sheet = { paper_declared: true, paper_name: 'ANSI B landscape', paper: [0, 0, 17, 11] as const, units: 'in' };
  assert.equal(paperWords(sheet), 'ANSI B landscape, inches');
  // Paper declared by its size alone is not the extents: its size says it.
  assert.equal(paperWords({ ...sheet, paper_name: null }), '17 × 11 in');
  assert.equal(paperWords({ ...sheet, paper_name: null, paper: [0, 0, 420, 297], units: 'mm' }), '420 × 297 mm');
  assert.equal(paperWords({ ...sheet, paper_name: null, paper: [0, 0, 8.5, 11], units: null }), '8.5 × 11');
  // No paper: the sheet sits on its extents.
  assert.equal(paperWords({ ...sheet, paper_declared: false, paper_name: null, paper: [-5, -5, 105, 55] }), 'Extents, inches');
  assert.equal(paperWords({ paper_declared: false, paper_name: null, paper: [0, 0, 1, 1], units: null }), 'Extents');
  assert.equal(paperWords({ ...sheet, units: null }), 'ANSI B landscape');
  assert.equal(coordinates(1.72, 6.5, 'in'), '1.7200, 6.5000 in');
  assert.equal(coordinates(-3, 0, null), '-3.0000, 0.0000');
});

test('an object is called by its name, its ID without the sheet, or its type', () => {
  assert.equal(nameOf({ id: 'DP-LD-101/device', name: 'FT-101', type: 'insert' }), 'FT-101');
  assert.equal(nameOf({ id: 'DP-LD-101/border', type: 'polyline' }), 'border');
  assert.equal(nameOf({ type: 'line' }), 'line');
  assert.equal(typeWords({ type: 'insert', block: 'TRANSMITTER' }), 'Insert of TRANSMITTER');
  assert.equal(typeWords({ type: 'circle' }), 'Circle');
});

test('on a phone the drawing left free is above quick properties, or left of them on a phone held on its side', () => {
  // Upright: the card spans the drawing, from 400 down.
  assert.deepEqual(freeBeside([8, 400, 382, 680], 390, 700), [0, 0, 390, 400]);
  // On its side: the card at the right, from the top down to 200.
  assert.deepEqual(freeBeside([568, 8, 836, 200], 844, 300), [0, 0, 568, 200]);
});

test('a change in the room quick properties leave moves the view only when it newly hides the object', () => {
  const peek = [0, 0, 390, 323] as const;
  const raised = [0, 0, 390, 62] as const;
  // On the screen, clear of the card before, under it now: newly hidden.
  assert.equal(newlyHidden([150, 270, 190, 290], peek, raised, 390, 700), true);
  // Panned off the screen by the drafter: left there.
  assert.equal(newlyHidden([-300, 270, -260, 290], peek, raised, 390, 700), false);
  // Already under the card before: not newly hidden.
  assert.equal(newlyHidden([150, 400, 190, 420], peek, raised, 390, 700), false);
  // The room grows, as when a wrapped prompt goes: nothing is hidden.
  assert.equal(newlyHidden([150, 270, 190, 290], raised, peek, 390, 700), false);
  assert.equal(newlyHidden([150, 20, 190, 40], raised, peek, 390, 700), false);
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

const plant = [
  { id: 'DP-LD-101/device', name: 'FT-101', type: 'insert', block: 'TRANSMITTER' },
  { id: 'DP-LD-101/field-wire-1', type: 'line' },
  { id: 'DP-LD-101/objects[7]', type: 'circle' },
];
const endpoint: Hovered = { snap: 'endpoint', object: 0, which: 'objects[3].from', with: null, at: [2.3, 6.5, 0], place: 0, count: 1 };

test("the running snaps are the renderer's bits, by the core's order, none while Object snap is off", () => {
  assert.equal(CORE_ORDER[0], 'endpoint');
  assert.equal(CORE_ORDER[10], 'nearest');
  // Endpoint, Midpoint, Centre and Intersection: bits 0, 1, 2 and 6.
  assert.equal(runningMask(true, DEFAULT_RUNNING), 0b100_0111);
  assert.equal(runningMask(false, DEFAULT_RUNNING), 0);
  // Perpendicular and Tangent act only inside a tool: no bit yet.
  assert.equal(runningMask(true, ['perpendicular', 'tangent']), 0);
  assert.equal(runningMask(true, ['nearest']), 1 << 10);
  // The menu lists ten, the last two inside a drawing tool.
  assert.deepEqual(SNAPS.filter((x) => x.tool).map((x) => x.k), ['perpendicular', 'tangent']);
});

test('Object snap and the running snaps are remembered, the defaults when nothing is kept or it cannot be read', () => {
  const kept = new Map<string, string>();
  const store = { getItem: (k: string) => kept.get(k) ?? null, setItem: (k: string, v: string) => void kept.set(k, v) };
  assert.deepEqual(readSnapSettings(store), { on: true, running: ['endpoint', 'midpoint', 'center', 'intersection'] });
  writeSnapSettings(store, { on: false, running: ['quadrant', 'node'] });
  assert.deepEqual(JSON.parse(kept.get(SNAPS_KEY) as string), { on: false, running: ['quadrant', 'node'] });
  assert.deepEqual(readSnapSettings(store), { on: false, running: ['quadrant', 'node'] });
  // A snap the page does not know is dropped; text that is not JSON gives the defaults.
  kept.set(SNAPS_KEY, JSON.stringify({ on: true, running: ['endpoint', 'extension'] }));
  assert.deepEqual(readSnapSettings(store).running, ['endpoint']);
  kept.set(SNAPS_KEY, 'not json');
  assert.deepEqual(readSnapSettings(store).running, [...DEFAULT_RUNNING]);
  assert.deepEqual(readSnapSettings(null).on, true);
});

test("the tag names the snap and its object, both of an intersection's, and Tab when there are more", () => {
  assert.deepEqual(snapTag(endpoint, plant), { main: 'Endpoint', sub: 'FT-101' });
  const crossing: Hovered = { snap: 'intersection', object: 1, which: null, with: 2, at: [1, 1, 0], place: 1, count: 3 };
  // An object with no name of its own and a numbered place is called by its type.
  assert.deepEqual(snapTag(crossing, plant), { main: 'Intersection', sub: 'field-wire-1 × circle  (2 of 3, Tab)' });
  assert.equal(snapWords(crossing, plant), 'Intersection of field-wire-1 and circle');
  // Under a finger, no Tab to step with.
  assert.deepEqual(snapTag(crossing, plant, false), { main: 'Intersection', sub: 'field-wire-1 × circle' });
});

test('a snap taken is read out, and copied as the snapped point a change set takes', () => {
  assert.deepEqual(snappedPoint(endpoint, plant), { snap: { id: 'DP-LD-101/device', snap: 'endpoint', which: 'objects[3].from' } });
  const crossing: Hovered = { snap: 'intersection', object: 1, which: '0', with: 2, at: [1, 1, 0], place: 0, count: 1 };
  assert.deepEqual(snappedPoint(crossing, plant), {
    snap: { id: 'DP-LD-101/field-wire-1', snap: 'intersection', with: { id: 'DP-LD-101/objects[7]' }, which: '0' },
  });
  // The nearest point is taken only from another snap; an object with no ID has no snapped point.
  assert.equal(snappedPoint({ ...endpoint, snap: 'nearest', which: null }, plant), null);
  assert.equal(snappedPoint(endpoint, [{ type: 'line' }]), null);
  const taken = takenOf(endpoint, plant, 'in');
  assert.deepEqual(taken, { label: 'Endpoint', rest: ' of FT-101: 2.3000, 6.5000 in', copy: true });
  const said = promptFor({ ...none, taken, selected: plant[0] ?? null });
  assert.equal(words(said.parts), 'Endpoint of FT-101: 2.3000, 6.5000 in [ <Copy as snapped point> <Clear> ]');
  // Without a snapped point, only Clear.
  assert.equal(words(promptFor({ ...none, taken: { ...taken, copy: false } }).parts), 'Endpoint of FT-101: 2.3000, 6.5000 in [ <Clear> ]');
});
