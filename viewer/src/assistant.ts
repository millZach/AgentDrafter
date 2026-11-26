// The assistant's panel (#156), as the prototype Zach picked on 2026-11-20
// shows it (layout B): the agents `agentdrafter serve` found on the PATH,
// the conversation its `/assistant/events` stream sends, and what the page
// makes of it: what the chip over the input says the drafter is looking
// at, one short line for each command the agent called, which proposal a
// `propose` made, and whether words typed in the command bar start a
// drawing tool or go to the assistant. Plain functions, tested without a
// browser. Whether a word is a tool's alias is the renderer's
// (`is_tool_word`), which the page passes in; what a command does is the
// core's, whose replies are shown as they come.

import { capital, operationsTitle, whoWords, type Entry, type JournalReport, type Operation } from './journal.ts';

/** An agent the server knows how to start, and whether it found it on the PATH when it started. */
export interface AgentInfo {
  agent: string;
  name: string;
  command: string;
  found: boolean;
}

/** How to install an agent, as the panel says it when none is found. */
export interface InstallLine {
  agent: string;
  name: string;
  line: string;
  page: string;
}

/** `GET /assistant`'s result, and `POST /assistant/find`'s. */
export interface AgentsReport {
  agents: AgentInfo[];
  install: InstallLine[];
}

/** What the drafter is looking at, as a message carries it: the sheet, its layout, and what is selected, by ID and by name. */
export interface LookingAt {
  sheet: string;
  layout: string | null;
  selected: string[];
  names: string[];
}

/** A command's reply, as `POST /commands/<name>` gives it. */
export interface CommandReply {
  ok: boolean;
  result?: unknown;
  error?: { code?: string; message?: string };
}

export interface CommandPart {
  kind: 'command';
  /** The registry's name for it; a tool that is not AgentDrafter's keeps its own. */
  command: string;
  input: unknown;
  /** Null while it runs. */
  reply: CommandReply | null;
  ok: boolean | null;
  /** Canonical path of the drawing set named in `input.set` or `reply.result.set`, resolved by `serve` (#227). */
  canonical_set?: string | null;
}

export interface WordsPart {
  kind: 'words';
  text: string;
}

export type Part = CommandPart | WordsPart;

export interface NoteItem {
  kind: 'note';
  text: string;
  at: string;
}

export interface DrafterItem {
  kind: 'drafter';
  text: string;
  via: 'chat' | 'command_bar';
  /** Null when the drafter left it off this message. */
  looking_at: LookingAt | null;
  at: string;
}

export interface AssistantItem {
  kind: 'assistant';
  agent: string;
  name: string;
  at: string;
  parts: Part[];
  done: boolean;
  error: string | null;
}

export type Item = NoteItem | DrafterItem | AssistantItem;

/** One event of `/assistant/events`: the whole conversation. */
export interface Conversation {
  set: string;
  /** Canonical path of the conversation's drawing set, resolved by `serve` (#227). */
  canonical_set?: string;
  /** The agent it runs on; null when none was found. */
  agent: string | null;
  /** A turn is running: the input waits. */
  busy: boolean;
  items: Item[];
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
const listed = (items: string[]) => (items.length <= 1 ? items.join('') : `${items.slice(0, -1).join(', ')} and ${items.at(-1)}`);

// ---------------------------------------------------------------- the chip

/** The selection in words: `1 selected: FT-101`, `nothing selected`, `3 selected: FT-101, TT-102 and 1 more`. */
function selectionWords(names: readonly string[], count: number): string {
  if (!count) return 'nothing selected';
  const shown = names.slice(0, 2);
  const more = count - shown.length;
  return `${count} selected: ${shown.join(', ')}${more > 0 ? ` and ${more} more` : ''}`;
}

/** The sheet in words, with its layout when it is drawn in several: `Sheet DP-LD-101`, `Sheet LOOPS, layout Wiring`. */
const sheetWords = (sheet: string, layout: string | null | undefined) => `Sheet ${sheet}${layout ? `, layout ${layout}` : ''}`;

/** What the chip over the input says: `Sheet DP-LD-101 · 1 selected: FT-101`, `Sheet DP-LD-101 · nothing selected`. */
export function chipWords(look: LookingAt): string {
  return `${sheetWords(look.sheet, look.layout)} · ${selectionWords(look.names, look.selected.length)}`;
}

/** What the chip says once × leaves it off the next message. */
export const CHIP_OFF = 'This message won’t say what you’re looking at.';
export const CHIP_BACK = 'Put it back';

/** What a sent message says it carried: `with Sheet DP-LD-101 · 1 selected: FT-101`, or that it did not. */
export function carriedWords(item: DrafterItem): string {
  return item.looking_at ? `with ${chipWords(item.looking_at)}` : 'without what you’re looking at';
}

/** Where a sent message came from, when it was not the chat's own input. */
export const viaWords = (item: DrafterItem): string | null => (item.via === 'command_bar' ? 'from the command bar' : null);

/**
 * What the page holds the drafter to be looking at: the sheet shown, its
 * layout, and the selected object, by its ID and name (its ID when it has
 * no name); an object with no ID is left out. Null when nothing is shown.
 */
export function lookingAtOf(sheet: string | null, layout: string | null, picked: { id?: string; name?: string } | null): LookingAt | null {
  if (!sheet) return null;
  const id = picked?.id;
  return { sheet, layout, selected: id ? [id] : [], names: id ? [picked?.name || id] : [] };
}

/** The same look, as a key: a `look-at` is sent again only when it changes. */
export const lookKey = (look: LookingAt | null): string => (look ? JSON.stringify([look.sheet.toUpperCase(), look.layout ?? '', look.selected]) : '');

// ---------------------------------------------------------------- command lines

/** One line for a command the agent called: its name, a few words, and whether it was refused or is running. */
export interface CommandLine {
  command: string;
  words: string;
  /** The core refused it: its name shows red. */
  refused: boolean;
  running: boolean;
}

/** At most `n` characters, cut with an ellipsis. */
export function cut(text: string, n = 60): string {
  return text.length <= n ? text : `${text.slice(0, n - 1).trimEnd()}…`;
}

const obj = (x: unknown): Record<string, unknown> => (x && typeof x === 'object' && !Array.isArray(x) ? (x as Record<string, unknown>) : {});
const str = (x: unknown): string | undefined => (typeof x === 'string' && x ? x : undefined);
const num = (x: unknown): number | undefined => (typeof x === 'number' && Number.isFinite(x) ? x : undefined);
const arr = (x: unknown): unknown[] => (Array.isArray(x) ? x : []);

/** A value as a short word: text or a number, or nothing for anything else. */
const word = (x: unknown): string | undefined => str(x) ?? (num(x) !== undefined ? String(x) : undefined);

/** An operation's target, whole: `move DP-LD-101/device`. Whatever the agent sent, of any shape. */
function operationLine(op: unknown): string {
  const o = obj(op);
  const name = str(o.op) ?? '?';
  const target = str(obj(o.target).id) ?? str(obj(o.target).name);
  if (name === 'set_attribute') return `set ${word(o.tag) ?? '?'} = ${word(o.value) ?? ''}${target ? ` on ${target}` : ''}`;
  if (target) return `${name} ${target}`;
  if (str(o.sheet)) return `${name} on ${o.sheet}`;
  return name;
}

/** A change set's operations in a line: the first, and how many more. */
function operationsLine(changeSet: unknown): string {
  const ops = arr(obj(changeSet).operations);
  if (!ops.length) return 'an empty change set';
  return `${operationLine(ops[0])}${ops.length > 1 ? ` and ${ops.length - 1} more` : ''}`;
}

/** A change set's operations as the journal reads them, only those of the shape it reads. */
const wellFormed = (ops: unknown): Operation[] => arr(ops).filter((o): o is Operation => typeof obj(o).op === 'string');

/** What a query asks, in a few words: `find DP-LD-101/device`, `attributes block TRANSMITTER`. */
function queryWords(input: Record<string, unknown>): string {
  const q = obj(input.query);
  const kind = str(q.kind) ?? 'query';
  const what =
    str(q.id) ??
    str(q.name) ??
    (str(q.block) ? `block ${q.block}` : undefined) ??
    (str(q.text) ? `“${q.text}”` : undefined) ??
    (str(q.tag) ? `${q.tag}${str(q.value) ? ` = ${q.value}` : ''}` : undefined) ??
    (str(q.value) ? `value ${q.value}` : undefined) ??
    (q.window ? 'in a window' : q.crossing ? 'crossing a box' : q.touching ? `touching ${str(obj(q.touching).id) ?? str(obj(q.touching).name) ?? '?'}` : undefined);
  const sheet = str(input.sheet);
  return [kind, what, !what && sheet ? `on ${sheet}` : undefined].filter(Boolean).join(' ');
}

/** The short JSON of an input, for a command the page has no words for. */
function shortJson(input: unknown): string {
  let text = '';
  try {
    text = JSON.stringify(input) ?? '';
  } catch {
    text = String(input);
  }
  return cut(text, 60);
}

/**
 * A command's line, as the chat shows it under the assistant's name:
 * `looking-at · Sheet DP-LD-101 · 1 selected`, `query · find
 * DP-LD-101/device`, `propose · move DP-LD-101/device → proposal 1`; one
 * the core refused says why, in its words.
 */
export function commandLine(part: CommandPart): CommandLine {
  try {
    return commandLineOf(part);
  } catch {
    // An input or a reply of a shape the page does not read: its JSON, short.
    const running = part.reply === null || part.ok === null;
    return { command: typeof part.command === 'string' ? part.command : 'tool', words: shortJson(part.input), refused: part.ok === false && !running, running };
  }
}

function commandLineOf(part: CommandPart): CommandLine {
  const input = obj(part.input);
  const reply = part.reply;
  const result = obj(reply?.result);
  const running = reply === null || part.ok === null;
  const refused = part.ok === false || reply?.ok === false;
  let words: string;
  switch (part.command) {
    case 'looking-at': {
      const sheet = str(result.sheet);
      const n = arr(result.selected).length;
      words = running ? 'what you’re looking at' : sheet ? `${sheetWords(sheet, str(result.layout))} · ${n ? `${n} selected` : 'nothing selected'}` : 'nothing recorded yet';
      break;
    }
    case 'look-at':
      words = str(input.sheet) ? `${sheetWords(str(input.sheet) as string, str(input.layout))} · ${arr(input.selected).length ? `${arr(input.selected).length} selected` : 'nothing selected'}` : shortJson(input);
      break;
    case 'query':
      words = queryWords(input);
      break;
    case 'propose': {
      const n = num(result.proposal);
      words = `${operationsLine(input.change_set)}${n !== undefined ? ` → proposal ${n}` : ''}`;
      break;
    }
    case 'change':
      words = operationsLine(input.change_set);
      break;
    case 'proposals': {
      const n = arr(result.proposals).length;
      words = running ? 'the proposals waiting' : `${n} waiting`;
      break;
    }
    case 'keep':
    case 'discard':
      words = `proposal ${num(input.proposal) ?? '?'}`;
      break;
    case 'journal': {
      const n = arr(result.entries).length;
      words = running ? 'the journal' : plural(n, 'entry', 'entries');
      break;
    }
    case 'check': {
      const sheets = arr(input.sheets).filter((s): s is string => typeof s === 'string');
      const found = arr(result.findings).length;
      const where = sheets.length ? listed(sheets) : 'every sheet';
      words = running ? where : `${where} · ${found ? plural(found, 'finding') : 'all pass'}`;
      break;
    }
    case 'display-list':
      words = [str(input.sheet) ?? str(input.source) ?? '', str(input.layout) ? `layout ${input.layout}` : ''].filter(Boolean).join(', ') || shortJson(input);
      break;
    default:
      words = shortJson(part.input);
  }
  if (refused && !running) {
    const error = obj(reply?.error);
    const why = str(error.message) ?? str(error.code) ?? 'refused';
    words = `${words ? `${words}: ` : ''}refused, ${cut(why.replace(/`/g, ''), 80)}`;
  }
  return { command: typeof part.command === 'string' ? part.command : 'tool', words, refused: refused && !running, running };
}

/** An input or a reply as the line opened shows it: JSON, two spaces in, a short list of numbers on one line. */
export function pretty(value: unknown): string {
  if (value === null || value === undefined) return '…';
  return JSON.stringify(value, null, 2).replace(/\[\s+([-\d.,\se]+?)\s+\]/g, (_m, inner: string) => `[${inner.replace(/\s+/g, ' ')}]`);
}

/** A drawing set's folder as written, the same however it is spelled: `./a/b/`, `a\\b` and `a/b` are one. */
export function sameSet(a: string, b: string): boolean {
  const plain = (s: string) =>
    s
      .replace(/\\/g, '/')
      .split('/')
      .filter((p) => p !== '' && p !== '.')
      .join('/');
  return plain(a) === plain(b);
}

/**
 * Matches a command part to the drawing set shown: prefers the canonical path
 * resolved by `serve` so tabs opened under different spellings (relative vs
 * absolute, symlinks) keep their chat cards (#227), falling back to normalized
 * path text when canonical paths are absent.
 */
function partMatchesSet(part: CommandPart, set: string, canonicalSet?: string): boolean {
  if (canonicalSet && part.canonical_set) return sameSet(part.canonical_set, canonicalSet);
  const on = str(obj(part.input).set) ?? str(obj(part.reply?.result).set);
  return !!on && sameSet(on, set);
}

/**
 * The proposal a part made on the drawing set `set`: a `propose` of that
 * set whose reply is ok, by its number; null for any other part, so a
 * proposal is matched by its set and its number, never its number alone.
 */
export function proposalMade(part: Part, set: string, canonicalSet?: string): number | null {
  if (part.kind !== 'command' || part.command !== 'propose' || !part.reply?.ok) return null;
  if (!partMatchesSet(part, set, canonicalSet)) return null;
  return num(obj(part.reply.result).proposal) ?? null;
}

/** The guide the assistant gave on the set `set`, by its number, from its `guide` (#157); null for any other part. */
export function guideMade(part: Part, set: string, canonicalSet?: string): number | null {
  if (part.kind !== 'command' || part.command !== 'guide' || !part.reply?.ok) return null;
  if (!partMatchesSet(part, set, canonicalSet)) return null;
  return num(obj(part.reply.result).guide) ?? null;
}

/** What a proposal the assistant made does, from its `propose`, for its card once it is no longer waiting: `Move device`. */
export function proposedTitle(part: CommandPart): string {
  const cs = obj(obj(part.input).change_set);
  return capital(operationsTitle(wellFormed(cs.operations), arr(cs.fixes).filter((f): f is string => typeof f === 'string')));
}

/** The sheets a proposal reached when it was made, from its `propose`'s reply. */
export function proposedSheets(part: CommandPart): string[] {
  return arr(obj(part.reply?.result).sheets).filter((s): s is string => typeof s === 'string');
}

/** What a proposal's sheets link says: `DP-LD-101 ›`, `7 sheets ›`. */
export const sheetsLink = (sheets: readonly string[]): string => (sheets.length > 1 ? `${sheets.length} sheets ›` : `${sheets[0] ?? 'Open'} ›`);

/** The words under a waiting proposal's card's title. */
export const waitingWords = (n: number): string => `Proposal ${n}, dashed on the drawing. Nothing changes until you keep it.`;

/** How a proposal that no longer waits ended, from the journal. */
export interface Ended {
  how: 'kept' | 'discarded';
  /** The journal's entry: the change set it landed as, or the discard. */
  entry: number;
  by: string | undefined;
  /** The entry that undid the change set it landed as. */
  undoneBy?: number;
}

export function proposalEnd(journal: JournalReport | null, n: number): Ended | null {
  const e: Entry | undefined = journal?.entries.find((x) => x.proposal === n && (x.kind === 'change_set' || x.kind === 'discard'));
  if (!e) return null;
  return e.kind === 'change_set'
    ? { how: 'kept', entry: e.entry, by: e.kept_by, undoneBy: e.undone_by }
    : { how: 'discarded', entry: e.entry, by: e.discarded_by };
}

/** What an ended proposal's card says: `Kept by you: change set 4, in the journal.`, `Discarded by you: nothing changed.` */
export function endedWords(end: Ended): string {
  const who = whoWords(end.by);
  if (end.how === 'discarded') return `Discarded by ${who}: nothing changed.`;
  return `Kept by ${who}: change set ${end.entry}, in the journal${end.undoneBy ? `, undone by entry ${end.undoneBy}` : ''}.`;
}

// ---------------------------------------------------------------- the agent's words

/** The agent's words, plain, in pieces, `**bold**` marked: nothing else of its Markdown is read, and no HTML ever. */
export function wordPieces(text: string): { text: string; bold: boolean }[] {
  const pieces: { text: string; bold: boolean }[] = [];
  const re = /\*\*([^*\n]+?)\*\*/g;
  let last = 0;
  for (let m = re.exec(text); m; m = re.exec(text)) {
    if (m.index > last) pieces.push({ text: text.slice(last, m.index), bold: false });
    pieces.push({ text: m[1] as string, bold: true });
    last = m.index + m[0].length;
  }
  if (last < text.length) pieces.push({ text: text.slice(last), bold: false });
  return pieces;
}

// ---------------------------------------------------------------- the command bar

/** What Enter does with the words typed in the command bar. */
export type Route = 'refused' | 'start' | 'input' | 'assistant';

/**
 * Where the command bar's words go on Enter: nowhere while a preview is
 * shown; to the tool running; a tool's alias or name starts it; words with
 * a letter that are no tool's go to the assistant; anything else, such as
 * a point with no tool running, to the renderer, which says what it is.
 */
export function typedRoute(text: string, toolRunning: boolean, previewOpen: boolean, isToolWord: (word: string) => boolean): Route {
  if (previewOpen) return 'refused';
  const t = text.trim();
  if (toolRunning || !t) return 'input';
  if (isToolWord(t)) return 'start';
  return /\p{L}/u.test(t) ? 'assistant' : 'start';
}

/** A short alias, of three letters at most, such as L, C, REC, M, E, T or DT, which Space enters, as in AutoCAD. */
const shortAlias = (t: string, isToolWord: (word: string) => boolean) => t.length <= 3 && /^\p{L}+$/u.test(t) && isToolWord(t);

/**
 * Whether Space is Enter in the command bar: inside a tool (not while its
 * text is typed, which the caller says), with nothing typed, or after a
 * short alias. A whole word, such as MOVE, waits for Enter, so a sentence
 * that starts with it can be typed for the assistant.
 */
export function spaceIsEnter(text: string, toolRunning: boolean, isToolWord: (word: string) => boolean): boolean {
  if (toolRunning) return true;
  const t = text.trim();
  return !t || shortAlias(t, isToolWord);
}

/**
 * The hint beside the command bar's field while words are typed with no
 * tool running and no preview open: `Enter: ask the assistant` for words
 * that are no tool's; `Enter: Move · keep typing to ask the assistant`
 * for a tool's whole name; nothing for a short alias or no letter.
 */
export function askHint(text: string, toolRunning: boolean, previewOpen: boolean, isToolWord: (word: string) => boolean, toolName: (word: string) => string | null): string {
  const t = text.trim();
  if (toolRunning || previewOpen || !t || !/\p{L}/u.test(t)) return '';
  if (isToolWord(t)) return shortAlias(t, isToolWord) ? '' : `Enter: ${toolName(t) ?? capital(t.toLowerCase())} · keep typing to ask the assistant`;
  return 'Enter: ask the assistant';
}

// ---------------------------------------------------------------- the agent

/** The panel header's agent: a picker when both are found, the one's name, none, or not known yet. */
export type AgentHeader =
  | { kind: 'loading' }
  | { kind: 'picker'; current: string; options: { agent: string; name: string }[] }
  | { kind: 'one'; agent: string; name: string }
  | { kind: 'none' };

export function agentHeader(report: AgentsReport | null, running: string | null): AgentHeader {
  if (!report) return { kind: 'loading' };
  const found = report.agents.filter((a) => a.found);
  const first = found[0];
  if (!first) return { kind: 'none' };
  if (found.length === 1) return { kind: 'one', agent: first.agent, name: first.name };
  const current = found.find((a) => a.agent === running) ?? first;
  return { kind: 'picker', current: current.agent, options: found.map((a) => ({ agent: a.agent, name: a.name })) };
}

/** The name of the agent the conversation runs on, for `<name> is working`; null for none. */
export function agentName(report: AgentsReport | null, running: string | null): string | null {
  const h = agentHeader(report, running);
  if (h.kind === 'one') return h.name;
  if (h.kind === 'picker') return h.options.find((o) => o.agent === h.current)?.name ?? null;
  return null;
}

export const HEADER_SUB = 'Your own agent, started by AgentDrafter, on';

/** The panel's intro, before the first message. */
export const INTRO =
  'Ask about the drawing, or say what to change. It is told the sheet you have open and what you select, as the chip above the input says. What it changes waits, dashed on the drawing, until you keep it. Words typed in the command bar come here too.';

/** What the no-agent box says it looked for: `Claude Code (claude) and Codex (codex), and found neither`. */
export function lookedFor(report: AgentsReport): string {
  const names = report.agents.map((a) => `${a.name} (${a.command})`);
  if (names.length === 1) return `${names[0]} and did not find it`;
  return `${listed(names)}, and found ${names.length === 2 ? 'neither' : 'none of them'}`;
}

export function noAgentWords(report: AgentsReport): string {
  return `The assistant runs on an agent you already have, signed in as you are now: no API key goes in AgentDrafter. When it started, AgentDrafter looked on your PATH for ${lookedFor(report)}.`;
}

/** The chat input's placeholder. */
export function inputPlaceholder(name: string | null, busy: boolean): string {
  if (!name) return 'No agent yet: install one first';
  return busy ? `${name} is working…` : 'Ask, or say what to change…';
}

/** What a phone's peek offers, to open the chat. */
export function peekAskWords(name: string | null, busy: boolean, known: boolean): string {
  if (!known) return 'Ask the assistant…';
  if (!name) return 'No agent found: how to install one ›';
  return busy ? `${name} is working…` : 'Ask the assistant…';
}

/** What the page says when a message did not go, in the server's words or its own for the codes it knows. */
export function notSentWords(code: string, message: string, name: string | null): string {
  if (code === 'assistant_busy') return `${name ?? 'The assistant'} is still working: one message at a time.`;
  if (code === 'no_agent') return 'No agent was found: install Claude Code or Codex first, as the panel says.';
  return message;
}

/** The proposals a conversation's assistant made, by number, in the order it made them. */
export function proposalsMade(items: readonly Item[], set: string, canonicalSet?: string): number[] {
  const out: number[] = [];
  for (const item of items) {
    if (item.kind !== 'assistant') continue;
    for (const part of item.parts) {
      const n = proposalMade(part, set, canonicalSet);
      if (n !== null) out.push(n);
    }
  }
  return out;
}

/** What a proposal's card in the chat lets the drafter do now, and why not. */
export interface ChatCardActs {
  keep: boolean;
  discard: boolean;
  /** Why Keep or Discard is off, in a line; null when both are on. */
  note: string | null;
}

/**
 * The chat card's Keep and Discard, held to what the card on the drawing
 * holds to: neither while the journal is stepped through, since they act
 * on the set as it is; Keep only with a current verdict from the
 * proposal's own drawing, which says the set would take it, so a proposal
 * not drawn on the sheet shown is opened to check it first.
 */
export function chatCardActs(state: { stepping: boolean; drawn: boolean; checking: boolean; refused: boolean }): ChatCardActs {
  if (state.stepping) return { keep: false, discard: false, note: 'Go back to live to keep or discard it.' };
  if (!state.drawn) return { keep: false, discard: true, note: 'Open it to see how it stands before you keep it.' };
  if (state.checking) return { keep: false, discard: true, note: null };
  return { keep: !state.refused, discard: true, note: null };
}

/** Whether the conversation has a message yet: the intro shows until it has. */
export const hasMessages = (items: readonly Item[]): boolean => items.some((i) => i.kind !== 'note');
