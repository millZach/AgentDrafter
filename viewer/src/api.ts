// How the page reaches the HTTP API: the commands of the core's registry,
// at POST /commands/<name>, on the server that served the page.
//
// When `agentdrafter serve` listens beyond the local machine, every
// command needs the token it printed when it started. Its start line gives
// the viewer's address with the token in the fragment, `#token=...`,
// which a browser never sends to a server. The page takes it from there
// once, keeps it in this tab's session storage, and takes it off the
// address shown, so it is not left in the address bar, the history or a
// shared link. It is sent only as `Authorization: Bearer`, never in a query
// string.

/** Where this tab keeps the token. */
export const TOKEN_KEY = 'agentdrafter-token';

export interface Place {
  pathname: string;
  search: string;
  hash: string;
}

export interface KeepsText {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export interface RewritesAddress {
  replaceState(data: unknown, unused: string, url: string): void;
}

/**
 * The token for this tab: the one in the address's fragment, which is then
 * kept and taken off the address, or the one kept before.
 */
export function takeToken(place: Place, storage: KeepsText, history: RewritesAddress): string | null {
  const fragment = new URLSearchParams(place.hash.replace(/^#/, ''));
  const given = fragment.get('token');
  if (given !== null) {
    fragment.delete('token');
    const rest = fragment.toString();
    history.replaceState(null, '', place.pathname + place.search + (rest ? `#${rest}` : ''));
    if (given) {
      storage.setItem(TOKEN_KEY, given);
      return given;
    }
  }
  return storage.getItem(TOKEN_KEY);
}

/**
 * What the page shows, from its address's query: `source`, a drawing set's
 * folder, a sheet description or a DWG, as the server's own folder names
 * it (by default the folder the server started in), `sheet`, which of its
 * sheets (by default the first), and `layout`, which layout of a sheet
 * drawn in several (by default its first).
 */
export function wanted(search: string): { source: string; sheet: string | null; layout: string | null } {
  const query = new URLSearchParams(search);
  return {
    source: query.get('source') || '.',
    sheet: query.get('sheet') || null,
    layout: query.get('layout') || null,
  };
}

/** A command's request: where it goes and what it carries. */
export function request(name: string, input: unknown, token: string | null): { url: string; init: RequestInit } {
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  if (token) headers.Authorization = `Bearer ${token}`;
  return {
    url: `/commands/${encodeURIComponent(name)}`,
    init: { method: 'POST', headers, body: JSON.stringify(input), cache: 'no-store', credentials: 'omit' },
  };
}

/** Why the server refused or failed a command, for the page to say. */
export class CommandError extends Error {
  readonly status: number;
  readonly code: string;
  /** What the reply says was done, when it says: an accept that found failed checks lists them. */
  readonly result: unknown;
  constructor(status: number, code: string, message: string, result: unknown = undefined) {
    super(message);
    this.status = status;
    this.code = code;
    this.result = result;
  }
}

/**
 * Runs a command and gives its reply's text when it is ok; otherwise a
 * CommandError with the reply's error.
 */
export async function run(
  name: string,
  input: unknown,
  token: string | null,
  send: typeof fetch = fetch,
): Promise<string> {
  const { url, init } = request(name, input, token);
  return replyOf(url, init, send);
}

/** Sends a request and gives its reply's text when it is ok; otherwise a CommandError with the reply's error. */
async function replyOf(url: string, init: RequestInit, send: typeof fetch): Promise<string> {
  let response: Response;
  try {
    response = await send(url, init);
  } catch (e) {
    throw new CommandError(0, 'unreachable', `the server could not be reached: ${(e as Error).message}`);
  }
  const text = await response.text();
  if (response.ok) return text;
  let code = 'failed';
  let message = `the server answered ${response.status}`;
  let result: unknown;
  try {
    const reply = JSON.parse(text) as { result?: unknown; error?: { code?: string; message?: string } };
    code = reply.error?.code ?? code;
    message = reply.error?.message ?? message;
    result = reply.result;
  } catch {
    // Not a JSON reply: say the status.
  }
  throw new CommandError(response.status, code, message, result);
}

/** Runs a command and gives its reply's result, read; a CommandError when it is not ok. */
export async function call<T>(
  name: string,
  input: unknown,
  token: string | null,
  send: typeof fetch = fetch,
): Promise<T> {
  const text = await run(name, input, token, send);
  return (JSON.parse(text) as { result: T }).result;
}

/** One Server-Sent Event: its name and its data. */
export interface StreamEvent {
  event: string;
  data: string;
}

/**
 * The whole events in `text`, as a stream of Server-Sent Events writes
 * them, each ended by a blank line, and what is left after the last, still
 * coming. A comment line, which starts with a colon, says nothing.
 */
export function readEvents(text: string): { events: StreamEvent[]; rest: string } {
  const events: StreamEvent[] = [];
  const blocks = text.replace(/\r\n?/g, '\n').split('\n\n');
  const rest = blocks.pop() ?? '';
  for (const block of blocks) {
    let event = 'message';
    const data: string[] = [];
    for (const line of block.split('\n')) {
      if (!line || line.startsWith(':')) continue;
      const colon = line.indexOf(':');
      const field = colon < 0 ? line : line.slice(0, colon);
      const value = colon < 0 ? '' : line.slice(colon + 1).replace(/^ /, '');
      if (field === 'event') event = value;
      else if (field === 'data') data.push(value);
    }
    if (data.length) events.push({ event, data: data.join('\n') });
  }
  return { events, rest };
}

/** The request that opens a drawing set's event stream. */
export function eventsRequest(set: string, token: string | null): { url: string; init: RequestInit } {
  const { init } = request('events', { set }, token);
  return { url: '/events', init };
}

/** What the stream says of itself. */
export type StreamState = 'open' | 'reconnecting' | 'refused';

/**
 * Reads a stream of Server-Sent Events: `request` opens it, and each
 * event is given to `heard`. A browser's EventSource sends no headers, so
 * the stream is read with fetch, which sends the token as every command
 * does. A stream that ends is opened again, a second later and then more
 * slowly; one the server refuses, for want of its token, is not, nor one
 * `refuses` says it refuses. Gives back what stops it.
 */
function follow(
  request: () => { url: string; init: RequestInit },
  heard: (e: StreamEvent) => void,
  state: (now: StreamState) => void,
  send: typeof fetch,
  refuses: (status: number) => boolean = (status) => status === 401 || status === 403,
): () => void {
  const stop = new AbortController();
  let wait = 1000;
  const open = async () => {
    while (!stop.signal.aborted) {
      try {
        const { url, init } = request();
        const response = await send(url, { ...init, signal: stop.signal });
        if (refuses(response.status)) {
          state('refused');
          return;
        }
        if (!response.ok || !response.body) throw new Error(`the server answered ${response.status}`);
        state('open');
        wait = 1000;
        const reader = response.body.pipeThrough(new TextDecoderStream()).getReader();
        let kept = '';
        for (;;) {
          const { value, done } = await reader.read();
          if (done) break;
          const { events, rest } = readEvents(kept + value);
          kept = rest;
          for (const e of events) heard(e);
        }
      } catch {
        // Closed, or not reached: opened again below.
      }
      if (stop.signal.aborted) return;
      state('reconnecting');
      await new Promise((done) => setTimeout(done, wait));
      wait = Math.min(wait * 2, 8000);
    }
  };
  void open();
  return () => stop.abort();
}

/**
 * Watches the drawing set `set` through the server's event stream: each
 * time its journal changes on disk, `journal` is given the `journal`
 * command's reply, and each time a proposal is made, kept or discarded,
 * `proposals` the `proposals` command's. Gives back what stops it.
 */
export function watch(
  set: string,
  token: string | null,
  journal: (reply: string) => void,
  state: (now: StreamState) => void,
  send: typeof fetch = fetch,
  proposals: (reply: string) => void = () => {},
): () => void {
  return follow(
    () => eventsRequest(set, token),
    (e) => {
      if (e.event === 'journal') journal(e.data);
      else if (e.event === 'proposals') proposals(e.data);
    },
    state,
    send,
  );
}

// The assistant's routes (#156): HTTP only, beside the commands, with the
// token as every command has it. `GET /assistant` says which agents
// `serve` found on the PATH, `POST /assistant/find` looks again, `POST
// /assistant/messages` sends the drafter's message, `POST
// /assistant/agent` starts a conversation on another agent, and `POST
// /assistant/events` streams the conversation.

/** The assistant's routes, by the part of the path after `/assistant`. */
export type AssistantRoute = '' | 'find' | 'messages' | 'agent' | 'events';

/** An assistant route's request: GET for `/assistant` itself, POST with its body for the rest. */
export function assistantRequest(route: AssistantRoute, body: unknown, token: string | null): { url: string; init: RequestInit } {
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  if (token) headers.Authorization = `Bearer ${token}`;
  const url = route ? `/assistant/${route}` : '/assistant';
  const init: RequestInit = route
    ? { method: 'POST', headers, body: JSON.stringify(body ?? {}), cache: 'no-store', credentials: 'omit' }
    : { method: 'GET', headers, cache: 'no-store', credentials: 'omit' };
  return { url, init };
}

/** Calls an assistant route and gives its reply's result, read; a CommandError when it is not ok. */
export async function assistantCall<T>(route: Exclude<AssistantRoute, 'events'>, body: unknown, token: string | null, send: typeof fetch = fetch): Promise<T> {
  const { url, init } = assistantRequest(route, body, token);
  const text = await replyOf(url, init, send);
  return (JSON.parse(text) as { result: T }).result;
}

/**
 * Follows the assistant's conversation on the set `set`: each `assistant`
 * event, the whole conversation, is given to `conversation` as its data,
 * as `/events` is followed. Gives back what stops it.
 */
export function watchAssistant(
  set: string,
  token: string | null,
  conversation: (reply: string) => void,
  state: (now: StreamState) => void,
  send: typeof fetch = fetch,
): () => void {
  return follow(
    () => assistantRequest('events', { set }, token),
    (e) => {
      if (e.event === 'assistant') conversation(e.data);
    },
    state,
    send,
    // A request the server refuses, such as for a folder that is no drawing set, is not made again.
    (status) => status >= 400 && status < 500 && status !== 408 && status !== 429,
  );
}
