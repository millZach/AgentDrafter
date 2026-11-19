import assert from 'node:assert/strict';
import { mock, test } from 'node:test';

import { attachGestures, Glide, HOLD_MS, type Surface } from '../src/gestures.ts';
import type { View } from '../src/view.ts';

// The gestures on a made-up element, with the clock in hand: what the
// surface is told, in order.
type Said = [string, ...unknown[]];

function rig() {
  const g = globalThis as unknown as Record<string, unknown>;
  g.window ??= globalThis;
  g.requestAnimationFrame ??= () => 0;
  g.cancelAnimationFrame ??= () => {};
  const listeners = new Map<string, (e: unknown) => void>();
  const element = {
    addEventListener: (type: string, fn: (e: unknown) => void) => listeners.set(type, fn),
    removeEventListener: (type: string) => listeners.delete(type),
    getBoundingClientRect: () => ({ left: 0, top: 0 }),
    setPointerCapture: () => {},
  };
  const said: Said[] = [];
  let view: View = { left: 0, top: 0, scale: 1 };
  const surface: Surface = {
    view: () => view,
    show: (v) => {
      view = v;
      said.push(['show']);
    },
    limits: () => ({ min: 1e-6, max: 1e6 }),
    touched: () => {},
    tapped: (at, pointer) => said.push(['tapped', at, pointer]),
    held: (at) => said.push(['held', at]),
    holdMoved: (at) => said.push(['holdMoved', at]),
    holdEnded: (at, taken) => said.push(['holdEnded', at, taken]),
    holdTakenBack: () => said.push(['holdTakenBack']),
  };
  const detach = attachGestures(element as unknown as HTMLElement, surface, new Glide(surface));
  let now = 1000;
  const fire = (type: string, id: number, x: number, y: number, pointerType = 'touch', after = 8) => {
    now += after;
    listeners.get(type)?.({
      type,
      pointerId: id,
      pointerType,
      button: 0,
      clientX: x,
      clientY: y,
      timeStamp: now,
      target: null,
      preventDefault: () => {},
    });
  };
  // The clock moves on `ms`, as the page's events' times do.
  const wait = (ms: number) => {
    now += ms;
    mock.timers.tick(ms);
  };
  return { said, fire, wait, detach, kinds: () => said.map((s) => s[0]) };
}

test('a hold comes where the finger rests, after it has moved on short of panning', () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  try {
    const { said, fire, wait, detach } = rig();
    fire('pointerdown', 1, 100, 100);
    // 5 pixels on, toward a snap: more than a tremor, less than a pan.
    fire('pointermove', 1, 103, 100);
    fire('pointermove', 1, 105, 100);
    wait(HOLD_MS + 10);
    assert.deepEqual(said.find((s) => s[0] === 'held'), ['held', [105, 100]]);
    assert.ok(!said.some((s) => s[0] === 'show'), 'it did not pan');
    detach();
  } finally {
    mock.timers.reset();
  }
});

test('a second finger ends a hold with nothing taken and pinches', () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  try {
    const { fire, wait, detach, kinds } = rig();
    fire('pointerdown', 1, 100, 100);
    wait(HOLD_MS + 10);
    assert.deepEqual(kinds(), ['held']);
    fire('pointerdown', 2, 200, 100);
    fire('pointermove', 2, 240, 100);
    fire('pointerup', 2, 240, 100);
    fire('pointermove', 1, 90, 100);
    fire('pointerup', 1, 90, 100);
    const after = kinds();
    assert.equal(after[1], 'holdTakenBack');
    assert.ok(after.includes('show'), 'the two fingers pinched');
    for (const never of ['holdMoved', 'holdEnded', 'tapped']) assert.ok(!after.includes(never), `${never}: ${after}`);
    detach();
  } finally {
    mock.timers.reset();
  }
});

test('a drag that panned is never also a tap, by finger or by mouse', () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  try {
    for (const pointer of ['touch', 'mouse']) {
      const { fire, detach, kinds } = rig();
      fire('pointerdown', 1, 100, 100, pointer);
      fire('pointermove', 1, 104, 100, pointer);
      fire('pointermove', 1, 108, 100, pointer);
      fire('pointerup', 1, 108, 100, pointer);
      assert.ok(kinds().includes('show'), `${pointer}: it panned`);
      assert.ok(!kinds().includes('tapped'), `${pointer}: ${kinds()}`);
      detach();
    }
    // A click with a jitter short of panning is a click.
    const { fire, detach, kinds } = rig();
    fire('pointerdown', 1, 100, 100, 'mouse');
    fire('pointermove', 1, 102, 101, 'mouse');
    fire('pointerup', 1, 102, 101, 'mouse');
    assert.deepEqual(kinds(), ['tapped']);
    detach();
  } finally {
    mock.timers.reset();
  }
});

test('a finger held still, its lift reaching a busy page before the hold was shown, is a hold and a lift', () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  try {
    const { said, fire, detach, kinds } = rig();
    // Down and up handled back to back, their own times 400 ms apart; the
    // hold's timer has not fired.
    fire('pointerdown', 1, 100, 100);
    fire('pointerup', 1, 101, 100, 'touch', 400);
    assert.deepEqual(kinds(), ['held', 'holdEnded']);
    assert.deepEqual(said[1], ['holdEnded', [101, 100], true]);
    detach();
    // A quick tap is still a tap.
    const quick = rig();
    quick.fire('pointerdown', 1, 100, 100);
    quick.fire('pointerup', 1, 100, 100, 'touch', 120);
    assert.deepEqual(quick.kinds(), ['tapped']);
    quick.detach();
  } finally {
    mock.timers.reset();
  }
});
