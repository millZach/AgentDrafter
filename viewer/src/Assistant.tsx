// The assistant's panel (#156), as the prototype Zach picked on 2026-11-20
// lays it out (layout B): on the right on a laptop, in the bottom sheet on
// a phone. Its header names the agent, a picker when both are found; its
// tabs are Chat and Journal. The chat holds the drafter's messages, each
// saying what it carried of what the drafter looks at, and the assistant's
// replies: a line for each command it called, which opens to its input
// and reply, its words, and a card for each proposal it made, with Keep
// and Discard. Under the chat, the chip that says what the next message
// carries, and the input. Plain components: what they do is the page's
// (App.tsx), and what a command means is the core's.

import { Fragment, useLayoutEffect, useRef, useState, type ReactNode } from 'react';

import {
  agentHeader,
  agentName,
  carriedWords,
  chipWords,
  CHIP_BACK,
  CHIP_OFF,
  commandLine,
  endedWords,
  hasMessages,
  HEADER_SUB,
  inputPlaceholder,
  INTRO,
  noAgentWords,
  pretty,
  proposalEnd,
  proposalMade,
  proposedTitle,
  sheetsLink,
  viaWords,
  waitingWords,
  wordPieces,
  type AgentsReport,
  type AssistantItem,
  type CommandPart,
  type Conversation,
  type DrafterItem,
  type LookingAt,
} from './assistant.ts';
import type { JournalReport } from './journal.ts';
import { proposalTitle, type CardLine, type Proposal, type Standing } from './proposals.ts';

export type Side = 'chat' | 'journal';

/** How a waiting proposal stands, for its card in the chat: as build 7's card on the drawing says it. */
export interface ChatCardView {
  standing: Standing | null;
  lines: CardLine[];
  /** Its preview is on its way, on the sheet shown: Keep waits for it. */
  checking: boolean;
}

export interface AssistantPanelProps {
  /** The agents `serve` found, or null until it says. */
  agents: AgentsReport | null;
  /** Why the assistant cannot be reached here; null when it can. */
  unreachable: string | null;
  conversation: Conversation | null;
  /** What the drafter is looking at now, which the next message carries; null with no sheet shown. */
  look: LookingAt | null;
  /** × pressed: the next message leaves it off. */
  chipOff: boolean;
  onChipOff(off: boolean): void;
  side: Side;
  onSide(side: Side): void;
  journal: JournalReport | null;
  journalBody: ReactNode;
  proposals: readonly Proposal[];
  cardView(p: Proposal): ChatCardView;
  proposalBusy: boolean;
  onOpenProposal(p: Proposal): void;
  onKeep(p: Proposal): void;
  onDiscard(p: Proposal): void;
  /** Sends a message; false when it did not go, and its words stay in the input. */
  onSend(text: string): Promise<boolean>;
  onPickAgent(agent: string): void;
  onLookAgain(): void;
  lookingAgain: boolean;
  /** The input focused: a phone raises the sheet to full. */
  onInputFocus(): void;
}

export function AssistantPanel(props: AssistantPanelProps) {
  const { agents, unreachable, conversation, side, onSide, proposals } = props;
  const header = agentHeader(agents, conversation?.agent ?? null);
  const name = agentName(agents, conversation?.agent ?? null);
  const busy = !!conversation?.busy;
  const chat = useRef<HTMLDivElement>(null);
  // The chat keeps to its newest item while the drafter is at the bottom.
  const atBottom = useRef(true);
  const items = conversation?.items ?? [];
  // A message the drafter sends brings the chat back to its newest.
  const sent = items.filter((i) => i.kind === 'drafter').length;
  useLayoutEffect(() => {
    atBottom.current = true;
  }, [sent]);
  useLayoutEffect(() => {
    const el = chat.current;
    if (el && side === 'chat' && atBottom.current) el.scrollTop = el.scrollHeight;
  }, [conversation, side, proposals]);
  return (
    <>
      <div className="ahead">
        <div className="ahrow">
          <b>Assistant</b>
          <AgentPill header={header} busy={busy} onPick={props.onPickAgent} />
        </div>
        {(header.kind === 'one' || header.kind === 'picker') && (
          <div className="ahsub">
            {HEADER_SUB} <code>agentdrafter mcp</code>
          </div>
        )}
        <div className="sidetabs" role="tablist">
          <button
            type="button"
            role="tab"
            className={`stab${side === 'chat' ? ' on' : ''}`}
            aria-selected={side === 'chat'}
            data-side="chat"
            onClick={() => {
              atBottom.current = true;
              onSide('chat');
            }}
          >
            Chat
            {side !== 'chat' && proposals.length > 0 && (
              <span className="pcount" title={`${proposals.length} waiting proposal${proposals.length > 1 ? 's' : ''}`}>
                {proposals.length}
              </span>
            )}
          </button>
          <button
            type="button"
            role="tab"
            className={`stab${side === 'journal' ? ' on' : ''}`}
            aria-selected={side === 'journal'}
            data-side="journal"
            onClick={() => onSide('journal')}
          >
            Journal
          </button>
        </div>
      </div>
      <div
        className="sidebody chatbody"
        ref={chat}
        hidden={side !== 'chat'}
        data-chat
        onScroll={(e) => {
          const el = e.currentTarget;
          atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 30;
        }}
      >
        {unreachable ? (
          <p className="muted" data-unreachable>
            {unreachable}
          </p>
        ) : header.kind === 'none' && agents ? (
          <NoAgent report={agents} looking={props.lookingAgain} onLookAgain={props.onLookAgain} />
        ) : null}
        <div className="chat">
          {items.map((item, i) =>
            item.kind === 'note' ? (
              <div key={i} className="sys" data-item={i} data-kind="note">
                {item.text}
              </div>
            ) : item.kind === 'drafter' ? (
              <DrafterMessage key={i} index={i} item={item} />
            ) : (
              <AssistantMessage key={i} index={i} item={item} {...props} />
            ),
          )}
          {busy && items.at(-1)?.kind !== 'assistant' && name && <Working name={name} />}
          {!unreachable && name && !hasMessages(items) && (
            <div className="intro" data-intro>
              {INTRO}
            </div>
          )}
        </div>
      </div>
      <div className="sidebody" hidden={side !== 'journal'} data-journal-tab>
        {props.journalBody}
      </div>
      {side === 'chat' && <Composer {...props} name={unreachable ? null : name} busy={busy} />}
    </>
  );
}

function AgentPill({ header, busy, onPick }: { header: ReturnType<typeof agentHeader>; busy: boolean; onPick(agent: string): void }) {
  if (header.kind === 'loading') return null;
  if (header.kind === 'none') {
    return (
      <span className="agent none" data-agent="">
        <i />
        No agent found
      </span>
    );
  }
  if (header.kind === 'one') {
    return (
      <span className="agent" data-agent={header.agent} title="Started by AgentDrafter, pointed at agentdrafter mcp">
        <i />
        <b>{header.name}</b>
      </span>
    );
  }
  return (
    <span className="agent" data-agent={header.current} title="Started by AgentDrafter, pointed at agentdrafter mcp">
      <i />
      <select
        aria-label="The agent the assistant runs on"
        data-act="agent"
        value={header.current}
        disabled={busy}
        onKeyDown={(e) => e.stopPropagation()}
        onChange={(e) => onPick(e.currentTarget.value)}
      >
        {header.options.map((o) => (
          <option key={o.agent} value={o.agent}>
            {o.name}
          </option>
        ))}
      </select>
    </span>
  );
}

function NoAgent({ report, looking, onLookAgain }: { report: AgentsReport; looking: boolean; onLookAgain(): void }) {
  return (
    <div className="noagent" data-noagent>
      <h4>No agent found</h4>
      <p>{noAgentWords(report)}</p>
      <p>Install one, then look again:</p>
      {report.install.map((i) => (
        <div key={i.agent} className="inst" data-install={i.agent}>
          <b>{i.name}</b>
          <code>{i.line}</code>
          {i.page && (
            <a href={i.page} target="_blank" rel="noreferrer noopener">
              How to set it up ›
            </a>
          )}
        </div>
      ))}
      <div>
        <button type="button" className="btn primary" data-act="lookagain" disabled={looking} onClick={onLookAgain}>
          {looking ? 'Looking…' : 'Look again'}
        </button>
      </div>
    </div>
  );
}

function DrafterMessage({ index, item }: { index: number; item: DrafterItem }) {
  const via = viaWords(item);
  return (
    <div className="msg me" data-item={index} data-kind="drafter" data-via={item.via}>
      <span className="said">{item.text}</span>
      <span className={`ctx${item.looking_at ? '' : ' off'}`} data-ctx>
        {carriedWords(item)}
      </span>
      {via && <span className="via">{via}</span>}
    </div>
  );
}

function Working({ name }: { name: string }) {
  return (
    <div className="working" data-working>
      <span aria-hidden="true">
        <i />
        <i />
        <i />
      </span>
      {name} is working
    </div>
  );
}

type MessageProps = AssistantPanelProps & { index: number; item: AssistantItem };

/**
 * The assistant's reply, its parts in the order it made them: its
 * commands, a line each, and its words. A proposal it made comes as a card
 * after the words that follow its `propose`, as the prototype shows it.
 */
function AssistantMessage({ index, item, ...props }: MessageProps) {
  const [open, setOpen] = useState<ReadonlySet<number>>(new Set());
  const toggle = (k: number) => {
    const next = new Set(open);
    if (next.has(k)) next.delete(k);
    else next.add(k);
    setOpen(next);
  };
  const out: ReactNode[] = [];
  let lines: ReactNode[] = [];
  let cards: { n: number; part: CommandPart }[] = [];
  const flushLines = () => {
    if (lines.length) out.push(<div key={`l${out.length}`} className="tls">{lines}</div>);
    lines = [];
  };
  const flushCards = () => {
    for (const c of cards) out.push(<ChatProposal key={`c${c.n}`} n={c.n} part={c.part} {...props} />);
    cards = [];
  };
  item.parts.forEach((part, k) => {
    if (part.kind === 'words') {
      flushLines();
      if (part.text.trim()) {
        out.push(
          <div key={`w${k}`} className="ai-words" data-words>
            {wordPieces(part.text).map((p, j) => (p.bold ? <b key={j}>{p.text}</b> : <Fragment key={j}>{p.text}</Fragment>))}
          </div>,
        );
      }
      flushCards();
      return;
    }
    // A new run of commands: the cards of the last one come first.
    if (!lines.length) flushCards();
    const line = commandLine(part);
    const shown = open.has(k);
    lines.push(
      <div key={k} className="tl" data-tl={part.command}>
        <button type="button" className="tl-h" aria-expanded={shown} onClick={() => toggle(k)}>
          <span className={`tl-c${line.refused ? ' bad' : ''}`}>{line.command}</span>
          <span className="tl-w">· {line.words}</span>
          <span className="tl-x" aria-hidden="true">
            {line.running ? '…' : shown ? '▾' : '▸'}
          </span>
        </button>
        {shown && (
          <div className="tl-b">
            <small>Input</small>
            <pre data-tl-input>{pretty(part.input)}</pre>
            <small>Reply</small>
            <pre data-tl-reply>{pretty(part.reply)}</pre>
          </div>
        )}
      </div>,
    );
    const made = proposalMade(part);
    if (made !== null) cards.push({ n: made, part });
  });
  flushLines();
  flushCards();
  return (
    <div className="msg ai" data-item={index} data-kind="assistant">
      <div className="ai-who">
        <i />
        Assistant · {item.name}
      </div>
      {out}
      {!item.done && <Working name={item.name} />}
      {item.error && (
        <div className="ai-err" role="alert" data-error>
          {item.error}
        </div>
      )}
    </div>
  );
}

/**
 * A proposal the assistant made, where it made it: while it waits, its
 * number, what it does, its sheets, which open it as build 7's card does,
 * how it stands now, and Discard and Keep; once ended, how, from the
 * journal.
 */
function ChatProposal({ n, part, proposals, journal, cardView, proposalBusy, onOpenProposal, onKeep, onDiscard }: AssistantPanelProps & { n: number; part: CommandPart }) {
  const p = proposals.find((x) => x.proposal === n);
  if (!p) {
    const end = proposalEnd(journal, n);
    return (
      <div className={`pcc done${end?.how === 'kept' ? ' kept' : ''}`} data-pcc={n} data-ended={end?.how ?? ''}>
        <div className="pcc-h">
          <span className="pc-n">{n}</span>
          <span className="t">{proposedTitle(part)}</span>
          <span className="o">{end ? (end.how === 'kept' ? 'Kept' : 'Discarded') : ''}</span>
        </div>
        <div className="gone">{end ? endedWords(end) : `Proposal ${n}. Reading how it stands…`}</div>
      </div>
    );
  }
  const view = cardView(p);
  const refused = view.standing === 'bad';
  return (
    <div className={`pcc${refused ? ' bad' : ''}`} data-pcc={n} role="group" aria-label={`Proposal ${n}`}>
      <button type="button" className="pcc-h" title="Show it on each of its sheets" data-pcc-open onClick={() => onOpenProposal(p)}>
        <span className="pc-n">{n}</span>
        <span className="t">{proposalTitle(p)}</span>
        <span className="o">{sheetsLink(p.sheets)}</span>
      </button>
      <div className="pcc-m">{waitingWords(n)}</div>
      <div className="pcc-st">
        {view.lines.map((l) => (
          <div key={l.tone} className={`pl ${l.tone}`} data-pl={l.tone}>
            <i className={`vd ${l.tone}`} aria-hidden="true">
              {l.icon}
            </i>
            <span>{l.text}</span>
          </div>
        ))}
        {view.checking && <div className="pl muted">Checking whether it would land…</div>}
      </div>
      <div className="pcc-acts">
        <button type="button" className="btn sm" data-pc="discard" disabled={proposalBusy} onClick={() => onDiscard(p)}>
          Discard
        </button>
        <button
          type="button"
          className={`btn sm ${view.standing === 'warn' ? 'warn' : 'primary'}`}
          data-pc="keep"
          disabled={proposalBusy || view.checking || refused}
          title={refused ? 'The set would refuse it now' : `Keep it: it lands on ${p.sheets.length > 1 ? `${p.sheets.length} sheets` : (p.sheets[0] ?? 'its sheet')}`}
          onClick={() => onKeep(p)}
        >
          {refused ? 'Can’t keep' : 'Keep'}
        </button>
      </div>
    </div>
  );
}

/** The chip that says what the next message carries, the input and Send. */
function Composer({
  name,
  busy,
  look,
  chipOff,
  onChipOff,
  onSend,
  onInputFocus,
}: AssistantPanelProps & { name: string | null; busy: boolean }) {
  const [text, setText] = useState('');
  const box = useRef<HTMLTextAreaElement>(null);
  const fit = () => {
    const el = box.current;
    if (!el) return;
    el.style.height = '';
    el.style.height = `${Math.min(96, el.scrollHeight + 2)}px`;
  };
  const send = async () => {
    const t = text.trim();
    if (!t || !name || busy) return;
    setText('');
    if (box.current) box.current.style.height = '';
    // What did not go comes back, so nothing typed is lost.
    if (!(await onSend(t))) setText((now) => now || t);
  };
  return (
    <div className="composer" data-composer>
      {name && (
        <div className="ctxrow">
          {chipOff ? (
            <span className="ctxoff" data-chip-off>
              {CHIP_OFF}{' '}
              <button type="button" data-act="ctxon" onClick={() => onChipOff(false)}>
                {CHIP_BACK}
              </button>
            </span>
          ) : look ? (
            <span className="ctxchip" data-chip>
              <span>{chipWords(look)}</span>
              <button type="button" aria-label="Leave it off this message" title="Leave it off this message" data-act="ctxoff" onClick={() => onChipOff(true)}>
                &times;
              </button>
            </span>
          ) : null}
        </div>
      )}
      <div className="crow">
        <textarea
          ref={box}
          className="chatin"
          data-chatin
          rows={1}
          value={text}
          disabled={!name}
          placeholder={inputPlaceholder(name, busy)}
          aria-label="A message for the assistant"
          enterKeyHint="send"
          onFocus={onInputFocus}
          onChange={(e) => {
            setText(e.currentTarget.value);
            fit();
          }}
          onKeyDown={(e) => {
            // The input's keys are its own, not the command bar's.
            e.stopPropagation();
            if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing && e.keyCode !== 229) {
              e.preventDefault();
              void send();
            } else if (e.key === 'Escape') {
              e.preventDefault();
              e.currentTarget.blur();
            }
          }}
        />
        <button type="button" className="btn primary sendbtn" data-act="send" disabled={!name || busy || !text.trim()} onClick={() => void send()}>
          Send
        </button>
      </div>
    </div>
  );
}
