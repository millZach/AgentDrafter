// What the bottom sheet holds, in the picked look (variant A): the peek,
// always in reach, with the stream's state, the newest entry, Undo last and
// Accept; the journal, which steps through the set's entries; and the
// inspector, which shows the object the reviewer picked. Plain components:
// what they do is the page's (App.tsx).

import { useState } from 'react';

import {
  changedBy,
  clock,
  entryState,
  entryTitle,
  operationWords,
  undoneWith,
  viaWords,
  type Entry,
  type JournalReport,
} from './journal.ts';
import type { SheetObject } from './probe.ts';

export interface PeekProps {
  /** What the page is watching, in a few words. */
  line: string;
  /** Whether the line's dot shows the stream open. */
  live: boolean;
  journal: JournalReport | null;
  failing: ReadonlySet<string>;
  /** The change set Undo last would undo. */
  undoable: number | null;
  /** Whether Accept can do nothing now, for a reason the server will give. */
  blocked: boolean;
  busy: string | null;
  onShow(entry: number): void;
  onUndo(): void;
  onAccept(): void;
}

export function Peek({ line, live, journal, failing, undoable, blocked, busy, onShow, onUndo, onAccept }: PeekProps) {
  const last = journal?.entries.at(-1);
  const state = last && journal ? entryState(last, journal, failing) : null;
  return (
    <>
      <div className={`va-agent${live ? ' working' : ''}`}>
        <i />
        {line}
      </div>
      {last && state ? (
        <button type="button" className="va-last" onClick={() => onShow(last.entry)}>
          <span className="je-n">{last.entry}</span>
          <span className="va-last-t">{entryTitle(last)}</span>
          <span className={`badge b-${state.cls}`}>{state.label}</span>
        </button>
      ) : (
        <div className="va-last empty">{journal ? 'No change sets yet.' : 'No journal to show.'}</div>
      )}
      <div className="va-actions">
        <button type="button" className="btn" data-act="undo" disabled={!undoable || busy !== null} onClick={onUndo}>
          &#x21B6; Undo last{undoable ? ` (#${undoable})` : ''}
        </button>
        <button
          type="button"
          className={`btn primary${blocked ? ' blocked' : ''}`}
          data-act="accept"
          disabled={!journal || busy !== null}
          onClick={onAccept}
        >
          {busy === 'accept' ? 'Checking…' : 'Accept…'}
        </button>
      </div>
    </>
  );
}

export interface JournalListProps {
  journal: JournalReport;
  failing: ReadonlySet<string>;
  /** The entry shown as it was, or null for the set as it is. */
  at: number | null;
  /** The entry whose changes are marked. */
  focused: number | null;
  busy: string | null;
  onStep(by: number): void;
  onShow(entry: number): void;
  onUndo(changeSet: number): void;
}

export function JournalList({ journal, failing, at, focused, busy, onStep, onShow, onUndo }: JournalListProps) {
  const [open, setOpen] = useState<ReadonlySet<number>>(new Set());
  const entries = journal.entries;
  const index = focused === null ? -1 : entries.findIndex((e) => e.entry === focused);
  const toggle = (entry: Entry) => {
    if (entry.entry === focused && open.has(entry.entry)) {
      const next = new Set(open);
      next.delete(entry.entry);
      setOpen(next);
      return;
    }
    setOpen(new Set(open).add(entry.entry));
    onShow(entry.entry);
  };
  const items = [...entries].reverse();
  return (
    <>
      <div className="va-jhead">
        <b>Journal</b>
        <span className="muted">
          {entries.length} entr{entries.length === 1 ? 'y' : 'ies'}
        </span>
        <div className="stepper">
          <button type="button" aria-label="Older" disabled={!entries.length} onClick={() => onStep(-1)}>
            &lsaquo;
          </button>
          <span>
            {index >= 0 ? index + 1 : '-'} / {entries.length}
          </span>
          <button type="button" aria-label="Newer" disabled={!entries.length} onClick={() => onStep(1)}>
            &rsaquo;
          </button>
        </div>
      </div>
      {journal.inconsistent && <p className="err">{journal.inconsistent.message}</p>}
      <ol className="va-journal">
        {items.map((e) => {
          const state = entryState(e, journal, failing);
          const shown = open.has(e.entry);
          const sheets = [
            ...(e.landed_on ?? []).map((s) => ({ s, ok: true })),
            ...(e.failed_on ?? []).map((f) => ({ s: f.sheet, ok: false })),
          ];
          const later = undoneWith(journal, e.entry).length;
          return (
            <li
              key={e.entry}
              className={`je je-${state.cls}${focused === e.entry ? ' on' : ''}${at === e.entry ? ' at' : ''}`}
              data-entry={e.entry}
            >
              <button type="button" className="je-head" onClick={() => toggle(e)}>
                <span className="je-n">{e.entry}</span>
                <span className="je-main">
                  <span className="je-t">{entryTitle(e)}</span>
                  <span className="je-m">
                    {sheets.map(({ s, ok }) => (
                      <span key={`${s}${ok}`} className={`sh ${ok ? 'sh-landed' : 'sh-failed'}`}>
                        {s} {ok ? '✓' : '✕'}
                      </span>
                    ))}
                    <span className="muted">{clock(e.at)}</span>
                  </span>
                </span>
                <span className={`badge b-${state.cls}`}>{state.label}</span>
              </button>
              {shown && (
                <div className="je-det">
                  {e.error && <p className="err">{e.error.message}</p>}
                  {(e.failed_on ?? []).map((f) => (
                    <p key={f.sheet} className="err">
                      <b>{f.sheet} not changed.</b> {f.message}
                    </p>
                  ))}
                  {e.kind === 'accept' && (
                    <p className="note">
                      Accepted through {viaWords(e.via)}.
                      {e.failed_checks?.length
                        ? ` Accepted despite ${e.failed_checks.length} failed check${e.failed_checks.length > 1 ? 's' : ''}.`
                        : ''}
                    </p>
                  )}
                  {e.issued_by && <p className="note">Issued by the accept in entry {e.issued_by}: it cannot be undone.</p>}
                  {e.undone_by && <p className="muted">Undone by entry {e.undone_by}.</p>}
                  {e.undo_state === 'pruned' && !e.undone_by && (
                    <p className="muted">Its undo state was pruned by entry {e.pruned_by}: it cannot be undone.</p>
                  )}
                  {(e.change_set?.operations ?? []).length > 0 && (
                    <ul className="ops">
                      {(e.change_set?.operations ?? []).map((o, i) => (
                        <li key={i}>
                          <span>{o.sheet ?? o.target?.sheet ?? o.target?.id?.split('/')[0] ?? ''}</span>
                          <code>{operationWords(o)}</code>
                        </li>
                      ))}
                    </ul>
                  )}
                  {journal.can_undo.includes(e.entry) && (
                    <div className="je-acts">
                      <button
                        type="button"
                        className="btn sm danger"
                        disabled={busy !== null}
                        onClick={() => onUndo(e.entry)}
                      >
                        Undo{later ? ` this and ${later} later` : ''}
                      </button>
                    </div>
                  )}
                </div>
              )}
            </li>
          );
        })}
        <li className="revsep">The start of the journal</li>
      </ol>
    </>
  );
}

export interface InspectorProps {
  object: SheetObject;
  sheet: string;
  journal: JournalReport | null;
  /** The failed checks the object fails, in words. */
  fails: string[];
  onBack(): void;
  onShow(entry: number): void;
}

export function Inspector({ object, sheet, journal, fails, onBack, onShow }: InspectorProps) {
  const rows: [string, string][] = [
    ['Type', object.block ? `Insert of ${object.block}` : object.type],
    ['ID', object.id ?? '(none)'],
    ['Name', object.name ?? '(none)'],
    ['Layer', object.layer],
    ['Sheet', sheet],
  ];
  const attributes = object.attributes ?? [];
  const history = journal && object.id ? changedBy(journal, object.id) : [];
  return (
    <div className="va-inspect">
      <div className="va-ihead">
        <button type="button" className="btn sm" data-back onClick={onBack}>
          &lsaquo; Journal
        </button>
        <span className="muted">Picked object</span>
      </div>
      <h3 className="insp-title">{object.name ?? object.id ?? object.type}</h3>
      {fails.map((f) => (
        <p key={f} className="err">
          &#x2715; {f}
        </p>
      ))}
      <table className="kv">
        <tbody>
          {rows.map(([k, v]) => (
            <tr key={k}>
              <th>{k}</th>
              <td>{v}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <h4>Attributes</h4>
      <table className="kv attrs">
        <tbody>
          {attributes.length ? (
            attributes.map((a) => (
              <tr key={a.tag}>
                <th>{a.tag}</th>
                <td>{a.value}</td>
              </tr>
            ))
          ) : (
            <tr>
              <td className="muted">None</td>
            </tr>
          )}
        </tbody>
      </table>
      <h4>Changed by</h4>
      {history.length ? (
        <div className="chips">
          {history.map((e) => (
            <button type="button" key={e.entry} className="chip" onClick={() => onShow(e.entry)}>
              #{e.entry} {entryTitle(e)}
            </button>
          ))}
        </div>
      ) : (
        <p className="muted">No change set in the journal changed it.</p>
      )}
    </div>
  );
}
