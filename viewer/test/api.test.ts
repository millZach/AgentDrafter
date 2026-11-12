import assert from 'node:assert/strict';
import { test } from 'node:test';

import { CommandError, request, run, takeToken, TOKEN_KEY, wanted } from '../src/api.ts';

class Storage {
  readonly kept = new Map<string, string>();
  getItem(key: string) {
    return this.kept.get(key) ?? null;
  }
  setItem(key: string, value: string) {
    this.kept.set(key, value);
  }
}

class History {
  readonly urls: string[] = [];
  replaceState(_data: unknown, _unused: string, url: string) {
    this.urls.push(url);
  }
}

const TOKEN = 'ab'.repeat(32);

test('the token is taken from the fragment, kept for the tab and taken off the address', () => {
  const storage = new Storage();
  const history = new History();
  const place = { pathname: '/', search: '?source=demo-plant', hash: `#token=${TOKEN}` };
  assert.equal(takeToken(place, storage, history), TOKEN);
  assert.equal(storage.getItem(TOKEN_KEY), TOKEN);
  assert.deepEqual(history.urls, ['/?source=demo-plant'], 'the address keeps its query and loses the token');
  // A reload has no fragment: the kept token is used.
  const again = new History();
  assert.equal(takeToken({ pathname: '/', search: '', hash: '' }, storage, again), TOKEN);
  assert.deepEqual(again.urls, [], 'an address with no token is left alone');
});

test('other parts of the fragment stay, and an address with no token anywhere gives none', () => {
  const history = new History();
  assert.equal(takeToken({ pathname: '/', search: '', hash: `#token=${TOKEN}&x=1` }, new Storage(), history), TOKEN);
  assert.deepEqual(history.urls, ['/#x=1']);
  assert.equal(takeToken({ pathname: '/', search: `?token=${TOKEN}`, hash: '' }, new Storage(), new History()), null,
    'a token in the query string is not taken');
});

test('the page shows the folder the server started in unless the address names another', () => {
  assert.deepEqual(wanted(''), { source: '.', sheet: null });
  assert.deepEqual(wanted('?source=demo-plant&sheet=DP-LD-102'), { source: 'demo-plant', sheet: 'DP-LD-102' });
});

test('a command is posted as JSON with the token as a bearer, never in the address', () => {
  const { url, init } = request('display-list', { source: '.' }, TOKEN);
  assert.equal(url, '/commands/display-list');
  assert.equal(init.method, 'POST');
  const headers = init.headers as Record<string, string>;
  assert.equal(headers['Content-Type'], 'application/json');
  assert.equal(headers.Authorization, `Bearer ${TOKEN}`);
  assert.ok(!url.includes(TOKEN));
  assert.equal(init.body, '{"source":"."}');
  const local = request('display-list', {}, null);
  assert.equal((local.init.headers as Record<string, string>).Authorization, undefined, 'no token on the local machine');
});

test('a refused command gives the reply’s error', async () => {
  const refused = async () =>
    new Response(JSON.stringify({ ok: false, error: { code: 'unauthorized', message: 'needs the token' } }), {
      status: 401,
    });
  await assert.rejects(run('display-list', {}, null, refused), (e: unknown) => {
    assert.ok(e instanceof CommandError);
    assert.equal(e.status, 401);
    assert.equal(e.code, 'unauthorized');
    assert.equal(e.message, 'needs the token');
    return true;
  });
  const ok = async () => new Response('{"ok":true,"result":{}}', { status: 200 });
  assert.equal(await run('display-list', {}, null, ok), '{"ok":true,"result":{}}');
  const down = async () => {
    throw new TypeError('connection refused');
  };
  await assert.rejects(run('display-list', {}, null, down), /could not be reached/);
});
