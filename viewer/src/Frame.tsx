// The drafting app's frame round the drawing, in the look Zach picked
// (#150), with the drawing tools of #152: the ribbon, with the tools and
// the Layer box that sets the current layer; the command bar under the
// drawing, which prompts and takes what is typed; the status bar, with the
// cursor's place, the paper, Grid, Object snap, Ortho and the connection;
// the dialog an edit to a template's object asks first in; and a phone's
// small box for text. Plain components: what they do is the page's (App.tsx), and
// what they say is worked out in frame.ts.

import { useState, type ReactNode, type RefObject } from 'react';

import {
  PANELS,
  SNAPS,
  TOGGLES,
  templateWords,
  type Act,
  type LayerEntry,
  type LiveState,
  type Prompt,
  type SnapEntry,
  type SnapKind,
  type TemplateEdit,
  type Tool,
  type Toggle,
  type ToolKey,
} from './frame.ts';

/** Each tool's icon, drawn on a 28-unit square. */
const ICONS: Record<ToolKey, ReactNode> = {
  line: (
    <>
      <path d="M6 22L22 6" />
      <rect className="pt" x="3.5" y="19.5" width="5" height="5" />
      <rect className="pt" x="19.5" y="3.5" width="5" height="5" />
    </>
  ),
  circle: (
    <>
      <circle cx="14" cy="14" r="10" />
      <path d="M12 14h4M14 12v4" />
      <rect className="pt" x="21.5" y="11.5" width="5" height="5" />
    </>
  ),
  rectangle: (
    <>
      <rect x="4" y="7" width="20" height="14" />
      <rect className="pt" x="1.5" y="4.5" width="5" height="5" />
      <rect className="pt" x="21.5" y="18.5" width="5" height="5" />
    </>
  ),
  text: <path d="M6 23L14 4l8 19M9 16h10" />,
  move: (
    <path d="M14 3v22M3 14h22M14 3l-3.5 3.5M14 3l3.5 3.5M14 25l-3.5-3.5M14 25l3.5-3.5M3 14l3.5-3.5M3 14l3.5 3.5M25 14l-3.5-3.5M25 14l-3.5 3.5" />
  ),
  erase: <path d="M16 4l8 8-11 11H8l-4-4zM10 10l8 8M4 25h20" />,
};

export interface RibbonProps {
  /** A phone's: the ribbon folded behind Tools, which also holds the status bar's toggles. */
  folded: boolean;
  toggles: Record<Toggle, boolean>;
  onToggle(toggle: Toggle): void;
  /** A drawing tool was clicked or tapped: it starts, or ends when it runs. */
  onTool(tool: Tool): void;
  /** The tool running, shown pressed. */
  running: ToolKey | null;
  /** The sheet's layers, for the Layer box, and the current layer, which new objects go on. */
  layers: readonly LayerEntry[];
  current: string;
  onLayer(name: string): void;
  /** The box is opened: its layers are asked for again. */
  onLayersOpen?(): void;
  /** A layer's colour on screen, `#rrggbb`, for its swatch. */
  swatch(layer: LayerEntry | undefined): string;
  /** The selected object's layer, which the box shows while no tool runs; null with nothing selected. */
  selected: { name: string; color: string } | null;
  /** Opens the Object snap menu, from a phone's Status panel. */
  onSnaps(): void;
}

export function Ribbon({ folded, toggles, onToggle, onTool, running, layers, current, onLayer, onLayersOpen, swatch, selected, onSnaps }: RibbonProps) {
  const [tip, setTip] = useState<{ tool: Tool; left: number; top: number } | null>(null);
  // While an object is selected and no tool runs, the box shows its layer,
  // as the frame did; otherwise the current layer, which it sets.
  const showing = selected && !running ? selected : null;
  const layer = layers.find((l) => l.name === current);
  return (
    <nav className="ribbon" aria-label="Tools">
      {PANELS.map((panel) => (
        <section key={panel.title} className="panel">
          <div className="row">
            {panel.tools.map((tool) => (
              <button
                key={tool.key}
                type="button"
                className={`rb${running === tool.key ? ' on' : ''}`}
                data-tool={tool.key}
                aria-pressed={running === tool.key}
                aria-label={`${tool.label} (${tool.alias})`}
                onPointerEnter={(e) => {
                  if (e.pointerType !== 'mouse') return;
                  const r = e.currentTarget.getBoundingClientRect();
                  setTip({ tool, left: Math.min(r.left, window.innerWidth - 270), top: r.bottom + 8 });
                }}
                onPointerLeave={() => setTip(null)}
                onClick={() => {
                  setTip(null);
                  onTool(tool);
                }}
              >
                <svg viewBox="0 0 28 28" aria-hidden="true">
                  {ICONS[tool.key]}
                </svg>
                <span>{tool.label}</span>
              </button>
            ))}
          </div>
          <h3>{panel.title}</h3>
        </section>
      ))}
      <section className="panel">
        <div className="row">
          <div className="layerbox">
            <label className="lay" title="The current layer: new objects go on it">
              <span className="sw" style={{ background: showing ? showing.color || 'transparent' : swatch(layer) }} />
              {showing ? (
                <span className="ln" data-layer>
                  {showing.name}
                </span>
              ) : (
                <select
                  className="ln"
                  data-layer
                  aria-label="The current layer"
                  value={current}
                  onChange={(e) => onLayer(e.currentTarget.value)}
                  onFocus={() => onLayersOpen?.()}
                >
                  {(layers.length ? layers : [{ name: current }]).map((l) => (
                    <option key={l.name} value={l.name}>
                      {l.name}
                      {l.locked ? ' (locked)' : ''}
                      {l.on === false || l.frozen ? ' (not shown)' : ''}
                    </option>
                  ))}
                </select>
              )}
            </label>
            <small>{showing ? 'The selected object’s layer' : 'Current layer: new objects go on it'}</small>
          </div>
        </div>
        <h3>Layer</h3>
      </section>
      {folded && (
        <section className="panel">
          <div className="row">
            {TOGGLES.map((t) => (
              <button
                key={t.key}
                type="button"
                className="tog2"
                data-toggle={t.key}
                aria-pressed={toggles[t.key]}
                onClick={() => onToggle(t.key)}
              >
                {t.label}
              </button>
            ))}
            <button type="button" className="tog2" data-snaps onClick={onSnaps}>
              Snaps…
            </button>
          </div>
          <h3>Status</h3>
        </section>
      )}
      {tip && (
        <div className="tip" role="tooltip" style={{ left: tip.left, top: tip.top }}>
          <b>
            {tip.tool.label}
            <kbd>{tip.tool.alias}</kbd>
          </b>
          {tip.tool.tip}
        </div>
      )}
    </nav>
  );
}

export interface CommandBarProps {
  prompt: Prompt;
  /** The last lines of what happened, over the drawing, on a laptop. */
  history: string[];
  onAct(act: Act): void;
  bar: RefObject<HTMLDivElement | null>;
  /** The field typing goes into: aliases, points, lengths, options' letters and a text's words. */
  field: RefObject<HTMLInputElement | null>;
  /** What the field says when it is empty. */
  placeholder: string;
  /** A phone's: the field shows only when asked for, by ⌨ Type, or when the text itself is typed. */
  touch: boolean;
  fieldShown: boolean;
  /** Whether a phone offers ⌨ Type: not beside a selected object's options, which keep the bar one line. */
  typeOffered?: boolean;
  onShowField(): void;
  /** Enter, or Space where Space is Enter, with what the field holds. */
  onEnter(text: string): void;
  /** Esc in the field. */
  onEscape(): void;
  /** What the field holds, as it is typed: a text's words show on the drawing. */
  onTyping(text: string): void;
  /** Whether Space is a space: the text itself is being typed. */
  typingText: boolean;
  /** That the objects picked are part of the template, said while the tool runs. */
  warning: TemplateEdit | null;
  /** What the keys do now. */
  note: string;
}

/** The command bar under the drawing: it prompts, with options to click, and takes what is typed. */
export function CommandBar({
  prompt,
  history,
  onAct,
  bar,
  field,
  placeholder,
  touch,
  fieldShown,
  typeOffered = true,
  onShowField,
  onEnter,
  onEscape,
  onTyping,
  typingText,
  warning,
  note,
}: CommandBarProps) {
  return (
    <div className="cmd" data-ui>
      <div className="hist" aria-hidden="true">
        {history.map((line, i) => (
          <div key={`${i}${line}`}>{line}</div>
        ))}
      </div>
      {warning && (
        <div className="cmdwarn" role="alert" data-warning>
          <span aria-hidden="true">&#9888;</span>
          <span>
            <b>{warning.title}</b> {warning.detail}
          </span>
        </div>
      )}
      <div className="cmdbar" ref={bar} aria-label="Command bar">
        <span className="chev">&gt;_</span>
        <span className="cmdline" role="status">
          {prompt.command && <span className="cmdname">{prompt.command}</span>}
          {prompt.parts.map((part, i) =>
            'text' in part ? (
              // The brackets and dots between options, which a phone, whose options are buttons, leaves out.
              <span key={i} className={/^\s*[[\]·]?\s*$/.test(part.text) ? 'brk' : undefined}>
                {part.text}
              </span>
            ) : 'snap' in part ? (
              <span key={i} className="snapw">
                {part.snap}
              </span>
            ) : 'layer' in part ? (
              <span key={i} className="laytag">
                {part.layer}{' '}
              </span>
            ) : 'error' in part ? (
              <span key={i} className="cmderr" data-error>
                {part.error}
              </span>
            ) : (
              <button
                key={i}
                type="button"
                className={`opt${part.main ? ' main' : ''}`}
                data-opt={part.act}
                onClick={() => onAct(part.act)}
              >
                {part.option}
              </button>
            ),
          )}
          {touch && !fieldShown && typeOffered && (
            <button type="button" className="kbd" data-type onClick={onShowField} aria-label="Type in the command bar">
              &#9000; Type
            </button>
          )}
        </span>
        <input
          ref={field}
          className="cmdin"
          hidden={touch && !fieldShown}
          autoComplete="off"
          autoCapitalize="off"
          spellCheck={false}
          enterKeyHint="go"
          aria-label="Type a command, a point or a length"
          placeholder={placeholder}
          onChange={(e) => onTyping(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || (e.key === ' ' && !typingText)) {
              e.preventDefault();
              e.stopPropagation();
              const text = e.currentTarget.value;
              e.currentTarget.value = '';
              onEnter(text);
            } else if (e.key === 'Escape') {
              e.preventDefault();
              e.stopPropagation();
              if (e.currentTarget.value && !typingText) e.currentTarget.value = '';
              else onEscape();
            } else if (e.key !== 'F3' && e.key !== 'F7' && e.key !== 'F8') {
              // The page's own keys stay out of the field's typing.
              e.stopPropagation();
            }
          }}
        />
        <span className="cmdnote">{note}</span>
      </div>
    </div>
  );
}

export interface TemplateDialogProps {
  edit: TemplateEdit;
  /** The sheet shown, marked among them. */
  sheet: string;
  onCancel(): void;
  onConfirm(): void;
}

/** An edit to a template's object asks first, naming every sheet it changes (Zach, 2026-11-19). */
export function TemplateDialog({ edit, sheet, onCancel, onConfirm }: TemplateDialogProps) {
  const words = templateWords(edit);
  return (
    <div className="va-modal" role="dialog" aria-modal="true" aria-label="Template warning" data-template-dialog>
      <div className="dialog tw">
        <h3>{words.title}</h3>
        <p>{words.body}</p>
        <div className="sheets">
          {edit.sheets.map((s) => (
            <span key={s} className={s.toUpperCase() === sheet.toUpperCase() ? 'here' : ''}>
              {s}
            </span>
          ))}
        </div>
        <div className="dialog-acts">
          <button type="button" className="btn" data-tw="cancel" onClick={onCancel}>
            Cancel
          </button>
          <button
            type="button"
            className="btn warn"
            data-tw="all"
            ref={(el) => el?.focus()}
            onClick={onConfirm}
          >
            {words.button}
          </button>
        </div>
      </div>
    </div>
  );
}

export interface TextBoxProps {
  /** The height new text is drawn at, in the sheet's unit. */
  height: number;
  /** The sheet's unit, such as `in`. */
  unit: string;
  onAdd(text: string, height: number): void;
  onCancel(): void;
  onTyping(text: string): void;
}

/** A phone's small box for a Text tool's words and height (Zach, 2026-11-19). */
export function TextBox({ height, unit, onAdd, onCancel, onTyping }: TextBoxProps) {
  const [text, setText] = useState('');
  const [h, setH] = useState(String(height));
  const add = () => {
    if (!text.trim()) return;
    onAdd(text, Number(h) > 0 ? Number(h) : height);
  };
  return (
    <div className="tbox" data-ui aria-label="Text" role="group">
      <label>
        Text
        <input
          data-tbox-text
          autoComplete="off"
          placeholder="Type the text"
          enterKeyHint="done"
          value={text}
          ref={(el) => {
            if (el && document.activeElement !== el && !text) el.focus({ preventScroll: true });
          }}
          onChange={(e) => {
            setText(e.currentTarget.value);
            onTyping(e.currentTarget.value);
          }}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === 'Enter') add();
            if (e.key === 'Escape') onCancel();
          }}
        />
      </label>
      <div className="two">
        <label>
          Height{unit ? `, ${unit}` : ''}
          <input data-tbox-height inputMode="decimal" value={h} onChange={(e) => setH(e.currentTarget.value)} onKeyDown={(e) => e.stopPropagation()} />
        </label>
        <div className="acts">
          <button type="button" className="btn" onClick={onCancel}>
            Cancel
          </button>
          <button type="button" className="btn primary" data-tbox-add disabled={!text.trim()} onClick={add}>
            Add
          </button>
        </div>
      </div>
    </div>
  );
}

export interface StatusBarProps {
  /** A sheet drawn in several layouts: a tab for each, at the left, as AutoCAD's. */
  layouts: ReactNode;
  coords: RefObject<HTMLSpanElement | null>;
  paper: string;
  toggles: Record<Toggle, boolean>;
  onToggle(toggle: Toggle): void;
  live: LiveState;
  line: string;
  /** Opens the Object snap menu above `anchor`, as its ▴ or a right-click on it does. */
  onSnapMenu(anchor: HTMLElement): void;
  snapMenuOpen: boolean;
}

export function StatusBar({ layouts, coords, paper, toggles, onToggle, live, line, onSnapMenu, snapMenuOpen }: StatusBarProps) {
  return (
    <footer className="status">
      {layouts}
      <span className="coords" ref={coords}>
        &nbsp;
      </span>
      <span className="paperlabel">{paper}</span>
      {TOGGLES.map((t) => {
        const toggle = (
          <button
            key={t.key}
            type="button"
            className="tog"
            data-toggle={t.key}
            aria-pressed={toggles[t.key]}
            title={t.key === 'osnap' ? 'Object snap: the cursor catches ends, middles, centres and crossings. Right-click or ▴ to choose which.' : t.title}
            onClick={() => onToggle(t.key)}
            onContextMenu={
              t.key === 'osnap'
                ? (e) => {
                    e.preventDefault();
                    onSnapMenu(e.currentTarget);
                  }
                : undefined
            }
          >
            {t.label}
            <kbd>{t.fkey}</kbd>
          </button>
        );
        if (t.key !== 'osnap') return toggle;
        return (
          <span key={t.key} className="osgroup">
            {toggle}
            <button
              type="button"
              className="osmenubtn"
              aria-haspopup="menu"
              aria-expanded={snapMenuOpen}
              aria-label="Choose the object snaps"
              onClick={(e) => onSnapMenu(e.currentTarget)}
            >
              &#9650;
            </button>
          </span>
        );
      })}
      <span className="grow" />
      <span className={`live ${live}`}>
        <i />
        <span>{line}</span>
      </span>
    </footer>
  );
}

/** A snap's AutoSnap marker, as the menu shows it beside its name. */
export function SnapMarker({ entry, size = 22 }: { entry: SnapEntry; size?: number }) {
  return (
    <svg viewBox="-11 -11 22 22" width={size} height={size} aria-hidden="true">
      <path d={entry.path(7)} />
    </svg>
  );
}

export interface SnapMenuProps {
  on: boolean;
  running: ReadonlySet<SnapKind>;
  /** Where it opens: above the status bar's Object snap on a laptop, a card on a phone. */
  place: { left: number; bottom: number } | null;
  onToggle(): void;
  onSnap(kind: SnapKind): void;
  onDefaults(): void;
  onClear(): void;
}

/**
 * The Object snap menu: snapping on and off (F3), each snap with its
 * marker and a line on what it means, ticked when it runs, Defaults and
 * Clear all. Perpendicular and Tangent will act only inside a drawing
 * tool, from its last point: not built yet.
 */
export function SnapMenu({ on, running, place, onToggle, onSnap, onDefaults, onClear }: SnapMenuProps) {
  const row = (x: SnapEntry) => (
    <li key={x.k}>
      <button
        type="button"
        className="osrow"
        role="menuitemcheckbox"
        aria-checked={running.has(x.k)}
        data-snap={x.k}
        onClick={() => onSnap(x.k)}
      >
        <span className="ck">{running.has(x.k) ? '✓' : ''}</span>
        <SnapMarker entry={x} />
        <span>
          {x.label}
          <small>{x.hint}</small>
        </span>
      </button>
    </li>
  );
  return (
    <div
      className={`osmenu${on ? '' : ' off'}${place ? '' : ' card'}`}
      role="menu"
      aria-label="Object snaps"
      data-ui
      style={place ? { left: place.left, bottom: place.bottom } : undefined}
    >
      <header>
        <b>Object snaps</b>
        <button type="button" className="sw2" role="switch" aria-checked={on} onClick={onToggle}>
          <i />
          {on ? 'On · F3' : 'Off · F3'}
        </button>
      </header>
      <ul>
        {SNAPS.filter((x) => !x.tool).map(row)}
        <li className="grp">Inside a drawing tool, from the last point: coming</li>
        {SNAPS.filter((x) => x.tool).map(row)}
      </ul>
      <footer>
        <small>The cursor catches the ticked ones.</small>
        <button type="button" className="btn sm" data-defaults onClick={onDefaults}>
          Defaults
        </button>
        <button type="button" className="btn sm" data-clear onClick={onClear}>
          Clear all
        </button>
      </footer>
    </div>
  );
}
