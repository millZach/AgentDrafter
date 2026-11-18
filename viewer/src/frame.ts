// What the drafting app's frame says, worked out apart from React so it is
// tested on its own: the drawing tools the ribbon shows before they work,
// what the command bar prompts, the drawing's units in words, the cursor's
// coordinates, the status bar's toggles, and where quick properties sit
// beside the selected object. None of it is drawing logic: what is drawn,
// and what a tap picks, is the renderer's (ADR-0007).

/** A drawing tool, as the ribbon shows it. */
export interface Tool {
  key: ToolKey;
  /** Its name on the ribbon. */
  label: string;
  /** AutoCAD's command, as the command bar names it. */
  command: string;
  /** AutoCAD's alias, which the command bar will take. */
  alias: string;
  /** What it does, in plain words. */
  tip: string;
}

export type ToolKey = 'line' | 'circle' | 'rectangle' | 'text' | 'move' | 'erase';

/** The ribbon's panels and their tools, in order. Every one is still to come. */
export const PANELS: { title: string; tools: Tool[] }[] = [
  {
    title: 'Draw',
    tools: [
      { key: 'line', label: 'Line', command: 'LINE', alias: 'L', tip: 'Draws straight lines. Click a start point, then keep clicking.' },
      { key: 'circle', label: 'Circle', command: 'CIRCLE', alias: 'C', tip: 'Click the centre, then click how big it is, or type a radius.' },
      { key: 'rectangle', label: 'Rectangle', command: 'RECTANG', alias: 'REC', tip: 'Click one corner, then the opposite corner.' },
      { key: 'text', label: 'Text', command: 'TEXT', alias: 'T', tip: 'Click where the text starts, type it, and press Enter.' },
    ],
  },
  {
    title: 'Modify',
    tools: [
      { key: 'move', label: 'Move', command: 'MOVE', alias: 'M', tip: 'Pick an object, a point to hold it by, then where that point goes.' },
      { key: 'erase', label: 'Erase', command: 'ERASE', alias: 'E', tip: 'Click an object to remove it. Undo brings it back.' },
    ],
  },
];

/** What a tool still to come says when it is tapped. */
export const SOON = 'Coming soon. For now an agent draws, through change sets.';

/** The status bar's toggles: Grid draws; Object snap and Ortho hold their state until drawing tools come. */
export type Toggle = 'grid' | 'osnap' | 'ortho';

export const TOGGLES: { key: Toggle; label: string; fkey: string; title: string }[] = [
  { key: 'grid', label: 'Grid', fkey: 'F7', title: 'The grid behind the drawing' },
  { key: 'osnap', label: 'Object snap', fkey: 'F3', title: 'When drawing tools come, the cursor catches ends, middles and centres' },
  { key: 'ortho', label: 'Ortho', fkey: 'F8', title: 'When drawing tools come, lines stay level or upright' },
];

/** The toggle a function key turns, as AutoCAD's do: F3, F7 and F8. */
export function toggleFor(key: string): Toggle | null {
  return TOGGLES.find((t) => t.fkey === key)?.key ?? null;
}

/** What turning a toggle says in the command bar's history. */
export function toggledWords(toggle: Toggle, on: boolean): string {
  const t = TOGGLES.find((x) => x.key === toggle) as (typeof TOGGLES)[number];
  return `${t.label} ${on ? 'on' : 'off'}${toggle === 'grid' ? '' : ': it acts once drawing tools come'}`;
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
 * A drawing unit in words, as the core names it (`in`, `mm`, ...). A
 * drawing that states none, such as a DWG whose INSUNITS is 0, is unitless:
 * its scale is not a unit, so none is shown.
 */
export function unitWords(units: string | null | undefined): { name: string; short: string } {
  return (units && UNITS[units]) || { name: 'unitless', short: '' };
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
  if (s.paper_declared && s.paper_name) return `${s.paper_name}, ${name}`;
  if (s.paper_declared) {
    const [left, bottom, right, top] = s.paper;
    return `${size(right - left)} × ${size(top - bottom)}${short ? ` ${short}` : ''}`;
  }
  return `Extents, ${name}`;
}

/** The cursor's place on the drawing, as the status bar shows it. */
export function coordinates(x: number, y: number, units: string | null | undefined): string {
  const { short } = unitWords(units);
  return `${x.toFixed(3)}, ${y.toFixed(3)}${short ? ` ${short}` : ''}`;
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

/** A part of the command bar's line: words, or an option to click. */
export type Part = { text: string } | { option: string; act: Act };

export type Act = 'clear' | 'older' | 'newer' | 'live' | 'ok';

/** What the command bar prompts: a command's name, when one is named, and its line. */
export interface Prompt {
  command: string | null;
  parts: Part[];
}

export interface PromptState {
  /** A tool still to come that was just tapped. */
  soon: Tool | null;
  /** The entry shown as it was, or null for the set as it is. */
  at: number | null;
  /** Whether that entry cannot be shown. */
  cannot: boolean;
  /** Whether the live stream is open, or there is no journal to watch. */
  reaching: boolean;
  /** The selected object, if any. */
  selected: Named | null;
}

const options = (...acts: [string, Act][]): Part[] => [
  { text: '[ ' },
  ...acts.flatMap(([option, act], i) => [...(i ? [{ text: ' ' }] : []), { option, act }]),
  { text: ' ]' },
];

/** What the command bar says now: prompts only, until drawing tools come. */
export function promptFor(s: PromptState): Prompt {
  if (s.soon) {
    // What it will do, in plain words, as its tip says on hover, for a tap
    // or a key that has no hover.
    return {
      command: s.soon.command,
      parts: [{ text: `(${s.soon.alias}) ${s.soon.tip} ${SOON} ` }, ...options(['OK', 'ok'])],
    };
  }
  const stepping = options(['Older', 'older'], ['Newer', 'newer'], ['Live', 'live']);
  if (s.at !== null && s.cannot) return { command: null, parts: [{ text: `Entry ${s.at} cannot be shown ` }, ...stepping] };
  if (s.at !== null) return { command: null, parts: [{ text: `Showing entry ${s.at} as it was, read-only ` }, ...stepping] };
  if (s.reaching) return { command: null, parts: [{ text: 'Reaching the server… changes show when it is back' }] };
  if (s.selected) return { command: null, parts: [{ text: `${nameOf(s.selected)} selected ` }, ...options(['Clear', 'clear'])] };
  return { command: null, parts: [{ text: 'Select an object to see its properties' }] };
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
