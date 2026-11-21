// The drafting app's frame round the drawing, in the look Zach picked
// (#150), with the drawing tools of #152: the ribbon, with the tools and
// the Layer box that sets the current layer; the command bar under the
// drawing, which prompts and takes what is typed; the status bar, with the
// cursor's place, the paper, Grid, Object snap, Ortho and the connection;
// the band a change set to a template's objects is shown in before it
// lands (#154); and a phone's small box for text. Plain components: what they do is the page's (App.tsx), and
// what they say is worked out in frame.ts.

import { forwardRef, useEffect, useRef, useState, type ChangeEvent, type ClipboardEvent, type KeyboardEvent, type ReactNode, type RefObject } from 'react';

import {
  PANELS,
  SNAPS,
  TOGGLES,
  fieldKey,
  isUndoKey,
  letGoOnOptionPress,
  pastedLines,
  checkAgainOffered,
  keepButton,
  previewButton,
  previewSummary,
  previewTitle,
  previewVerdicts,
  type Act,
  type PreviewReport,
  type PreviewState,
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
  copy: (
    <>
      <rect x="3.5" y="9.5" width="12" height="12" rx="1" />
      <rect x="12.5" y="4.5" width="12" height="12" rx="1" strokeDasharray="3 2" />
      <rect className="pt" x="1" y="19" width="5" height="5" />
    </>
  ),
  array: (
    <>
      <rect x="3.5" y="3.5" width="6" height="6" />
      <rect x="11" y="3.5" width="6" height="6" strokeDasharray="2 1.5" />
      <rect x="18.5" y="3.5" width="6" height="6" strokeDasharray="2 1.5" />
      <rect x="3.5" y="11" width="6" height="6" strokeDasharray="2 1.5" />
      <rect x="11" y="11" width="6" height="6" strokeDasharray="2 1.5" />
      <rect x="18.5" y="11" width="6" height="6" strokeDasharray="2 1.5" />
      <path d="M3 22h22M3 20v4M25 20v4" />
    </>
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
  /**
   * The selected object's layer, which the box shows while no tool runs,
   * and, when it can be edited, moves it to the layer picked (#153); null
   * with nothing selected.
   */
  selected: { name: string; color: string; movable?: boolean; label?: string } | null;
  /** A layer picked with an object selected: the object moves to it. */
  onMoveTo?(name: string): void;
  /** Opens the Object snap menu, from a phone's Status panel. */
  onSnaps(): void;
}

export function Ribbon({ folded, toggles, onToggle, onTool, running, layers, current, onLayer, onLayersOpen, swatch, selected, onMoveTo, onSnaps }: RibbonProps) {
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
                aria-label={`${tool.label} (${tool.alias}${tool.also ? ` or ${tool.also}` : ''})`}
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
              {showing && !showing.movable ? (
                <span className="ln" data-layer>
                  {showing.name}
                </span>
              ) : (
                <select
                  className="ln"
                  data-layer
                  aria-label={showing ? `The layer of ${showing.label ?? 'the selected object'}` : 'The current layer'}
                  value={showing ? showing.name : current}
                  onChange={(e) => (showing ? onMoveTo?.(e.currentTarget.value) : onLayer(e.currentTarget.value))}
                  onFocus={() => onLayersOpen?.()}
                >
                  {(() => {
                    const shown = showing ? showing.name : current;
                    return layers.some((l) => l.name === shown) ? layers : [{ name: shown } as LayerEntry, ...layers];
                  })().map((l) => (
                    <option key={l.name} value={l.name}>
                      {l.name}
                      {l.locked ? ' (locked)' : ''}
                      {l.on === false || l.frozen ? ' (not shown)' : ''}
                    </option>
                  ))}
                </select>
              )}
            </label>
            <small>
              {showing
                ? showing.movable
                  ? `Pick a layer to move ${showing.label ?? 'it'} onto it`
                  : 'The selected object’s layer'
                : 'Current layer: new objects go on it'}
            </small>
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
            {tip.tool.also && <kbd>{tip.tool.also}</kbd>}
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
  /** A change set shown before it lands: above the bar on a laptop, in its place on a phone. */
  band?: ReactNode;
  /** What the keys do now. */
  note: string;
  /**
   * What Enter does with the words typed, when they go to the assistant
   * (#156): `Enter: ask the assistant`; empty for nothing to say.
   */
  hint?: string;
  /** Whether Space is Enter for what the field holds; without it, always, but while a text's words are typed. */
  spaceEnters?(text: string): boolean;
  /** What the field holds, each time it changes, emptied by Enter or Esc among them: the hint follows it. */
  onTyped?(text: string): void;
  /** Ctrl+Z in the field with nothing typed, when it takes something back: Copy's last copy placed (#66). */
  onUndoKey?(): void;
  /** Array's panel, which a phone docks above the warning and the bar (#66); a laptop's sits beside the copies. */
  panel?: ReactNode;
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
  band = null,
  hint = '',
  spaceEnters,
  onTyped,
  onUndoKey,
  panel = null,
}: CommandBarProps) {
  return (
    <div className="cmd" data-ui>
      <div className="hist" aria-hidden="true">
        {history.map((line, i) => (
          <div key={`${i}${line}`}>{line}</div>
        ))}
      </div>
      {touch && panel}
      {warning && (
        <div className="cmdwarn" role="alert" data-warning>
          <span aria-hidden="true">&#9888;</span>
          <span>
            <b>{warning.title}</b> {warning.detail}
          </span>
        </div>
      )}
      {band && touch ? (
        <div className="pvwrap" ref={bar}>
          {band}
        </div>
      ) : (
        band
      )}
      <div className={`cmdbar${hint ? ' asking' : ''}`} ref={band && touch ? undefined : bar} aria-label="Command bar" hidden={!!band && touch}>
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
                // A press keeps the focus where it was, so Enter after
                // clicking Undo enters at the prompt, as AutoCAD's command
                // line takes it, rather than pressing Undo again (#66); a
                // field being edited, or this option reached by Tab, lets
                // it go first.
                onMouseDown={(e) => {
                  const a = document.activeElement as HTMLElement | null;
                  const was = a && a !== document.body ? { tag: a.tagName, option: !!a.closest?.('.opt'), commandField: a === field.current } : null;
                  if (letGoOnOptionPress(was)) a?.blur();
                  e.preventDefault();
                }}
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
          // While a preview is shown nothing is typed: the arrow keys step through its sheets.
          disabled={!!band}
          autoComplete="off"
          autoCapitalize="off"
          spellCheck={false}
          enterKeyHint="go"
          aria-label="Type a command, a point, a length, or words for the assistant"
          placeholder={placeholder}
          onChange={(e) => {
            onTyping(e.currentTarget.value);
            onTyped?.(e.currentTarget.value);
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || (e.key === ' ' && !typingText && (spaceEnters?.(e.currentTarget.value) ?? true))) {
              e.preventDefault();
              e.stopPropagation();
              const text = e.currentTarget.value;
              e.currentTarget.value = '';
              onTyped?.('');
              onEnter(text);
            } else if (onUndoKey && isUndoKey(e) && !e.currentTarget.value) {
              // Ctrl+Z with nothing typed takes back Copy's last copy placed.
              e.preventDefault();
              e.stopPropagation();
              onUndoKey();
            } else if (e.key === 'Escape') {
              e.preventDefault();
              e.stopPropagation();
              // Esc drops what is typed; with nothing typed, or a text's
              // words, it ends the tool too.
              const typed = e.currentTarget.value;
              e.currentTarget.value = '';
              onTyped?.('');
              if (!typed || typingText) onEscape();
            } else if (e.key !== 'F3' && e.key !== 'F7' && e.key !== 'F8') {
              // The page's own keys stay out of the field's typing.
              e.stopPropagation();
            }
          }}
        />
        {hint && (
          <span className="askhint" data-askhint>
            {hint}
          </span>
        )}
        <span className="cmdnote" hidden={!!hint}>
          {note}
        </span>
      </div>
    </div>
  );
}

export interface PreviewBandProps {
  /** The edit, as the renderer names it, with the template and every sheet it reaches. */
  edit: TemplateEdit;
  /** The sheet shown. */
  sheet: string;
  /** The core's preview, once the first sheet's reply has come; null before. */
  report: PreviewReport | null;
  /** That the set changed since the report was drawn: it is drawn again before Change all can send it. */
  refreshing?: boolean;
  onStep(by: number): void;
  onCancel(): void;
  onGo(): void;
  onCheckAgain(): void;
  /**
   * A proposal opened (#155), in the band's PROPOSAL form: its number, who
   * made it and when, what has landed since, and Discard. Its Keep is the
   * band's go, and × or Esc closes it, leaving it waiting.
   */
  proposal?: { number: number; proposed: string; stale: string | null; refused?: string | null; onDiscard(): void };
}

const TONE: Record<PreviewState, string> = { land: 'ok', check: 'warn', refused: 'bad', held: 'held' };

/** The core's words as a sentence, ending in a full stop. */
const sentence = (text: string) => (/[.!?]$/.test(text.trim()) ? text.trim() : `${text.trim()}.`);

/**
 * A change set drawn before it lands (#154), variant B of the prototype
 * Zach picked on 2026-11-19: a band above the command bar, the drawing
 * itself showing the sheet as it would be. It names the edit and its
 * reach, steps through the sheets it reaches, says what would become of
 * the sheet shown and of them all, and lands it on all of them or cancels.
 */
export function PreviewBand({ edit, sheet, report, refreshing = false, onStep, onCancel, onGo, onCheckAgain, proposal }: PreviewBandProps) {
  const t = previewTitle(edit, proposal?.number);
  const n = edit.sheets.length;
  const i = Math.max(0, edit.sheets.findIndex((s) => s.toUpperCase() === sheet.toUpperCase()));
  const verdict = report ? previewVerdicts(report).get(sheet.toUpperCase()) : undefined;
  const summary = report ? previewSummary(report) : null;
  const button = report
    ? { ...(proposal ? keepButton(report) : previewButton(report)) }
    : { label: proposal ? 'Keep' : n > 1 ? `Change all ${n} sheets` : 'Change the template', disabled: true };
  // A change landed since it was drawn: nothing is sent until it is drawn again.
  if (refreshing) button.disabled = true;
  // A proposal the core now refuses whole cannot be drawn: nothing can land.
  const refused = proposal?.refused ?? null;
  if (refused) {
    button.label = 'Nothing can land';
    button.disabled = true;
  }
  const fails = !!report && !report.change.failed && [...previewVerdicts(report).values()].some((v) => v.state === 'check');
  return (
    <div
      className={`pvband${proposal ? ' prop' : ''}`}
      role="region"
      aria-label={proposal ? `Proposal ${proposal.number}` : 'Preview'}
      data-preview
      data-proposal={proposal?.number}
      data-pv-refreshing={refreshing || undefined}
    >
      <div className="pvtop">
        <span className="pvhd">
          <span className="pvchip">{t.chip}</span>
          <span className="pvtitle">
            <b>{t.title}</b> {t.reach}
          </span>
        </span>
        <span className="pvnav">
          <button type="button" aria-label="Previous sheet" data-pv="prev" disabled={n < 2} onClick={() => onStep(-1)}>
            &#9664;
          </button>
          <span data-pv-sheet>
            <b>{edit.sheets[i] ?? sheet}</b>
            <small>
              {' '}
              {i + 1} of {n}
            </small>
          </span>
          <button type="button" aria-label="Next sheet" data-pv="next" disabled={n < 2} onClick={() => onStep(1)}>
            &#9654;
          </button>
        </span>
        <span className="pvacts">
          {proposal ? (
            <button type="button" className="btn" data-pv="discard" onClick={proposal.onDiscard}>
              Discard
            </button>
          ) : (
            <button type="button" className="btn" data-pv="cancel" onClick={onCancel}>
              Cancel
            </button>
          )}
          {report && !refreshing && checkAgainOffered(report) && (
            <button type="button" className="btn" data-pv="recheck" onClick={onCheckAgain}>
              Closed it: check again
            </button>
          )}
          <button
            type="button"
            className={`btn ${proposal && !fails ? 'primary' : 'warn'}`}
            data-pv={proposal ? 'keep' : 'go'}
            disabled={button.disabled}
            onClick={onGo}
          >
            {button.label}
          </button>
          {proposal && (
            <button type="button" className="pvx" data-pv="close" aria-label="Close it: it keeps waiting" title="Close it: it keeps waiting" onClick={onCancel}>
              &times;
            </button>
          )}
        </span>
      </div>
      {proposal && (
        <div className="pvby" data-pv-by>
          {proposal.proposed}
        </div>
      )}
      {proposal?.stale && (
        <div className="pvsum stale" data-pv-stale>
          <i className="vd stale" aria-hidden="true">
            ↻
          </i>{' '}
          {proposal.stale}
        </div>
      )}
      <div className={`pvline ${refused ? 'bad' : verdict && !refreshing ? TONE[verdict.state] : ''}`} data-pv-verdict>
        {refused ? (
          <>
            <i className="vd bad" aria-hidden="true">
              ✕
            </i>
            <span data-pv-refused>
              <b>Refused now.</b> {sentence(refused.replace(/`/g, ''))} It cannot be drawn on the set as it stands; Discard ends it.
            </span>
          </>
        ) : refreshing ? (
          <span className="muted">The set changed: drawing {sheet} as it would be again…</span>
        ) : verdict ? (
          <>
            <i className={`vd ${TONE[verdict.state]}`} aria-hidden="true">
              {verdict.icon}
            </i>
            <span>
              <b>{verdict.word}.</b> {verdict.why}
            </span>
          </>
        ) : (
          <span className="muted">Drawing {sheet} as it would be…</span>
        )}
      </div>
      {summary && !refreshing && !refused && (
        <div className={`pvsum ${summary.tone}`} data-pv-summary>
          {summary.text}
        </div>
      )}
      {report?.warning && !refreshing && (
        <div className="pvsum warn" data-pv-warning>
          {report.warning}
        </div>
      )}
    </div>
  );
}

/** A tab's mark in a preview: what would become of its sheet. */
export function VerdictMark({ state }: { state: PreviewState }) {
  const icon = { land: '✓', check: '!', refused: '✕', held: '–' }[state];
  return (
    <i className={`vd ${TONE[state]}`} aria-label={{ land: 'Would land', check: 'Would fail a check', refused: 'Refused', held: 'Held back' }[state]}>
      {icon}
    </i>
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

export interface TextEditProps {
  /** The text's contents as they are. */
  value: string;
  /** An mtext's, which take more than one line: Shift+Enter, or a phone's return, starts the next. */
  multiline: boolean;
  /** Where its contents are written, and what an edit changes, said before it lands. */
  note: string | null;
  /** Its words are done: they land, unless they are as they were. */
  onDone(text: string): void;
  /** Esc or Cancel: the contents are put back, and nothing lands. */
  onCancel(): void;
  /** Why the renderer would refuse these contents, if it would: line breaks pasted into one line, which the browser strips, are checked by it first. */
  refuse?(text: string): string | null;
}

/**
 * The key and paste rules both editors of a text's contents share: Enter
 * and Esc are an input method's while it composes; line breaks pasted
 * into one line are refused, said under the field, before the browser
 * strips them.
 */
function useTextKeys(multiline: boolean, refuse: ((text: string) => string | null) | undefined, setText: (text: string) => void) {
  const [refused, setRefused] = useState<string | null>(null);
  const does = (e: KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>) =>
    fieldKey({ key: e.key, shiftKey: e.shiftKey, isComposing: e.nativeEvent.isComposing, keyCode: e.keyCode }, multiline);
  const paste = (e: ClipboardEvent<HTMLInputElement | HTMLTextAreaElement>) => {
    const el = e.currentTarget;
    const end = el.value.length;
    const next = pastedLines(el.value, el.selectionStart ?? end, el.selectionEnd ?? end, e.clipboardData.getData('text'), multiline);
    if (next === null) return;
    e.preventDefault();
    const why = refuse?.(next) ?? null;
    setRefused(why);
    if (!why) setText(next);
  };
  return { does, paste, refused, setRefused };
}

/**
 * A text's contents edited on the drawing, on a laptop, as AutoCAD's
 * TEXTEDIT: a box over the text, placed by the drawing as it pans and
 * zooms. Enter or a click away lands it; Esc puts it back.
 */
export const TextOnDrawing = forwardRef<HTMLDivElement, TextEditProps>(function TextOnDrawing({ value, multiline, note, onDone, onCancel, refuse }, box) {
  const [text, setText] = useState(value);
  const { does, paste, refused, setRefused } = useTextKeys(multiline, refuse, setText);
  // Enter or Esc has dealt with it: the blur that follows does nothing more.
  const settled = useRef(false);
  const field = useRef<HTMLInputElement & HTMLTextAreaElement>(null);
  // It takes the keys as it opens, its words selected, as AutoCAD's does.
  useEffect(() => {
    field.current?.focus({ preventScroll: true });
    field.current?.select();
  }, []);
  const keys = (e: KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>) => {
    e.stopPropagation();
    const key = does(e);
    if (key === 'cancel') {
      e.preventDefault();
      settled.current = true;
      onCancel();
    } else if (key === 'land') {
      e.preventDefault();
      settled.current = true;
      onDone(text);
    }
  };
  const props = {
    'data-text-edit': true,
    value: text,
    autoComplete: 'off',
    spellCheck: false,
    ref: field,
    onChange: (e: ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      setRefused(null);
      setText(e.currentTarget.value);
    },
    onKeyDown: keys,
    onPaste: paste,
    onBlur: () => {
      if (!settled.current) onDone(text);
    },
  };
  return (
    <div className={`tedit${multiline ? ' multi' : ''}`} ref={box} data-ui role="group" aria-label="Edit the text">
      {multiline ? <textarea {...props} rows={Math.max(2, text.split('\n').length)} /> : <input {...props} />}
      {note && <p className="tnote">{note}</p>}
      {refused && (
        <p className="terr" role="alert" data-text-refused>
          {refused}
        </p>
      )}
      <small>{multiline ? 'Enter or click away: done · Shift+Enter: new line · Esc: cancel' : 'Enter or click away: done · Esc: cancel'}</small>
    </div>
  );
});

/** A phone's small box for a text's contents, as for a Text tool's words (build 4): Done lands them, Cancel puts them back. */
export function TextEditBox({ value, multiline, note, onDone, onCancel, refuse }: TextEditProps) {
  const [text, setText] = useState(value);
  const { does, paste, refused, setRefused } = useTextKeys(multiline, refuse, setText);
  const keys = (e: KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>) => {
    e.stopPropagation();
    const key = does(e);
    if (key === 'cancel') onCancel();
    // A phone's return is a new line in an mtext: Done lands it.
    else if (key === 'land' && !multiline) onDone(text);
  };
  const props = {
    'data-tbox-text': true,
    autoComplete: 'off',
    spellCheck: false,
    value: text,
    onChange: (e: ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      setRefused(null);
      setText(e.currentTarget.value);
    },
    onKeyDown: keys,
    onPaste: paste,
  };
  return (
    <div className="tbox" data-ui aria-label="Edit the text" role="group" data-text-box>
      <label>
        Contents
        {multiline ? <textarea {...props} rows={3} /> : <input {...props} enterKeyHint="done" />}
      </label>
      {note && <p className="tnote">{note}</p>}
      {refused && (
        <p className="terr" role="alert" data-text-refused>
          {refused}
        </p>
      )}
      <div className="two">
        <span />
        <div className="acts">
          <button type="button" className="btn" data-tbox-cancel onClick={onCancel}>
            Cancel
          </button>
          <button type="button" className="btn primary" data-tbox-done disabled={!text.trim()} onClick={() => onDone(text)}>
            Done
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
