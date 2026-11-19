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

/** What came of a tool's step: lines for the history, and a change set to send. */
export interface ToolOutcome {
  log: string[];
  send?: ToolSend;
}

/** The template dialog's words: its heading, what it says, and its button. */
export function templateWords(edit: TemplateEdit): { title: string; body: string; button: string } {
  const file = edit.path.split('/').pop() ?? edit.path;
  return {
    title: edit.title,
    body: `${edit.detail} The template ${file} changes, and so does every sheet built from it:`,
    button: edit.sheets.length > 1 ? `Change all ${edit.sheets.length} sheets` : 'Change the template',
  };
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

/** The current layer of a sheet whose layers are `layers`: the one kept, if the sheet has it, or layer 0. */
export function currentLayerOf(kept: string | null, layers: readonly LayerEntry[]): string {
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
