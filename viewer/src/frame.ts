// What the drafting app's frame says, worked out apart from React so it is
// tested on its own: the drawing tools on the ribbon, what the command bar
// prompts, the drawing's units in words, the cursor's coordinates, the
// status bar's toggles, the current layer, where an object comes from, and
// where quick properties sit beside the selected object. None of it is
// drawing logic: what is drawn, what a tap picks and what a drawing tool
// does are the renderer's (ADR-0007), and what an edit means is the
// core's; this file only puts their words in the frame.

/** A drawing tool, as the ribbon shows it. */
export interface Tool {
  key: ToolKey;
  /** Its name on the ribbon. */
  label: string;
  /** AutoCAD's command, as the command bar names it. */
  command: string;
  /** AutoCAD's alias, which the command bar takes: the renderer reads it (crates/agentdrafter-viewer/src/tools.rs). */
  alias: string;
  /** What it does, in plain words. */
  tip: string;
}

export type ToolKey = 'line' | 'circle' | 'rectangle' | 'text' | 'move' | 'erase';

/** The ribbon's panels and their tools, in order. */
export const PANELS: { title: string; tools: Tool[] }[] = [
  {
    title: 'Draw',
    tools: [
      { key: 'line', label: 'Line', command: 'LINE', alias: 'L', tip: 'Draws straight lines. Click a start point, then keep clicking. Enter or Done ends it.' },
      { key: 'circle', label: 'Circle', command: 'CIRCLE', alias: 'C', tip: 'Click the centre, then a point on the circle, or type the radius.' },
      { key: 'rectangle', label: 'Rectangle', command: 'RECTANG', alias: 'REC', tip: 'Click one corner, then the opposite corner, or type its size as @width,height.' },
      { key: 'text', label: 'Text', command: 'TEXT', alias: 'T', tip: 'Click where the text starts, type it, and press Enter.' },
    ],
  },
  {
    title: 'Modify',
    tools: [
      { key: 'move', label: 'Move', command: 'MOVE', alias: 'M', tip: 'Pick objects, a point to hold them by, then where that point goes.' },
      { key: 'erase', label: 'Erase', command: 'ERASE', alias: 'E', tip: 'Pick objects to remove. Undo brings them back.' },
    ],
  },
];

/** The tool of this name, as the page names it. */
export function toolOf(key: string): Tool | undefined {
  return PANELS.flatMap((p) => p.tools).find((t) => t.key === key);
}

/** The status bar's toggles: Grid draws, Object snap catches snaps, Ortho holds Line and Move level or upright. */
export type Toggle = 'grid' | 'osnap' | 'ortho';

export const TOGGLES: { key: Toggle; label: string; fkey: string; title: string }[] = [
  { key: 'grid', label: 'Grid', fkey: 'F7', title: 'The grid behind the drawing' },
  { key: 'osnap', label: 'Object snap', fkey: 'F3', title: 'The cursor catches ends, middles, centres and crossings' },
  { key: 'ortho', label: 'Ortho', fkey: 'F8', title: 'Ortho: lines you draw and moves stay level or upright' },
];

/** The toggle a function key turns, as AutoCAD's do: F3, F7 and F8. */
export function toggleFor(key: string): Toggle | null {
  return TOGGLES.find((t) => t.fkey === key)?.key ?? null;
}

/** What turning a toggle says in the command bar's history. */
export function toggledWords(toggle: Toggle, on: boolean): string {
  const t = TOGGLES.find((x) => x.key === toggle) as (typeof TOGGLES)[number];
  return `${t.label} ${on ? 'on' : 'off'}`;
}

/** What the connection's dot shows: the stream open, reaching the server, the set as it was, or nothing to watch. */
export type LiveState = 'open' | 'down' | 'past' | 'none';

/** The connection's state, as the status bar and a phone's peek say it. */
export function liveLine(
  at: number | null,
  noJournal: boolean,
  stream: 'open' | 'reconnecting' | 'refused',
): { state: LiveState; line: string } {
  if (at !== null) return { state: 'past', line: `Showing entry ${at} as it was: read-only` };
  if (noJournal) return { state: 'none', line: 'Not a drawing set: there is no journal to watch' };
  if (stream === 'open') return { state: 'open', line: 'Live: changes show as they land' };
  if (stream === 'refused') {
    return { state: 'down', line: 'The server refused the live stream: open the viewer from the address serve printed' };
  }
  return { state: 'down', line: 'Reaching the server…' };
}

const UNITS: Record<string, { name: string; short: string }> = {
  in: { name: 'inches', short: 'in' },
  ft: { name: 'feet', short: 'ft' },
  mm: { name: 'millimetres', short: 'mm' },
  cm: { name: 'centimetres', short: 'cm' },
  m: { name: 'metres', short: 'm' },
};

/**
 * A drawing unit in words, as the core names it (`in`, `mm`, ...). With
 * none, the page cannot tell a drawing that states no unit (a DWG whose
 * INSUNITS is 0) from one whose unit the core does not carry (miles, say),
 * so it says nothing of the unit: no word and no suffix, and never one
 * guessed from the scale.
 */
export function unitWords(units: string | null | undefined): { name: string; short: string } {
  return (units && UNITS[units]) || { name: '', short: '' };
}

/** What the status bar says of the paper. */
export interface PaperOf {
  /** Whether the sheet declares its paper, by name or by size. */
  paper_declared: boolean;
  paper_name: string | null;
  /** Left, bottom, right, top, in drawing units. */
  paper: readonly [number, number, number, number];
  units: string | null;
}

const size = (v: number) => String(Number(v.toFixed(3)));

/**
 * The status bar's paper: its name and its units; its size, when it is
 * declared by size alone, such as `17 × 11 in`; or that the sheet sits on
 * its extents.
 */
export function paperWords(s: PaperOf): string {
  const { name, short } = unitWords(s.units);
  const unit = name ? `, ${name}` : '';
  if (s.paper_declared && s.paper_name) return `${s.paper_name}${unit}`;
  if (s.paper_declared) {
    const [left, bottom, right, top] = s.paper;
    return `${size(right - left)} × ${size(top - bottom)}${short ? ` ${short}` : ''}`;
  }
  return `Extents${unit}`;
}

/** The cursor's place on the drawing, as the status bar shows it. */
export function coordinates(x: number, y: number, units: string | null | undefined): string {
  const { short } = unitWords(units);
  return `${x.toFixed(4)}, ${y.toFixed(4)}${short ? ` ${short}` : ''}`;
}

/** An object as the command bar and quick properties call it. */
export interface Named {
  id?: string;
  name?: string;
  type: string;
  block?: string;
}

/** Its name, or its ID without the sheet, or its type. */
export function nameOf(o: Named): string {
  return o.name ?? (o.id ? o.id.split('/').slice(1).join('/') || o.id : o.type);
}

/** Its type in words: an insert by its block. */
export function typeWords(o: Named): string {
  return o.block ? `Insert of ${o.block}` : o.type.charAt(0).toUpperCase() + o.type.slice(1);
}

/** A part of the command bar's line: words, an option to click, a snap's name, the layer a tool draws on, or what went wrong. */
export type Part =
  | { text: string }
  | { option: string; act: Act; main?: boolean }
  | { snap: string }
  | { layer: string }
  | { error: string };

/** What an option does: the page's own, or a running tool's (`tool:done`), which the renderer takes. */
export type Act = 'clear' | 'older' | 'newer' | 'live' | 'copy' | 'move' | 'erase' | `tool:${string}`;

/** What the command bar prompts: a command's name, when one is named, and its line. */
export interface Prompt {
  command: string | null;
  parts: Part[];
}

export interface PromptState {
  /** The drawing tool running, as the renderer says it, or what went wrong with what was typed; null with neither. */
  tool?: ToolState | null;
  /** The entry shown as it was, or null for the set as it is. */
  at: number | null;
  /** Whether that entry cannot be shown. */
  cannot: boolean;
  /** Whether the live stream is open, or there is no journal to watch. */
  reaching: boolean;
  /** The selected object, if any. */
  selected: Named | null;
  /** A snap just taken with no tool running: its point read out. */
  taken?: Taken | null;
  /** A phone's words: a finger taps and holds where a mouse clicks. */
  touch?: boolean;
  /** Whether a change set is shown before it lands, which the band above the bar holds. */
  previewing?: boolean;
  /** Whether what is shown before it lands is a proposal opened, which Esc closes and leaves waiting (#155). */
  proposal?: boolean;
  /** A text whose contents are being edited on the drawing, or in a phone's box. */
  editing?: Named | null;
}

/** A snap taken with no tool running, as the command bar reads it out. */
export interface Taken {
  /** The snap's name, such as `Endpoint`. */
  label: string;
  /** The rest of the read-out: ` of FT-101: 2.3000, 6.5000 in`. */
  rest: string;
  /** Whether it can be copied as a snapped point. */
  copy: boolean;
}

const options = (...acts: [string, Act][]): Part[] => [
  { text: '[ ' },
  ...acts.flatMap(([option, act], i) => [...(i ? [{ text: ' ' }] : []), { option, act }]),
  { text: ' ]' },
];

/** What the command bar says now: the tool running's prompt, or the frame's. */
export function promptFor(s: PromptState): Prompt {
  if (s.previewing) {
    // The band above says what would happen; the bar says how to step. A
    // proposal opened is closed, not cancelled: it keeps waiting.
    return {
      command: s.proposal ? 'PROPOSAL' : 'PREVIEW',
      parts: [
        {
          text: s.touch
            ? 'Nothing has landed. ◀ ▶ or the tabs show each sheet'
            : `Nothing has landed yet. ◀ ▶, the arrow keys or the tabs show each sheet; ${s.proposal ? 'Esc closes it, and it keeps waiting' : 'Esc cancels'}`,
        },
      ],
    };
  }
  const t = s.tool;
  if (t?.tool && s.at === null) {
    // The renderer's words, with its options to click, and what went wrong.
    const parts: Part[] = [];
    if (t.layer !== null) parts.push({ layer: `on ${t.layer}` });
    parts.push({ text: `${t.prompt} ` });
    if (t.options.length) {
      parts.push({ text: '[ ' });
      t.options.forEach((o, i) => {
        if (i) parts.push({ text: ' · ' });
        parts.push({ option: o.label, act: `tool:${o.act}`, main: o.main });
      });
      parts.push({ text: ' ]' });
    }
    if (t.error) parts.push({ error: t.error });
    return { command: t.command, parts };
  }
  if (t?.error && s.at === null) return { command: null, parts: [{ error: `${t.error} ` }, ...options(['OK', 'tool:ok'])] };
  const stepping = options(['Older', 'older'], ['Newer', 'newer'], ['Live', 'live']);
  if (s.at !== null && s.cannot) return { command: null, parts: [{ text: `Entry ${s.at} cannot be shown ` }, ...stepping] };
  if (s.at !== null) return { command: null, parts: [{ text: `Showing entry ${s.at} as it was, read-only ` }, ...stepping] };
  if (s.reaching) return { command: null, parts: [{ text: 'Reaching the server… changes show when it is back' }] };
  if (s.editing) {
    // A text's contents edited on the drawing, as AutoCAD's TEXTEDIT.
    return {
      command: 'TEXTEDIT',
      parts: [
        {
          text: s.touch
            ? `${nameOf(s.editing)}: type in the box. Done applies it, Cancel puts it back`
            : `${nameOf(s.editing)}: type on the drawing. Enter or clicking away applies it, Esc puts it back`,
        },
      ],
    };
  }
  if (s.taken) {
    const acts: [string, Act][] = s.taken.copy ? [['Copy as snapped point', 'copy'], ['Clear', 'clear']] : [['Clear', 'clear']];
    return { command: null, parts: [{ snap: s.taken.label }, { text: `${s.taken.rest} ` }, ...options(...acts)] };
  }
  if (s.selected) {
    // Move and Erase take what is selected, as AutoCAD's do.
    return { command: null, parts: [{ text: `${nameOf(s.selected)} selected ` }, ...options(['Move', 'move'], ['Erase', 'erase'], ['Clear', 'clear'])] };
  }
  return {
    command: null,
    parts: [
      {
        text: s.touch
          ? 'Pick a tool in Tools, or tap an object to see it; touch and hold to read a point'
          : 'Type a command, such as L for Line, or select an object, or click a snap to read its point',
      },
    ],
  };
}

/** A template edit the page asks about first: what the renderer says of it. */
export interface TemplateEdit {
  title: string;
  detail: string;
  /** The template, as the rows write its path. */
  path: string;
  /** Every sheet built from it, which the edit changes. */
  sheets: string[];
  /** The edit in a few words, as a preview names it: `Move junction-box`. */
  edit: string;
}

/** What the renderer says of the drawing tool running (`Viewer.tool_state`). */
export interface ToolState {
  tool: ToolKey | null;
  command: string | null;
  /** The layer a tool that draws draws on. */
  layer: string | null;
  prompt: string;
  options: { label: string; act: string; main: boolean }[];
  error: string | null;
  /** That the objects picked are the template's, and what an edit to them reaches. */
  warning: TemplateEdit | null;
  placeholder: string;
  /** Whether what is typed is the text itself, so Space is a space. */
  typing_text: boolean;
  takes_points: boolean;
  selecting: boolean;
  picked: number[];
  points: [number, number][];
  /** The text a Text tool is typing: each line where it goes, the one being typed last. */
  text: { height: number; lines: { text: string; at: [number, number] }[] } | null;
}

/** A change set a tool has ready (`Outcome.send`). */
export interface ToolSend {
  change_set: { operations: unknown[] };
  what: string;
  sheets: string[];
  template?: TemplateEdit;
}

/** Where a quick property's value is written, as its chip says it (`Viewer.props_fields`). */
export type PropTone = 'template' | 'row_value' | 'unset';

/** One row of quick properties an edit can change, as the renderer gives it (`Viewer.props_fields`). */
export interface PropField {
  /** What an edit names it: `layer`, `contents`, `radius`, `diameter` or `attribute:N`. */
  key: string;
  label: string;
  value: string;
  editable: boolean;
  /** An mtext's contents, which take more than one line. */
  multiline: boolean;
  numeric: boolean;
  /** One of an insert's attributes. */
  attribute: boolean;
  chip?: { words: string; tone: PropTone };
  /** What an edit to it changes, said before it is edited. */
  note?: string;
}

/** What an edit in quick properties came to (`Viewer.props_edit`): the change set to send, what went wrong, or neither when the value is as it was. */
export interface PropEdited {
  error?: string;
  send?: ToolSend;
}

/** What came of a commit of a quick property: sent (or asked about first), as it was, or what went wrong. */
export type Committed = { kind: 'sent' } | { kind: 'same' } | { kind: 'error'; message: string };

/** A key pressed in a field quick properties edit, or in a text edited on the drawing. */
export interface FieldKey {
  key: string;
  shiftKey?: boolean;
  /** Whether an input method is composing, as for Japanese or Chinese. */
  isComposing?: boolean;
  /** 229 while an input method has the key, in browsers that do not say isComposing. */
  keyCode?: number;
}

/**
 * What a key does in a field: Enter lands it (Shift+Enter is a new line in
 * one of more lines), Esc puts it back. While an input method composes,
 * Enter and Esc are its own, to confirm or dismiss a candidate: nothing.
 */
export function fieldKey(e: FieldKey, multiline: boolean): 'land' | 'cancel' | null {
  if (e.isComposing || e.keyCode === 229) return null;
  if (e.key === 'Escape') return 'cancel';
  if (e.key === 'Enter' && !(multiline && e.shiftKey)) return 'land';
  return null;
}

/**
 * What landing a field sends: what it holds, unless that is what it showed
 * when it was focused, or its value; then nothing. So Enter on a field not
 * typed in, a field put back, or one left again before its last edit's
 * sheet is drawn, sends nothing.
 */
export function toSend(holds: string, atFocus: string, value: string): string | null {
  return holds === atFocus || holds === value ? null : holds;
}

/**
 * A paste into a one-line field: when it holds a line break, which the
 * browser would strip before anything could refuse it, the text the field
 * would hold, for the renderer to refuse; null when the paste is the
 * browser's to take, as into a field of more lines.
 */
export function pastedLines(value: string, start: number, end: number, pasted: string, multiline: boolean): string | null {
  if (multiline || !/[\r\n]/.test(pasted)) return null;
  return value.slice(0, start) + pasted + value.slice(end);
}

/** What a tap or a click was on: the object's place in the drawing loaded, and its ID, if it has one. */
export interface Tapped {
  place: number;
  id: string | null;
}

/**
 * What the second tap or click of a double one does, after a first on
 * `first` (null for none, or a drawing loaded since): on the same object,
 * found by its ID, which a drawing loaded between them may have moved to
 * another place, it opens it, as a text to edit, and zooms if it is not
 * one; on two objects, a phone's second tap selects its own, and a
 * laptop's double click zooms, as on nothing.
 */
export function secondTap(first: Tapped | null, now: Tapped, touch: boolean): 'open' | 'pick' | 'zoom' {
  if (now.place < 0) return 'zoom';
  const same = first !== null && (first.id !== null || now.id !== null ? first.id === now.id : first.place === now.place);
  if (same) return 'open';
  return touch ? 'pick' : 'zoom';
}

/** A view of a sheet: the journal's entry it is as it was at, null for the set as it is, its sheet and its layout. */
export interface SheetView {
  at: number | null;
  sheet: string | null;
  layout: string | null;
}

/**
 * Whether quick properties and the Layer box edit: only while the view
 * asked for and the drawing loaded are the same, and both the set as it
 * is. A step back to an entry whose drawing has not come yet leaves the
 * live drawing on the canvas, and it is not edited then. An asked sheet or
 * layout of null is the first.
 */
export function editsHere(asked: SheetView, loaded: (SheetView & { layouts: string[] }) | null): boolean {
  if (!loaded || asked.at !== null || loaded.at !== null) return false;
  const same = (a: string | null, b: string | null) => (a ?? '').toUpperCase() === (b ?? '').toUpperCase();
  if (asked.sheet !== null && !same(asked.sheet, loaded.sheet)) return false;
  const first = loaded.layouts[0] ?? null;
  return asked.layout === null ? loaded.layout === null || same(loaded.layout, first) : same(asked.layout, loaded.layout);
}

/** What came of a tool's step: lines for the history, and the change sets to send, in order. */
export interface ToolOutcome {
  log: string[];
  sends?: ToolSend[];
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
const listed = (items: string[]) => (items.length <= 1 ? items.join('') : `${items.slice(0, -1).join(', ')} and ${items.at(-1)}`);

/**
 * A change set drawn before it lands (#154), as `display-list` with the
 * change set gives it under `preview`, less what the renderer alone draws.
 */
export interface PreviewReport {
  /** The dry run's reply: each sheet the change set reaches, and whether it would take its part. */
  change: {
    landed: number;
    failed: number;
    sheets: { sheet: string; ok: boolean; error?: { code: string; message: string } }[];
  };
  /** The checker's reply on the sheets it would land on. */
  checks?: { result?: { findings: { sheet: string; rule: string; message: string }[] } };
  /** Each object it would add, change or delete, by its ID. */
  objects: { id: string; how: 'added' | 'changed' | 'deleted' }[];
  /** On the sheet drawn, a box round each check it would fail, `{min, max}` in drawing units. */
  clouds?: { rule: string; bounds: { min: [number, number]; max: [number, number] } }[];
  /** Said when the sheet as it is cannot be drawn, as for one the change set repairs: its old places are not shown. */
  warning?: string;
}

/** What would become of one sheet: it would land, land but fail a check, refuse it, or be held back by another that refuses a template edit. */
export type PreviewState = 'land' | 'check' | 'refused' | 'held';

export interface SheetVerdict {
  state: PreviewState;
  /** Its mark on the band and on its tab. */
  icon: string;
  /** `Would land`, `Would land, but fails a check`, `Refused` or `Held back`. */
  word: string;
  /** Why, or what it would change. */
  why: string;
}

const VERDICTS: Record<PreviewState, [string, string]> = {
  land: ['✓', 'Would land'],
  check: ['!', 'Would land, but fails a check'],
  refused: ['✕', 'Refused'],
  held: ['–', 'Held back'],
};

/** The core's words, without the backquotes it puts round names. */
const plain = (text: string) => text.replace(/`/g, '');

/** What would become of each sheet the change set reaches, by its name in capitals, in the set's order. */
export function previewVerdicts(p: PreviewReport): Map<string, SheetVerdict> {
  const findings = p.checks?.result?.findings ?? [];
  const out = new Map<string, SheetVerdict>();
  for (const s of p.change.sheets) {
    const key = s.sheet.toUpperCase();
    const fails = findings.filter((f) => f.sheet.toUpperCase() === key);
    const state: PreviewState = !s.ok ? (s.error?.code === 'shared_edit_failed' ? 'held' : 'refused') : fails.length ? 'check' : 'land';
    let why: string;
    if (state === 'check') {
      // The first check it would fail, and how many more: the band stays short.
      const more = fails.length - 1;
      why = `${plain(fails[0]?.message ?? '')} (${fails[0]?.rule ?? ''})${more ? `; ${more === 1 ? '1 more check fails' : `${more} more checks fail`}.` : ''}`;
    }
    else if (!s.ok) why = plain(s.error?.message ?? 'It would not take its part.');
    else {
      const n = { added: 0, changed: 0, deleted: 0 };
      for (const o of p.objects) if (o.id.toUpperCase().startsWith(`${key}/`)) n[o.how] += 1;
      const said = [
        n.added ? `${plural(n.added, 'object')} added` : '',
        n.changed ? `${plural(n.changed, 'object')} changed` : '',
        n.deleted ? `${plural(n.deleted, 'object')} erased` : '',
      ].filter(Boolean);
      why = `${said.length ? said.join(', ') : 'Nothing on it changes'}. No check fails.`;
    }
    const [icon, word] = VERDICTS[state];
    out.set(key, { state, icon, word, why });
  }
  return out;
}

/** The band's summary of every sheet: its tone and its words. */
export function previewSummary(p: PreviewReport): { tone: 'ok' | 'warn' | 'bad'; text: string } {
  const v = previewVerdicts(p);
  const names = (state: PreviewState) => p.change.sheets.filter((s) => v.get(s.sheet.toUpperCase())?.state === state).map((s) => s.sheet);
  const n = p.change.sheets.length;
  const refusing = names('refused');
  const fails = names('check');
  if (refusing.length && !p.change.landed) {
    return {
      tone: 'bad',
      text: n > 1 ? `Nothing would change. ${listed(refusing)} would refuse it, so all ${n} sheets are held back.` : 'Nothing would change: the sheet would refuse it.',
    };
  }
  if (refusing.length) {
    const landing = p.change.sheets.filter((s) => s.ok).map((s) => s.sheet);
    return { tone: 'bad', text: `${listed(landing)} would take it and ${listed(refusing)} would refuse it, which would leave the set inconsistent.` };
  }
  if (fails.length) {
    const who = fails.length === n ? (n > 1 ? 'every one' : 'it') : listed(fails);
    return {
      tone: 'warn',
      text: `${n > 1 ? `All ${n} would take it` : 'The sheet would take it'}, but ${who} would then fail a check. You can still change ${n > 1 ? 'them' : 'it'}; Accept warns of failed checks.`,
    };
  }
  return { tone: 'ok', text: n > 1 ? `All ${n} would take it, and no check fails.` : 'The sheet would take it, and no check fails.' };
}

/** The band's button: Change all, or why nothing can land. */
export function previewButton(p: PreviewReport): { label: string; disabled: boolean } {
  const n = p.change.sheets.length;
  if (p.change.failed) return { label: p.change.landed ? 'Some sheets would refuse it' : 'Nothing can land', disabled: true };
  return { label: n > 1 ? `Change all ${n} sheets` : 'Change the template', disabled: false };
}

/**
 * Whether a refusal could pass a moment later, so the band offers to check
 * again: a sheet whose DWG cannot be written, as when it is open in AutoCAD.
 * Every other refusal is the records', and asking again changes nothing.
 */
export function checkAgainOffered(p: PreviewReport): boolean {
  return p.change.sheets.some((s) => !s.ok && s.error?.code === 'write_failed');
}

/** The band's heading: its chip, the edit, and its reach; for a proposal opened (#155), its number. */
export function previewTitle(edit: TemplateEdit, proposal?: number): { chip: string; title: string; reach: string } {
  const n = edit.sheets.length;
  if (proposal !== undefined) return { chip: `Proposal ${proposal}`, title: edit.edit, reach: n > 1 ? `on ${n} sheets` : `on ${edit.sheets[0] ?? 'its sheet'}` };
  return { chip: 'Preview', title: edit.edit, reach: n > 1 ? `on ${n} sheets` : 'on its one sheet' };
}

/** A proposal's band's Keep: Keep, or Nothing can land while any sheet would refuse it, since a keep lands on all of them or none. */
export function keepButton(p: PreviewReport): { label: string; disabled: boolean } {
  // A proposal is kept on every sheet it reaches or on none (#155).
  if (p.change.failed) return { label: 'Nothing can land', disabled: true };
  return { label: 'Keep', disabled: false };
}

/** The sheet `by` steps on from `sheet` among `sheets`, round from the last to the first. */
export function previewStep(sheets: readonly string[], sheet: string, by: number): string {
  const n = sheets.length;
  const i = Math.max(0, sheets.findIndex((s) => s.toUpperCase() === sheet.toUpperCase()));
  return sheets[(((i + by) % n) + n) % n] ?? sheet;
}

/**
 * Whether the drawing shown, or the one on its way, is a preview's: while
 * a preview is shown, and after it ends until the set as it is has been
 * drawn in its place. Nothing is picked, snapped or started on it.
 */
export function onPreviewDrawing(
  previewing: boolean,
  reply: { preview: boolean } | null,
  summary: { preview?: unknown } | null,
): boolean {
  return previewing || !!reply?.preview || summary?.preview != null;
}

/**
 * Whether a preview's report is of the set as it is now, so Change all may
 * send it: its drawing is the one shown, drawn at the generation the set
 * is at now. A change or an undo landing since makes it old until the
 * preview is drawn again.
 */
export function previewIsCurrent(p: {
  previewing: boolean;
  reported: boolean;
  /** Whether the journal's first snapshot has come, and the preview was drawn again on it. */
  journalRead: boolean;
  /** The drawing shown: whether it is a preview's, and the generation it was asked at. */
  reply: { preview: boolean; generation: string } | null;
  /** The generation the report was drawn at. */
  reportedAt: string;
  /** The generation the set is at now. */
  now: string;
}): boolean {
  return p.previewing && p.reported && p.journalRead && !!p.reply?.preview && p.reply.generation === p.reportedAt && p.reportedAt === p.now;
}

/**
 * Whether a journal snapshot asks for an open preview to be drawn again: the
 * first valid one, which may already hold a change set that landed before
 * it was read, and so was never an event; later ones draw again only the
 * sheets their new entries touched, which the preview's version follows.
 */
export function previewRedrawnOn(previewing: boolean, firstSnapshot: boolean): boolean {
  return previewing && firstSnapshot;
}

/**
 * The generation a preview's report is of, once `reply` comes: the reply's,
 * when the drawing the renderer loaded is its text, since a reply of the
 * same text is not drawn again; otherwise the one it had. `loadedText` is
 * the text the renderer said it loaded, never one read at mount.
 */
export function reportGeneration(reply: { preview: boolean; text: string; generation: string } | null, loadedText: string | null, had: string): string {
  return reply?.preview && loadedText !== null && reply.text === loadedText ? reply.generation : had;
}

/**
 * Whether the drawing shown is the one asked for with the preview keyed
 * `key`, loaded by the renderer: its reply was asked for with that key, and
 * its text is the one loaded. Until it is, a proposal opened after another
 * says nothing of what it would do, and its Keep stays off.
 */
export function drawnFor(key: number, reply: { pvKey: number | null; text: string } | null, loadedText: string | null): boolean {
  return !!reply && reply.pvKey === key && loadedText !== null && reply.text === loadedText;
}

/**
 * What the command bar's entry does with `text`: nothing, while the drawing
 * shown or on its way is a preview's, so not even an empty Enter repeats
 * the last tool on it; start the tool `text` names, when none runs; or go
 * to the running tool, an empty Enter with none running repeating the last.
 */
export function enteredAs(text: string, toolRunning: boolean, onPreview: boolean): 'refused' | 'start' | 'input' {
  if (onPreview) return 'refused';
  return !toolRunning && text.trim() ? 'start' : 'input';
}

/** The layout to ask for once `to` is shown in place of `from`: the same, on the same sheet; on another, its first. */
export function layoutOn(from: string, to: string, layout: string | null): string | null {
  return from.toUpperCase() === to.toUpperCase() ? layout : null;
}

/** A sheet's template and the sheets built from it, as `display-list --origins` gives them. */
export interface TemplateOf {
  path: string;
  sheets: string[];
}

/**
 * Where an object comes from, as quick properties say it: the template,
 * which an edit to it changes on every sheet built from it, or this
 * sheet's row; null when the core did not say, as for a sheet description.
 */
export function originWords(from: string | undefined, template: TemplateOf | null | undefined): string | null {
  // One short line, so the card keeps its room on a phone.
  if (from === 'row') return 'This sheet’s row';
  if (from === 'template' && template) {
    const n = template.sheets.length;
    return n > 1 ? `The template (${n} sheets)` : 'The template (this sheet only)';
  }
  return null;
}

/** A layer as `query layers` lists it. */
export interface LayerEntry {
  name: string;
  /** Its colour: an index, or a name such as a true colour. */
  color?: number | string;
  locked?: boolean;
  on?: boolean;
  frozen?: boolean;
}

/** A layer's colour index, for the swatch and the rubber band; 7 for any colour that is not an index. */
export function colorIndex(color: number | string | undefined): number {
  return typeof color === 'number' && color >= 1 && color <= 255 ? color : 7;
}

/** Where the browser keeps the current layer of each set it has drawn on. */
export const LAYER_KEY = 'agentdrafter-current-layer';

/** The current layer kept for the set at `source`, if any. */
export function readCurrentLayer(keeps: Keeps | null, source: string): string | null {
  try {
    const kept = JSON.parse(keeps?.getItem(LAYER_KEY) ?? '{}') as Record<string, unknown>;
    const layer = kept[source];
    return typeof layer === 'string' ? layer : null;
  } catch {
    return null;
  }
}

/** Keeps `layer` as the set's current layer, if the browser lets it. */
export function writeCurrentLayer(keeps: Keeps | null, source: string, layer: string): void {
  try {
    let kept: Record<string, unknown> = {};
    try {
      kept = JSON.parse(keeps?.getItem(LAYER_KEY) ?? '{}') as Record<string, unknown>;
    } catch {
      // A value that is not ours is replaced.
    }
    keeps?.setItem(LAYER_KEY, JSON.stringify({ ...kept, [source]: layer }));
  } catch {
    // A browser that keeps nothing: the layer lasts the visit.
  }
}

/**
 * Whether the renderer dropped the tool running, `was`, when a sheet was
 * drawn: another sheet or layout ends it with nothing sent. The command
 * bar's field is cleared then, as Esc clears it, so what was typed for the
 * dropped tool is not entered into what comes next.
 */
export function droppedOnLoad(was: string | null, now: string | null): boolean {
  return was !== null && now === null;
}

/**
 * The layers the Layer box holds while they are asked for again: the same
 * sheet's (`of`, as the page keys it, the same as `was`) stay, so a layer
 * the sheet has not got does not come back as current while the reply is
 * on its way; another sheet's go, null, until its own come.
 */
export function layersWhileAsked(
  was: string,
  of: string,
  held: LayerEntry[] | null,
): LayerEntry[] | null {
  return was === of ? held : null;
}

/**
 * The current layer of a sheet whose layers are `layers`: the one kept, if
 * the sheet has it, or layer 0. While the layers are still being asked for
 * (null), the one kept, so nothing drawn meanwhile goes on 0 by mistake.
 */
export function currentLayerOf(kept: string | null, layers: readonly LayerEntry[] | null): string {
  if (layers === null) return kept ?? '0';
  return kept !== null && layers.some((l) => l.name === kept) ? kept : '0';
}

/** One of AutoCAD's object snaps, as the core names it. */
export type SnapKind =
  | 'endpoint'
  | 'midpoint'
  | 'center'
  | 'quadrant'
  | 'insertion'
  | 'node'
  | 'intersection'
  | 'apparent_intersection'
  | 'perpendicular'
  | 'tangent'
  | 'nearest';

/** The core's order of the snaps (`SnapKind::ALL`), which gives each its bit for the renderer. */
export const CORE_ORDER: readonly SnapKind[] = [
  'endpoint',
  'midpoint',
  'center',
  'quadrant',
  'insertion',
  'node',
  'intersection',
  'apparent_intersection',
  'perpendicular',
  'tangent',
  'nearest',
];

/** A snap as the Object snap menu lists it: its name, a line on what it means, and its marker. */
export interface SnapEntry {
  k: SnapKind;
  label: string;
  hint: string;
  /** Acts only inside a drawing tool, from the last point: not built yet. */
  tool?: boolean;
  /** AutoCAD's AutoSnap marker, an SVG path round 0,0 of half size `s`, y down. */
  path(s: number): string;
}

const circlePath = (r: number, cx = 0, cy = 0) =>
  `M${cx + r} ${cy}A${r} ${r} 0 1 0 ${cx - r} ${cy}A${r} ${r} 0 1 0 ${cx + r} ${cy}Z`;

/** The snaps the menu lists, in its order, as the prototype Zach picked (#151) lists them. */
export const SNAPS: readonly SnapEntry[] = [
  { k: 'endpoint', label: 'Endpoint', hint: 'The ends of lines, arcs and polyline corners', path: (s) => `M${-s} ${-s}H${s}V${s}H${-s}Z` },
  { k: 'midpoint', label: 'Midpoint', hint: 'The middle of a line, an arc or a polyline side', path: (s) => `M0 ${-s}L${s} ${s * 0.8}H${-s}Z` },
  { k: 'center', label: 'Centre', hint: 'The centre of a circle or an arc', path: (s) => circlePath(s) },
  { k: 'quadrant', label: 'Quadrant', hint: 'A circle’s 0°, 90°, 180° and 270° points', path: (s) => `M0 ${-s}L${s} 0L0 ${s}L${-s} 0Z` },
  { k: 'intersection', label: 'Intersection', hint: 'Where two objects cross', path: (s) => `M${-s} ${-s}L${s} ${s}M${s} ${-s}L${-s} ${s}` },
  {
    k: 'insertion',
    label: 'Insertion',
    hint: 'Where a block or a text is placed',
    path: (s) => `M${-s} ${-s * 0.2}H${s * 0.2}V${s}H${-s}ZM${-s * 0.2} ${-s * 0.2}V${-s}H${s}V${s * 0.2}H${s * 0.2}`,
  },
  {
    k: 'node',
    label: 'Node',
    hint: 'Where a text’s baseline starts, and points',
    path: (s) => `${circlePath(s)}M${-s * 0.7} ${-s * 0.7}L${s * 0.7} ${s * 0.7}M${s * 0.7} ${-s * 0.7}L${-s * 0.7} ${s * 0.7}`,
  },
  { k: 'nearest', label: 'Nearest', hint: 'The nearest point on any object', path: (s) => `M${-s} ${-s}H${s}L${-s} ${s}H${s}Z` },
  { k: 'perpendicular', label: 'Perpendicular', hint: 'Square to an object, from the last point', tool: true, path: (s) => `M${-s} ${-s}V${s}H${s}M${-s} 0H0V${s}` },
  { k: 'tangent', label: 'Tangent', hint: 'Touching a circle, from the last point', tool: true, path: (s) => `${circlePath(s * 0.7, 0, s * 0.3)}M${-s} ${-s * 0.4}H${s}` },
];

/** The snap's name, as the tag, the menu and the read-out say it. */
export function snapLabel(k: SnapKind): string {
  return SNAPS.find((x) => x.k === k)?.label ?? 'Apparent intersection';
}

/**
 * The running snaps by default: AutoCAD's (Endpoint, Centre, Intersection
 * and Extension, OSMODE 4133), less Extension, which the core has not got,
 * and with Midpoint, which a beginner reaches for first.
 */
export const DEFAULT_RUNNING: readonly SnapKind[] = ['endpoint', 'midpoint', 'center', 'intersection'];

/**
 * The renderer's bits for the running snaps: bit `i` for the `i`th of the
 * core's order; none while Object snap is off. Perpendicular and Tangent
 * act only in a drawing tool, from its last point, so they have no bit yet.
 */
export function runningMask(on: boolean, running: Iterable<SnapKind>): number {
  if (!on) return 0;
  let mask = 0;
  for (const k of running) {
    if (k === 'perpendicular' || k === 'tangent') continue;
    const i = CORE_ORDER.indexOf(k);
    if (i >= 0) mask |= 1 << i;
  }
  return mask;
}

/** Object snap (F3) and the running snaps, as the page remembers them. */
export interface SnapSettings {
  on: boolean;
  running: SnapKind[];
}

/** Where the browser keeps them, for this page, across visits. */
export const SNAPS_KEY = 'agentdrafter-object-snaps';

/** Somewhere to keep text, as a browser's localStorage is. */
export interface Keeps {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

/** The snap settings kept, or the defaults when there are none, or they cannot be read. */
export function readSnapSettings(keeps: Keeps | null): SnapSettings {
  const defaults = { on: true, running: [...DEFAULT_RUNNING] };
  try {
    const text = keeps?.getItem(SNAPS_KEY);
    if (!text) return defaults;
    const kept = JSON.parse(text) as Partial<SnapSettings>;
    const running = Array.isArray(kept.running) ? kept.running.filter((k): k is SnapKind => SNAPS.some((x) => x.k === k)) : null;
    return { on: typeof kept.on === 'boolean' ? kept.on : true, running: running ?? defaults.running };
  } catch {
    return defaults;
  }
}

/** Keeps the snap settings, if the browser lets it. */
export function writeSnapSettings(keeps: Keeps | null, settings: SnapSettings): void {
  try {
    keeps?.setItem(SNAPS_KEY, JSON.stringify(settings));
  } catch {
    // A browser that keeps nothing, such as a private window: the settings last the visit.
  }
}

/** The snap the cursor holds, as the renderer says it (`Viewer.hovered`). */
export interface Hovered {
  snap: SnapKind;
  /** The object's place in the sheet's objects. */
  object: number;
  which: string | null;
  /** For an intersection, the other object's place. */
  with: number | null;
  at: [number, number, number];
  /** Its place among the snaps in reach, and how many there are. */
  place: number;
  count: number;
}

/** An object as the snap's tag names it: its name, its ID's own part, or its block or type. */
function objectWords(o: Named | undefined): string {
  if (!o) return '?';
  if (o.name) return o.name;
  const own = o.id ? o.id.split('/').slice(1).join('/') : '';
  if (own && !/^objects\[/.test(own)) return own;
  return o.block ?? o.type;
}

/**
 * What the tag beside the marker says: the snap, and its object, or both of
 * an intersection's; with `tab`, as with a keyboard, which of the snaps in
 * reach it is, for Tab to step through.
 */
export function snapTag(h: Hovered, objects: readonly Named[], tab = true): { main: string; sub: string } {
  const of = h.with !== null ? `${objectWords(objects[h.object])} × ${objectWords(objects[h.with])}` : objectWords(objects[h.object]);
  return { main: snapLabel(h.snap), sub: tab && h.count > 1 ? `${of}  (${h.place + 1} of ${h.count}, Tab)` : of };
}

/** The snap in words: `Endpoint of FT-101`, `Intersection of C-101 and 101+`. */
export function snapWords(h: Hovered, objects: readonly Named[]): string {
  const label = snapLabel(h.snap);
  return h.with !== null
    ? `${label} of ${objectWords(objects[h.object])} and ${objectWords(objects[h.with])}`
    : `${label} of ${objectWords(objects[h.object])}`;
}

/**
 * The snapped point a change set's operation takes for it
 * (docs/change-sets.md, Snapped points): the object by its ID, the snap,
 * `which` of its points, and an intersection's other object. None for the
 * nearest point, which a snapped point takes only from another snap, and
 * for an object with no ID, such as one of a DWG.
 */
export function snappedPoint(h: Hovered, objects: readonly Named[]): { snap: Record<string, unknown> } | null {
  const id = objects[h.object]?.id;
  if (!id || h.snap === 'nearest' || h.snap === 'perpendicular' || h.snap === 'tangent') return null;
  const snap: Record<string, unknown> = { id, snap: h.snap };
  if (h.with !== null) {
    const other = objects[h.with]?.id;
    if (!other) return null;
    snap.with = { id: other };
  }
  if (h.which) snap.which = h.which;
  return { snap };
}

/** A snap taken with no tool running, read out as AutoCAD's ID command reads a point. */
export function takenOf(h: Hovered, objects: readonly Named[], units: string | null | undefined): Taken {
  const words = snapWords(h, objects);
  const label = snapLabel(h.snap);
  return {
    label,
    rest: `${words.slice(label.length)}: ${coordinates(h.at[0], h.at[1], units)}`,
    copy: snappedPoint(h, objects) !== null,
  };
}

/** A box on the screen, in CSS pixels: left, top, right, bottom. */
export type Box = readonly [number, number, number, number];

/**
 * The part of a phone's drawing, `width` by `height`, that quick
 * properties at `card` (in the drawing's pixels) leave free: above the
 * card when it spans the drawing, as on a phone held upright; left of it,
 * down to its bottom, when it sits at the right, as on one held on its
 * side.
 */
export function freeBeside(card: Box, width: number, height: number): Box {
  const [left, top, , bottom] = card;
  if (left > width / 2) return [0, 0, left, Math.min(height, bottom)];
  return [0, 0, width, Math.max(0, top)];
}

/** Whether `box` lies wholly inside `room`. */
const inside = (box: Box, room: Box): boolean =>
  box[0] >= room[0] && box[2] <= room[2] && box[1] >= room[1] && box[3] <= room[3];

/**
 * Whether a change in the room quick properties leave, from `before` to
 * `now`, newly hides the selected object at `box`, on a drawing `width` by
 * `height`: it is on the screen, was clear of the card before, and is not
 * now. Only then is the view moved for it. An object the drafter panned
 * away, or one already under the card, is left where it is, and room that
 * grows hides nothing.
 */
export function newlyHidden(box: Box, before: Box, now: Box, width: number, height: number): boolean {
  const onScreen = box[2] > 0 && box[0] < width && box[3] > 0 && box[1] < height;
  return onScreen && inside(box, before) && !inside(box, now);
}

/**
 * Where quick properties go beside the selected object, whose box on the
 * screen is `object`: to its right, or to its left when there is no room
 * there, level with its top, kept inside `room` (the drawing's free area)
 * by `margin`. Gives the card's left and top.
 */
export function placeBeside(
  object: Box,
  card: { width: number; height: number },
  room: Box,
  gap = 16,
  margin = 8,
): [number, number] {
  const [ol, ot, or] = object;
  const [rl, rt, rr, rb] = room;
  let x = or + gap;
  if (x + card.width > rr - margin) x = ol - gap - card.width;
  x = Math.max(rl + margin, Math.min(rr - margin - card.width, x));
  const y = Math.max(rt + margin, Math.min(rb - margin - card.height, ot));
  return [x, y];
}
