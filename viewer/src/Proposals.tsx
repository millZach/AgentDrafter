// A waiting proposal's card on the drawing (#155), as the prototype Zach
// picked on 2026-11-19 draws it (its variant B): at the change it marks,
// joined to it by a leader, who proposed it and what it does, how it
// stands now, and Keep and Discard. On a phone it is a small chip, which
// a tap opens into a short card.

import { whoWords } from './journal.ts';
import { ago, standingWords, type CardLine, type Proposal, type Standing } from './proposals.ts';

export interface ProposalCardProps {
  proposal: Proposal;
  title: string;
  lines: CardLine[];
  /** How it stands now; null while its preview is on its way. */
  standing: Standing | null;
  now: number;
  /** A phone's: a chip until it is tapped, then a short card. */
  compact: boolean;
  /** On a phone, whether the chip has been opened into its card. */
  open: boolean;
  onOpenChip(open: boolean): void;
  /** Shows it on each of its sheets, in the band. */
  onShow(): void;
  onKeep(): void;
  onDiscard(): void;
  /** While a keep or a discard is on its way. */
  busy: boolean;
}

const ICON: Record<Standing, string> = { ok: '✓', warn: '!', bad: '✕' };

export function ProposalCard({ proposal: p, title, lines, standing, now, compact, open, onOpenChip, onShow, onKeep, onDiscard, busy }: ProposalCardProps) {
  const n = p.sheets.length;
  const who = whoWords(p.by);
  const tone = standing ?? 'ok';
  if (compact && !open) {
    return (
      <button
        type="button"
        className={`pchip ${tone}`}
        data-pcard={p.proposal}
        data-pchip
        aria-label={`Proposal ${p.proposal} from ${who}: ${title}`}
        onClick={() => onOpenChip(true)}
      >
        <span className="pc-n">{p.proposal}</span>
        <span>{who}</span>
        {standing && (
          <i className={`vd ${standing}`} aria-hidden="true">
            {ICON[standing]}
          </i>
        )}
      </button>
    );
  }
  const refused = standing === 'bad';
  return (
    <div className={`pcard ${tone}${compact ? ' compact' : ''}`} data-pcard={p.proposal} role="group" aria-label={`Proposal ${p.proposal}`}>
      {compact && (
        <button type="button" className="pc-x" aria-label="Fold it back to its chip" onClick={() => onOpenChip(false)}>
          &times;
        </button>
      )}
      <button type="button" className="pc-head" title="Show it on each of its sheets" data-pc="show" onClick={onShow}>
        <span className="pc-n">{p.proposal}</span>
        <span className="pc-t">
          <b>{who}</b>
          {compact ? '' : ` · ${ago(p.at, now)}`}
        </span>
        <span className="pc-open">{n > 1 ? `${n} sheets ›` : '›'}</span>
      </button>
      <div className="pc-w">{title}</div>
      {compact ? (
        <div className="pc-s">
          {lines.map((l) => (
            <i key={l.tone} className={`vd ${l.tone}`} title={l.text} aria-hidden="true">
              {l.icon}
            </i>
          ))}
          <span>{standing ? standingWords[standing] : 'Checking…'}</span>
        </div>
      ) : (
        <div className="pc-st">
          {lines.map((l) => (
            <div key={l.tone} className={`pl ${l.tone}`} data-pl={l.tone}>
              <i className={`vd ${l.tone}`} aria-hidden="true">
                {l.icon}
              </i>
              <span>{l.text}</span>
            </div>
          ))}
          {!standing && <div className="pl muted">Checking whether it would land…</div>}
        </div>
      )}
      <div className="pc-acts">
        <button type="button" className="btn sm" data-pc="discard" disabled={busy} onClick={onDiscard}>
          Discard
        </button>
        <button
          type="button"
          className={`btn sm ${standing === 'warn' ? 'warn' : 'primary'}`}
          data-pc="keep"
          disabled={busy || !standing || refused}
          title={refused ? 'The set would refuse it now' : `Keep it: it lands on ${n > 1 ? `${n} sheets` : (p.sheets[0] ?? 'its sheet')}`}
          onClick={onKeep}
        >
          {refused ? 'Can’t keep' : 'Keep'}
        </button>
      </div>
    </div>
  );
}
