// The drawing: a canvas the Rust renderer draws on with WebGL2, from the
// display list the server sent, as dark model space with its grid. This
// component only says where the sheet sits (the view) and when to draw;
// what is drawn, and how, is the renderer's (crates/agentdrafter-viewer),
// and so is what a tap picks: the core finds the object drawn nearest the
// point in the display list, so the page holds no hit-test of its own. It
// also keeps quick properties beside the picked object as the view moves,
// and says where the cursor is on the drawing. The renderer finds the
// object snap nearest the cursor among the running ones and draws the
// crosshair and the snap's marker; this component says where the cursor
// is, keeps the snap's tag beside its marker, takes a snap clicked, steps
// to the next with Tab, and on a phone brings up the magnifier, a second
// renderer drawing the same sheet three times larger, while a finger is
// held still. While a drawing tool runs, which the renderer holds, a click,
// a tap or a lifted finger goes to the tool, a right-click is Enter, and
// this component shows what the renderer says at the cursor and the text a
// Text tool is typing where it will go.

import { useEffect, useRef, useState, type RefObject } from 'react';

import { freeBeside, newlyHidden, placeBeside, snapTag, type Box, type Hovered, type Named, type ToolState } from './frame.ts';

import { screen_color, Viewer } from './wasm/agentdrafter_viewer.js';
import { attachGestures, Glide, type Surface } from './gestures.ts';
import {
  fit,
  focusOn,
  keepInSight,
  limits,
  PULSE_MS,
  pulseStrength,
  toDrawing,
  toScreen,
  zoomAt,
  type Area,
  type Limits,
  type Paper,
  type View,
} from './view.ts';
import { installProbe, type Draw, type Summary } from './probe.ts';

/** An object a halo marks: its place in the sheet's objects, and whether it was added. */
export interface Mark {
  place: number;
  added: boolean;
}

/** A request to show a part of a sheet: the places of its objects, or the whole sheet when none. */
export interface Focus {
  /** A new key asks again. */
  key: number;
  sheet: string;
  places: number[];
}

export interface DrawingProps {
  /** The text of the `display-list` reply to draw; null before one comes. */
  reply: string | null;
  /** Pixels at the bottom of the drawing the command bar, and a phone's bottom sheet, cover. */
  coveredBelow: number;
  /** Only the fit button, as on a phone, where fingers zoom. */
  compact: boolean;
  /** Whether the grid is drawn behind the drawing. */
  grid: boolean;
  /**
   * Quick properties, which a laptop shows beside the picked object: placed
   * at each draw, so they follow it as the view moves. A phone's sit above
   * the command bar, where its styles put them.
   */
  card?: RefObject<HTMLElement | null>;
  /** The objects a change set touched, marked with halos. */
  marks: Mark[];
  /** The object the reviewer picked, marked blue; null for none. */
  picked: number | null;
  /** When the marked change set landed, by performance.now(), so its halos pulse; null to hold them still. */
  pulseSince: number | null;
  /** Shows a part of the sheet once it is drawn. */
  focus: Focus | null;
  onLoaded(summary: Summary): void;
  /** A sheet could not be drawn. */
  onError(message: string): void;
  /** The renderer could not start, as where there is no WebGL2: nothing can be drawn. */
  onUnavailable(message: string): void;
  /** A tap or a click picked the object at this place, or nothing. */
  onPick(place: number | null): void;
  /**
   * What quick properties on a phone have to sit in: a new value, as the
   * journal comes to rest at another height or the command bar grows,
   * brings the selected object out from under the card again.
   */
  space?: string;
  /** Where the cursor is on the drawing, in drawing units, and the snap it holds; null once it has left. */
  onCursor?(at: [number, number] | null, snapped: Hovered | null): void;
  /** The object snaps the cursor catches, as the renderer's bits; 0 when Object snap is off. */
  snapMask?: number;
  /** A snap clicked, or lifted from under a held finger, with no tool running. */
  onTakeSnap?(snap: Hovered): void;
  /** A held finger lifted where no snap was, at this drawing point. */
  onNoSnap?(at: [number, number]): void;
  /** The colour the picked object shows in, `#rrggbb`, as the renderer says; empty for none. */
  onPickedColor?(color: string): void;
  /** The drawing tool running, as the renderer says it; null with none. */
  tool?: ToolState | null;
  /** What came of a click, a tap or a lifted finger given to the tool running, as the renderer says it. */
  onToolOutcome?(outcome: string): void;
  /** A right-click on the drawing while a tool runs: Enter. */
  onToolEnter?(): void;
  /** Ortho (F8), which the renderer holds Line and Move level or upright by. */
  ortho?: boolean;
  /** The current layer and its colour index, which new objects go on and a rubber band is drawn in. */
  layer?: { name: string; color: number };
  /** Filled in with what lets the page reach the renderer: the tools run in it. */
  handle?: RefObject<DrawingHandle | null>;
}

/** What the page reaches the renderer by. */
export interface DrawingHandle {
  viewer(): Viewer | null;
  /** Draws again on the next frame, as after a tool's step. */
  redraw(): void;
}

/** How near a tap must come to an object to pick it, in CSS pixels. */
const PICK_REACH = 16;

/** On a laptop, the room at the right the zoom buttons take, which a fitted sheet stays clear of. */
const ZOOM_COLUMN = 46;

/** How near the cursor a snap is caught, in CSS pixels: AutoCAD's aperture is 10; a finger needs more. */
const REACH_MOUSE = 12;
const REACH_TOUCH = 22;

/** Two snaps whose distances differ by less than this, in CSS pixels, are as near: AutoCAD's order picks. */
const TIE = 2;

/** The magnifier: how much larger, its largest radius, and how far above the finger, in CSS pixels. */
const LOUPE_ZOOM = 3;
const LOUPE_RADIUS = 70;
const LOUPE_LIFT = 52;

const hoveredOf = (v: Viewer | null): Hovered | null => {
  const text = v?.hovered();
  return text ? (JSON.parse(text) as Hovered) : null;
};

/** Device pixels to a CSS pixel: the canvas's backing size over its size. */
const ratioOf = (canvas: HTMLCanvasElement): number =>
  canvas.clientWidth > 0 ? canvas.width / canvas.clientWidth : window.devicePixelRatio || 1;

export function Drawing({
  reply,
  coveredBelow,
  compact,
  grid,
  card,
  marks,
  picked,
  pulseSince,
  focus,
  onLoaded,
  onError,
  onUnavailable,
  onPick,
  onCursor,
  onPickedColor,
  space,
  snapMask = 0,
  onTakeSnap,
  onNoSnap,
  tool = null,
  onToolOutcome,
  onToolEnter,
  ortho = false,
  layer,
  handle,
}: DrawingProps) {
  const box = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const viewer = useRef<Viewer | null>(null);
  const view = useRef<View>({ left: 0, top: 0, scale: 1 });
  const paper = useRef<Paper | null>(null);
  const bounds = useRef<Limits>({ min: 1e-9, max: 1e9 });
  const frame = useRef(0);
  const draws = useRef<Draw[]>([]);
  // Whether the view is the fitted one, which follows the screen's size
  // until the reviewer pans or zooms.
  const followFit = useRef(true);
  const glide = useRef<Glide | null>(null);
  const covered = useRef(coveredBelow);
  const loadedReply = useRef<string | null>(null);
  const loadedSheet = useRef<string | null>(null);
  const shownMarks = useRef({ marks, picked });
  const pendingFocus = useRef<Focus | null>(null);
  const pick = useRef(onPick);
  const cursor = useRef(onCursor);
  const colorOf = useRef(onPickedColor);
  const narrow = useRef(compact);
  // The grid as the page has it, for a renderer made anew.
  const gridNow = useRef(grid);
  gridNow.current = grid;
  // Where a mouse rests on the canvas, in CSS pixels from its top left, so
  // the status bar's coordinates follow a view that moves under it.
  const pointer = useRef<[number, number] | null>(null);
  // The picked object's box in drawing units, which quick properties sit beside.
  const pickedBox = useRef<number[]>([]);
  // Snapping: the running snaps, the snap the cursor holds, the drawing
  // point it was found at (so Tab's choice holds until the cursor moves),
  // what each object is called, the tag beside the marker, and the
  // magnifier: its canvas, its renderer, and the finger held.
  const mask = useRef(snapMask);
  const hovered = useRef<Hovered | null>(null);
  const hoverKey = useRef('');
  const objects = useRef<Named[]>([]);
  const tag = useRef<HTMLDivElement>(null);
  const loupe = useRef<HTMLCanvasElement>(null);
  const loupeViewer = useRef<Viewer | null>(null);
  const loupeReply = useRef<string | null>(null);
  const held = useRef<[number, number] | null>(null);
  const take = useRef(onTakeSnap);
  const noSnap = useRef(onNoSnap);
  take.current = onTakeSnap;
  noSnap.current = onNoSnap;
  // The tool running, and where what it says at the cursor and the text it
  // is typing are shown.
  const toolNow = useRef(tool);
  toolNow.current = tool;
  const toolOutcome = useRef(onToolOutcome);
  toolOutcome.current = onToolOutcome;
  const toolEnter = useRef(onToolEnter);
  toolEnter.current = onToolEnter;
  const orthoNow = useRef(ortho);
  orthoNow.current = ortho;
  const layerNow = useRef(layer);
  layerNow.current = layer;
  const dyn = useRef<HTMLDivElement>(null);
  const typed = useRef<HTMLDivElement>(null);
  /** Whether a tool runs that takes the drawing's clicks and taps. */
  // Asked of the renderer, which holds the tool, so a tap made before the
  // page has drawn the tool's new step still goes to it.
  const toolActive = () => !!viewer.current?.tool_wants_clicks();
  const [, setReady] = useState(false);
  covered.current = coveredBelow;
  shownMarks.current = { marks, picked };
  pick.current = onPick;
  cursor.current = onCursor;
  colorOf.current = onPickedColor;
  narrow.current = compact;

  // Quick properties beside the picked object, on a laptop.
  const placeCard = () => {
    const el = card?.current;
    const c = canvas.current;
    if (!el || !c) return;
    if (narrow.current) {
      el.style.left = '';
      el.style.top = '';
      el.style.visibility = '';
      return;
    }
    const b = pickedBox.current;
    if (b.length !== 4) {
      el.style.visibility = 'hidden';
      return;
    }
    const v = view.current;
    const [l, t] = toScreen(v, b[0] as number, b[3] as number);
    const [r, bottom] = toScreen(v, b[2] as number, b[1] as number);
    const a = area();
    const [x, y] = placeBeside([l, t, r, bottom], { width: el.offsetWidth, height: el.offsetHeight }, [
      a.x,
      a.y,
      a.x + a.width,
      a.y + a.height,
    ]);
    el.style.left = `${x}px`;
    el.style.top = `${y}px`;
    el.style.visibility = 'visible';
  };

  // The crosshair and the snap the cursor holds, in step with the view:
  // found again only when the drawing point under the cursor moves.
  const hoverNow = () => {
    const v = viewer.current;
    if (!v) return;
    const at = pointer.current;
    if (held.current && toolActive()) {
      // A held finger aims the tool: its rubber band follows the finger,
      // to the snap the magnifier shows.
      v.hide_crosshair();
      const [fx, fy] = toDrawing(view.current, held.current[0], held.current[1]);
      v.set_tool_pointer(fx, fy);
      v.hover(fx, fy, REACH_TOUCH / view.current.scale, TIE / view.current.scale);
      hovered.current = null;
      hoverKey.current = '';
      return;
    }
    if (!at || held.current) {
      v.hide_crosshair();
      v.clear_hover();
      hovered.current = null;
      hoverKey.current = '';
      return;
    }
    const scale = view.current.scale;
    const [x, y] = toDrawing(view.current, at[0], at[1]);
    v.set_crosshair(x, y, true);
    const key = `${x},${y},${scale},${mask.current}`;
    if (key === hoverKey.current) return;
    hoverKey.current = key;
    v.hover(x, y, REACH_MOUSE / scale, TIE / scale);
    hovered.current = hoveredOf(v);
  };
  // The snap's name and its object, beside its marker.
  const placeTag = (h: Hovered | null, x: number, y: number, centred: boolean) => {
    const el = tag.current;
    if (!el) return;
    if (!h) {
      el.style.display = 'none';
      return;
    }
    // A finger has no Tab to step with.
    const { main, sub } = snapTag(h, objects.current, !centred);
    const b = document.createElement('b');
    b.textContent = main;
    el.replaceChildren(b, ` · ${sub}`);
    el.style.display = 'block';
    const c = canvas.current;
    const w = el.offsetWidth;
    const tall = el.offsetHeight;
    const width = c?.clientWidth ?? 0;
    const height = c?.clientHeight ?? 0;
    const left = centred ? x - w / 2 : x;
    // Beside a marker, below it and to its right, `y` 12 below it: above it
    // where below would leave the drawing.
    const top = !centred && y + tall > height - 4 ? y - 24 - tall : y;
    el.style.left = `${Math.max(4, Math.min(width - w - 4, left))}px`;
    el.style.top = `${Math.max(4, Math.min(height - tall - 4, top))}px`;
  };

  const draw = () => {
    frame.current = 0;
    const v = viewer.current;
    const c = canvas.current;
    if (!v || !c) return;
    hoverNow();
    const began = performance.now();
    v.draw(view.current.left, view.current.top, view.current.scale, ratioOf(c));
    const kept = draws.current;
    kept.push({ at: began, ms: performance.now() - began });
    if (kept.length > 2000) kept.splice(0, kept.length - 2000);
    placeCard();
    placeToolWords();
    const at = pointer.current;
    if (held.current) return;
    const h = hovered.current;
    if (h) {
      const [mx, my] = toScreen(view.current, h.at[0], h.at[1]);
      placeTag(h, mx + 12, my + 12, false);
    } else {
      placeTag(null, 0, 0, false);
    }
    if (at) cursor.current?.(toDrawing(view.current, at[0], at[1]), h);
  };
  const redraw = () => {
    if (!frame.current) frame.current = requestAnimationFrame(draw);
  };
  // What the renderer says at the cursor while a tool runs, as AutoCAD's
  // dynamic input shows the length and angle; and the text a Text tool is
  // typing, where it will go, at its height.
  const placeToolWords = () => {
    const v = viewer.current;
    const el = dyn.current;
    if (el) {
      const words = v && toolActive() ? v.tool_cursor_words() : '';
      const at = pointer.current ?? held.current;
      if (words && at) {
        const [a, b] = JSON.parse(words) as [string, string | null];
        const first = document.createElement('b');
        first.textContent = a;
        const kids: Node[] = [first];
        if (b) {
          const second = document.createElement('span');
          second.textContent = b;
          kids.push(second);
        }
        el.replaceChildren(...kids);
        el.style.display = 'flex';
        const c = canvas.current;
        const width = c?.clientWidth ?? 0;
        const height = (c?.clientHeight ?? 0) - covered.current;
        // Below and right of the cursor, as AutoCAD's; above a finger.
        let x = at[0] + 18;
        let y = held.current ? at[1] - 140 : at[1] + 18;
        if (x + el.offsetWidth > width - 4) x = at[0] - 18 - el.offsetWidth;
        if (y + el.offsetHeight > height - 4) y = at[1] - 18 - el.offsetHeight;
        el.style.left = `${Math.max(4, x)}px`;
        el.style.top = `${Math.max(4, y)}px`;
      } else {
        el.style.display = 'none';
      }
    }
    const box = typed.current;
    // The renderer's own, as each key changes it, not the page's copy.
    const text = v?.tool_typing_text() ? (JSON.parse(v.tool_state(false)) as ToolState).text : null;
    if (box) {
      if (!text) {
        if (box.childElementCount) box.replaceChildren();
        return;
      }
      const v2 = view.current;
      // A text's height is its capitals', about 0.7 of the font's size.
      const size = (text.height / 0.7) * v2.scale;
      const color = layerColor.current;
      const lines = text.lines.map((line, i) => {
        const el2 = document.createElement('div');
        el2.className = `tline${i === text.lines.length - 1 ? ' current' : ''}`;
        el2.textContent = line.text;
        const [x, y] = toScreen(v2, line.at[0], line.at[1]);
        el2.style.left = `${x}px`;
        el2.style.top = `${y}px`;
        el2.style.fontSize = `${size}px`;
        el2.style.color = color;
        return el2;
      });
      box.replaceChildren(...lines);
    }
  };
  // The current layer's colour on screen, for the text being typed.
  const layerColor = useRef('#eef1f4');
  const area = (): Area => {
    const c = canvas.current;
    const width = c?.clientWidth ?? 0;
    const height = c?.clientHeight ?? 0;
    return {
      x: 0,
      y: 0,
      width: narrow.current ? width : Math.max(120, width - ZOOM_COLUMN),
      height: Math.max(120, height - covered.current),
    };
  };
  const fitted = (): View | null => {
    const a = area();
    // Not laid out yet: the size will come, and the fit with it.
    if (!paper.current || a.width < 2) return null;
    return fit(paper.current, a);
  };
  const show = (next: View) => {
    const c = canvas.current;
    view.current =
      paper.current && c ? keepInSight(next, paper.current, { width: c.clientWidth, height: c.clientHeight }) : next;
    redraw();
  };
  const surface: Surface = {
    view: () => view.current,
    show,
    limits: () => bounds.current,
    touched: () => {
      followFit.current = false;
      // The drafter takes the view: the page's own glide is theirs to end.
      revealing.current = null;
    },
    tapped: ([sx, sy], pointerType) => {
      const v = viewer.current;
      if (!v || !loadedReply.current) return;
      if (toolActive()) {
        // A tool takes the click or the tap: a point, on the snap in reach
        // of where it is, or an object to pick.
        const [x, y] = toDrawing(view.current, sx, sy);
        if (pointerType === 'mouse') {
          pointer.current = [sx, sy];
          hoverNow();
        } else {
          v.hover(x, y, REACH_TOUCH / view.current.scale, TIE / view.current.scale);
        }
        toolOutcome.current?.(v.tool_click(x, y, PICK_REACH / view.current.scale));
        if (pointerType !== 'mouse') v.clear_hover();
        redraw();
        return;
      }
      // A click on a snap reads its point out; anything else picks. The
      // snap is the one at the click itself, found now, not the one a
      // frame not yet drawn would have found.
      if (pointerType === 'mouse') {
        pointer.current = [sx, sy];
        hoverNow();
      }
      const h = hovered.current;
      if (pointerType === 'mouse' && h) {
        take.current?.(h);
        return;
      }
      const [x, y] = toDrawing(view.current, sx, sy);
      const place = v.pick(x, y, PICK_REACH / view.current.scale);
      pick.current(place >= 0 ? place : null);
    },
    held: (p) => {
      held.current = p;
      showLoupe(p);
      redraw();
    },
    holdMoved: (p) => {
      held.current = p;
      showLoupe(p);
    },
    holdTakenBack: () => {
      held.current = null;
      viewer.current?.clear_tool_pointer();
      hideLoupe();
      redraw();
    },
    tapsOnly: () => toolActive(),
    holdEnded: (p, taken) => {
      if (toolActive()) {
        // Lifting gives the tool the point the magnifier shows: the snap
        // under the finger, found again where it lifts, or the point.
        held.current = null;
        hideLoupe();
        const v = viewer.current;
        v?.clear_tool_pointer();
        if (v && taken) {
          const [x, y] = toDrawing(view.current, p[0], p[1]);
          v.hover(x, y, REACH_TOUCH / view.current.scale, TIE / view.current.scale);
          toolOutcome.current?.(v.tool_click(x, y, PICK_REACH / view.current.scale));
          v.clear_hover();
        }
        redraw();
        return;
      }
      // The snap is found again where the finger lifts, on what is drawn
      // now and of the snaps running now: a sheet drawn anew under the
      // finger, or Object snap turned off from a keyboard, since it was
      // last found takes nothing it no longer shows.
      showLoupe(p);
      const h = mask.current ? hoveredOf(loupeViewer.current) : null;
      held.current = null;
      hideLoupe();
      redraw();
      if (!taken) return;
      if (h) take.current?.(h);
      else noSnap.current?.(toDrawing(view.current, p[0], p[1]));
    },
  };
  // The magnifier a finger holds up: the drawing under the finger three
  // times larger, above it, with the crosshair and the snap it would take.
  const showLoupe = (p: [number, number]) => {
    const el = loupe.current;
    const c = canvas.current;
    if (!el || !c || !loadedReply.current) return;
    if (!loupeViewer.current) {
      try {
        loupeViewer.current = new Viewer(el);
      } catch {
        return;
      }
      loupeReply.current = null;
    }
    const lv = loupeViewer.current;
    if (loupeReply.current !== loadedReply.current) {
      lv.load(loadedReply.current);
      loupeReply.current = loadedReply.current;
    }
    lv.set_grid(gridNow.current);
    lv.set_running(mask.current);
    const width = c.clientWidth;
    const radius = Math.min(LOUPE_RADIUS, width / 4);
    const ratio = window.devicePixelRatio || 1;
    const side = Math.round(radius * 2 * ratio);
    if (el.width !== side) {
      el.width = side;
      el.height = side;
    }
    el.style.width = `${radius * 2}px`;
    el.style.height = `${radius * 2}px`;
    const cx = Math.max(radius + 6, Math.min(width - radius - 6, p[0]));
    let cy = p[1] - radius - LOUPE_LIFT;
    if (cy < radius + 8) cy = p[1] + radius + LOUPE_LIFT;
    el.style.left = `${cx - radius}px`;
    el.style.top = `${cy - radius}px`;
    el.style.display = 'block';
    const scale = view.current.scale;
    const [fx, fy] = toDrawing(view.current, p[0], p[1]);
    lv.hover(fx, fy, REACH_TOUCH / scale, TIE / scale);
    lv.set_crosshair(fx, fy, false);
    const zoomed = scale * LOUPE_ZOOM;
    lv.draw(fx - radius / zoomed, fy + radius / zoomed, zoomed, ratio);
    const h = hoveredOf(lv);
    // Its tag under the magnifier, or above it when it sits below the finger.
    const below = cy < p[1];
    placeTag(
      h ?? { snap: 'endpoint', object: -1, which: null, with: null, at: [fx, fy, 0], place: 0, count: 1 },
      cx,
      below ? cy + radius + 6 : cy - radius - 28,
      true,
    );
    if (!h && tag.current) {
      tag.current.replaceChildren(
        Object.assign(document.createElement('b'), { textContent: mask.current ? 'No snap here' : 'Object snap is off' }),
      );
    }
    cursor.current?.([fx, fy], h);
  };
  const hideLoupe = () => {
    if (loupe.current) loupe.current.style.display = 'none';
    placeTag(null, 0, 0, false);
  };

  const middle = (): [number, number] => {
    const a = area();
    return [a.x + a.width / 2, a.y + a.height / 2];
  };
  const fitSheet = (ms: number) => {
    followFit.current = true;
    const target = fitted();
    if (!target) return;
    bounds.current = limits(target);
    if (ms > 0 && glide.current) glide.current.to(target, ms, middle());
    else {
      // An immediate fit ends any glide still running, or its next frame
      // would put back the view it was heading for.
      glide.current?.stop();
      show(target);
    }
  };
  const zoomBy = (factor: number) => {
    followFit.current = false;
    const [x, y] = middle();
    glide.current?.to(zoomAt(view.current, x, y, view.current.scale * factor, bounds.current), 220, [x, y]);
  };
  const applyMarks = () => {
    const v = viewer.current;
    if (!v) return;
    const { marks: m, picked: p } = shownMarks.current;
    // Two lists, as the renderer takes them.
    const places = m.map((mark) => mark.place);
    const added = m.map((mark) => mark.added);
    v.set_marks(Uint32Array.from(places), Uint8Array.from(added.map((a) => (a ? 1 : 0))), p ?? -1);
    pickedBox.current = p === null ? [] : [...v.bounds_of(Uint32Array.from([p]))];
    colorOf.current?.(p === null ? '' : v.object_color(p));
    const probe = window.agentdrafterViewer;
    if (probe) probe.marked = { places, added, picked: p };
    redraw();
  };
  // Shows the part of the sheet a focus asks for, once that sheet is drawn.
  const applyFocus = () => {
    const f = pendingFocus.current;
    const v = viewer.current;
    if (!f || !v || f.sheet.toUpperCase() !== loadedSheet.current?.toUpperCase()) return;
    pendingFocus.current = null;
    const whole = fitted();
    if (!whole) return;
    const b = f.places.length ? v.bounds_of(Uint32Array.from(f.places)) : [];
    if (b.length !== 4) {
      fitSheet(450);
      return;
    }
    followFit.current = false;
    bounds.current = limits(whole);
    const target = focusOn([b[0], b[1], b[2], b[3]] as unknown as Paper, area(), whole);
    glide.current?.to(target, 450, middle());
  };

  const load = (text: string): boolean => {
    const v = viewer.current;
    if (!v) return false;
    try {
      const summary = JSON.parse(v.load(text)) as Summary;
      objects.current = summary.objects;
      v.set_running(mask.current);
      hoverKey.current = '';
      const sameSheet = loadedSheet.current?.toUpperCase() === summary.sheet.toUpperCase();
      loadedReply.current = text;
      loadedSheet.current = summary.sheet;
      paper.current = summary.paper;
      // The same sheet drawn again, as a change set lands on it or the
      // journal is stepped through, keeps the reviewer's view.
      if (!sameSheet || followFit.current) fitSheet(0);
      else redraw();
      // A finger held: the magnifier shows what is drawn now.
      if (held.current) showLoupe(held.current);
      const probe = window.agentdrafterViewer;
      if (probe) probe.summary = summary;
      onLoaded(summary);
      applyMarks();
      applyFocus();
      return true;
    } catch (e) {
      onError(String((e as Error).message ?? e));
      return false;
    }
  };

  // The renderer, the canvas's size, and pan and zoom: once.
  useEffect(() => {
    const c = canvas.current;
    const b = box.current;
    if (!c || !b) return;
    const start = (): boolean => {
      try {
        viewer.current?.free();
        viewer.current = new Viewer(c);
        // What the page holds that the renderer draws by: a renderer made
        // anew, as after a lost context, starts from it, not its defaults.
        viewer.current.set_grid(gridNow.current);
        viewer.current.set_running(mask.current);
        viewer.current.set_ortho(orthoNow.current);
        if (layerNow.current) viewer.current.set_layer(layerNow.current.name, layerNow.current.color);
        hoverKey.current = '';
        return true;
      } catch (e) {
        viewer.current = null;
        onUnavailable(String((e as Error).message ?? e));
        return false;
      }
    };
    if (!start()) return;
    setReady(true);
    let laidOut = { width: c.clientWidth, height: c.clientHeight };
    const size = () => {
      const ratio = window.devicePixelRatio || 1;
      const width = Math.max(1, Math.round(c.clientWidth * ratio));
      const height = Math.max(1, Math.round(c.clientHeight * ratio));
      const before = laidOut;
      laidOut = { width: c.clientWidth, height: c.clientHeight };
      if (c.width !== width || c.height !== height) {
        c.width = width;
        c.height = height;
      }
      if (followFit.current) {
        // Still the fitted view: fitted again to the screen as it is now.
        fitSheet(0);
      } else if (paper.current && before.width > 1) {
        // Moved by the reviewer: the drawing point at the middle stays there.
        const v = view.current;
        const dx = (laidOut.width - before.width) / 2;
        const dy = (laidOut.height - before.height) / 2;
        view.current = { left: v.left - dx / v.scale, top: v.top + dy / v.scale, scale: v.scale };
      }
      draw();
    };
    const resized = new ResizeObserver(size);
    resized.observe(c);
    // A change of device pixels to a CSS pixel, such as the window moved to
    // another screen or the page zoomed, changes the canvas's pixels.
    let resolution: MediaQueryList | null = null;
    const watchResolution = () => {
      resolution?.removeEventListener('change', onResolution);
      resolution = window.matchMedia(`(resolution: ${window.devicePixelRatio || 1}dppx)`);
      resolution.addEventListener('change', onResolution);
    };
    const onResolution = () => {
      size();
      watchResolution();
    };
    watchResolution();
    size();
    glide.current = new Glide(surface);
    const detach = attachGestures(b, surface, glide.current);
    // The cursor's place on the drawing, for the status bar.
    const hover = (e: PointerEvent) => {
      if (e.pointerType !== 'mouse') return;
      const r = c.getBoundingClientRect();
      pointer.current = [e.clientX - r.left, e.clientY - r.top];
      redraw();
    };
    const away = () => {
      pointer.current = null;
      redraw();
      cursor.current?.(null, null);
    };
    // Tab steps to the next snap in reach, as AutoCAD's does, while the
    // drawing has the keys: from a button or a field it moves the focus on.
    const tab = (e: KeyboardEvent) => {
      const h = hovered.current;
      if (e.key !== 'Tab' || !pointer.current || !h || h.count < 2 || held.current) return;
      const focused = document.activeElement;
      if (focused && focused !== document.body && focused !== b && focused !== c) return;
      e.preventDefault();
      viewer.current?.next_snap();
      hovered.current = hoveredOf(viewer.current);
      redraw();
    };
    // A press on the drawing gives it the keys back from a control, but
    // not from the command bar while a Text tool's words are typed there.
    const keysHere = (e: PointerEvent) => {
      if ((e.target as Element | null)?.closest?.('button')) return;
      if (viewer.current?.tool_typing_text()) return;
      const focused = document.activeElement;
      if (focused instanceof HTMLElement && focused !== document.body) focused.blur();
    };
    window.addEventListener('keydown', tab);
    // A right-click is Enter while a tool runs, as AutoCAD's is.
    const menu = (e: MouseEvent) => {
      if (!toolNow.current?.tool) return;
      e.preventDefault();
      toolEnter.current?.();
    };
    b.addEventListener('contextmenu', menu);
    if (handle) handle.current = { viewer: () => viewer.current, redraw };
    b.addEventListener('pointerdown', keysHere);
    b.addEventListener('pointermove', hover);
    b.addEventListener('pointerleave', away);
    const lost = (e: Event) => e.preventDefault();
    const restored = () => {
      if (start() && loadedReply.current) load(loadedReply.current);
      redraw();
    };
    c.addEventListener('webglcontextlost', lost);
    c.addEventListener('webglcontextrestored', restored);
    const probe = installProbe({
      viewer: () => viewer.current,
      hovered: () => hovered.current,
      rehover: () => {
        hoverKey.current = '';
        redraw();
      },
      canvas: c,
      view: () => view.current,
      show,
      draws: draws.current,
      redraw: draw,
    });
    return () => {
      probe();
      detach();
      b.removeEventListener('pointerdown', keysHere);
      b.removeEventListener('contextmenu', menu);
      if (handle) handle.current = null;
      b.removeEventListener('pointermove', hover);
      b.removeEventListener('pointerleave', away);
      window.removeEventListener('keydown', tab);
      loupeViewer.current?.free();
      loupeViewer.current = null;
      resized.disconnect();
      resolution?.removeEventListener('change', onResolution);
      c.removeEventListener('webglcontextlost', lost);
      c.removeEventListener('webglcontextrestored', restored);
      if (frame.current) cancelAnimationFrame(frame.current);
      viewer.current?.free();
      viewer.current = null;
    };
    // The surface reads refs, so the handlers are attached once.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // A new sheet, or the same sheet drawn anew: drawn, and fitted to the
  // screen when it is another sheet.
  useEffect(() => {
    if (reply && viewer.current && reply !== loadedReply.current) load(reply);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reply]);

  // The halos follow what the page marks.
  useEffect(() => {
    applyMarks();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [marks, picked]);

  // On a phone, an object picked where quick properties open over it is
  // brought into the drawing's free part: above the card, as a phone held
  // upright has it, or left of it, as one held on its side has it. Where
  // the card begins is measured, not assumed. When only the room the card
  // has changes (`space`), as the journal comes to rest at another height
  // or the command bar grows or shrinks, the view moves only if that newly
  // hides the object: never to pull back an object the drafter panned away.
  const revealed = useRef<{ picked: number | null; compact: boolean; free: Box } | null>(null);
  // The page's own glide to reveal the selection, while it runs: the view
  // it is heading for, and when it ends. A drafter's touch ends it.
  const revealing = useRef<{ target: View; until: number } | null>(null);
  useEffect(() => {
    const c = canvas.current;
    const el = card?.current;
    const b = pickedBox.current;
    if (!compact || picked === null || !c || !el || b.length !== 4) {
      revealed.current = null;
      return;
    }
    const cr = c.getBoundingClientRect();
    const qr = el.getBoundingClientRect();
    const free = freeBeside([qr.left - cr.left, qr.top - cr.top, qr.right - cr.left, qr.bottom - cr.top], cr.width, cr.height);
    const before = revealed.current;
    revealed.current = { picked, compact, free };
    // While the page's own glide runs, the drafter has not moved the view:
    // the object is judged where that glide takes it, and a new glide may
    // take its place.
    const own = revealing.current && performance.now() < revealing.current.until ? revealing.current.target : null;
    const v = own ?? view.current;
    const [l, t] = toScreen(v, b[0] as number, b[3] as number);
    const [r, bottom] = toScreen(v, b[2] as number, b[1] as number);
    const sameSelection = before !== null && before.picked === picked && before.compact === compact;
    if (sameSelection && !own && !newlyHidden([l, t, r, bottom], before.free, free, cr.width, cr.height)) return;
    const m = 12;
    if (l >= free[0] + m && r <= free[2] - m && t >= free[1] + m && bottom <= free[3] - m) return;
    followFit.current = false;
    const [fx, fy] = [(free[0] + free[2]) / 2, (free[1] + free[3]) / 2];
    const target = { ...v, left: v.left + ((l + r) / 2 - fx) / v.scale, top: v.top - ((t + bottom) / 2 - fy) / v.scale };
    revealing.current = { target, until: performance.now() + 300 };
    glide.current?.to(target, 300, [fx, fy]);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [picked, compact, space]);

  // The running snaps: none while Object snap is off.
  useEffect(() => {
    mask.current = snapMask;
    viewer.current?.set_running(snapMask);
    hoverKey.current = '';
    // A finger held: the magnifier shows the snaps running now.
    if (held.current) showLoupe(held.current);
    redraw();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [snapMask]);

  // Ortho and the current layer, which the tools draw by.
  useEffect(() => {
    viewer.current?.set_ortho(ortho);
    redraw();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ortho]);
  useEffect(() => {
    if (!layer) return;
    viewer.current?.set_layer(layer.name, layer.color);
    layerColor.current = screen_color(layer.color);
    redraw();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [layer?.name, layer?.color]);
  // A tool's step: drawn again, its rubber band and its words with it, and
  // the snap under the cursor found again, as a tool picking objects
  // catches none.
  useEffect(() => {
    hoverKey.current = '';
    redraw();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tool]);

  // The grid on or off.
  useEffect(() => {
    viewer.current?.set_grid(grid);
    redraw();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [grid]);

  // A card that changes size, or a laptop that becomes a phone, places it again.
  useEffect(() => {
    const el = card?.current;
    if (!el) return;
    placeCard();
    const resized = new ResizeObserver(() => placeCard());
    resized.observe(el);
    return () => resized.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [card, compact, picked]);

  // A change set that just landed pulses its halos, then holds them.
  useEffect(() => {
    const v = viewer.current;
    if (!v) return;
    if (pulseSince === null) {
      v.set_pulse(1);
      redraw();
      return;
    }
    let pulsing = 0;
    const step = () => {
      const ms = performance.now() - pulseSince;
      viewer.current?.set_pulse(pulseStrength(ms));
      redraw();
      pulsing = ms < PULSE_MS ? requestAnimationFrame(step) : 0;
    };
    pulsing = requestAnimationFrame(step);
    return () => {
      if (pulsing) cancelAnimationFrame(pulsing);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pulseSince]);

  // A part of a sheet to show, now or once its sheet is drawn.
  useEffect(() => {
    if (!focus) return;
    pendingFocus.current = focus;
    applyFocus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focus?.key]);

  // The bottom sheet's peek changes the room the fitted sheet has.
  useEffect(() => {
    if (followFit.current) {
      fitSheet(0);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [coveredBelow]);

  return (
    <div className="viewport" ref={box}>
      <canvas className={`stage${compact ? '' : ' crosshair'}`} ref={canvas} aria-label="The drawing" data-tool={tool?.tool ?? undefined} />
      <canvas className="loupe" ref={loupe} aria-hidden="true" />
      <div className="snaptag" ref={tag} aria-live="polite" />
      <div className="dyn" ref={dyn} aria-hidden="true" data-dyn />
      <div className="typed" ref={typed} aria-hidden="true" data-typed />
      <div className="zoombtns">
        {!compact && (
          <>
            <button type="button" aria-label="Zoom in" onClick={() => zoomBy(1.6)}>
              +
            </button>
            <button type="button" aria-label="Zoom out" onClick={() => zoomBy(1 / 1.6)}>
              &minus;
            </button>
          </>
        )}
        <button type="button" className="fit" aria-label="Fit the sheet" title="Show the whole sheet" onClick={() => fitSheet(450)}>
          Fit
        </button>
      </div>
    </div>
  );
}
