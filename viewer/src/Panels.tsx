// What the side panel and the cards hold: the peek, which a phone always
// shows, with the stream's state, the newest entry, Undo last and Accept;
// the journal, which steps through the set's entries; quick properties,
// which show the object the drafter selected and edit its layer, a
// text's contents, an insert's attributes and a circle's radius; and the
// assistant's place.
// Plain components: what they do is the page's (App.tsx).

import { useEffect, useRef, useState, type ChangeEvent, type KeyboardEvent } from 'react';

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
import { nameOf, typeWords, type Committed, type LiveState, type PropField } from './frame.ts';
import type { SheetObject } from './probe.ts';

export interface PeekProps {
  /** What the page is watching, in a few words. */
  line: string;
  /** What the line's dot shows: the stream open, reaching the server, the set as it was, or nothing to watch. */
  live: LiveState;
  journal: JournalReport | null;
  failing: ReadonlySet<string>;
  /** The change set Undo last would undo. */
  undoable: number | null;
  /** Whether Accept can do nothing now, for a reason the server will give. */
  blocked: boolean;
  busy: string | null;
  /** Whether the reviewer is looking at the set as it was: Undo and Accept are off until they are back to live. */
  stepping: boolean;
  onShow(entry: number): void;
  onUndo(): void;
  onAccept(): void;
}

export function Peek({ line, live, journal, failing, undoable, blocked, busy, stepping, onShow, onUndo, onAccept }: PeekProps) {
  const last = journal?.entries.at(-1);
  const state = last && journal ? entryState(last, journal, failing) : null;
  return (
    <>
      <div className={`live ${live}`}>
        <i />
        <span>{line}</span>
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
        <button
          type="button"
          className="btn"
          data-act="undo"
          disabled={!undoable || busy !== null || stepping}
          onClick={onUndo}
        >
          &#x21B6; Undo last{undoable ? ` (#${undoable})` : ''}
        </button>
        <button
          type="button"
          className={`btn primary${blocked ? ' blocked' : ''}`}
          data-act="accept"
          disabled={!journal || busy !== null || stepping}
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
                        disabled={busy !== null || at !== null}
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

export interface QuickPropertiesProps {
  object: SheetObject;
  /** The colour it shows in on screen, `#rrggbb`, for its swatch; empty when it draws nothing. */
  color: string;
  journal: JournalReport | null;
  /** The failed checks the object fails, in words. */
  fails: string[];
  /** Where it comes from, the template or the sheet's row, in words; null when the core did not say. */
  origin?: string | null;
  /** The rows an edit can change, as the renderer gives them; empty while it has not. */
  fields?: PropField[];
  /** The sheet's layers, by name, which the Layer field picks among. */
  layers?: string[];
  /** Lands an edit of a field: one change set, or asked about first. */
  onCommit?(key: string, value: string): Committed;
  /** Moves on when an edit asked about was cancelled, so every field shows its value again. */
  resetKey?: number;
  onClose(): void;
  onShow(entry: number): void;
}

/**
 * Quick properties: what the selected object is, beside it on a laptop and
 * as a card above the command bar on a phone. Its layer, a text's
 * contents, an insert's attribute values and a circle's radius or
 * diameter are edited in place (#153): an edit lands on Enter or on
 * leaving the field, and Esc puts the value back and lands nothing.
 */
export function QuickProperties({
  object,
  color,
  journal,
  fails,
  origin = null,
  fields = [],
  layers = [],
  onCommit,
  resetKey = 0,
  onClose,
  onShow,
}: QuickPropertiesProps) {
  const edited = fields.filter((f) => !f.attribute);
  const before: [string, string][] = [['Type', typeWords(object)]];
  // Until the renderer gives the fields, the layer shows as it did.
  if (!edited.length) before.push(['Layer', object.layer]);
  const after: [string, string][] = [
    ['ID', object.id ?? '(none)'],
    ['Name', object.name ?? '(none)'],
    ...(object.block ? [['Block', object.block] as [string, string]] : []),
    ...(origin ? [['Comes from', origin] as [string, string]] : []),
  ];
  const attributeFields = fields.filter((f) => f.attribute);
  const attributes = object.attributes ?? [];
  const history = journal && object.id ? changedBy(journal, object.id) : [];
  const commit = (key: string, value: string): Committed => onCommit?.(key, value) ?? { kind: 'same' };
  const plain = (rows: [string, string][]) =>
    rows.map(([k, v]) => (
      <tr key={k}>
        <th>{k}</th>
        <td>{v}</td>
      </tr>
    ));
  const row = (f: PropField) => <PropRow key={f.key} field={f} layers={layers} onCommit={commit} resetKey={resetKey} />;
  return (
    <>
      <header>
        <span className="sw" style={color ? { background: color } : undefined} />
        <b>{nameOf(object)}</b>
        <button type="button" aria-label="Clear the selection" data-close onClick={onClose}>
          &times;
        </button>
      </header>
      <div className="qp-body">
        {fails.map((f) => (
          <p key={f} className="err">
            &#x2715; {f}
          </p>
        ))}
        <table className="kv">
          <tbody>
            {plain(before)}
            {edited.map(row)}
            {plain(after)}
          </tbody>
        </table>
        {(attributeFields.length > 0 || attributes.length > 0) && (
          <>
            <h5>Attributes</h5>
            <table className="kv attrs">
              <tbody>
                {attributeFields.length
                  ? attributeFields.map(row)
                  : attributes.map((a, i) => (
                      // A block may define a tag more than once, so the place keys it.
                      <tr key={i}>
                        <th>{a.tag}</th>
                        <td>{a.value === '' ? '(empty)' : a.value}</td>
                      </tr>
                    ))}
              </tbody>
            </table>
          </>
        )}
        <h5>Changed by</h5>
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
    </>
  );
}

interface PropRowProps {
  field: PropField;
  layers: string[];
  onCommit(key: string, value: string): Committed;
  resetKey: number;
}

/**
 * One row of quick properties an edit can change: its label, with a chip
 * that says where its value is written, and its field. The note that says
 * what an edit changes shows while the field is being edited.
 */
function PropRow({ field, layers, onCommit, resetKey }: PropRowProps) {
  // What is typed, while it differs from the value; kept once sent, until
  // the sheet drawn again shows the value or the edit is cancelled.
  const [draft, setDraft] = useState<string | null>(null);
  const [focused, setFocused] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Enter or Esc has dealt with the field, so leaving it does nothing more.
  const settled = useRef(false);
  useEffect(() => {
    setDraft(null);
    setError(null);
  }, [field.value, resetKey]);
  const shown = draft ?? field.value;
  const label = (
    <th>
      {field.label}
      {field.chip && (
        <small className={`qchip ${field.chip.tone}`} data-chip={field.chip.tone}>
          {field.chip.words}
        </small>
      )}
    </th>
  );
  if (!field.editable) {
    return (
      <tr data-prop-row={field.key}>
        {label}
        <td>{field.value === '' && field.attribute ? '(empty)' : field.value}</td>
      </tr>
    );
  }
  /** Lands what is typed: false when it says what went wrong instead. */
  const land = (value: string): boolean => {
    const done = onCommit(field.key, value);
    if (done.kind === 'error') {
      setError(done.message);
      return false;
    }
    setError(null);
    if (done.kind === 'same') setDraft(null);
    return true;
  };
  const focus = () => {
    settled.current = false;
    setFocused(true);
  };
  let input;
  if (field.key === 'layer') {
    const names = layers.includes(field.value) ? layers : [field.value, ...layers];
    input = (
      <select
        className="qin"
        data-prop={field.key}
        aria-label={field.label}
        value={shown}
        onFocus={focus}
        onBlur={() => setFocused(false)}
        onKeyDown={(e) => e.stopPropagation()}
        onChange={(e) => {
          const name = e.currentTarget.value;
          setDraft(name);
          // A layer picked lands at once, as AutoCAD's does.
          if (!land(name)) setDraft(null);
        }}
      >
        {names.map((n) => (
          <option key={n} value={n}>
            {n}
          </option>
        ))}
      </select>
    );
  } else {
    const keys = (e: KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      // The field's keys are its own, not the command bar's.
      e.stopPropagation();
      if (e.key === 'Escape') {
        e.preventDefault();
        settled.current = true;
        setDraft(null);
        setError(null);
        e.currentTarget.blur();
      } else if (e.key === 'Enter' && !(field.multiline && e.shiftKey)) {
        e.preventDefault();
        settled.current = true;
        if (land(shown)) e.currentTarget.blur();
        else settled.current = false;
      }
    };
    const leave = () => {
      setFocused(false);
      if (settled.current) {
        settled.current = false;
        return;
      }
      // Leaving the field lands it, as Enter does; what went wrong is said,
      // and the value put back.
      if (draft !== null && draft !== field.value) {
        if (!land(draft)) setDraft(null);
      } else {
        setDraft(null);
      }
    };
    const props = {
      className: 'qin',
      'data-prop': field.key,
      'aria-label': field.label,
      value: shown,
      autoComplete: 'off',
      spellCheck: false,
      onFocus: focus,
      onChange: (e: ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => setDraft(e.currentTarget.value),
      onKeyDown: keys,
      onBlur: leave,
    };
    input = field.multiline ? (
      <textarea {...props} rows={Math.min(6, Math.max(2, shown.split('\n').length))} />
    ) : (
      <input {...props} inputMode={field.numeric ? 'decimal' : undefined} enterKeyHint="done" />
    );
  }
  return (
    <>
      <tr data-prop-row={field.key} className="qrow">
        {label}
        <td>{input}</td>
      </tr>
      {focused && field.note && (
        <tr className="qnote">
          <td colSpan={2} data-note={field.key}>
            {field.note}
          </td>
        </tr>
      )}
      {error && (
        <tr className="qnote err">
          <td colSpan={2} role="alert" data-prop-error={field.key}>
            {error}
          </td>
        </tr>
      )}
    </>
  );
}

/** The assistant's place in the side panel, which build 8 fills. */
export function Assistant() {
  return (
    <div className="assist">
      <h4>The assistant</h4>
      <p>Your own agent will draft with you here: ask it what something is, have it draw, and keep or discard what it proposes.</p>
      <div className="ghost">
        Not here yet. Agents already draft through change sets from the command line, MCP, Python or HTTP; their work
        lands in the Journal and on the drawing.
      </div>
    </div>
  );
}
