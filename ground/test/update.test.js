const test = require('node:test');
const assert = require('node:assert');
const U = require('../web/js/update.js');

const HOUR = 3600 * 1000;
const LISTED = [
  { tag_name: 'ground-v0.8.0', draft: false, prerelease: false },
  { tag_name: 'ground-v0.10.1', draft: false, prerelease: false },
  { tag_name: 'ground-v0.11.0', draft: false, prerelease: true },
  { tag_name: 'ground-v0.12.0', draft: true, prerelease: false },
  { tag_name: 'fsw-v9.0.0', draft: false, prerelease: false },
  { tag_name: 'v9.9.9', draft: false, prerelease: false },
  { tag_name: 'ground-v1.0.0-rc1', draft: false, prerelease: false },
  { tag_name: 'ground-v0.7.0', draft: false, prerelease: false },
  {},
];

// A browser's storage and a network that answers as told, and counts how often it is asked.
function world(answer) {
  const kept = {};
  const w = {
    asked: 0, answer,
    storage: { getItem: (k) => (k in kept ? kept[k] : null), setItem: (k, v) => { kept[k] = v; } },
    fetch: async (url, opts) => {
      w.asked++; w.url = url; w.opts = opts;
      if (w.answer instanceof Error) throw w.answer;
      return { ok: w.answer.status === 200, status: w.answer.status, json: async () => w.answer.body };
    },
  };
  U.forget();
  return w;
}
const ask = (w, now, running, storage) => U.check({ fetch: w.fetch, storage: storage === undefined ? w.storage : storage, now, running });

test('only a released dashboard version counts, and numbers are compared as numbers', () => {
  assert.deepStrictEqual(U.versions(LISTED), ['0.8.0', '0.10.1', '0.7.0']);
  assert.deepStrictEqual(U.versions(null), []);
  assert.deepStrictEqual(U.versions({ message: 'Not Found' }), []);
  assert.equal(U.newest(['0.8.0', '0.10.1', '0.7.0'], '0.7.0'), '0.10.1');
  assert.equal(U.newest(['0.7.0'], '0.7.0'), null);
  assert.equal(U.newest(['0.6.9'], '0.7.0'), null);
  assert.equal(U.newest([], '0.7.0'), null);
});

test('a newer release is offered with where to get it', async () => {
  const w = world({ status: 200, body: LISTED });
  assert.deepStrictEqual(await ask(w, 1000, '0.7.0'),
    { version: '0.10.1', url: 'https://github.com/ThePrivatePanda/DeSCENT/releases/tag/ground-v0.10.1' });
  assert.equal(w.url, 'https://api.github.com/repos/ThePrivatePanda/DeSCENT/releases?per_page=30');
  // Running the newest, nothing is said.
  assert.equal(await ask(w, 2000, '0.10.1'), null);
});

test('GitHub is asked once in six hours however often the page asks', async () => {
  const w = world({ status: 200, body: LISTED });
  for (let minute = 0; minute < 359; minute++) await ask(w, minute * 60000, '0.7.0');
  assert.equal(w.asked, 1);
  await ask(w, 6 * HOUR, '0.7.0');
  assert.equal(w.asked, 2);
});

test('no network, a private repo and a refusal all say nothing, and are tried again in half an hour', async () => {
  for (const answer of [new TypeError('Failed to fetch'), { status: 404, body: { message: 'Not Found' } }, { status: 403, body: { message: 'rate limit' } },
    { status: 200, body: 'not a list' }]) {
    const w = world(answer);
    assert.equal(await ask(w, 0, '0.7.0'), null);
    for (let minute = 1; minute < 30; minute++) assert.equal(await ask(w, minute * 60000, '0.7.0'), null);
    assert.equal(w.asked, 1, 'not asked again for half an hour');
    if (answer.status !== 200) {
      await ask(w, 30 * 60000, '0.7.0');
      assert.equal(w.asked, 2);
    }
  }
});

test('a release seen before is still offered when the network has gone', async () => {
  const w = world({ status: 200, body: LISTED });
  assert.equal((await ask(w, 0, '0.7.0')).version, '0.10.1');
  w.answer = new TypeError('Failed to fetch');
  assert.equal((await ask(w, 7 * HOUR, '0.7.0')).version, '0.10.1');
  assert.equal(w.asked, 2);
});

test('a browser with storage switched off asks no more often', async () => {
  const w = world({ status: 200, body: LISTED });
  const blocked = { getItem() { throw new Error('denied'); }, setItem() { throw new Error('denied'); } };
  for (let minute = 0; minute < 60; minute++) assert.equal((await ask(w, minute * 60000, '0.7.0', blocked)).version, '0.10.1');
  assert.equal(w.asked, 1);
  const none = world({ status: 200, body: LISTED });
  for (let minute = 0; minute < 60; minute++) await ask(none, minute * 60000, '0.7.0', null);
  assert.equal(none.asked, 1);
});

test('what is kept in the browser is not trusted to be the right shape', async () => {
  const w = world({ status: 200, body: LISTED });
  w.storage.setItem('dg-releases', '{"at":"yesterday"}');
  assert.equal((await ask(w, 0, '0.7.0')).version, '0.10.1');
  // A clock that went backwards asks again and does not wait six hours on a time that has not come.
  assert.equal(w.asked, 1);
  await ask(w, -5000, '0.7.0');
  assert.equal(w.asked, 2);
});
