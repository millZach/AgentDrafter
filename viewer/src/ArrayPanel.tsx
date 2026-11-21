// Array's panel (#66), as the prototype Zach picked shows it: the kind,
// how the spacing is given, the fields, a polar array's centre with Pick
// and whether its items turn, how many copies it makes, Cancel and Array.
// Beside the objects on a laptop, docked above the command bar on a phone.
// Every value is the renderer's: the page passes what is typed and shows
// what it says, so a total divided, a whole turn kept whole and what is
// sent are worked out in Rust (ADR-0007).

import { useEffect, useState, type KeyboardEvent, type MouseEvent, type RefObject } from 'react';

import { arrayInputMode, type ArrayField, type ArrayPanelState } from './frame.ts';

export interface ArrayPanelProps {
  panel: ArrayPanelState;
  /** A phone's: docked above the command bar, its words a tap's. */
  touch: boolean;
  /** A button pressed, by the renderer's act: `rectangular`, `total`, `pick-center`, `turn`, `array`, `cancel`… */
  onAct(act: string): void;
  /** A field typed in, by its key, with what it holds. */
  onValue(key: string, text: string): void;
  /** The element, which a laptop places beside the copies at each draw. */
  boxRef?: RefObject<HTMLDivElement | null>;
}

// A press keeps the focus where it was, so Enter after a click lands the
// array from the command bar or a field, rather than pressing the button
// again, as the command bar's options do.
const keepFocus = (e: MouseEvent) => e.preventDefault();

/** One field: what is typed stays as typed while it has the focus; else it shows the renderer's value. */
function Field({ field, onValue, onEnter }: { field: ArrayField; onValue(key: string, text: string): void; onEnter(): void }) {
  const [text, setText] = useState(field.value);
  const [focused, setFocused] = useState(false);
  useEffect(() => {
    if (!focused) setText(field.value);
  }, [field.value, focused]);
  return (
    <label>
      {field.label}
      <input
        data-k={field.key}
        value={text}
        inputMode={arrayInputMode(field)}
        enterKeyHint="done"
        autoComplete="off"
        spellCheck={false}
        onFocus={() => setFocused(true)}
        onBlur={() => {
          setFocused(false);
          setText(field.value);
        }}
        onChange={(e) => {
          setText(e.target.value);
          onValue(field.key, e.target.value);
        }}
        onKeyDown={(e: KeyboardEvent<HTMLInputElement>) => {
          if (e.key !== 'Enter') return;
          // Enter in a field lands the array, as Enter at the prompt does.
          e.preventDefault();
          e.stopPropagation();
          e.currentTarget.blur();
          onEnter();
        }}
      />
      {field.hint && <small>{field.hint}</small>}
    </label>
  );
}

export function ArrayPanel({ panel, touch, onAct, onValue, boxRef }: ArrayPanelProps) {
  const land = () => onAct('array');
  return (
    <div className={`arr${touch ? ' docked' : ''}`} ref={boxRef} data-arraypanel role="group" aria-label="Array">
      <div className="ah">
        <div className="akinds" role="group" aria-label="Kind of array">
          {(['rectangular', 'polar'] as const).map((k) => (
            <button key={k} type="button" data-kind={k} aria-pressed={panel.kind === k} onMouseDown={keepFocus} onClick={() => onAct(k)}>
              {k === 'rectangular' ? 'Rectangular' : 'Polar'}
            </button>
          ))}
        </div>
        <span className="aof">{panel.of}</span>
      </div>
      <div className="ahold">
        <span>Spacing</span>
        <div className="aseg" role="group" aria-label="Spacing">
          {(['between', 'total'] as const).map((h) => (
            <button key={h} type="button" data-hold={h} aria-pressed={panel.spacing === h} onMouseDown={keepFocus} onClick={() => onAct(h)}>
              {h === 'between' ? 'Between items' : 'Total, divided evenly'}
            </button>
          ))}
        </div>
      </div>
      <div className="grid2">
        {panel.fields.map((f) => (
          <Field key={`${panel.kind}:${f.key}`} field={f} onValue={onValue} onEnter={land} />
        ))}
      </div>
      {panel.center !== null && (
        <div className="apick">
          <span className="al">Centre</span>
          <span className={`av${panel.picking ? ' live' : ''}`} data-acenter>
            {panel.center}
          </span>
          {!panel.picking && (
            <button type="button" className="btn sm" data-act="pick-center" onMouseDown={keepFocus} onClick={() => onAct('pick-center')}>
              Pick
            </button>
          )}
        </div>
      )}
      {panel.kind === 'polar' && (
        <label className="achk" onMouseDown={keepFocus}>
          <input
            type="checkbox"
            data-k="rotate"
            checked={panel.rotate}
            onChange={(e) => onAct(e.target.checked ? 'turn' : 'no-turn')}
            onKeyDown={(e) => {
              if (e.key !== 'Enter') return;
              e.preventDefault();
              e.stopPropagation();
              land();
            }}
          />
          Turn the items as they go round
        </label>
      )}
      <div className="foot">
        <span className={panel.ready ? '' : 'bad'} data-acount>
          {panel.count}
        </span>
        <button type="button" className="btn" data-act="cancel" onMouseDown={keepFocus} onClick={() => onAct('cancel')}>
          Cancel
        </button>
        <button type="button" className="btn primary" data-act="array" disabled={!panel.ready} onMouseDown={keepFocus} onClick={land}>
          Array
        </button>
      </div>
    </div>
  );
}
