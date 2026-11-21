// The guide's banner and its card in the chat (#157), as the prototype Zach
// picked on 2026-11-21 shows them: the banner across the top of the drawing,
// under the tabs on a phone, with the step, its words, what to do now, a
// gentle note when the drafter did something else, and Do it for me, Skip
// and Stop; at the end, what each step came to and one Undo of the guide's
// steps. The words are worked out in `guides.ts`.

import {
  cardWords,
  dots,
  lastWords,
  MARKS,
  summaryOf,
  type CardState,
  type Guide,
  type GuideRun,
  type Note,
  type Phase,
  nowWords,
} from './guides.ts';

/** Each step's dot, in the banner's head. */
function Dots({ run }: { run: GuideRun }) {
  return (
    <span className="g-dots" aria-hidden="true">
      {dots(run).map((d, i) => (
        <i key={i} className={d} />
      ))}
    </span>
  );
}

function NoteLine({ note, onAct }: { note: Note; onAct(): void }) {
  return (
    <div className={`g-note ${note.tone}`} data-gnote={note.tone}>
      <span>{note.words}</span>
      {note.act && (
        <button type="button" className="btn sm" data-g="noteact" onClick={onAct}>
          {note.act.label}
        </button>
      )}
    </div>
  );
}

export interface GuideBannerProps {
  guide: Guide;
  run: GuideRun;
  /** What the step wants next, as the renderer says it. */
  phase: Phase | null;
  phone: boolean;
  /** The guide's sheet is not the one shown. */
  elsewhere: boolean;
  /** The change sets an undo can take now, from the journal. */
  canUndo: number[];
  /** Do it for me can land the step now. */
  canDoForMe: boolean;
  busy: boolean;
  onDoForMe(): void;
  onSkip(): void;
  onStop(): void;
  onNoteAct(): void;
  onBack(): void;
  onUndoAll(from: number): void;
  onClose(): void;
}

export function GuideBanner(props: GuideBannerProps) {
  const { guide, run, phone } = props;
  const n = guide.steps.length;
  if (run.end) {
    const s = summaryOf(run, guide, props.canUndo);
    return (
      <div className="gbanner" role="region" aria-label="Guide" aria-live="polite" data-guide="end">
        <div className="g-main">
          <div className="g-head">
            <span className="g-tag">GUIDE</span>
            <b data-g-head>{s.head}</b>
            <Dots run={run} />
            {!phone && <span className="g-title">{guide.title}</span>}
          </div>
          <ol className="g-sum">
            {s.items.map((it, i) => (
              <li key={i} className={`gs-${it.status}`}>
                <span className="m">{MARKS[it.status]}</span>
                <span>
                  {it.title}
                  {it.small && <small> · {it.small}</small>}
                </span>
              </li>
            ))}
          </ol>
          <div className="g-say small" data-g-line>
            {s.line}
          </div>
        </div>
        <div className="g-acts">
          {s.undoFrom !== null && (
            <button type="button" className="btn sm danger" data-g="undoall" disabled={props.busy} onClick={() => props.onUndoAll(s.undoFrom as number)}>
              {s.undoLabel}
            </button>
          )}
          <button type="button" className="btn sm primary" data-g="close" onClick={props.onClose}>
            Close
          </button>
        </div>
      </div>
    );
  }
  const step = guide.steps[run.at];
  const head = (
    <div className="g-head">
      <span className="g-tag">GUIDE</span>
      <b data-g-step>
        Step {run.at + 1} of {n}
      </b>
      <Dots run={run} />
      {!phone && <span className="g-title">{guide.title}</span>}
    </div>
  );
  if (props.elsewhere) {
    return (
      <div className="gbanner" role="region" aria-label="Guide" data-guide="elsewhere">
        <div className="g-main">
          {head}
          <div className="g-say">This guide runs on {guide.sheet}.</div>
        </div>
        <div className="g-acts">
          <button type="button" className="btn sm guide" onClick={props.onBack}>
            Back to {guide.sheet}
          </button>
          <button type="button" className="btn sm" data-g="stop" onClick={props.onStop}>
            {phone ? 'Stop' : 'Stop guide'}
          </button>
        </div>
      </div>
    );
  }
  const last = phone ? null : lastWords(run);
  const note = run.note;
  return (
    <div className="gbanner" role="region" aria-label="Guide" aria-live="polite" data-guide="run" data-guide-step={run.at + 1}>
      <div className="g-main">
        {head}
        {last && <div className={`g-last${last.tone === 'skip' ? ' skip' : ''}`}>{last.text}</div>}
        <div className="g-say" data-g-say>
          {step.say}
        </div>
        {note?.tone !== 'warn' && (
          <div className="g-now" data-g-now>
            {nowWords(step, props.phase, phone)}
          </div>
        )}
        {note && <NoteLine note={note} onAct={props.onNoteAct} />}
      </div>
      <div className="g-acts">
        <button
          type="button"
          className="btn sm guide"
          data-g="forme"
          disabled={!props.canDoForMe || props.busy}
          title={props.canDoForMe ? 'Lands the step as you would' : 'It waits on an earlier step'}
          onClick={props.onDoForMe}
        >
          Do it for me
        </button>
        <button type="button" className="btn sm" data-g="skip" onClick={props.onSkip}>
          {phone ? 'Skip' : 'Skip step'}
        </button>
        <button type="button" className="btn sm" data-g="stop" onClick={props.onStop}>
          {phone ? 'Stop' : 'Stop guide'}
        </button>
      </div>
    </div>
  );
}

export interface GuideCardProps {
  guide: Guide | null;
  /** The guide's number, for one not listed yet. */
  number: number;
  run: GuideRun | null;
  onStart(): void;
  onStop(): void;
}

/** The guide's card in the chat, under the reply that gave it. */
export function GuideCard({ guide, number, run, onStart, onStop }: GuideCardProps) {
  if (!guide) {
    return (
      <div className="gcc" data-gcc="wait" data-guide-card={number}>
        <div className="gcc-h">
          <span className="g-tag">GUIDE</span>
          <b>Guide {number}</b>
        </div>
      </div>
    );
  }
  const mine = run && run.guide === guide.guide ? run : null;
  const state: CardState = !mine ? 'offer' : mine.end ? 'end' : 'run';
  const n = guide.steps.length;
  return (
    <div className="gcc" data-gcc={state} data-guide-card={guide.guide}>
      <div className="gcc-h">
        <span className="g-tag">GUIDE</span>
        <b title={guide.title}>{guide.title}</b>
        <span className="o">
          {n} step{n === 1 ? '' : 's'} · {guide.sheet}
        </span>
      </div>
      <ol className="gcc-l">
        {guide.steps.map((s, i) => {
          const st = mine?.status[i] ?? 'todo';
          const now = !!mine && !mine.end && mine.at === i;
          return (
            <li key={i} className={now ? 'gs-now' : `gs-${st}`}>
              <span className="m">{now ? '▸' : MARKS[st]}</span>
              <span>{s.title}</span>
            </li>
          );
        })}
      </ol>
      <div className="gcc-m">{cardWords(state, mine, n)}</div>
      <div className="g-acts start">
        {state === 'run' ? (
          <button type="button" className="btn sm" data-g="cardstop" onClick={onStop}>
            Stop the guide
          </button>
        ) : (
          <button type="button" className="btn sm guide" data-g="start" onClick={onStart}>
            {state === 'end' ? 'Start it again' : 'Start the guide'}
          </button>
        )}
      </div>
    </div>
  );
}
