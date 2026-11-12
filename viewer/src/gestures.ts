// Pan and zoom by mouse, trackpad and touch, as the picked prototype does:
// one pointer drags the sheet and flings it on release; two fingers pinch
// and pan at once; a double tap or double click zooms in where it lands;
// the wheel zooms where the pointer is, a trackpad's pinch (a wheel with
// ctrlKey) faster. Every change goes through the surface, which draws it on
// the next frame.

import { between, ease, pinch, panBy, zoomAt, type Limits, type PinchStart, type View } from './view.ts';

export interface Surface {
  view(): View;
  /** Shows `view` from the next frame, kept in sight. */
  show(view: View): void;
  limits(): Limits;
  /** The reviewer took the view in hand: it no longer follows the fit. */
  touched(): void;
  /** A tap or a click, not a drag, at a point of the element: a pick. */
  tapped?(at: [number, number]): void;
}

type Point = [number, number];

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
  let downAt: { p: Point; t: number } | null = null;
  let lastTap: { p: Point; t: number } | null = null;
  let multi = false;
  const moves: { t: number; dx: number; dy: number }[] = [];

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
    if (pointers.size === 1) {
      last = p;
      downAt = { p, t: performance.now() };
      multi = false;
      moves.length = 0;
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
    if (pointers.size === 1 && last) {
      const dx = p[0] - last[0];
      const dy = p[1] - last[1];
      last = p;
      surface.show(panBy(surface.view(), dx, dy));
      moves.push({ t: performance.now(), dx, dy });
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
    const now = performance.now();
    const tap = !multi && distance(p, start.p) < 10 && now - start.t < 450;
    if (tap) {
      if (lastTap && now - lastTap.t < 320 && distance(p, lastTap.p) < 30) {
        lastTap = null;
        const view = surface.view();
        glide.to(zoomAt(view, p[0], p[1], view.scale * 2.2, surface.limits()), 260, p);
      } else {
        lastTap = { p, t: now };
        surface.tapped?.(p);
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
