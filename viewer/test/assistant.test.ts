import assert from 'node:assert/strict';
import { test } from 'node:test';

import { assistantRequest, readEvents } from '../src/api.ts';
import {
  agentHeader,
  agentName,
  askHint,
  carriedWords,
  chipWords,
  commandLine,
  cut,
  endedWords,
  hasMessages,
  inputPlaceholder,
  lookedFor,
  lookingAtOf,
  lookKey,
  noAgentWords,
  notSentWords,
  peekAskWords,
  pretty,
  proposalEnd,
  proposalMade,
  proposalsMade,
  proposedSheets,
  proposedTitle,
  sheetsLink,
  spaceIsEnter,
  typedRoute,
  viaWords,
  waitingWords,
  wordPieces,
  type AgentsReport,
  type CommandPart,
  type Conversation,
  type DrafterItem,
} from '../src/assistant.ts';
import type { JournalReport } from '../src/journal.ts';

// The renderer's word for a tool: its aliases and names, any case, as
// crates/agentdrafter-viewer/src/tools.rs lists them.
const ALIASES = ['L', 'LINE', 'C', 'CIRCLE', 'REC', 'RECTANG', 'RECTANGLE', 'T', 'DT', 'TEXT', 'DTEXT', 'M', 'MOVE', 'E', 'ERASE'];
const isToolWord = (w: string) => ALIASES.includes(w.toUpperCase());
const toolName = (w: string) => ({ MOVE: 'Move', LINE: 'Line', CIRCLE: 'Circle', ERASE: 'Erase', TEXT: 'Text' })[w.toUpperCase()] ?? null;

const BOTH: AgentsReport = {
  agents: [
    { agent: 'claude', name: 'Claude Code', command: 'claude', found: true },
    { agent: 'codex', name: 'Codex', command: 'codex', found: true },
  ],
  install: [
    { agent: 'claude', name: 'Claude Code', line: 'npm install -g @anthropic-ai/claude-code', page: 'https://docs.anthropic.com/en/docs/claude-code/setup' },
    { agent: 'codex', name: 'Codex', line: 'npm install -g @openai/codex', page: 'https://github.com/openai/codex' },
  ],
};
const found = (claude: boolean, codex: boolean): AgentsReport => ({
  ...BOTH,
  agents: [
    { ...(BOTH.agents[0] as AgentsReport['agents'][0]), found: claude },
    { ...(BOTH.agents[1] as AgentsReport['agents'][0]), found: codex },
  ],
});

const command = (name: string, input: unknown, reply: CommandPart['reply'], ok: boolean | null = reply ? reply.ok : null): CommandPart => ({
  kind: 'command',
  command: name,
  input,
  reply,
  ok,
});

test('the chip says the sheet and what is selected', () => {
  assert.equal(chipWords({ sheet: 'DP-LD-101', layout: null, selected: ['DP-LD-101/device'], names: ['FT-101'] }), 'Sheet DP-LD-101 · 1 selected: FT-101');
  assert.equal(chipWords({ sheet: 'DP-LD-101', layout: null, selected: [], names: [] }), 'Sheet DP-LD-101 · nothing selected');
  assert.equal(chipWords({ sheet: 'LOOPS', layout: 'Wiring', selected: [], names: [] }), 'Sheet LOOPS, layout Wiring · nothing selected');
  assert.equal(
    chipWords({ sheet: 'DP-LD-101', layout: null, selected: ['a', 'b', 'c'], names: ['FT-101', 'C-101', 'JB-100'] }),
    'Sheet DP-LD-101 · 3 selected: FT-101, C-101 and 1 more',
  );
});

test('what the drafter looks at: an object with no ID is left out, and its name is its ID without one', () => {
  assert.equal(lookingAtOf(null, null, null), null);
  assert.deepEqual(lookingAtOf('DP-LD-101', null, { id: 'DP-LD-101/device', name: 'FT-101' }), {
    sheet: 'DP-LD-101',
    layout: null,
    selected: ['DP-LD-101/device'],
    names: ['FT-101'],
  });
  assert.deepEqual(lookingAtOf('DP-LD-101', null, { name: 'no id' }), { sheet: 'DP-LD-101', layout: null, selected: [], names: [] });
  assert.deepEqual(lookingAtOf('DP-LD-101', null, { id: 'DP-LD-101/x' })?.names, ['DP-LD-101/x']);
  // The key moves only with the sheet, the layout and the selection.
  const a = lookingAtOf('DP-LD-101', null, { id: 'DP-LD-101/device', name: 'FT-101' });
  assert.equal(lookKey(a), lookKey(lookingAtOf('dp-ld-101', null, { id: 'DP-LD-101/device', name: 'other' })));
  assert.notEqual(lookKey(a), lookKey(lookingAtOf('DP-LD-101', null, null)));
  assert.notEqual(lookKey(a), lookKey(lookingAtOf('DP-LD-101', 'Wiring', { id: 'DP-LD-101/device' })));
  assert.equal(lookKey(null), '');
});

test('a sent message says what it carried, and that it came from the command bar', () => {
  const sent: DrafterItem = {
    kind: 'drafter',
    text: 'move this 1 to the right',
    via: 'chat',
    looking_at: { sheet: 'DP-LD-101', layout: null, selected: ['DP-LD-101/device'], names: ['FT-101'] },
    at: '2026-11-20T10:21:03Z',
  };
  assert.equal(carriedWords(sent), 'with Sheet DP-LD-101 · 1 selected: FT-101');
  assert.equal(viaWords(sent), null);
  const bare: DrafterItem = { ...sent, via: 'command_bar', looking_at: null };
  assert.equal(carriedWords(bare), 'without what you’re looking at');
  assert.equal(viaWords(bare), 'from the command bar');
});

test('a command line for looking-at, query and propose', () => {
  const look = commandLine(
    command('looking-at', { set: 'demo-plant' }, { ok: true, result: { sheet: 'DP-LD-101', layout: null, selected: ['DP-LD-101/device'], at: '2026-11-20T10:21:03Z' } }),
  );
  assert.deepEqual(look, { command: 'looking-at', words: 'Sheet DP-LD-101 · 1 selected', refused: false, running: false });
  assert.equal(commandLine(command('looking-at', {}, { ok: true, result: { sheet: null, layout: null, selected: [], at: null } })).words, 'nothing recorded yet');
  assert.equal(commandLine(command('looking-at', {}, { ok: true, result: { sheet: 'DP-LD-101', layout: null, selected: [] } })).words, 'Sheet DP-LD-101 · nothing selected');
  const query = commandLine(command('query', { source: 'demo-plant', query: { kind: 'find', id: 'DP-LD-101/device' } }, { ok: true, result: { count: 1 } }));
  assert.equal(`${query.command} · ${query.words}`, 'query · find DP-LD-101/device');
  assert.equal(commandLine(command('query', { source: 'demo-plant', query: { kind: 'attributes', block: 'TRANSMITTER' } }, null)).words, 'attributes block TRANSMITTER');
  assert.equal(commandLine(command('query', { source: 'demo-plant', sheet: 'DP-LD-102', query: { kind: 'extents' } }, null)).words, 'extents on DP-LD-102');
  const propose = commandLine(
    command(
      'propose',
      { set: 'demo-plant', change_set: { operations: [{ op: 'move', target: { id: 'DP-LD-101/device' }, by: [1, 0, 0] }] } },
      { ok: true, result: { proposal: 1, sheets: ['DP-LD-101'] } },
    ),
  );
  assert.equal(`${propose.command} · ${propose.words}`, 'propose · move DP-LD-101/device → proposal 1');
  const two = commandLine(
    command(
      'propose',
      {
        change_set: {
          operations: [
            { op: 'set_attribute', target: { id: 'DP-LD-101/title-block' }, tag: 'CHECKED', value: 'JK' },
            { op: 'move', target: { name: 'service' } },
          ],
        },
      },
      null,
    ),
  );
  assert.equal(two.words, 'set CHECKED = JK on DP-LD-101/title-block and 1 more');
  assert.equal(two.running, true);
});

test('a command line for proposals, journal, check, display-list and any other', () => {
  assert.equal(commandLine(command('proposals', { set: 'demo-plant' }, { ok: true, result: { proposals: [{}, {}] } })).words, '2 waiting');
  assert.equal(commandLine(command('journal', { set: 'demo-plant' }, { ok: true, result: { entries: [{}] } })).words, '1 entry');
  assert.equal(commandLine(command('journal', { set: 'demo-plant' }, { ok: true, result: { entries: [{}, {}, {}] } })).words, '3 entries');
  assert.equal(commandLine(command('check', { source: 'demo-plant', sheets: ['DP-LD-101'] }, { ok: true, result: { findings: [] } })).words, 'DP-LD-101 · all pass');
  assert.equal(commandLine(command('check', { source: 'demo-plant' }, { ok: true, result: { findings: [{}, {}] } })).words, 'every sheet · 2 findings');
  assert.equal(commandLine(command('display-list', { source: 'demo-plant', sheet: 'DP-LD-101' }, { ok: true, result: {} })).words, 'DP-LD-101');
  assert.equal(commandLine(command('keep', { set: 'demo-plant', proposal: 3 }, { ok: true, result: {} })).words, 'proposal 3');
  // A tool that is not AgentDrafter's keeps its own name; its input is shown short.
  const other = commandLine(command('Read', { file_path: '/a/very/long/path/that/goes/on/and/on/and/on/to/a/file/somewhere.yaml' }, { ok: true, result: 'x' }));
  assert.equal(other.command, 'Read');
  assert.ok(other.words.startsWith('{"file_path":"/a/very/long') && other.words.endsWith('…') && other.words.length === 60, other.words);
  assert.equal(cut('short'), 'short');
});

test('a refused command says why, in the core’s words, its name red; a running one says it runs', () => {
  const refused = commandLine(
    command('change', { set: 'demo-plant', change_set: { operations: [{ op: 'move', target: { id: 'DP-LD-101/device' } }] } }, {
      ok: false,
      error: { code: 'unknown_command', message: 'there is no command `change` here' },
    }),
  );
  assert.equal(refused.refused, true);
  assert.equal(refused.words, 'move DP-LD-101/device: refused, there is no command change here');
  const running = commandLine(command('looking-at', { set: 'demo-plant' }, null, null));
  assert.equal(running.running, true);
  assert.equal(running.refused, false);
});

test('an input or reply opened is pretty JSON, a short list of numbers on one line', () => {
  assert.equal(pretty({ by: [1, 0, 0] }), '{\n  "by": [1, 0, 0]\n}');
  assert.equal(pretty(null), '…');
  assert.equal(pretty({ ok: true }), '{\n  "ok": true\n}');
});

test('which proposal a part made: a propose whose reply is ok', () => {
  const made = command('propose', { change_set: { operations: [{ op: 'move', target: { id: 'DP-LD-101/device' } }] } }, { ok: true, result: { proposal: 4, sheets: ['DP-LD-101'] } });
  assert.equal(proposalMade(made), 4);
  assert.equal(proposedTitle(made), 'Move device');
  assert.deepEqual(proposedSheets(made), ['DP-LD-101']);
  assert.equal(proposalMade(command('propose', {}, { ok: false, error: { code: 'usage' } })), null);
  assert.equal(proposalMade(command('propose', {}, null)), null);
  assert.equal(proposalMade(command('query', {}, { ok: true, result: { proposal: 1 } })), null);
  assert.equal(proposalMade({ kind: 'words', text: 'proposal 1' }), null);
  const items: Conversation['items'] = [
    { kind: 'note', text: 'On Claude Code', at: '' },
    { kind: 'assistant', agent: 'claude', name: 'Claude Code', at: '', parts: [made, { kind: 'words', text: 'Proposed.' }], done: true, error: null },
  ];
  assert.deepEqual(proposalsMade(items), [4]);
  assert.equal(hasMessages(items), true);
  assert.equal(hasMessages([{ kind: 'note', text: 'On Claude Code', at: '' }]), false);
});

test('a proposal’s card: its sheets, its words, and how it ended, from the journal', () => {
  assert.equal(sheetsLink(['DP-LD-101']), 'DP-LD-101 ›');
  assert.equal(sheetsLink(['DP-LD-101', 'DP-LD-102', 'DP-LD-104', 'DP-LD-201', 'DP-LD-202', 'DP-LD-301', 'DP-LD-302']), '7 sheets ›');
  assert.equal(waitingWords(1), 'Proposal 1, dashed on the drawing. Nothing changes until you keep it.');
  const journal: JournalReport = {
    set: 'demo-plant',
    message: '',
    can_undo: [1],
    entries: [
      { entry: 1, kind: 'change_set', at: '', proposal: 1, proposed_by: 'Assistant', kept_by: 'drafter' },
      { entry: 2, kind: 'discard', at: '', proposal: 2, proposed_by: 'Assistant', discarded_by: 'drafter' },
      { entry: 3, kind: 'failed_attempt', at: '', proposal: 3, kept_by: 'drafter' },
    ],
  };
  const kept = proposalEnd(journal, 1);
  assert.deepEqual(kept, { how: 'kept', entry: 1, by: 'drafter', undoneBy: undefined });
  assert.equal(endedWords(kept!), 'Kept by you: change set 1, in the journal.');
  assert.equal(endedWords(proposalEnd(journal, 2)!), 'Discarded by you: nothing changed.');
  // A keep the set refused leaves it waiting: it has not ended.
  assert.equal(proposalEnd(journal, 3), null);
  assert.equal(proposalEnd(null, 1), null);
  assert.equal(endedWords({ how: 'kept', entry: 5, by: 'agent (MCP)', undoneBy: 6 }), 'Kept by agent (MCP): change set 5, in the journal, undone by entry 6.');
});

test('the agent’s words are plain text: only **bold** is read, and nothing becomes HTML', () => {
  assert.deepEqual(wordPieces('Proposed: move **FT-101** 1 to the right.\nIt waits.'), [
    { text: 'Proposed: move ', bold: false },
    { text: 'FT-101', bold: true },
    { text: ' 1 to the right.\nIt waits.', bold: false },
  ]);
  assert.deepEqual(wordPieces('<b>not</b> *this*'), [{ text: '<b>not</b> *this*', bold: false }]);
  assert.deepEqual(wordPieces(''), []);
});

test('the command bar: a tool’s word starts it, other words go to the assistant', () => {
  assert.equal(typedRoute('L', false, false, isToolWord), 'start');
  assert.equal(typedRoute('move', false, false, isToolWord), 'start');
  assert.equal(typedRoute('  rec ', false, false, isToolWord), 'start');
  assert.equal(typedRoute('what is this', false, false, isToolWord), 'assistant');
  assert.equal(typedRoute('move this 1 to the right', false, false, isToolWord), 'assistant');
  assert.equal(typedRoute('Ölwechsel', false, false, isToolWord), 'assistant');
  // No letter: the renderer says what it is.
  assert.equal(typedRoute('1,2', false, false, isToolWord), 'start');
  // Inside a tool everything is the tool's; nothing typed is Enter.
  assert.equal(typedRoute('what is this', true, false, isToolWord), 'input');
  assert.equal(typedRoute('', false, false, isToolWord), 'input');
  assert.equal(typedRoute('   ', false, false, isToolWord), 'input');
  // Nothing goes anywhere while a preview is shown.
  assert.equal(typedRoute('what is this', false, true, isToolWord), 'refused');
});

test('Space is Enter for nothing typed, a short alias, or inside a tool; a whole word waits for Enter', () => {
  for (const alias of ['L', 'c', 'REC', 'M', 'E', 'T', 'DT']) assert.equal(spaceIsEnter(alias, false, isToolWord), true, alias);
  assert.equal(spaceIsEnter('', false, isToolWord), true);
  assert.equal(spaceIsEnter('MOVE', false, isToolWord), false);
  assert.equal(spaceIsEnter('erase', false, isToolWord), false);
  assert.equal(spaceIsEnter('what', false, isToolWord), false);
  assert.equal(spaceIsEnter('abc', false, isToolWord), false, 'three letters that are no alias');
  assert.equal(spaceIsEnter('1,2', false, isToolWord), false);
  assert.equal(spaceIsEnter('anything at all', true, isToolWord), true);
});

test('the command bar’s hint', () => {
  assert.equal(askHint('what is this', false, false, isToolWord, toolName), 'Enter: ask the assistant');
  assert.equal(askHint('MOVE', false, false, isToolWord, toolName), 'Enter: Move · keep typing to ask the assistant');
  assert.equal(askHint('dtext', false, false, isToolWord, toolName), 'Enter: Dtext · keep typing to ask the assistant');
  assert.equal(askHint('M', false, false, isToolWord, toolName), '');
  assert.equal(askHint('REC', false, false, isToolWord, toolName), '');
  assert.equal(askHint('', false, false, isToolWord, toolName), '');
  assert.equal(askHint('1,2', false, false, isToolWord, toolName), '');
  assert.equal(askHint('what is this', true, false, isToolWord, toolName), '', 'not inside a tool');
  assert.equal(askHint('what is this', false, true, isToolWord, toolName), '', 'not with a preview open');
});

test('the header’s agent: a picker for both, the one found, or none', () => {
  assert.deepEqual(agentHeader(null, null), { kind: 'loading' });
  assert.deepEqual(agentHeader(BOTH, 'codex'), {
    kind: 'picker',
    current: 'codex',
    options: [
      { agent: 'claude', name: 'Claude Code' },
      { agent: 'codex', name: 'Codex' },
    ],
  });
  assert.equal((agentHeader(BOTH, null) as { current: string }).current, 'claude', 'the first found until the conversation says');
  assert.deepEqual(agentHeader(found(false, true), 'codex'), { kind: 'one', agent: 'codex', name: 'Codex' });
  assert.deepEqual(agentHeader(found(false, false), null), { kind: 'none' });
  assert.equal(agentName(BOTH, 'codex'), 'Codex');
  assert.equal(agentName(found(true, false), 'claude'), 'Claude Code');
  assert.equal(agentName(found(false, false), null), null);
  assert.equal(agentName(null, null), null);
});

test('the words with no agent, the input and the peek', () => {
  assert.equal(lookedFor(found(false, false)), 'Claude Code (claude) and Codex (codex), and found neither');
  assert.equal(
    noAgentWords(found(false, false)),
    'The assistant runs on an agent you already have, signed in as you are now: no API key goes in AgentDrafter. When it started, AgentDrafter looked on your PATH for Claude Code (claude) and Codex (codex), and found neither.',
  );
  assert.equal(inputPlaceholder(null, false), 'No agent yet: install one first');
  assert.equal(inputPlaceholder('Claude Code', false), 'Ask, or say what to change…');
  assert.equal(inputPlaceholder('Claude Code', true), 'Claude Code is working…');
  assert.equal(peekAskWords('Claude Code', false, true), 'Ask the assistant…');
  assert.equal(peekAskWords('Codex', true, true), 'Codex is working…');
  assert.equal(peekAskWords(null, false, true), 'No agent found: how to install one ›');
  assert.equal(peekAskWords(null, false, false), 'Ask the assistant…');
  assert.equal(notSentWords('assistant_busy', 'busy', 'Claude Code'), 'Claude Code is still working: one message at a time.');
  assert.equal(notSentWords('no_agent', 'none', null), 'No agent was found: install Claude Code or Codex first, as the panel says.');
  assert.equal(notSentWords('usage', 'the text is empty', 'Codex'), 'the text is empty');
});

test('the assistant’s routes carry the token, and its stream’s events are read as the journal’s', () => {
  const get = assistantRequest('', null, 'tok');
  assert.equal(get.url, '/assistant');
  assert.equal(get.init.method, 'GET');
  assert.equal(get.init.body, undefined);
  assert.equal((get.init.headers as Record<string, string>).Authorization, 'Bearer tok');
  const post = assistantRequest('messages', { set: 'demo-plant', text: 'hi', via: 'chat', looking_at: null }, null);
  assert.equal(post.url, '/assistant/messages');
  assert.equal(post.init.method, 'POST');
  assert.equal(post.init.body, '{"set":"demo-plant","text":"hi","via":"chat","looking_at":null}');
  assert.equal((post.init.headers as Record<string, string>).Authorization, undefined);
  assert.equal(assistantRequest('find', undefined, null).init.body, '{}');
  assert.equal(assistantRequest('events', { set: 'demo-plant' }, null).url, '/assistant/events');
  const { events } = readEvents(': still watching\n\nevent: assistant\ndata: {"ok":true,"result":{"busy":false}}\n\n');
  assert.deepEqual(events, [{ event: 'assistant', data: '{"ok":true,"result":{"busy":false}}' }]);
});
