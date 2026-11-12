// The bottom sheet of the picked look (variant A): on a phone it peeks up
// from the bottom with Undo last and Accept always in reach, and drags up
// to half or nearly all of the screen for the journal; on a laptop it is a
// panel on the right. The journal, Undo last and Accept themselves come
// with the next part of the viewer (#18): here they hold their places,
// switched off.

import { useEffect, useLayoutEffect, useRef, useState } from 'react';

type Snap = 'peek' | 'half' | 'full';

export interface BottomSheetProps {
  /** A laptop's width: a panel on the right, not a sheet. */
  wide: boolean;
  /** Says how many pixels at the bottom of the drawing the peek covers. */
  onCovers(pixels: number): void;
}

const clamp = (v: number, low: number, high: number) => Math.min(high, Math.max(low, v));

export function BottomSheet({ wide, onCovers }: BottomSheetProps) {
  const sheet = useRef<HTMLElement>(null);
  const grab = useRef<HTMLDivElement>(null);
  const peek = useRef<HTMLDivElement>(null);
  const [snap, setSnap] = useState<Snap>('peek');
  const drag = useRef<{ y0: number; s0: number; t: number; last: number; v: number; moved: boolean } | null>(null);
  const y = useRef(0);

  const room = () => sheet.current?.parentElement?.clientHeight ?? 0;
  const peekHeight = () => (grab.current?.offsetHeight ?? 0) + (peek.current?.offsetHeight ?? 0);
  const snapY = (name: Snap) => {
    const height = room();
    return name === 'peek' ? height - peekHeight() : name === 'half' ? Math.round(height * 0.45) : Math.round(height * 0.08);
  };
  const place = (top: number, animate: boolean) => {
    const el = sheet.current;
    if (!el) return;
    if (wide) {
      el.style.transform = '';
      el.style.height = '';
      return;
    }
    y.current = top;
    el.style.transition = animate ? 'transform .28s cubic-bezier(.2,.8,.2,1)' : 'none';
    el.style.transform = `translateY(${top}px)`;
    el.style.height = `${room() - top}px`;
  };
  const snapTo = (name: Snap, animate = true) => {
    setSnap(name);
    place(snapY(name), animate);
  };

  // Placed at its snap whenever the screen changes size.
  useLayoutEffect(() => {
    const fit = () => {
      place(snapY(snap), false);
      onCovers(wide ? 0 : peekHeight());
    };
    fit();
    const parent = sheet.current?.parentElement;
    if (!parent) return;
    const resized = new ResizeObserver(fit);
    resized.observe(parent);
    return () => resized.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wide, snap]);

  useEffect(() => {
    const el = sheet.current;
    if (!el) return;
    const down = (e: PointerEvent) => {
      if (wide || !(e.target as Element).closest('.va-grab, .va-peek') || (e.target as Element).closest('button')) return;
      drag.current = { y0: e.clientY, s0: y.current, t: performance.now(), last: e.clientY, v: 0, moved: false };
      el.setPointerCapture(e.pointerId);
    };
    const move = (e: PointerEvent) => {
      const d = drag.current;
      if (!d) return;
      const now = performance.now();
      d.v = (e.clientY - d.last) / Math.max(1, now - d.t);
      d.t = now;
      d.last = e.clientY;
      if (Math.abs(e.clientY - d.y0) > 6) d.moved = true;
      place(clamp(d.s0 + e.clientY - d.y0, snapY('full'), snapY('peek')), false);
    };
    const up = () => {
      const d = drag.current;
      if (!d) return;
      drag.current = null;
      if (!d.moved) {
        snapTo(snap === 'peek' ? 'half' : 'peek');
        return;
      }
      const order: Snap[] = ['full', 'half', 'peek'];
      let best: Snap = order.reduce<Snap>(
        (b, n) => (Math.abs(snapY(n) - y.current) < Math.abs(snapY(b) - y.current) ? n : b),
        'peek',
      );
      if (Math.abs(d.v) > 0.5) best = order[clamp(order.indexOf(snap) + (d.v > 0 ? 1 : -1), 0, 2)] ?? best;
      snapTo(best);
    };
    el.addEventListener('pointerdown', down);
    el.addEventListener('pointermove', move);
    el.addEventListener('pointerup', up);
    el.addEventListener('pointercancel', up);
    return () => {
      el.removeEventListener('pointerdown', down);
      el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerup', up);
      el.removeEventListener('pointercancel', up);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wide, snap]);

  return (
    <section className="va-sheet" ref={sheet} data-snap={snap} aria-label="Journal">
      <div className="va-grab" ref={grab}>
        <span />
      </div>
      <div className="va-peek" ref={peek}>
        <div className="va-agent">
          <i />
          Watching change sets land comes next
        </div>
        <div className="va-actions">
          <button type="button" className="btn" disabled title="Comes with the journal (#18)">
            &#x21B6; Undo last
          </button>
          <button type="button" className="btn primary" disabled title="Comes with the journal (#18)">
            Accept&hellip;
          </button>
        </div>
      </div>
      <div className="va-body">
        <div className="va-jhead">
          <b>Journal</b>
        </div>
        <p className="muted">
          The journal, Undo last and Accept come with the next part of the viewer (#18): change sets landing live,
          stepping through them, and accepting or undoing. Until then the viewer shows the drawing set&rsquo;s sheets
          as they are saved.
        </p>
      </div>
    </section>
  );
}
