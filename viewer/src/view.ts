// Where the sheet sits on the screen, and how pans and zooms move it.
//
// A view names the drawing point at the canvas's top left corner and how
// many CSS pixels a drawing unit spans, as a render's reply names its top
// left and its pixels per unit: the drawing's point (x, y) is at
// ((x - left) * scale, (top - y) * scale), counted from the top left. The
// drawing's y goes up and the screen's down.

export interface View {
  left: number;
  top: number;
  scale: number;
}

/** A sheet of paper in drawing units: left, bottom, right, top. */
export type Paper = readonly [number, number, number, number];

/** An area of the screen in CSS pixels, from the canvas's top left. */
export interface Area {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Limits {
  min: number;
  max: number;
}

export const clamp = (value: number, low: number, high: number): number =>
  Math.min(high, Math.max(low, value));

export function toScreen(view: View, x: number, y: number): [number, number] {
  return [(x - view.left) * view.scale, (view.top - y) * view.scale];
}

export function toDrawing(view: View, sx: number, sy: number): [number, number] {
  return [view.left + sx / view.scale, view.top - sy / view.scale];
}

/** The view that shows the whole paper in `area`, `pad` pixels inside it. */
export function fit(paper: Paper, area: Area, pad = 10): View {
  const [left, bottom, right, top] = paper;
  const width = Math.max(right - left, 1e-9);
  const height = Math.max(top - bottom, 1e-9);
  const scale = Math.max(
    Math.min((area.width - 2 * pad) / width, (area.height - 2 * pad) / height),
    1e-9,
  );
  // The paper's middle at the area's middle.
  const cx = (left + right) / 2;
  const cy = (bottom + top) / 2;
  const sx = area.x + area.width / 2;
  const sy = area.y + area.height / 2;
  return { left: cx - sx / scale, top: cy + sy / scale, scale };
}

/**
 * How far the reviewer may zoom: out to half the fitted size, and in until
 * the paper's longer side spans 32 times the fitted size, where the display
 * list's curves are still finer than a pixel.
 */
export function limits(fitted: View): Limits {
  return { min: fitted.scale * 0.5, max: fitted.scale * 32 };
}

/** The view zoomed to `scale`, the drawing point under (sx, sy) kept there. */
export function zoomAt(view: View, sx: number, sy: number, scale: number, bounds: Limits): View {
  const next = clamp(scale, bounds.min, bounds.max);
  const [x, y] = toDrawing(view, sx, sy);
  return { left: x - sx / next, top: y + sy / next, scale: next };
}

/** The view moved by (dx, dy) CSS pixels: the drawing follows the pointer. */
export function panBy(view: View, dx: number, dy: number): View {
  return { left: view.left - dx / view.scale, top: view.top + dy / view.scale, scale: view.scale };
}

/**
 * The view moved as little as it takes to keep `margin` pixels of the paper
 * on the screen, so the sheet cannot be flung away.
 */
export function keepInSight(view: View, paper: Paper, size: { width: number; height: number }, margin = 60): View {
  const [left, bottom, right, top] = paper;
  const [x0, y0] = toScreen(view, left, top);
  const [x1, y1] = toScreen(view, right, bottom);
  let dx = 0;
  let dy = 0;
  if (x1 < margin) dx = margin - x1;
  else if (x0 > size.width - margin) dx = size.width - margin - x0;
  if (y1 < margin) dy = margin - y1;
  else if (y0 > size.height - margin) dy = size.height - margin - y0;
  return dx || dy ? panBy(view, dx, dy) : view;
}

/** Where two fingers began, and the view then. */
export interface PinchStart {
  view: View;
  a: [number, number];
  b: [number, number];
}

/**
 * The view while two fingers pinch: zoomed by how far apart they are now
 * against when they began, and the drawing point that was under their
 * middle kept under it as they move.
 */
export function pinch(start: PinchStart, a: [number, number], b: [number, number], bounds: Limits): View {
  const d0 = Math.max(1, Math.hypot(start.b[0] - start.a[0], start.b[1] - start.a[1]));
  const d1 = Math.hypot(b[0] - a[0], b[1] - a[1]);
  const scale = clamp((start.view.scale * d1) / d0, bounds.min, bounds.max);
  const m0: [number, number] = [(start.a[0] + start.b[0]) / 2, (start.a[1] + start.b[1]) / 2];
  const m1: [number, number] = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
  const [x, y] = toDrawing(start.view, m0[0], m0[1]);
  return { left: x - m1[0] / scale, top: y + m1[1] / scale, scale };
}

/**
 * A view part way from `from` to `to`, `t` from 0 to 1: the zoom eased in
 * steps of equal ratio, and the point at the area's middle moved straight.
 */
export function between(from: View, to: View, t: number, middle: [number, number]): View {
  const scale = Math.exp(Math.log(from.scale) + (Math.log(to.scale) - Math.log(from.scale)) * t);
  const [ax, ay] = toDrawing(from, middle[0], middle[1]);
  const [bx, by] = toDrawing(to, middle[0], middle[1]);
  const x = ax + (bx - ax) * t;
  const y = ay + (by - ay) * t;
  return { left: x - middle[0] / scale, top: y + middle[1] / scale, scale };
}

/** Eases in and out over 0 to 1. */
export const ease = (t: number): number => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);
