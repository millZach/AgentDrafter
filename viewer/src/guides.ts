// Guides (#157, drafting app build 9): the assistant's guide, run on the
// drafter's own drawing. The core lists each guide with its steps worked
// out on the set as it stands (`guides`, sent on the event stream), checks
// each change set sent for a step against the step's operation (`change`'s
// reply, under `guide`), and marks the step in the journal. The renderer
// says what the step wants next from the tool's own state, where its ring
// goes, and whether a point or a pick is the step's (`guide.rs`).
//
// What is left is the page's, here, tested without a browser: which step
// runs, what each step came to, the banner's words, the gentle notes, the
// summary at the end and its one Undo, and the guide's mark in the journal.
// Nothing here works out a point or checks an operation (ADR-0007), and
// nothing here undoes the drafter's work unasked.

/** A point to click or an object to pick, as `guides` lists it. */
export interface GuidePoint {
  label?: string;
  at?: [number, number, number];
  snap?: { id: string; snap: string; which?: string };
  object?: string;
  waits_on?: number;
  error?: string;
}

/** One step, worked out on the set as it stands. */
export interface GuideStep {
  step: number;
  say: string;
  title: string;
  tool?: string;
  setting?: { current_layer: string };
  completes?: Record<string, unknown>;
  needs?: number[];
  points: GuidePoint[];
  /** Its operation with each `made_by` named, ready to send; none while a step it needs made nothing. */
  operation?: Record<string, unknown>;
  landed?: number[];
  /** The newest change set in place that completes it, as the core checked it. */
  completed_by?: number;
  made?: string;
}

/** A guide, as `guides` lists it. */
export interface Guide {
  guide: number;
  by: string;
  at: string;
  base_entry: number;
  title: string;
  sheet: string;
  steps: GuideStep[];
  error?: string;
}

export interface GuidesReport {
  set: string;
  /** The journal's newest entry, which the listing reflects. */
  newest_entry?: number;
  guides: Guide[];
}

/** A change set sent for a step, checked by the core against the step's operation. */
export interface GuideChecked {
  guide: number;
  step: number;
  of: number;
  completes: boolean;
  message: string;
  made?: string;
}

/** The step a journal entry was sent for. */
export interface GuideMark {
  guide: number;
  step: number;
  of: number;
  /** Whether it completes the step, as the core checked it before it landed. */
  completes?: boolean;
}

/** What the renderer says of a point taken or an object picked that the step does not want. */
export interface RendererNote {
  words: string;
  take_back: 'point' | 'pick';
  place?: number;
}

/** What the step wants next, as the renderer says it. */
export type Phase =
  | { phase: 'start' }
  | { phase: 'pick'; point: number }
  | { phase: 'click'; point: number }
  | { phase: 'finish' }
  | { phase: 'type' }
  /** Move, Copy or Erase has picked what the step picks: Enter ends picking. */
  | { phase: 'end_pick' }
  /** Move or Copy wants a base point the step leaves to the drafter. */
  | { phase: 'base' }
  /** Move or Copy has its base point and wants where it goes, typed. */
  | { phase: 'place' }
  /** A step with no rings wants another of the points its tool takes, such as a line's second end. */
  | { phase: 'free' };

/** What a step came to. */
export type StepStatus = 'todo' | 'done' | 'forme' | 'skipped' | 'needs';

/** What a note's button does: undoes a change set, takes back a point or a pick, or ends a tool. */
export type NoteAct =
  | { kind: 'undo'; label: string; entry: number }
  | { kind: 'take_back'; label: string; what: 'point' | 'pick'; place: number }
  | { kind: 'end_tool'; label: string };

/** A gentle note in the banner: what the drafter did against what the step wants. */
export interface Note {
  tone: 'warn' | 'info';
  words: string;
  act?: NoteAct;
}

/** A guide running, on the page. */
export interface GuideRun {
  /** Monotonic ID for this start of the guide, so a late reply from an earlier run is ignored. */
  runId: number;
  guide: number;
  /** The step running, from 0. */
  at: number;
  status: StepStatus[];
  /** Why a step was skipped for another. */
  why: string[];
  /** The change sets sent for its steps while it ran, by their entries. */
  entries: number[];
  /** The change set that completed each step, while it stays in place. */
  doneBy: (number | null)[];
  note: Note | null;
  /** The step just ended, for the line above the words. */
  last: { at: number; status: StepStatus; entry?: number } | null;
  /** Done when every step has had its turn; stopped by Stop. */
  end: null | 'done' | 'stopped';
  /** The undo that took the guide's steps back. */
  undone: number | null;
  /** Do it for me was pressed for the step running. */
  forMe: boolean;
}

/** The tools as the ribbon names them, with the alias the banner tells a laptop to type. */
export const TOOL_WORDS: Record<string, { label: string; alias: string }> = {
  line: { label: 'Line', alias: 'L' },
  circle: { label: 'Circle', alias: 'C' },
  rectangle: { label: 'Rectangle', alias: 'REC' },
  text: { label: 'Text', alias: 'T' },
  move: { label: 'Move', alias: 'M' },
  copy: { label: 'Copy', alias: 'CO' },
  erase: { label: 'Erase', alias: 'E' },
};

export const toolLabel = (tool: string): string => TOOL_WORDS[tool]?.label ?? tool;

const finished = (s: StepStatus) => s === 'done' || s === 'forme';

let nextRunId = 0;

/** A guide started: its first step running, nothing landed. */
export function startRun(guide: Guide): GuideRun {
  nextRunId += 1;
  return {
    runId: nextRunId,
    guide: guide.guide,
    at: 0,
    status: guide.steps.map(() => 'todo'),
    why: guide.steps.map(() => ''),
    entries: [],
    doneBy: guide.steps.map(() => null),
    note: null,
    last: null,
    end: null,
    undone: null,
    forMe: false,
  };
}

/**
 * The run moved to its next step: a step that names what a step not done
 * makes is skipped too, and says why; past the last, the guide is done.
 */
export function advance(run: GuideRun, guide: Guide): GuideRun {
  const status = [...run.status];
  const why = [...run.why];
  let at = run.at + 1;
  while (at < guide.steps.length) {
    const missing = (guide.steps[at]?.needs ?? []).find((n) => !finished(status[n - 1] ?? 'todo'));
    if (missing === undefined) break;
    status[at] = 'needs';
    why[at] = `it needs what step ${missing} makes`;
    at += 1;
  }
  return { ...run, at, status, why, note: null, forMe: false, end: at >= guide.steps.length ? 'done' : null };
}

/** The step running done: by the drafter, or for them; the next follows. */
export function stepDone(run: GuideRun, guide: Guide, entry?: number): GuideRun {
  const status = [...run.status];
  const s: StepStatus = run.forMe ? 'forme' : 'done';
  status[run.at] = s;
  const doneBy = [...run.doneBy];
  doneBy[run.at] = entry ?? null;
  return advance({ ...run, status, doneBy, last: { at: run.at, status: s, entry } }, guide);
}

/**
 * The run against the guide as `guides` lists it, after the journal
 * changed: a step whose change set was undone, from any interface, is to do
 * again, and the guide steps back to the first such. `newest` is the
 * journal's newest entry the listing reflects: a step done by a later one
 * than it, which the listing has not seen yet, stays done.
 */
export function synced(run: GuideRun, guide: Guide, newest: number): GuideRun {
  if (run.end) return run;
  const status = [...run.status];
  const doneBy = [...run.doneBy];
  let back = -1;
  guide.steps.forEach((step, i) => {
    const by = doneBy[i];
    if (by === null || by === undefined || by > newest) return;
    if (step.completed_by !== undefined) {
      doneBy[i] = step.completed_by;
      return;
    }
    status[i] = 'todo';
    doneBy[i] = null;
    if (back < 0) back = i;
  });
  if (back < 0) return doneBy.every((d, i) => d === run.doneBy[i]) ? run : { ...run, doneBy };
  return { ...run, status, doneBy, at: Math.min(run.at, back), note: null, last: null, forMe: false };
}

/** Skip: the step is passed over, and any that needs what it makes with it. */
export function skipStep(run: GuideRun, guide: Guide): GuideRun {
  const status = [...run.status];
  status[run.at] = 'skipped';
  return advance({ ...run, status, last: { at: run.at, status: 'skipped' } }, guide);
}

/** Stop: the guide ends where it is, with what has landed left as it is. */
export function stopRun(run: GuideRun): GuideRun {
  return { ...run, end: 'stopped', note: null };
}

/**
 * A change set sent for the step running came back: one that completes it
 * moves the guide on; one that does not lands as the drafter's own edit,
 * and the note offers to undo it. The guide waits.
 */
export function landed(run: GuideRun, guide: Guide, checked: GuideChecked, entry: number | undefined, sentRunId?: number): GuideRun {
  // A late reply for another guide's step, or an earlier run of the same guide, is not this run's.
  if (checked.guide !== run.guide || (sentRunId !== undefined && sentRunId !== run.runId)) return run;
  const entries = entry !== undefined ? [...run.entries, entry] : run.entries;
  const next = { ...run, entries };
  if (checked.step !== run.at + 1 || run.end) return next;
  if (checked.completes) return stepDone(next, guide, entry);
  return {
    ...next,
    forMe: false,
    note: {
      tone: 'warn',
      words: `${checked.message} Undo it?`,
      act: entry !== undefined ? { kind: 'undo', label: 'Undo it', entry } : undefined,
    },
  };
}

/** A point or a pick the renderer says the step does not want: the note, which offers to take it back. */
export function clickNote(note: RendererNote): Note {
  const pick = note.take_back === 'pick';
  return {
    tone: 'warn',
    words: `${note.words} ${pick ? 'Take it out?' : 'Undo it?'}`,
    act: { kind: 'take_back', label: pick ? 'Take it out' : 'Undo it', what: note.take_back, place: note.place ?? 0 },
  };
}

/** A tool started while a step runs: none when it is the step's own; otherwise a note offering to end it. */
export function toolNote(step: GuideStep, tool: string, layer: string): Note | null {
  const started = toolLabel(tool);
  if (step.setting) {
    const want = step.setting.current_layer;
    if (sameName(layer, want)) return null;
    return {
      tone: 'warn',
      words: `${started} would draw on layer ${layer}; this step wants ${want} current first. End ${started} and pick ${want}?`,
      act: { kind: 'end_tool', label: `End ${started}` },
    };
  }
  if (!step.tool || step.tool === tool) return null;
  return {
    tone: 'warn',
    words: `That started ${started}; this step wants ${toolLabel(step.tool)}. End it?`,
    act: { kind: 'end_tool', label: `End ${started}` },
  };
}

/**
 * Undo it of a change set, `entry`, checked against the journal as it is
 * when it is pressed: an undo takes every later change set with it, so one
 * with later edits in place is not undone, and the note says why.
 */
export function undoItBlocked(entry: number, canUndo: number[]): Note | null {
  const later = canUndo.filter((e) => e > entry).length;
  if (!later) return null;
  return {
    tone: 'warn',
    words: `Undo it would also take back ${later} later edit${later === 1 ? '' : 's'}, so nothing was undone. Undo from the journal if you mean to.`,
  };
}

/**
 * Undo the guide's steps, checked against the journal as it is when it is
 * pressed: the words on the button, `label`, still say what it takes back,
 * or nothing is undone and the reason is given.
 */
export function undoAllBlocked(run: GuideRun, guide: Guide, canUndo: number[], from: number, label: string): string | null {
  const now = summaryOf(run, guide, canUndo);
  if (now.undoFrom === from && now.undoLabel === label) return null;
  if (now.undoFrom === null) return 'Its steps can no longer be undone.';
  return `The journal changed since: the button now says “${now.undoLabel}”.`;
}

/**
 * What pulses for the step, from the phase the renderer reports: the
 * Layer box for a setting step; the step's tool until it runs; Done when
 * Enter or Done is what the step wants next, picking ended or the step's
 * points all taken (Move lands on its last point, so has no Done); and
 * nothing while the drafter picks, clicks a ring, gives a base point,
 * types where it goes or types the text, or while a note waits.
 */
export function pulseOf(step: GuideStep | null, phase: Phase | null, warned: boolean): string | undefined {
  if (!step || warned) return undefined;
  if (step.setting) return 'layer';
  switch (phase?.phase ?? 'start') {
    case 'start':
      return step.tool;
    case 'end_pick':
      return 'done';
    case 'finish':
      return step.tool === 'move' ? undefined : 'done';
    case 'pick':
    case 'click':
    case 'base':
    case 'place':
    case 'free':
    case 'type':
      return undefined;
  }
  return undefined;
}

/**
 * Whether Skip or Stop ends the tool running: only the step's own tool,
 * running on the guide's sheet, whose points were the step's; a tool the
 * drafter started for themselves, such as the same tool on another sheet,
 * keeps what it holds.
 */
export function endsStepTool(
  step: GuideStep | undefined,
  running: string | null | undefined,
  shown: string | null | undefined,
  guideSheet: string | null | undefined,
): boolean {
  return !!step?.tool && !!running && step.tool === running && !!shown && !!guideSheet && sameName(shown, guideSheet);
}

/** The toast once guide `n` starts: its own Start gone, any other guide's left. */
export function toastAfterStart<T extends { guides?: number[] }>(toast: T | null, n: number): T | null {
  const rest = (toast?.guides ?? []).filter((g) => g !== n);
  return toast && rest.length ? { ...toast, guides: rest } : null;
}

/** The toast for guides given since the last listing: one Start for each. */
export function givenToast(given: Guide[], who: (by: string) => string): { title: string; detail: string; guides: number[] } | null {
  const first = given[0];
  if (!first) return null;
  const about = (g: Guide) => `Guide ${g.guide}: ${g.steps.length} step${g.steps.length === 1 ? '' : 's'} on ${g.sheet}`;
  if (given.length === 1) {
    return { title: `${who(first.by)} gives a guide: ${first.title}`, detail: `${about(first)}, for you to do on your drawing.`, guides: [first.guide] };
  }
  return {
    title: `${given.length} guides given: ${given.map((g) => g.title).join('; ')}`,
    detail: `${given.map(about).join(' · ')}, for you to do on your drawing.`,
    guides: given.map((g) => g.guide),
  };
}

/** The step's tool ended with nothing drawn, as Esc ends it: the step waits. */
export function endedNote(step: GuideStep): Note | null {
  if (!step.tool) return null;
  const label = toolLabel(step.tool);
  return { tone: 'info', words: `${label} ended with nothing drawn, so the step waits. Start ${label} again to go on, or skip the step.` };
}

const sameName = (a: string, b: string) => a.trim().toUpperCase() === b.trim().toUpperCase();

/** A layer made current while a setting step runs: whether it is the step's, and the note when it is not. */
export function layerChosen(step: GuideStep, layer: string): { done: boolean; note: Note | null } {
  const want = step.setting?.current_layer;
  if (!want) return { done: false, note: null };
  if (sameName(layer, want)) return { done: true, note: null };
  return { done: false, note: { tone: 'warn', words: `That made ${layer} current; the step wants ${want}. Pick ${want} in the Layer box.` } };
}

/** What the drafter does next, in the banner's words: from the step and what the renderer says it wants. */
export function nowWords(step: GuideStep, phase: Phase | null, phone: boolean): string {
  const click = phone ? 'Tap' : 'Click';
  if (step.setting) {
    const want = step.setting.current_layer;
    return phone ? `Open Tools, and pick ${want} in the Layer box.` : `Pick ${want} in the ribbon’s Layer box.`;
  }
  const tool = step.tool ?? '';
  const label = toolLabel(tool);
  const point = (i: number) => step.points[i];
  const named = (i: number) => point(i)?.label ?? 'the ring';
  switch (phase?.phase ?? 'start') {
    case 'start':
      return phone
        ? `Open Tools and tap ${label}: it pulses.`
        : `Start ${label}: type ${TOOL_WORDS[tool]?.alias ?? label} and Enter, or click ${label} in the ribbon.`;
    case 'pick': {
      const i = (phase as { point: number }).point;
      return `${click} ${named(i)}, in the ring, to pick it.`;
    }
    case 'click': {
      const i = (phase as { point: number }).point;
      const p = point(i);
      if (p?.waits_on) return `The ring waits on step ${p.waits_on}.`;
      const snap = p?.snap ? `, where the ${snapWords(p.snap.snap)} snap shows` : '';
      return `${click} the ring: ${named(i)}${snap}.${phone && i === 0 ? ' Hold for the magnifier.' : ''}`;
    }
    case 'type': {
      const contents = typeof step.operation?.contents === 'string' ? (step.operation.contents as string) : null;
      if (!contents) return phone ? 'Type the text in the box, then Add.' : 'Type the text, then press Enter twice.';
      return phone ? `Type ${contents} in the box, then Add.` : `Type ${contents}, then press Enter twice.`;
    }
    case 'end_pick':
      if (tool === 'erase') return phone ? 'Tap Done to erase it.' : 'Press Enter, or click Done, to erase it.';
      return phone ? 'Tap Done: what the step picks is picked.' : 'Press Enter, or click Done: what the step picks is picked.';
    case 'base':
      return `${click} any point, as the base point.`;
    case 'free':
      return `${click} a point for the ${label.toLowerCase()}, as the step says.`;
    case 'place':
    case 'finish': {
      // Copy, once it has placed the step's copies, wants Done.
      if (tool === 'copy' && phase?.phase === 'finish') return phone ? 'Tap Done to place the copy.' : 'Press Enter, or click Done, to place the copy.';
      if (tool === 'move' || tool === 'copy') {
        const by = step.operation?.by;
        if (Array.isArray(by) && by.length >= 2) {
          const at = `@${fmt(by[0] as number)},${fmt(by[1] as number)}`;
          if (phone) return `Tap ⌨ Type, type ${at} and Go.`;
          return `Type ${at} and press Enter.`;
        }
        return `${click} where it goes.`;
      }
      if (tool === 'erase') return phone ? 'Tap Done to erase it.' : 'Press Enter, or click Done, to erase it.';
      return phone ? `Tap Done to end the ${label.toLowerCase()}.` : `Press Enter, or click Done, to end the ${label.toLowerCase()}.`;
    }
  }
  return '';
}

const fmt = (n: number) => String(Math.round(n * 1e6) / 1e6);

/** A snap's name in words, as the banner says it: `centre` for AutoCAD's center. */
export function snapWords(snap: string): string {
  const words: Record<string, string> = { center: 'Centre', endpoint: 'Endpoint', midpoint: 'Midpoint', quadrant: 'Quadrant', insertion: 'Insertion', node: 'Node', intersection: 'Intersection' };
  return words[snap] ?? snap.replace(/_/g, ' ');
}

/** The line above the step's words: how the step before ended. */
export function lastWords(run: GuideRun): { text: string; tone: 'ok' | 'skip' } | null {
  const l = run.last;
  if (!l) return null;
  const n = l.at + 1;
  if (l.status === 'skipped') return { text: `– Step ${n} skipped`, tone: 'skip' };
  const cs = l.entry !== undefined ? ` · change set ${l.entry}` : '';
  return { text: `${l.status === 'forme' ? `✓ Step ${n} done for you` : `✓ Step ${n} done`}${cs}`, tone: 'ok' };
}

/** Each step's dot: done, skipped, the one running, or to come. */
export function dots(run: GuideRun): ('done' | 'skip' | 'now' | '')[] {
  return run.status.map((s, i) => (i === run.at && !run.end ? 'now' : finished(s) ? 'done' : s === 'todo' ? '' : 'skip'));
}

/** A step's mark in a list: ✓ done, – skipped, · to come. */
export const MARKS: Record<StepStatus, string> = { done: '✓', forme: '✓', skipped: '–', needs: '–', todo: '·' };

/** The summary the banner shows at the end. */
export interface Summary {
  head: string;
  items: { status: StepStatus; title: string; small: string }[];
  line: string;
  /** The change set Undo the guide’s steps undoes, with every later one; none when there is nothing to undo. */
  undoFrom: number | null;
  undoLabel: string;
}

/**
 * The summary: each step and what it came to, what landed, and one Undo of
 * the guide's steps while they can be undone. `canUndo` is the journal's.
 */
export function summaryOf(run: GuideRun, guide: Guide, canUndo: number[]): Summary {
  const n = guide.steps.length;
  const done = run.status.filter(finished).length;
  const head =
    run.end === 'stopped'
      ? `Stopped at step ${run.at + 1} of ${n}`
      : `Finished: ${done} of ${n} steps done${done < n ? `, ${n - done} not done` : ''}`;
  const items = guide.steps.map((s, i) => {
    const st = run.status[i] ?? 'todo';
    const small =
      st === 'forme' ? 'done for you' : st === 'skipped' ? 'skipped' : st === 'needs' ? `skipped: ${run.why[i]}` : st === 'todo' ? 'not reached' : '';
    return { status: st, title: s.title, small };
  });
  const mine = run.entries.filter((e) => canUndo.includes(e));
  if (run.undone !== null) {
    return { head, items, line: `Undone: entry ${run.undone} took back change set${run.entries.length > 1 ? 's' : ''} ${listed(run.entries.map(String))}.`, undoFrom: null, undoLabel: '' };
  }
  if (!run.entries.length) return { head, items, line: 'Nothing landed.', undoFrom: null, undoLabel: '' };
  const from = mine.length ? Math.min(...mine) : null;
  const later = from === null ? 0 : canUndo.filter((e) => e > from && !run.entries.includes(e)).length;
  return {
    head,
    items,
    line: `What you drew is in the journal: change set${run.entries.length > 1 ? 's' : ''} ${listed(run.entries.map(String))}, each undone as any edit.`,
    undoFrom: from,
    undoLabel: `Undo the guide’s steps${later ? ` and ${later} later` : ''}`,
  };
}

const listed = (items: string[]) => (items.length <= 1 ? items.join('') : `${items.slice(0, -1).join(', ')} and ${items.at(-1)}`);

/** The guide an assistant's command gave, from its reply: `guide`'s number. */
export function guideGiven(command: string, reply: unknown): number | null {
  if (command !== 'guide') return null;
  const r = reply as { ok?: boolean; result?: { guide?: number } } | null;
  return r?.ok && typeof r.result?.guide === 'number' ? r.result.guide : null;
}

/** The guide the drafter is offered, by what the chat card says of it. */
export type CardState = 'offer' | 'run' | 'end';

/** The chat card's line under its steps. */
export function cardWords(state: CardState, run: GuideRun | null, steps: number): string {
  if (state === 'offer') return 'Each step lands as a change set, as your own edits do; at the end, one Undo takes the guide’s steps back.';
  if (state === 'run' && run) return `Running: step ${run.at + 1} of ${steps}.`;
  return run?.end === 'stopped' ? 'Stopped.' : 'Finished.';
}

/**
 * The change set a step's tool sends, marked with the step, so the core
 * checks it and the journal marks it. Only an operation step on the
 * guide's own sheet is marked; a setting step is never a change set's.
 */
export function marked<T extends { operations?: unknown[] }>(changeSet: T, run: GuideRun | null, guide: Guide | null, sheet: string | null): T & { guide?: { guide: number; step: number } } {
  if (!run || !guide || run.end || !sheet || !sameName(sheet, guide.sheet)) return changeSet;
  const step = guide.steps[run.at];
  if (!step || step.setting) return changeSet;
  return { ...changeSet, guide: { guide: run.guide, step: step.step } };
}

/**
 * The change set a template preview's Change all sends (#264): keeps its
 * `guide` mark and returns its `runId` while that exact guide run is still
 * active; if the guide ended or restarted while the preview was open, strips
 * `guide` so the edit lands without a stale mark.
 */
export function previewConfirmSend<T extends { operations?: unknown[]; guide?: { guide: number; step: number } }>(
  changeSet: T,
  guideRunId: number | undefined,
  run: GuideRun | null,
): { changeSet: T; runId: number | undefined } {
  if (guideRunId === undefined || !changeSet.guide) {
    return { changeSet, runId: undefined };
  }
  if (run && !run.end && run.runId === guideRunId) {
    return { changeSet, runId: guideRunId };
  }
  const rest = { ...changeSet };
  delete rest.guide;
  return { changeSet: rest, runId: undefined };
}
