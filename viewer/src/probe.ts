// What the viewer shows tests and proofs, as `window.agentdrafterViewer`:
// the sheet loaded, the view, how long each frame's draw took, and the
// canvas drawn at a fixed view and size, to hold it to the PNG `agentdrafter
// render` draws of the same sheet. It reads; it changes nothing the
// reviewer sees, beyond one frame drawn and drawn again.

import type { Viewer } from './wasm/agentdrafter_viewer.js';
import type { Paper, View } from './view.ts';

/** What the renderer says of a loaded sheet. */
export interface Summary {
  source: string;
  /** What the record is called, for the heading, such as `demo-plant`. */
  name: string;
  sheet: string;
  sheets: string[];
  millimetres_per_unit: number;
  paper: Paper;
  segments: number;
  triangles: number;
  fills_not_drawn: number;
  /** The journal's entry the set is drawn as it was at, or null for the set as it is. */
  entry: number | null;
  /** What each of the sheet's objects is, by its place. */
  objects: SheetObject[];
}

/** One of the sheet's objects, as the core says what it is. */
export interface SheetObject {
  id?: string;
  name?: string;
  type: string;
  layer: string;
  block?: string;
  attributes?: { tag: string; value: string }[];
}

/** What the halos mark now. */
export interface Marked {
  places: number[];
  added: boolean[];
  picked: number | null;
}

/** One draw of the canvas: when it began, and how long the call took. */
export interface Draw {
  at: number;
  ms: number;
}

export interface Capture {
  /** The drawing point at the image's top left, and pixels per drawing unit. */
  left: number;
  top: number;
  scale: number;
  width: number;
  height: number;
  /** Draw white round the paper, as a PNG's paper is everywhere. */
  white?: boolean;
}

export interface Probe {
  summary: Summary | null;
  /** What the halos mark, once a sheet is drawn. */
  marked?: Marked;
  /** The box round the objects at these places, [left, bottom, right, top] in drawing units; empty when they draw nothing. */
  bounds(places: number[]): number[];
  view(): View;
  /** Shows `view` from the next frame, as a pan or a zoom does. */
  show(view: View): void;
  /** Each draw of the canvas since the last call: when, and how long. */
  takeDraws(): Draw[];
  /** The canvas drawn at `capture`, one CSS pixel to a pixel: RGBA, base64. */
  capture(capture: Capture): string;
}

declare global {
  interface Window {
    agentdrafterViewer?: Probe;
  }
}

interface Parts {
  viewer(): Viewer | null;
  canvas: HTMLCanvasElement;
  view(): View;
  show(view: View): void;
  draws: Draw[];
  redraw(): void;
}

/** Installs the probe; gives back what takes it off. */
export function installProbe(parts: Parts): () => void {
  const probe: Probe = {
    summary: null,
    bounds(places) {
      return [...(parts.viewer()?.bounds_of(Uint32Array.from(places)) ?? [])];
    },
    view: parts.view,
    show: parts.show,
    takeDraws() {
      return parts.draws.splice(0, parts.draws.length);
    },
    capture({ left, top, scale, width, height, white = true }) {
      const viewer = parts.viewer();
      if (!viewer) throw new Error('no renderer');
      const c = parts.canvas;
      const kept = { width: c.width, height: c.height };
      c.width = width;
      c.height = height;
      viewer.set_white_surround(white);
      viewer.draw(left, top, scale, 1);
      const pixels = viewer.pixels();
      viewer.set_white_surround(false);
      c.width = kept.width;
      c.height = kept.height;
      parts.redraw();
      let text = '';
      for (let i = 0; i < pixels.length; i += 0x8000) {
        text += String.fromCharCode(...pixels.subarray(i, i + 0x8000));
      }
      return btoa(text);
    },
  };
  window.agentdrafterViewer = probe;
  return () => {
    if (window.agentdrafterViewer === probe) delete window.agentdrafterViewer;
  };
}
