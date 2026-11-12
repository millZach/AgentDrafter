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
 * it (by default the folder the server started in), and `sheet`, which of
 * its sheets (by default the first).
 */
export function wanted(search: string): { source: string; sheet: string | null } {
  const query = new URLSearchParams(search);
  return { source: query.get('source') || '.', sheet: query.get('sheet') || null };
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
  constructor(status: number, code: string, message: string) {
    super(message);
    this.status = status;
    this.code = code;
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
  try {
    const reply = JSON.parse(text) as { error?: { code?: string; message?: string } };
    code = reply.error?.code ?? code;
    message = reply.error?.message ?? message;
  } catch {
    // Not a JSON reply: say the status.
  }
  throw new CommandError(response.status, code, message);
}
