// Pan and zoom by mouse, trackpad and touch, as the picked prototype does:
// one pointer drags the sheet and flings it on release; two fingers pinch
// and pan at once; a double tap or double click zooms in where it lands;
// the wheel zooms where the pointer is, a trackpad's pinch (a wheel with
// ctrlKey) faster. Every change goes through the surface, which draws it on
// the next frame. A finger held still for HOLD_MS brings the magnifier
// where it is, and moves it until it lifts, which takes the snap it shows;
// a pointer pans only once it has moved SLOP pixels from where it went
// down, so a hold is not lost to a tremor nor a click to a jitter, and a
// gesture that panned is never also a tap. A finger that moves on more than
// STILL pixels, short of panning, is held still anew from there. A second
// finger ends a hold, with nothing taken, and pinches.
//
// A finger is judged by its own events' times, when it moved, not by when
// the page got round to them: on a slow device the moves of a finger that
// is dragging can reach the page after the hold's timer has fired. A hold
// the timer began is taken back, with nothing taken, when a move comes
// that the finger made before the hold was due.

import { between, ease, pinch, panBy, zoomAt, type Limits, type PinchStart, type View } from './view.ts';

export interface Surface {
  view(): View;
  /** Shows `view` from the next frame, kept in sight. */
  show(view: View): void;
  limits(): Limits;
  /** The reviewer took the view in hand: it no longer follows the fit. */
  touched(): void;
  /** A tap or a click, not a drag, at a point of the element, and the pointer that made it: a pick, or a click on a snap. */
  tapped?(at: [number, number], pointer: string): void;
  /** A finger held still: the magnifier comes up there. */
  held?(at: [number, number]): void;
  /** The held finger moves. */
  holdMoved?(at: [number, number]): void;
  /** The held finger lifts, `taken` when it lifted rather than was cancelled. */
  holdEnded?(at: [number, number], taken: boolean): void;
  /** What looked like a hold was a finger moving, or a quick tap, all along: the magnifier goes, and nothing is taken. */
  holdTakenBack?(): void;
}

type Point = [number, number];

/** How long a finger is held still before the magnifier comes, in milliseconds. */
export const HOLD_MS = 350;

/** How far a finger moves, in CSS pixels, before it pans rather than holds. */
export const SLOP = 6;

/** How far a held finger may stray, in CSS pixels, a tremor: further, and it is moving. */
export const STILL = 3;

/** Moves the surface's view to `to` over `ms` milliseconds; stops on a new gesture. */
export class Glide {
  private frame = 0;
  private readonly surface: Surface;

  constructor(surface: Surface) {
    this.surface = surface;
  }

  stop(): void {
    if (this.frame) cancelAnimationFrame(this.frame);
    this.frame = 0;
  }

  to(target: View, ms: number, middle: Point): void {
    this.stop();
    const from = this.surface.view();
    if (ms <= 0) {
      this.surface.show(target);
      return;
    }
    const began = performance.now();
    const step = (now: number) => {
      const t = Math.min(1, (now - began) / ms);
      this.surface.show(between(from, target, ease(t), middle));
      this.frame = t < 1 ? requestAnimationFrame(step) : 0;
    };
    this.frame = requestAnimationFrame(step);
  }

  /** Lets a fling run on, slowing, from `velocity` CSS pixels a frame. */
  fling(velocity: Point): void {
    this.stop();
    let [vx, vy] = velocity;
    if (Math.hypot(vx, vy) < 1.5) return;
    const step = () => {
      vx *= 0.93;
      vy *= 0.93;
      this.surface.show(panBy(this.surface.view(), vx, vy));
      this.frame = Math.hypot(vx, vy) > 0.25 ? requestAnimationFrame(step) : 0;
    };
    this.frame = requestAnimationFrame(step);
  }
}

/** Attaches pan and zoom to `element`; gives back what takes them off. */
export function attachGestures(element: HTMLElement, surface: Surface, glide: Glide): () => void {
  const pointers = new Map<number, Point>();
  let pinchStart: PinchStart | null = null;
  let last: Point | null = null;
  // Where the first pointer went down, and when, by its event's time; where
  // it has rested since when, by its events' times; and where it is.
  let downAt: { p: Point; t: number } | null = null;
  let still: { p: Point; t: number } | null = null;
  let current: Point | null = null;
  let lastTap: { p: Point; t: number } | null = null;
  let multi = false;
  const moves: { t: number; dx: number; dy: number }[] = [];
  // A finger held: its timer, whether the magnifier is up, and whether the
  // finger has moved far enough to pan.
  let holdTimer = 0;
  let holding = false;
  let panning = false;
  // Whether the pointer down is a finger or a pen, which may hold.
  let mayHold = false;
  const stopHold = () => {
    if (holdTimer) clearTimeout(holdTimer);
    holdTimer = 0;
  };
  // The hold comes HOLD_MS after the finger came to rest, where it is then.
  const armHold = () => {
    stopHold();
    if (!mayHold || !surface.held) return;
    holdTimer = window.setTimeout(() => {
      holdTimer = 0;
      if (pointers.size === 1 && !panning && !multi && current) {
        holding = true;
        surface.held?.(current);
      }
    }, HOLD_MS);
  };
  // A hold that was never one: the magnifier goes.
  const takeBack = () => {
    holding = false;
    surface.holdTakenBack?.();
  };
  // Whether an event, by its own time, came before the hold was due.
  const beforeHold = (e: PointerEvent) => !!still && e.timeStamp - still.t < HOLD_MS;

  const local = (e: PointerEvent | WheelEvent): Point => {
    const r = element.getBoundingClientRect();
    return [e.clientX - r.left, e.clientY - r.top];
  };
  const distance = (a: Point, b: Point) => Math.hypot(a[0] - b[0], a[1] - b[1]);
  const twoFingers = (): [Point, Point] => {
    const [a, b] = [...pointers.values()];
    return [a as Point, b as Point];
  };

  const down = (e: PointerEvent) => {
    if (e.pointerType === 'mouse' && e.button !== 0) return;
    if ((e.target as Element | null)?.closest?.('button')) return;
    glide.stop();
    surface.touched();
    try {
      element.setPointerCapture(e.pointerId);
    } catch {
      // The pointer is already gone.
    }
    const p = local(e);
    pointers.set(e.pointerId, p);
    stopHold();
    // A second finger: a pinch, never a hold.
    if (holding && pointers.size > 1) takeBack();
    if (pointers.size === 1) {
      last = p;
      current = p;
      downAt = { p, t: e.timeStamp };
      still = downAt;
      multi = false;
      panning = false;
      mayHold = e.pointerType !== 'mouse';
      moves.length = 0;
      armHold();
    } else if (pointers.size === 2) {
      const [a, b] = twoFingers();
      pinchStart = { view: surface.view(), a, b };
      multi = true;
    }
  };

  const move = (e: PointerEvent) => {
    if (!pointers.has(e.pointerId)) return;
    const p = local(e);
    pointers.set(e.pointerId, p);
    if (pointers.size === 1 && !panning && !multi) {
      current = p;
      // A finger that moves on past a tremor rests anew where it is.
      if (still && distance(p, still.p) > STILL) {
        // The hold came up while this move, made before it was due, was on
        // its way: the finger was moving all along.
        if (holding && beforeHold(e)) takeBack();
        if (!holding) {
          still = { p, t: e.timeStamp };
          armHold();
        }
      }
    }
    if (holding) {
      surface.holdMoved?.(p);
      return;
    }
    if (pointers.size === 1 && last && !panning) {
      // A pointer pans once it has moved far enough; until then a finger may hold.
      if (!downAt || distance(p, downAt.p) < SLOP) return;
      panning = true;
      stopHold();
    }
    if (pointers.size === 1 && last) {
      const dx = p[0] - last[0];
      const dy = p[1] - last[1];
      last = p;
      surface.show(panBy(surface.view(), dx, dy));
      moves.push({ t: e.timeStamp, dx, dy });
      if (moves.length > 8) moves.shift();
    } else if (pointers.size >= 2 && pinchStart) {
      const [a, b] = twoFingers();
      surface.show(pinch(pinchStart, a, b, surface.limits()));
    }
  };

  const up = (e: PointerEvent) => {
    if (!pointers.has(e.pointerId)) return;
    const p = local(e);
    pointers.delete(e.pointerId);
    stopHold();
    // A finger lifted before the hold was due, by its own time, was a tap:
    // a hold that came up meanwhile is taken back.
    if (holding && beforeHold(e)) takeBack();
    if (holding) {
      holding = false;
      pointers.clear();
      last = null;
      downAt = null;
      surface.holdEnded?.(p, e.type === 'pointerup');
      return;
    }
    if (pointers.size === 1) {
      // One finger lifted from a pinch: the other pans on from where it is.
      last = [...pointers.values()][0] ?? null;
      pinchStart = null;
      moves.length = 0;
      return;
    }
    if (pointers.size > 0) {
      const [a, b] = twoFingers();
      pinchStart = { view: surface.view(), a, b };
      return;
    }
    pinchStart = null;
    last = null;
    const start = downAt;
    downAt = null;
    if (!start || e.type !== 'pointerup') return;
    // Taps and flings by the events' own times, as the finger made them.
    const now = e.timeStamp;
    // A gesture that panned was a drag, never also a tap.
    const tap = !multi && !panning && distance(p, start.p) < 10 && now - start.t < 450;
    if (tap) {
      if (lastTap && now - lastTap.t < 320 && distance(p, lastTap.p) < 30) {
        lastTap = null;
        const view = surface.view();
        glide.to(zoomAt(view, p[0], p[1], view.scale * 2.2, surface.limits()), 260, p);
      } else {
        lastTap = { p, t: now };
        surface.tapped?.(p, e.pointerType);
      }
      return;
    }
    if (!multi) {
      const recent = moves.filter((m) => now - m.t < 90);
      if (recent.length >= 2) {
        const span = Math.max(16, now - (recent[0]?.t ?? now));
        const vx = (recent.reduce((s, m) => s + m.dx, 0) / span) * 16;
        const vy = (recent.reduce((s, m) => s + m.dy, 0) / span) * 16;
        glide.fling([vx, vy]);
      }
    }
  };

  const wheel = (e: WheelEvent) => {
    e.preventDefault();
    glide.stop();
    surface.touched();
    const lines = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? 400 : 1;
    const dy = e.deltaY * lines;
    const p = local(e);
    const view = surface.view();
    surface.show(zoomAt(view, p[0], p[1], view.scale * Math.exp(-dy * (e.ctrlKey ? 0.01 : 0.0018)), surface.limits()));
  };

  // A page's own pinch and double-tap zoom stay off the drawing.
  const gesture = (e: Event) => e.preventDefault();

  element.addEventListener('pointerdown', down);
  element.addEventListener('pointermove', move);
  element.addEventListener('pointerup', up);
  element.addEventListener('pointercancel', up);
  element.addEventListener('wheel', wheel, { passive: false });
  element.addEventListener('gesturestart', gesture);
  element.addEventListener('gesturechange', gesture);
  return () => {
    glide.stop();
    element.removeEventListener('pointerdown', down);
    element.removeEventListener('pointermove', move);
    element.removeEventListener('pointerup', up);
    element.removeEventListener('pointercancel', up);
    element.removeEventListener('wheel', wheel);
    element.removeEventListener('gesturestart', gesture);
    element.removeEventListener('gesturechange', gesture);
  };
}
