// The drawing: a canvas the Rust renderer draws on with WebGL2, from the
// display list the server sent, as dark model space with its grid. This
// component only says where the sheet sits (the view) and when to draw;
// what is drawn, and how, is the renderer's (crates/agentdrafter-viewer),
// and so is what a tap picks: the core finds the object drawn nearest the
// point in the display list, so the page holds no hit-test of its own. It
// also keeps quick properties beside the picked object as the view moves,
// and says where the cursor is on the drawing.

import { useEffect, useRef, useState, type RefObject } from 'react';

import { freeBeside, placeBeside } from './frame.ts';

import { Viewer } from './wasm/agentdrafter_viewer.js';
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
  /** Where the mouse is on the drawing, in drawing units, or null once it has left. */
  onCursor?(at: [number, number] | null): void;
  /** The colour the picked object shows in, `#rrggbb`, as the renderer says; empty for none. */
  onPickedColor?(color: string): void;
}

/** How near a tap must come to an object to pick it, in CSS pixels. */
const PICK_REACH = 16;

/** On a laptop, the room at the right the zoom buttons take, which a fitted sheet stays clear of. */
const ZOOM_COLUMN = 46;

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

  const draw = () => {
    frame.current = 0;
    const v = viewer.current;
    const c = canvas.current;
    if (!v || !c) return;
    const began = performance.now();
    v.draw(view.current.left, view.current.top, view.current.scale, ratioOf(c));
    const kept = draws.current;
    kept.push({ at: began, ms: performance.now() - began });
    if (kept.length > 2000) kept.splice(0, kept.length - 2000);
    placeCard();
    const at = pointer.current;
    if (at) cursor.current?.(toDrawing(view.current, at[0], at[1]));
  };
  const redraw = () => {
    if (!frame.current) frame.current = requestAnimationFrame(draw);
  };
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
    },
    tapped: ([sx, sy]) => {
      const v = viewer.current;
      if (!v || !loadedReply.current) return;
      const [x, y] = toDrawing(view.current, sx, sy);
      const place = v.pick(x, y, PICK_REACH / view.current.scale);
      pick.current(place >= 0 ? place : null);
    },
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
      const sameSheet = loadedSheet.current?.toUpperCase() === summary.sheet.toUpperCase();
      loadedReply.current = text;
      loadedSheet.current = summary.sheet;
      paper.current = summary.paper;
      // The same sheet drawn again, as a change set lands on it or the
      // journal is stepped through, keeps the reviewer's view.
      if (!sameSheet || followFit.current) fitSheet(0);
      else redraw();
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
      cursor.current?.(toDrawing(view.current, pointer.current[0], pointer.current[1]));
    };
    const away = () => {
      pointer.current = null;
      cursor.current?.(null);
    };
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
      canvas: c,
      view: () => view.current,
      show,
      draws: draws.current,
      redraw: draw,
    });
    return () => {
      probe();
      detach();
      b.removeEventListener('pointermove', hover);
      b.removeEventListener('pointerleave', away);
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
  // the card begins is measured, not assumed.
  useEffect(() => {
    const c = canvas.current;
    const el = card?.current;
    const b = pickedBox.current;
    if (!compact || picked === null || !c || !el || b.length !== 4) return;
    const cr = c.getBoundingClientRect();
    const qr = el.getBoundingClientRect();
    const free = freeBeside([qr.left - cr.left, qr.top - cr.top, qr.right - cr.left, qr.bottom - cr.top], cr.width, cr.height);
    const v = view.current;
    const [l, t] = toScreen(v, b[0] as number, b[3] as number);
    const [r, bottom] = toScreen(v, b[2] as number, b[1] as number);
    const m = 12;
    if (l >= free[0] + m && r <= free[2] - m && t >= free[1] + m && bottom <= free[3] - m) return;
    followFit.current = false;
    const [fx, fy] = [(free[0] + free[2]) / 2, (free[1] + free[3]) / 2];
    const target = { ...v, left: v.left + ((l + r) / 2 - fx) / v.scale, top: v.top - ((t + bottom) / 2 - fy) / v.scale };
    glide.current?.to(target, 300, [fx, fy]);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [picked, compact]);

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
      <canvas className="stage" ref={canvas} aria-label="The drawing" />
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
