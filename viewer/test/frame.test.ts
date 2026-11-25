import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  barNote,
  barWarning,
  letGoOnOptionPress,
  canToolUndo,
  isUndoKey,
  CORE_ORDER,
  editsHere,
  fieldKey,
  pastedLines,
  secondTap,
  toSend,
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
  droppedOnLoad,
  layersWhileAsked,
  LAYER_KEY,
  originWords,
  readCurrentLayer,
  checkAgainOffered,
  previewButton,
  previewStep,
  previewSummary,
  onPreviewDrawing,
  previewIsCurrent,
  previewRedrawnOn,
  reportGeneration,
  enteredAs,
  layoutOn,
  previewTitle,
  previewVerdicts,
  type PreviewReport,
  toolOf,
  writeCurrentLayer,
  type ToolState,
  type ArrayPanelState,
  arrayFraming,
  arrayInputMode,
  switchingAway,
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
  placed: 0,
  ...over,
});

test('the ribbon shows Line, Circle, Rectangle and Text in Draw, Move, Copy, Array and Erase in Modify, with AutoCAD aliases', () => {
  assert.deepEqual(
    PANELS.map((p) => [p.title, p.tools.map((t) => `${t.label} ${t.command} ${t.alias}${t.also ? ` ${t.also}` : ''}`)]),
    [
      ['Draw', ['Line LINE L', 'Circle CIRCLE C', 'Rectangle RECTANG REC', 'Text TEXT T']],
      ['Modify', ['Move MOVE M', 'Copy COPY CO CP', 'Array ARRAY AR', 'Erase ERASE E']],
    ],
  );
  // Array's tip says Esc sends nothing, unlike Copy's (#66).
  assert.match(toolOf('array')?.tip ?? '', /Esc sends nothing/);
  for (const tool of PANELS.flatMap((p) => p.tools)) assert.ok(tool.tip.length > 20, `${tool.label} has a tip in plain words`);
});

test('a text edited on the drawing says so in the command bar, as TEXTEDIT, and how it lands', () => {
  const text = { id: 'DP-LD-101/service', type: 'text' };
  const laptop = promptFor({ ...none, selected: text, editing: text });
  assert.equal(laptop.command, 'TEXTEDIT');
  assert.equal(words(laptop.parts), 'service: type on the drawing. Enter or clicking away applies it, Esc puts it back');
  const phone = promptFor({ ...none, selected: text, editing: text, touch: true });
  assert.equal(words(phone.parts), 'service: type in the box. Done applies it, Cancel puts it back');
  // Not while the set as it was is shown, which is read-only.
  assert.equal(promptFor({ ...none, at: 3, selected: text, editing: text }).command, null);
});

const SEVEN = ['DP-LD-101', 'DP-LD-102', 'DP-LD-104', 'DP-LD-201', 'DP-LD-202', 'DP-LD-301', 'DP-LD-302'];

/** A preview of a template edit on the demo plant's seven loop sheets, as the core gives it. */
function previewOf(sheets: { sheet: string; ok?: boolean; code?: string; message?: string }[], findings: { sheet: string; rule: string; message: string }[] = []): PreviewReport {
  const landed = sheets.filter((s) => s.ok !== false).length;
  return {
    change: {
      landed,
      failed: sheets.length - landed,
      sheets: sheets.map((s) => ({
        sheet: s.sheet,
        ok: s.ok !== false,
        ...(s.ok === false ? { error: { code: s.code ?? 'bad_value', message: s.message ?? 'no' } } : {}),
      })),
    },
    ...(landed ? { checks: { result: { findings } } } : {}),
    objects: sheets.filter((s) => s.ok !== false).map((s) => ({ id: `${s.sheet}/service`, how: 'changed' as const })),
  };
}

test('a preview says sheet by sheet whether the edit would land, fail a check, be refused or be held back', () => {
  // Every sheet would take it, and none fails a check.
  const clean = previewOf(SEVEN.map((sheet) => ({ sheet })));
  const v = previewVerdicts(clean);
  assert.deepEqual(v.get('DP-LD-101'), { state: 'land', icon: '✓', word: 'Would land', why: '1 object changed. No check fails.' });
  assert.deepEqual(previewSummary(clean), { tone: 'ok', text: 'All 7 would take it, and no check fails.' });
  assert.deepEqual(previewButton(clean), { label: 'Change all 7 sheets', disabled: false });
  assert.equal(checkAgainOffered(clean), false);

  // Two would then fail a check: they can still change.
  const failing = previewOf(
    SEVEN.map((sheet) => ({ sheet })),
    ['DP-LD-301', 'DP-LD-302'].map((sheet) => ({ sheet, rule: 'text_on_line', message: `\`${sheet}/objects[4]\` runs through \`${sheet}/service\`` })),
  );
  const fails = previewVerdicts(failing).get('DP-LD-301');
  assert.equal(fails?.state, 'check');
  assert.equal(fails?.word, 'Would land, but fails a check');
  assert.equal(fails?.why, 'DP-LD-301/objects[4] runs through DP-LD-301/service (text_on_line)', 'the core’s backquotes go');
  // Several on one sheet: the first, and how many more, so the band stays short.
  const several = previewOf([{ sheet: 'S-1' }], [
    { sheet: 'S-1', rule: 'text_on_line', message: 'a line runs through TERM 1' },
    { sheet: 'S-1', rule: 'text_on_line', message: 'a line runs through TERM 2' },
    { sheet: 'S-1', rule: 'off_sheet', message: 'it draws off the paper' },
  ]);
  assert.equal(previewVerdicts(several).get('S-1')?.why, 'a line runs through TERM 1 (text_on_line); 2 more checks fail.');
  assert.deepEqual(previewSummary(failing), {
    tone: 'warn',
    text: 'All 7 would take it, but DP-LD-301 and DP-LD-302 would then fail a check. You can still change them; Accept warns of failed checks.',
  });
  assert.equal(previewButton(failing).disabled, false);

  // One refuses: a template edit lands on all its sheets or none, so the
  // rest are held back and nothing can land.
  const refused = previewOf(
    SEVEN.map((sheet) =>
      sheet === 'DP-LD-202'
        ? { sheet, ok: false, code: 'write_failed', message: 'the sheet’s DWG cannot be written (Access is denied); it may be open in AutoCAD. Close it and send the change set again' }
        : { sheet, ok: false, code: 'shared_edit_failed', message: 'held back: `DP-LD-202` would not take the template edit' },
    ),
  );
  assert.equal(previewVerdicts(refused).get('DP-LD-202')?.state, 'refused');
  assert.equal(previewVerdicts(refused).get('DP-LD-202')?.icon, '✕');
  assert.deepEqual(previewVerdicts(refused).get('DP-LD-101'), {
    state: 'held',
    icon: '–',
    word: 'Held back',
    why: 'held back: DP-LD-202 would not take the template edit',
  });
  assert.deepEqual(previewSummary(refused), { tone: 'bad', text: 'Nothing would change. DP-LD-202 would refuse it, so all 7 sheets are held back.' });
  assert.deepEqual(previewButton(refused), { label: 'Nothing can land', disabled: true });
  // A DWG that cannot be written may be written a moment later: check again.
  assert.equal(checkAgainOffered(refused), true);
  // A refusal of the records' own cannot pass on its own: nothing to check again.
  const named = previewOf([
    { sheet: 'S-1', ok: false, code: 'shared_edit_failed' },
    { sheet: 'S-2', ok: false, code: 'duplicate_name' },
  ]);
  assert.equal(checkAgainOffered(named), false);
  assert.equal(previewSummary(named).text, 'Nothing would change. S-2 would refuse it, so all 2 sheets are held back.');
});

test('a preview names its edit, its reach and its sheets, and steps round them', () => {
  const edit = {
    title: 'This is part of the template.',
    detail: 'Moving it changes 7 sheets.',
    path: 'templates/loop-2-wire.yaml',
    sheets: SEVEN,
    edit: 'Move service',
  };
  assert.deepEqual(previewTitle(edit), { chip: 'Preview', title: 'Move service', reach: 'on 7 sheets' });
  assert.equal(previewTitle({ ...edit, sheets: ['DP-LD-103'] }).reach, 'on its one sheet');
  assert.equal(previewStep(SEVEN, 'DP-LD-301', 1), 'DP-LD-302');
  assert.equal(previewStep(SEVEN, 'DP-LD-302', 1), 'DP-LD-101', 'round from the last to the first');
  assert.equal(previewStep(SEVEN, 'dp-ld-101', -1), 'DP-LD-302');
  // One sheet only would take it: its own words.
  const one = previewOf([{ sheet: 'DP-LD-103' }]);
  assert.equal(previewSummary(one).text, 'The sheet would take it, and no check fails.');
  assert.deepEqual(previewButton(one), { label: 'Change the template', disabled: false });
  // A change set that lands on some and is refused on others would leave the set inconsistent.
  const partly = previewOf([{ sheet: 'A' }, { sheet: 'B', ok: false, code: 'locked', message: 'locked' }]);
  assert.equal(previewSummary(partly).text, 'A would take it and B would refuse it, which would leave the set inconsistent.');
  assert.deepEqual(previewButton(partly), { label: 'Some sheets would refuse it', disabled: true });
});

test('nothing is picked or started on a preview, nor on its drawing once it ends until the set as it is is drawn', () => {
  assert.equal(onPreviewDrawing(true, { preview: true }, { preview: {} }), true);
  // Cancelled: the band is gone, but the preview's drawing is still the one shown.
  assert.equal(onPreviewDrawing(false, { preview: true }, { preview: {} }), true);
  // The live reply has come, but the preview's drawing is still loaded.
  assert.equal(onPreviewDrawing(false, { preview: false }, { preview: {} }), true);
  assert.equal(onPreviewDrawing(false, { preview: false }, {}), false);
  assert.equal(onPreviewDrawing(false, null, null), false);
});

test('Change all waits for a preview drawn again once a change lands', () => {
  const shown = { previewing: true, reported: true, journalRead: true, reply: { preview: true, generation: '0:*:3' }, reportedAt: '0:*:3', now: '0:*:3' };
  assert.equal(previewIsCurrent(shown), true);
  // A change or an undo landed: the set moved on, and the report is old.
  assert.equal(previewIsCurrent({ ...shown, now: '0:*:4' }), false);
  // Drawn again, but the report shown is still the old one's.
  assert.equal(previewIsCurrent({ ...shown, reply: { preview: true, generation: '0:*:4' }, now: '0:*:4' }), false);
  assert.equal(previewIsCurrent({ ...shown, reply: { preview: true, generation: '0:*:4' }, reportedAt: '0:*:4', now: '0:*:4' }), true);
  // The journal cut back: a new epoch.
  assert.equal(previewIsCurrent({ ...shown, now: '1:*:3' }), false);
  assert.equal(previewIsCurrent({ ...shown, reported: false }), false);
  assert.equal(previewIsCurrent({ ...shown, previewing: false }), false);
});

test('a preview opened before the journal is read waits for it, and is drawn again on its first snapshot', () => {
  const shown = { previewing: true, reported: true, journalRead: false, reply: { preview: true, generation: '0:*:0' }, reportedAt: '0:*:0', now: '0:*:0' };
  assert.equal(previewIsCurrent(shown), false, 'not current before the first snapshot');
  assert.equal(previewRedrawnOn(true, true), true, 'the first snapshot draws an open preview again');
  assert.equal(previewRedrawnOn(true, false), false, 'a later one draws again only what its entries touched');
  assert.equal(previewRedrawnOn(false, true), false);
  // Drawn again on it: the version moved on, and the report follows the new reply.
  assert.equal(previewIsCurrent({ ...shown, journalRead: true, now: '0:*:1' }), false);
  assert.equal(previewIsCurrent({ ...shown, journalRead: true, reply: { preview: true, generation: '0:*:1' }, reportedAt: '0:*:1', now: '0:*:1' }), true);
});

test("Enter is refused on a preview's drawing, an empty one too, which would repeat the last tool", () => {
  assert.equal(enteredAs('', false, true), 'refused', 'Esc, then Enter while the set as it is loads');
  assert.equal(enteredAs('M', false, true), 'refused');
  assert.equal(enteredAs('', true, true), 'refused');
  assert.equal(enteredAs('', false, false), 'input', 'on the set as it is, an empty Enter repeats the last tool');
  assert.equal(enteredAs(' M ', false, false), 'start');
  assert.equal(enteredAs('0,0', true, false), 'input');
});

test('journal navigation to another sheet opens it on its own first layout', () => {
  assert.equal(layoutOn('DP-LD-101', 'DP-LD-104', 'Wiring'), null);
  assert.equal(layoutOn('DP-LD-101', 'dp-ld-101', 'Wiring'), 'Wiring');
  assert.equal(layoutOn('A', 'A', null), null);
});

test("a preview's report follows the text the renderer loaded, a context restored among it", () => {
  const reply = { preview: true, text: 'T', generation: '0:*:2' };
  assert.equal(reportGeneration(reply, 'T', '0:*:1'), '0:*:2', 'a redraw of the same text is current');
  assert.equal(reportGeneration(reply, 'other', '0:*:1'), '0:*:1', 'not before its own text is loaded');
  assert.equal(reportGeneration(reply, null, '0:*:1'), '0:*:1');
  assert.equal(reportGeneration({ ...reply, preview: false }, 'T', '0:*:1'), '0:*:1');
  assert.equal(reportGeneration(null, 'T', '0:*:1'), '0:*:1');
});

test('while a preview is shown the command bar says nothing has landed and how to step', () => {
  assert.deepEqual(promptFor({ ...none, previewing: true }), {
    command: 'PREVIEW',
    parts: [{ text: 'Nothing has landed yet. ◀ ▶, the arrow keys or the tabs show each sheet; Esc cancels' }],
  });
  assert.equal(words(promptFor({ ...none, previewing: true, touch: true }).parts), 'Nothing has landed. ◀ ▶ or the tabs show each sheet');
});

test('the command bar prompts for a command or a selection, and offers Move, Copy, Array, Erase and Clear on what is selected', () => {
  assert.equal(words(promptFor(none).parts), 'Type a command, such as L for Line, or select an object, or click a snap to read its point');
  assert.equal(words(promptFor({ ...none, touch: true }).parts), 'Pick a tool in Tools, or tap an object to see it; touch and hold to read a point');
  const selected = promptFor({ ...none, selected: { id: 'DP-LD-101/device', name: 'FT-101', type: 'insert', block: 'TRANSMITTER' } });
  assert.equal(selected.command, null);
  assert.equal(words(selected.parts), 'FT-101 selected [ <Move> <Copy> <Array> <Erase> <Clear> ]');
  assert.deepEqual(
    selected.parts.filter((p) => 'act' in p).map((p) => ('act' in p ? p.act : '')),
    ['move', 'copy-objects', 'array-objects', 'erase', 'clear'],
  );
  // A phone gives the name alone, so the options stay together.
  const tapped = promptFor({ ...none, touch: true, selected: { id: 'DP-LD-101/device', name: 'FT-101', type: 'insert', block: 'TRANSMITTER' } });
  assert.equal(words(tapped.parts), 'FT-101 [ <Move> <Copy> <Array> <Erase> <Clear> ]');
  // Kept to one line on a phone: the name gives way first, whatever the font (#66).
  assert.equal(tapped.oneLine, true);
  assert.ok(!selected.oneLine);
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

test('a tool the renderer dropped as another sheet or layout was drawn clears what was typed for it', () => {
  // Text running, `E` typed and not entered, then layout Wiring drawn: the
  // renderer drops Text, and the field's `E` must not start Erase later.
  assert.equal(droppedOnLoad('text', null), true);
  // The same sheet drawn again keeps the tool, and what is typed.
  assert.equal(droppedOnLoad('text', 'text'), false);
  // No tool was running: nothing to drop, nothing typed for one.
  assert.equal(droppedOnLoad(null, null), false);
});

test('Enter lands a field and Esc puts it back, but not while an input method composes', () => {
  assert.equal(fieldKey({ key: 'Enter' }, false), 'land');
  assert.equal(fieldKey({ key: 'Escape' }, false), 'cancel');
  assert.equal(fieldKey({ key: 'Enter', shiftKey: true }, true), null, 'Shift+Enter is a new line in an mtext');
  assert.equal(fieldKey({ key: 'Enter', shiftKey: true }, false), 'land');
  // Enter that confirms a candidate, and Esc that dismisses one, are the input method's.
  assert.equal(fieldKey({ key: 'Enter', isComposing: true }, false), null);
  assert.equal(fieldKey({ key: 'Escape', isComposing: true }, false), null);
  assert.equal(fieldKey({ key: 'Process', keyCode: 229 }, false), null);
  assert.equal(fieldKey({ key: 'Enter', keyCode: 229 }, false), null);
  assert.equal(fieldKey({ key: 'a' }, false), null);
});

test('a field landed as it showed when focused, or as its value, sends nothing', () => {
  // Untouched, or put back to what it showed.
  assert.equal(toSend('0.1235', '0.1235', '0.1235'), null);
  // Focused again before the sheet drawn again shows what was sent: as it showed then, nothing.
  assert.equal(toSend('101A+', '101A+', '101+'), null);
  // Typed back to its value.
  assert.equal(toSend('101+', '101A+', '101+'), null);
  assert.equal(toSend('0.4', '0.3000', '0.3000'), '0.4');
});

test('line breaks pasted into a one-line field are kept for the renderer to refuse, not stripped', () => {
  assert.equal(pastedLines('FT-101', 6, 6, ' A\nB', false), 'FT-101 A\nB');
  assert.equal(pastedLines('FT-101', 0, 2, 'X\r\nY', false), 'X\r\nY-101');
  // One line pasted, or into a field of more lines: the browser takes it.
  assert.equal(pastedLines('FT-101', 6, 6, ' A', false), null);
  assert.equal(pastedLines('NOTES', 5, 5, '\n2.', true), null);
});

test('the second tap of a double one opens only the object the first was on, found by its ID', () => {
  const service = { place: 4, id: 'DP-LD-101/service' };
  const signal = { place: 5, id: 'DP-LD-101/signal' };
  // The same text twice: it opens, on a phone and on a laptop.
  assert.equal(secondTap(service, service, true), 'open');
  assert.equal(secondTap(service, service, false), 'open');
  // A drawing loaded between them put the second object at the first's
  // old place: two objects, by their IDs, though the places match.
  assert.equal(secondTap(service, { place: 4, id: 'DP-LD-101/signal' }, true), 'pick');
  // The same object moved to another place is still the same.
  assert.equal(secondTap(service, { place: 9, id: 'DP-LD-101/service' }, true), 'open');
  // Two objects: a phone's second tap selects; a laptop's double click zooms, as before.
  assert.equal(secondTap(service, signal, true), 'pick');
  assert.equal(secondTap(service, signal, false), 'zoom');
  // Nothing remembered, or a tap on nothing.
  assert.equal(secondTap(null, signal, true), 'pick');
  assert.equal(secondTap(null, signal, false), 'zoom');
  assert.equal(secondTap(service, { place: -1, id: null }, true), 'zoom');
  // Objects with no ID, by their place in one drawing.
  assert.equal(secondTap({ place: 2, id: null }, { place: 2, id: null }, false), 'open');
  assert.equal(secondTap({ place: 2, id: null }, { place: 3, id: null }, false), 'zoom');
});

test('quick properties edit only while the drawing loaded is the view asked for and the set as it is', () => {
  const loaded = { at: null, sheet: 'DP-LD-101', layout: null, layouts: [] };
  assert.equal(editsHere({ at: null, sheet: null, layout: null }, loaded), true);
  assert.equal(editsHere({ at: null, sheet: 'dp-ld-101', layout: null }, loaded), true);
  // An entry asked for while the live drawing is still on the canvas.
  assert.equal(editsHere({ at: 3, sheet: 'DP-LD-101', layout: null }, loaded), false);
  // The entry's drawing loaded.
  assert.equal(editsHere({ at: 3, sheet: 'DP-LD-101', layout: null }, { ...loaded, at: 3 }), false);
  // Back to live asked for while the entry is still drawn.
  assert.equal(editsHere({ at: null, sheet: 'DP-LD-101', layout: null }, { ...loaded, at: 3 }), false);
  // Another sheet asked for, not drawn yet.
  assert.equal(editsHere({ at: null, sheet: 'DP-LD-102', layout: null }, loaded), false);
  // Layouts: the first is the one drawn when none is asked for.
  const laid = { at: null, sheet: 'LAY-001', layout: 'Assembly', layouts: ['Assembly', 'Wiring'] };
  assert.equal(editsHere({ at: null, sheet: 'LAY-001', layout: null }, laid), true);
  assert.equal(editsHere({ at: null, sheet: 'LAY-001', layout: 'wiring' }, laid), false);
  assert.equal(editsHere({ at: null, sheet: 'LAY-001', layout: 'wiring' }, { ...laid, layout: 'Wiring' }), true);
  assert.equal(editsHere({ at: null, sheet: null, layout: null }, null), false);
});

test('the command bar\'s note says what the keys do: with copies placed, Enter or Esc lands them and Ctrl+Z takes one back', () => {
  assert.equal(barNote(null), 'F3: snaps · F8: Ortho · Tab: the next snap here');
  assert.equal(barNote(toolState({ tool: 'line', takes_points: true })), 'Enter or right-click: done · Esc: end · F8: Ortho');
  assert.equal(
    barNote(toolState({ tool: 'line', takes_points: true, points: [[1, 1]], options: [{ label: 'Undo', act: 'undo', main: false }] })),
    'Enter or right-click: done · Ctrl+Z: one back · Esc: end · F8: Ortho',
  );
  assert.equal(barNote(toolState({ tool: 'text', typing_text: true })), 'Enter: next line · Enter again or Esc: done');
  // Copy before a copy is placed ends as any tool does.
  assert.equal(barNote(toolState({ tool: 'copy', takes_points: true, points: [[1, 1]] })), 'Enter or right-click: done · Esc: end · F8: Ortho');
  assert.equal(barNote(toolState({ tool: 'copy', takes_points: true, points: [[1, 1]], placed: 2 })), 'Enter or Esc: the copies land · Ctrl+Z: one back');
  // Array picking its objects ends as any tool does; its panel up, Esc sends nothing (#66).
  assert.equal(barNote(toolState({ tool: 'array', selecting: true })), 'Enter or right-click: done · Esc: end · F8: Ortho');
  assert.equal(barNote(toolState({ tool: 'array', array: arrayPanel({}) })), 'Enter: it lands · Esc: nothing is sent');
});

const arrayPanel = (over: Partial<ArrayPanelState>): ArrayPanelState => ({
  kind: 'rectangular',
  spacing: 'between',
  of: 'of terminal-2',
  fields: [
    { key: 'rows', label: 'Rows', hint: null, value: '3', whole: true },
    { key: 'columns', label: 'Columns', hint: null, value: '2', whole: true },
    { key: 'row_spacing', label: 'Row spacing, in', hint: '+ up, − down', value: '-0.5', whole: false },
    { key: 'column_spacing', label: 'Column spacing, in', hint: '+ right, − left', value: '1', whole: false },
  ],
  center: null,
  picking: false,
  rotate: true,
  count: '3 × 2: 5 copies',
  ready: true,
  ...over,
});

test('Array\'s panel: a whole number brings up the number pad, a spacing or an angle, which may be below 0, the full keyboard', () => {
  const panel = arrayPanel({});
  assert.deepEqual(
    panel.fields.map((f) => arrayInputMode(f)),
    ['numeric', 'numeric', 'text', 'text'],
  );
  assert.equal(arrayInputMode({ whole: false }), 'text');
});

test('Array\'s panel: the view comes to the copies when any is out of sight, and as it opens on a phone or on a small array', () => {
  const room: [number, number, number, number] = [0, 0, 1000, 700];
  // In sight and readable: left alone, opening or not.
  assert.equal(arrayFraming([200, 200, 600, 500], room, true, false), false);
  assert.equal(arrayFraming([200, 200, 600, 500], room, false, false), false);
  // A small array, as the panel opens on a laptop, is shown close; once open, left where it is.
  assert.equal(arrayFraming([200, 200, 260, 240], room, true, false), true);
  assert.equal(arrayFraming([200, 200, 260, 240], room, false, false), false);
  // On a phone, opening always comes to it: the panel covers the drawing's foot.
  assert.equal(arrayFraming([200, 200, 600, 500], room, true, true), true);
  // Out of sight on any side, or under the panel: come to it.
  assert.equal(arrayFraming([900, 200, 1100, 500], room, false, false), true);
  assert.equal(arrayFraming([200, 500, 600, 760], room, false, true), true);
  assert.equal(arrayFraming([-20, 200, 300, 500], room, false, false), true);
  assert.equal(arrayFraming([200, 4, 300, 500], room, false, false), true);
});

test('Array\'s panel goes beside its copies, to their right, or their left where there is no room', () => {
  const room: [number, number, number, number] = [0, 0, 1100, 760];
  // Right of the box, level with its top, 28 pixels clear.
  assert.deepEqual(placeBeside([300, 320, 580, 580], { width: 330, height: 220 }, room, 28), [608, 320]);
  // No room at the right: at the left.
  assert.deepEqual(placeBeside([700, 320, 900, 580], { width: 330, height: 220 }, room, 28), [342, 320]);
});

test('Ctrl+Z, or ⌘Z, is the undo key, in either case, but not with Shift or Alt', () => {
  const key = (over: Partial<{ key: string; ctrlKey: boolean; metaKey: boolean; altKey: boolean; shiftKey: boolean }>) => ({
    key: 'z',
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    ...over,
  });
  assert.ok(isUndoKey(key({ ctrlKey: true })));
  assert.ok(isUndoKey(key({ metaKey: true, key: 'Z' })));
  assert.ok(!isUndoKey(key({})), 'z alone is typed');
  assert.ok(!isUndoKey(key({ ctrlKey: true, shiftKey: true })), 'Ctrl+Shift+Z is redo elsewhere');
  assert.ok(!isUndoKey(key({ ctrlKey: true, altKey: true })));
  assert.ok(!isUndoKey(key({ ctrlKey: true, key: 'y' })));
});

test('Ctrl+Z acts only inside a running tool that offers Undo, never while typing text or outside a tool (#245)', () => {
  const undoOpt = [{ label: 'Undo', act: 'undo', main: false }];
  assert.equal(canToolUndo(null), false);
  assert.equal(canToolUndo(toolState({})), false);
  assert.equal(canToolUndo(toolState({ tool: 'line', takes_points: true })), false);
  assert.equal(canToolUndo(toolState({ tool: 'line', takes_points: true, options: undoOpt })), true);
  assert.equal(canToolUndo(toolState({ tool: 'circle', takes_points: true, options: undoOpt })), true);
  assert.equal(canToolUndo(toolState({ tool: 'rectangle', takes_points: true, options: undoOpt })), true);
  assert.equal(canToolUndo(toolState({ tool: 'move', takes_points: true, options: undoOpt })), true);
  assert.equal(canToolUndo(toolState({ tool: 'copy', takes_points: true, placed: 1, options: undoOpt })), true);
  assert.equal(canToolUndo(toolState({ tool: 'text', typing_text: true, options: undoOpt })), false);
});

test('Copy\'s template warning shows above its prompt in the renderer\'s words, but not while the band asks', () => {
  const copy = toolState({
    tool: 'copy',
    command: 'COPY',
    prompt: '2 copies placed, dashed until you finish. Click where the next goes, or type @dx,dy or a length',
    options: [
      { label: 'Undo', act: 'undo', main: false },
      { label: 'Done: copy 2', act: 'done', main: true },
    ],
    takes_points: true,
    placed: 2,
    // As the renderer gives it for the junction box, a template's, on a template seven sheets are built from.
    warning: {
      title: 'This is part of the template.',
      detail: 'This copies it on every sheet built from this template: 7 sheets. You will be asked before it lands.',
      path: 'templates/loop-2-wire.yaml',
      sheets: ['DP-LD-101', 'DP-LD-102', 'DP-LD-104', 'DP-LD-201', 'DP-LD-202', 'DP-LD-301', 'DP-LD-302'],
      edit: 'Copy junction-box, 2 copies',
    },
  });
  // The warning shows above the prompt while Copy runs, in the renderer's words.
  const shown = barWarning({ tool: copy, previewing: false });
  assert.equal(
    `${shown?.title} ${shown?.detail}`,
    'This is part of the template. This copies it on every sheet built from this template: 7 sheets. You will be asked before it lands.',
  );
  // Not while the copies are shown before they land, where the band asks instead; nor with no tool running.
  assert.equal(barWarning({ tool: copy, previewing: true }), null);
  assert.equal(barWarning({ tool: toolState({ warning: copy.warning }), previewing: false }), null);
  // Its prompt, with Undo and Done naming the copies placed.
  const p = promptFor({ ...none, tool: copy });
  assert.equal(p.command, 'COPY');
  assert.equal(
    words(p.parts),
    '2 copies placed, dashed until you finish. Click where the next goes, or type @dx,dy or a length [ <Undo> · <Done: copy 2> ]',
  );
});

test('an option pressed lets a field being edited go, and gives up the focus Tab gave it, but keeps the command bar\'s field', () => {
  // A quick properties field being edited lets go, so its edit lands as leaving it does.
  assert.equal(letGoOnOptionPress({ tag: 'INPUT', option: false, commandField: false }), true);
  assert.equal(letGoOnOptionPress({ tag: 'TEXTAREA', option: false, commandField: false }), true);
  assert.equal(letGoOnOptionPress({ tag: 'SELECT', option: false, commandField: false }), true);
  // An option reached by Tab gives the focus up, so Enter after the click enters at the prompt.
  assert.equal(letGoOnOptionPress({ tag: 'BUTTON', option: true, commandField: false }), true);
  // The command bar's field keeps it, and so does anything else, such as a ribbon button.
  assert.equal(letGoOnOptionPress({ tag: 'INPUT', option: false, commandField: true }), false);
  assert.equal(letGoOnOptionPress({ tag: 'BUTTON', option: false, commandField: false }), false);
  assert.equal(letGoOnOptionPress(null), false);
});

test('a pending array is dropped the moment another sheet or layout is asked for, not when it comes (#66)', () => {
  const shown = { sheet: 'DP-LD-101', layout: null };
  assert.equal(switchingAway('DP-LD-101', null, shown), false);
  assert.equal(switchingAway('dp-ld-101', null, shown), false);
  assert.equal(switchingAway('DP-LD-102', null, shown), true);
  // A layout asked for that is not the one drawn; none asked for is the one drawn.
  const laid = { sheet: 'LAY-001', layout: 'Assembly' };
  assert.equal(switchingAway('LAY-001', null, laid), false);
  assert.equal(switchingAway('LAY-001', 'assembly', laid), false);
  assert.equal(switchingAway('LAY-001', 'Wiring', laid), true);
  // Nothing drawn yet, or nothing asked for: no switch.
  assert.equal(switchingAway('DP-LD-102', null, null), false);
  assert.equal(switchingAway(null, null, shown), false);
});
