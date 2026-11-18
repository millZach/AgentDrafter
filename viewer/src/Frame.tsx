// The drafting app's frame round the drawing, in the look Zach picked
// (#150): the ribbon, with its drawing tools shown before they work; the
// command bar under the drawing, which prompts; and the status bar, with
// the cursor's place, the paper, Grid, Object snap, Ortho and the
// connection. Plain components: what they do is the page's (App.tsx), and
// what they say is worked out in frame.ts.

import { useState, type ReactNode, type RefObject } from 'react';

import {
  PANELS,
  SNAPS,
  SOON,
  TOGGLES,
  type Act,
  type LiveState,
  type Prompt,
  type SnapEntry,
  type SnapKind,
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
  /** A tool still to come was tapped. */
  onSoon(tool: Tool): void;
  /** The selected object's layer, and the colour it shows in; null with nothing selected. */
  layer: { name: string; color: string } | null;
  /** The tool just tapped, which flashes. */
  flashing: ToolKey | null;
  /** Opens the Object snap menu, from a phone's Status panel. */
  onSnaps(): void;
}

export function Ribbon({ folded, toggles, onToggle, onSoon, layer, flashing, onSnaps }: RibbonProps) {
  const [tip, setTip] = useState<{ tool: Tool; left: number; top: number } | null>(null);
  return (
    <nav className="ribbon" aria-label="Tools">
      {PANELS.map((panel) => (
        <section key={panel.title} className="panel">
          <div className="row">
            {panel.tools.map((tool) => (
              <button
                key={tool.key}
                type="button"
                className={`rb${flashing === tool.key ? ' flash' : ''}`}
                data-tool={tool.key}
                aria-disabled="true"
                aria-label={`${tool.label}, coming soon`}
                onPointerEnter={(e) => {
                  if (e.pointerType !== 'mouse') return;
                  const r = e.currentTarget.getBoundingClientRect();
                  setTip({ tool, left: Math.min(r.left, window.innerWidth - 270), top: r.bottom + 8 });
                }}
                onPointerLeave={() => setTip(null)}
                onClick={() => {
                  setTip(null);
                  onSoon(tool);
                }}
              >
                <svg viewBox="0 0 28 28" aria-hidden="true">
                  {ICONS[tool.key]}
                </svg>
                <span>{tool.label}</span>
              </button>
            ))}
          </div>
          <h3>
            {panel.title} <span className="soon">Soon</span>
          </h3>
        </section>
      ))}
      <section className="panel">
        <div className="row">
          <div className="layerbox">
            <div className="lay" title="Choosing the layer new objects go on comes with the drawing tools">
              <span className="sw" style={layer?.color ? { background: layer.color } : undefined} />
              <span className={`ln${layer ? '' : ' muted'}`} data-layer>
                {layer ? layer.name : 'No object selected'}
              </span>
              <span className="caret">&#9662;</span>
            </div>
            <small>{layer ? 'The selected object’s layer' : 'Select an object to see its layer'}</small>
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
          <span className="soonline">{SOON}</span>
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
}

/** The command bar under the drawing: it prompts, with options to click; typing comes with the drawing tools. */
export function CommandBar({ prompt, history, onAct, bar }: CommandBarProps) {
  return (
    <div className="cmd" data-ui>
      <div className="hist" aria-hidden="true">
        {history.map((line, i) => (
          <div key={`${i}${line}`}>{line}</div>
        ))}
      </div>
      <div className="cmdbar" ref={bar} role="status" aria-label="Command bar">
        <span className="chev">&gt;_</span>
        <span className="cmdline">
          {prompt.command && <span className="cmdname">{prompt.command}</span>}
          {prompt.parts.map((part, i) =>
            'text' in part ? (
              <span key={i}>{part.text}</span>
            ) : 'snap' in part ? (
              <span key={i} className="snapw">
                {part.snap}
              </span>
            ) : (
              <button key={i} type="button" className="opt" data-opt={part.act} onClick={() => onAct(part.act)}>
                {part.option}
              </button>
            ),
          )}
        </span>
        <span className="cmdnote">F3: snaps on/off · Tab: the next snap here</span>
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
 * Clear all. Perpendicular and Tangent act only inside a drawing tool,
 * from its last point, from build 4.
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
        <li className="grp">Inside a drawing tool, from the last point</li>
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
