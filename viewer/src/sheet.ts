// Click guard for the mobile bottom sheet (#204): a tap on the grab or a
// moved drag translates the sheet during `pointerup`, so the browser's
// trailing synthetic `click` can land on a journal entry or button sliding
// under the finger. Arming the guard swallows that one trailing click in
// document capture phase; a new `pointerdown` clears it immediately so a
// subsequent real tap is never blocked even if the prior gesture emitted no
// click.

export const SWALLOW_MS = 400;

export class SheetClickGuard {
  private armedAt: number | null = null;

  arm(now: number): void {
    this.armedAt = now;
  }

  clear(): void {
    this.armedAt = null;
  }

  /** Returns true and disarms when a trailing click should be swallowed. */
  take(now: number): boolean {
    if (this.armedAt === null) return false;
    const swallow = now - this.armedAt < SWALLOW_MS;
    this.armedAt = null;
    return swallow;
  }
}

export interface SheetPointerResult {
  /** Whether the gesture moved the sheet or toggled its snap, so its trailing click must be swallowed. */
  swallowClick: boolean;
  /** Whether a tap without drag should toggle the sheet between peek and half. */
  toggle: boolean;
}

/** Classifies a bottom-sheet pointer lift/cancel into click-swallow and snap-toggle decisions. */
export function finishSheetGesture(moved: boolean, tap: boolean, cancelled: boolean): SheetPointerResult {
  if (moved) return { swallowClick: true, toggle: false };
  if (!cancelled && tap) return { swallowClick: true, toggle: true };
  return { swallowClick: false, toggle: false };
}
