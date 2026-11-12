// The viewer, in the look the reviewer picked (variant A): the drawing
// fills the screen, the drawing set's sheets are tabs across the top, and
// the journal's sheet peeks up from the bottom, a panel on the right on a
// laptop.

import { useCallback, useEffect, useRef, useState } from 'react';

import { CommandError, run } from './api.ts';
import { BottomSheet } from './BottomSheet.tsx';
import { Drawing } from './Drawing.tsx';
import type { Summary } from './probe.ts';

export interface AppProps {
  /** The token for this tab, if the server needs one. */
  token: string | null;
  /** The drawing set, sheet description or DWG to show, as the server names it. */
  source: string;
  /** The sheet to show first; the set's first without one. */
  sheet: string | null;
}

type Status = { kind: 'loading'; message: string } | { kind: 'shown' } | { kind: 'failed'; message: string };

const WIDE = '(min-width: 900px)';

function useWide(): boolean {
  const [wide, setWide] = useState(() => window.matchMedia(WIDE).matches);
  useEffect(() => {
    const media = window.matchMedia(WIDE);
    const changed = () => setWide(media.matches);
    media.addEventListener('change', changed);
    return () => media.removeEventListener('change', changed);
  }, []);
  return wide;
}

/** What to say when the server refused or failed. */
function why(e: unknown, token: string | null): string {
  if (e instanceof CommandError && e.status === 401) {
    return token
      ? 'The server did not take this tab’s token: it makes a new one each time it starts. Open the viewer again from the address agentdrafter serve printed when it started.'
      : 'This server listens beyond the local machine, so the viewer needs the token it printed when it started. Open the viewer from the address agentdrafter serve printed, which ends in #token=…';
  }
  return e instanceof Error ? e.message : String(e);
}

export function App({ token, source, sheet }: AppProps) {
  const wide = useWide();
  const [wanted, setWanted] = useState<string | null>(sheet);
  const [reply, setReply] = useState<string | null>(null);
  const [summary, setSummary] = useState<Summary | null>(null);
  const [status, setStatus] = useState<Status>({ kind: 'loading', message: 'Drawing the sheet…' });
  const [covered, setCovered] = useState(0);
  const [unavailable, setUnavailable] = useState<string | null>(null);
  const replies = useRef(new Map<string, string>());
  const tabs = useRef<HTMLElement>(null);

  // Each sheet asked for is drawn by the server once, then kept.
  useEffect(() => {
    let current = true;
    const key = (wanted ?? '').toUpperCase();
    const kept = replies.current.get(key);
    if (kept) {
      // A sheet drawn before is shown again at once: whatever another sheet
      // was saying, loading or failing, is not this one's to say. The
      // request for that other sheet, if it is still out, is let go.
      setReply(kept);
      setStatus({ kind: 'shown' });
      return;
    }
    setStatus({ kind: 'loading', message: wanted ? `Drawing ${wanted}…` : 'Drawing the sheet…' });
    const input: Record<string, string> = { source };
    if (wanted) input.sheet = wanted;
    run('display-list', input, token).then(
      (text) => {
        // Kept even when the reviewer has moved on, for when they come back.
        replies.current.set(key, text);
        if (!current) return;
        setReply(text);
      },
      (e) => current && setStatus({ kind: 'failed', message: why(e, token) }),
    );
    return () => {
      current = false;
    };
  }, [wanted, source, token]);

  const loaded = useCallback(
    (shown: Summary) => {
      setSummary(shown);
      setStatus({ kind: 'shown' });
      if (reply) replies.current.set(shown.sheet.toUpperCase(), reply);
      // The address names the sheet shown, so a reload shows it again.
      const query = new URLSearchParams(window.location.search);
      query.set('sheet', shown.sheet);
      window.history.replaceState(null, '', `${window.location.pathname}?${query}`);
    },
    [reply],
  );
  const failed = useCallback((message: string) => setStatus({ kind: 'failed', message }), []);

  // The tab shown stays in sight on a phone's narrow strip.
  useEffect(() => {
    tabs.current?.querySelector('.tab.on')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }, [summary]);

  const sheets = summary?.sheets ?? [];
  const name = summary?.name ?? '';
  return (
    <div className="va">
      <header className="va-top">
        <div className="va-row">
          <div className="va-set">
            <b>{name}</b>
            {summary && (
              <span>
                {sheets.length} sheet{sheets.length === 1 ? '' : 's'}
              </span>
            )}
          </div>
        </div>
        <nav className="va-tabs" ref={tabs} aria-label="Sheets">
          {sheets.map((s) => (
            <button
              key={s}
              type="button"
              className={`tab${s === summary?.sheet ? ' on' : ''}`}
              aria-current={s === summary?.sheet ? 'page' : undefined}
              onClick={() => setWanted(s)}
            >
              <b>{s}</b>
            </button>
          ))}
        </nav>
      </header>
      <div className="va-main">
        <div className="va-vp">
          <Drawing
            reply={reply}
            coveredBelow={covered}
            compact={!wide}
            onLoaded={loaded}
            onError={failed}
            onUnavailable={setUnavailable}
          />
          {unavailable ? (
            <div className="va-note failed" role="alert">
              The viewer cannot draw here: {unavailable}.
            </div>
          ) : (
            status.kind !== 'shown' && (
              <div className={`va-note ${status.kind}`} role={status.kind === 'failed' ? 'alert' : 'status'}>
                {status.message}
              </div>
            )
          )}
        </div>
        <BottomSheet wide={wide} onCovers={setCovered} />
      </div>
    </div>
  );
}
