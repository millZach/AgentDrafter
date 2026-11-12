// The drawing: a canvas the Rust renderer draws on with WebGL2, from the
// display list the server sent. This component only says where the sheet
// sits (the view) and when to draw; what is drawn, and how, is the
// renderer's (crates/agentdrafter-viewer).

import { useEffect, useRef, useState } from 'react';

import { Viewer } from './wasm/agentdrafter_viewer.js';
import { attachGestures, Glide, type Surface } from './gestures.ts';
import { fit, keepInSight, limits, zoomAt, type Area, type Limits, type Paper, type View } from './view.ts';
import { installProbe, type Draw, type Summary } from './probe.ts';

export interface DrawingProps {
  /** The text of the `display-list` reply to draw; null before one comes. */
  reply: string | null;
  /** Pixels at the bottom of the drawing the bottom sheet covers. */
  coveredBelow: number;
  /** Only the fit button, as on a phone, where fingers zoom. */
  compact: boolean;
  onLoaded(summary: Summary): void;
  /** A sheet could not be drawn. */
  onError(message: string): void;
  /** The renderer could not start, as where there is no WebGL2: nothing can be drawn. */
  onUnavailable(message: string): void;
}

/** Device pixels to a CSS pixel: the canvas's backing size over its size. */
const ratioOf = (canvas: HTMLCanvasElement): number =>
  canvas.clientWidth > 0 ? canvas.width / canvas.clientWidth : window.devicePixelRatio || 1;

export function Drawing({ reply, coveredBelow, compact, onLoaded, onError, onUnavailable }: DrawingProps) {
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
  const [, setReady] = useState(false);
  covered.current = coveredBelow;

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
  };
  const redraw = () => {
    if (!frame.current) frame.current = requestAnimationFrame(draw);
  };
  const area = (): Area => {
    const c = canvas.current;
    const width = c?.clientWidth ?? 0;
    const height = c?.clientHeight ?? 0;
    return { x: 0, y: 0, width, height: Math.max(120, height - covered.current) };
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
    else show(target);
  };
  const zoomBy = (factor: number) => {
    followFit.current = false;
    const [x, y] = middle();
    glide.current?.to(zoomAt(view.current, x, y, view.current.scale * factor, bounds.current), 220, [x, y]);
  };

  const load = (text: string): boolean => {
    const v = viewer.current;
    if (!v) return false;
    try {
      const summary = JSON.parse(v.load(text)) as Summary;
      loadedReply.current = text;
      paper.current = summary.paper;
      fitSheet(0);
      const probe = window.agentdrafterViewer;
      if (probe) probe.summary = summary;
      onLoaded(summary);
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

  // A new sheet: drawn, and fitted to the screen.
  useEffect(() => {
    if (reply && viewer.current && reply !== loadedReply.current) load(reply);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reply]);

  // The bottom sheet's peek changes the room the fitted sheet has.
  useEffect(() => {
    if (followFit.current) {
      fitSheet(0);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [coveredBelow]);

  return (
    <div className="viewport" ref={box}>
      <canvas className="stage" ref={canvas} aria-label="The sheet" />
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
        <button type="button" aria-label="Fit the sheet" onClick={() => fitSheet(450)}>
          &#x2922;
        </button>
      </div>
    </div>
  );
}
