// Proposals as the page shows them (#155), as the prototype Zach picked on
// 2026-11-19 shows them on the drawing (its variant B): the `proposals`
// command's reply, which the server's event stream sends each time one is
// made, kept or discarded, and what the page makes of it: what each is
// called, which sheets it reaches, what its card says from the core's
// preview of it on the sheet shown, and where each card goes beside the
// change it marks. Plain functions, tested without a browser.

import { previewVerdicts, type PreviewReport } from './frame.ts';
import { capital, operationsTitle, whoWords, type JournalReport, type Operation } from './journal.ts';

/** A waiting proposal, as `proposals` lists it. */
export interface Proposal {
  proposal: number;
  by: string;
  at: string;
  /** The journal's newest entry it was made against. */
  base_entry: number;
  /** The change sets and undos that landed after it was made, by their entries. */
  changed_since: number[];
  change_set: { operations?: Operation[]; fixes?: string[] };
  /** The sheets it reached when it was made, in the set's order. */
  sheets: string[];
}

export interface ProposalsReport {
  set: string;
  message: string;
  proposals: Proposal[];
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
const listed = (items: string[]) => (items.length <= 1 ? items.join('') : `${items.slice(0, -1).join(', ')} and ${items.at(-1)}`);
const plain = (text: string) => text.replace(/`/g, '');

/** What a proposal does, in a few words, as its card names it: `Move service`. */
export const proposalTitle = (p: Proposal): string => capital(operationsTitle(p.change_set.operations, p.change_set.fixes));

/** The proposals that reach `sheet`, the oldest first. */
export const onSheet = (proposals: readonly Proposal[], sheet: string): Proposal[] =>
  proposals.filter((p) => p.sheets.some((s) => s.toUpperCase() === sheet.toUpperCase()));

/** How many proposals wait on each sheet, by its name in capitals, for its tab. */
export function tabCounts(proposals: readonly Proposal[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const p of proposals) for (const s of p.sheets) counts.set(s.toUpperCase(), (counts.get(s.toUpperCase()) ?? 0) + 1);
  return counts;
}

/** The entries that landed since a proposal was made, in words: `change sets 5 and 6`, `change set 5 and undo 7`. */
export function sinceWords(changed: readonly number[], journal: JournalReport | null): string {
  const kind = (n: number) => journal?.entries.find((e) => e.entry === n)?.kind;
  const sets = changed.filter((n) => kind(n) !== 'undo').map(String);
  const undos = changed.filter((n) => kind(n) === 'undo').map(String);
  return listed(
    [sets.length ? `change set${sets.length > 1 ? 's' : ''} ${listed(sets)}` : '', undos.length ? `undo${undos.length > 1 ? 's' : ''} ${listed(undos)}` : ''].filter(
      Boolean,
    ),
  );
}

/** How a proposal stands now, from the core's preview of it: it would land, land but fail a check, or the set would refuse it. */
export type Standing = 'ok' | 'warn' | 'bad';

export function standing(report: PreviewReport): Standing {
  if (report.change.failed > 0) return 'bad';
  const v = previewVerdicts(report);
  return [...v.values()].some((s) => s.state === 'check') ? 'warn' : 'ok';
}

/** One line of a proposal's card: its tone, its mark and its words. */
export interface CardLine {
  tone: 'stale' | Standing;
  icon: string;
  text: string;
}

/**
 * What a proposal's card says: that the set changed since it was made,
 * when it did; and what keeping it would do now, in the core's words for a
 * refusal: refused, landing but failing a check, or landing.
 */
export function cardLines(p: Proposal, report: PreviewReport | null, journal: JournalReport | null): CardLine[] {
  const lines: CardLine[] = [];
  if (p.changed_since.length) {
    lines.push({ tone: 'stale', icon: '↻', text: `Set changed since: ${sinceWords(p.changed_since, journal)} landed after it.` });
  }
  if (!report) return lines;
  const n = report.change.sheets.length;
  const refused = report.change.sheets.filter((s) => !s.ok && s.error?.code !== 'shared_edit_failed');
  if (report.change.failed > 0) {
    const first = refused[0] ?? report.change.sheets.find((s) => !s.ok);
    const why = plain(first?.error?.message ?? 'a sheet would not take it');
    lines.push({ tone: 'bad', icon: '✕', text: `Refused now${n > 1 && first ? ` on ${first.sheet}` : ''}: ${why}. Keep can’t land it.` });
    return lines;
  }
  const v = previewVerdicts(report);
  const fails = report.change.sheets.filter((s) => v.get(s.sheet.toUpperCase())?.state === 'check').map((s) => s.sheet);
  if (fails.length) {
    lines.push({ tone: 'warn', icon: '!', text: n > 1 ? `All ${n} would take it, but ${listed(fails)} would fail a check.` : 'It would land, but fail a check.' });
  } else {
    lines.push({ tone: 'ok', icon: '✓', text: n > 1 ? `All ${n} would take it.` : 'It would land.' });
  }
  return lines;
}

/** A phone's card, which is short: one word for how it stands. */
export const standingWords: Record<Standing, string> = { ok: 'Would land', warn: 'Fails a check', bad: 'Refused now' };

/** How long ago `at` was, as a card says it: `just now`, `4 min ago`, `2 h ago`, `3 days ago`. */
export function ago(at: string, now: number): string {
  const then = new Date(at).getTime();
  if (Number.isNaN(then)) return '';
  const s = Math.max(0, Math.round((now - then) / 1000));
  if (s < 60) return 'just now';
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} h ago`;
  return plural(Math.round(h / 24), 'day') + ' ago';
}

/** Who made it and when, and that nothing lands until it is kept, for the band of a proposal opened. */
export function proposedWords(p: Proposal, clock: string, now: number): string {
  return `Proposed by ${whoWords(p.by)} at ${clock}, ${ago(p.at, now)}. Nothing lands until you keep it.`;
}

/** What the band says of a proposal made before the set changed; null when it did not. */
export function staleWords(p: Proposal, journal: JournalReport | null): string | null {
  if (!p.changed_since.length) return null;
  return `The set has changed since it was proposed: ${sinceWords(p.changed_since, journal)} landed after it. Kept now, it applies to the set as it stands, and is drawn here as it would land now.`;
}

/** The phone's line in the bottom sheet's peek: how many wait here and in all. */
export function peekWords(here: number, all: number): string {
  return here ? `${plural(here, 'proposal')} on this sheet, ${all} in all` : `${plural(all, 'proposal')} waiting, marked on their tabs`;
}

/** A card to place: its size, and the box round what its proposal marks, `[left, top, right, bottom]` in CSS pixels on the drawing. */
export interface CardToPlace {
  w: number;
  h: number;
  box: [number, number, number, number];
}

/** Where a card goes, its top left, and its leader, from its edge to the change it marks, `[x1, y1, x2, y2]`. */
export interface Placed {
  x: number;
  y: number;
  leader: [number, number, number, number];
}

/**
 * Where each card goes, as the prototype puts them: beside the change it
 * marks, to its right, or to its left where there is no room, or below it;
 * kept inside the drawing, `width` by `height`, and moved down clear of a
 * card placed before it, the highest change's first. A leader joins each
 * card to its change, as a markup's leader does.
 */
export function placeCards(cards: readonly CardToPlace[], width: number, height: number): Placed[] {
  const out: Placed[] = cards.map(() => ({ x: 0, y: 0, leader: [0, 0, 0, 0] }));
  const placed: { x: number; y: number; w: number; h: number }[] = [];
  const order = cards.map((c, i) => ({ c, i })).sort((a, b) => a.c.box[1] - b.c.box[1]);
  const inside = (y: number, h: number) => Math.max(6, Math.min(height - h, y));
  for (const { c, i } of order) {
    const [l, t, r, b] = c.box;
    let x = r + 14;
    let y = (t + b) / 2 - c.h / 2;
    if (x + c.w > width - 6) x = l - c.w - 14;
    if (x < 6) {
      x = Math.max(6, Math.min(width - c.w - 6, l));
      y = b + 10;
    }
    y = inside(y, c.h);
    for (let k = 0; k < 4; k++) {
      for (const q of placed) {
        if (x < q.x + q.w + 6 && x + c.w + 6 > q.x && y < q.y + q.h + 6 && y + c.h + 6 > q.y) y = q.y + q.h + 8;
      }
    }
    y = inside(y, c.h);
    placed.push({ x, y, w: c.w, h: c.h });
    // From the card's nearest edge to the change's.
    const toX = x > r ? r + 4 : x + c.w < l ? l - 4 : (l + r) / 2;
    const toY = Math.min(Math.max(y + c.h / 2, t), b);
    const fromX = x > toX ? x : x + c.w > toX ? Math.min(Math.max(toX, x), x + c.w) : x + c.w;
    const fromY = x <= toX && x + c.w >= toX ? (y > toY ? y : y + c.h) : y + Math.min(c.h / 2, 18);
    out[i] = { x: Math.round(x), y: Math.round(y), leader: [fromX, fromY, toX, toY] };
  }
  return out;
}
