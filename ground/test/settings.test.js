// js/store.js: the drawer's two kinds of setting. Display ones stay in this browser; model
// ones are the app's, asked for on load and sent only when the operator changes one.
const test = require('node:test');
const assert = require('node:assert');
const S = require('../web/js/store.js');

function app(model) {
  const calls = [];
  const fetch = async (url, init) => {
    calls.push({ url, method: (init && init.method) || 'GET', body: init ? init.body : undefined });
    if (init && init.method === 'POST') {
      Object.assign(model, JSON.parse(init.body));
      return { ok: true, status: 200, json: async () => Object.assign({ ok: true }, model) };
    }
    return { ok: true, status: 200, json: async () => model };
  };
  return { calls, fetch };
}
function storage() {
  const kept = {};
  return { kept, getItem: (k) => (k in kept ? kept[k] : null), setItem: (k, v) => { kept[k] = v; } };
}

test('every status field in the drawer is one of the app\'s seven settings, and nothing else is', () => {
  const model = S.FIELDS.filter((f) => f.model).map((f) => f.model).sort();
  assert.deepStrictEqual(model, Object.keys(S.MODEL_DEFAULTS).sort());
  const display = S.FIELDS.filter((f) => f.key && !f.model).map((f) => f.key).sort();
  assert.deepStrictEqual(display, Object.keys(S.UI_DEFAULTS).sort());
  assert.ok(!S.FIELDS.some((f) => f.key === 'autosave' || f.key === 'autosaveS'), 'the crash autosave is gone');
});

test('loading asks the app and sends nothing', async () => {
  const { calls, fetch } = app({ lostAfterMs: 120000, staleIntervals: 4, staleMinMs: 5000, lowBatteryV: 3.6, weakRssiDbm: -110, accelSatMs2: 75, dedupeWindowMs: 1500 });
  const m = new S.ModelSettings({ fetch });
  await m.load();
  assert.deepStrictEqual(calls.map((c) => c.method + ' ' + c.url), ['GET /api/model-settings']);
  assert.strictEqual(m.values.lostAfterMs, 120000);
  assert.strictEqual(S.shown(S.field('lostMin'), S.defaults(), m.values), 2, 'the drawer says minutes');
  assert.strictEqual(S.shown(S.field('staleMinS'), S.defaults(), m.values), 5, 'and seconds');
});

test('changing a status field posts that one key, in the app\'s units, and keeps it out of this browser', async () => {
  const { calls, fetch } = app(Object.assign({}, S.MODEL_DEFAULTS));
  const seen = [];
  const m = new S.ModelSettings({ fetch, onChange: (v) => seen.push(v.lostAfterMs) });
  const ask = S.change('lostMin', '3');
  assert.deepStrictEqual(ask, { model: { lostAfterMs: 180000 } });
  await m.set(ask.model);
  assert.deepStrictEqual(calls, [{ url: '/api/model-settings', method: 'POST', body: '{"lostAfterMs":180000}' }]);
  assert.deepStrictEqual(seen, [180000]);
  const st = storage();
  S.saveSettings(Object.assign(S.defaults(), { lostMin: 3, lostAfterMs: 1 }), st);
  assert.deepStrictEqual(Object.keys(JSON.parse(st.kept['dg-settings-v1'])).sort(), Object.keys(S.UI_DEFAULTS).sort());
  assert.strictEqual(S.change('staleMinS', ''), null, 'an empty box is not zero');
  assert.strictEqual(S.change('weakRssiDbm', 'loud'), null);
});

test('a display field stays in this browser and asks nothing of the app', () => {
  assert.deepStrictEqual(S.change('graphWindowMin', '15'), { display: { graphWindowMin: 15 } });
  assert.deepStrictEqual(S.change('alarmSound', false), { display: { alarmSound: false } });
  assert.deepStrictEqual(S.change('fileTag', 'drop3'), { display: { fileTag: 'drop3' } });
  const st = storage();
  S.saveSettings(Object.assign(S.defaults(), { lineWidth: 2 }), st);
  assert.strictEqual(S.loadSettings(st).lineWidth, 2);
  // what 1.3 kept here: its status values are the app's now and are not read back
  st.kept['dg-settings-v1'] = JSON.stringify({ lostMin: 99, graphWindowMin: 5, autosave: false });
  const loaded = S.loadSettings(st);
  assert.strictEqual(loaded.graphWindowMin, 5);
  assert.ok(!('lostMin' in loaded) && !('autosave' in loaded));
});

test('a settings message from another screen updates the values, and only the keys it carries', () => {
  const seen = [];
  const m = new S.ModelSettings({ fetch: async () => { throw new Error('not asked'); }, onChange: () => seen.push(1) });
  assert.strictEqual(m.fromStream({ kind: 'settings', seq: 4, t: 1, lowBatteryV: 3.7 }), true);
  assert.strictEqual(m.values.lowBatteryV, 3.7);
  assert.strictEqual(m.values.lostAfterMs, S.MODEL_DEFAULTS.lostAfterMs);
  assert.strictEqual(m.fromStream({ kind: 'settings', lowBatteryV: 3.7 }), false, 'the same again is no change');
  assert.strictEqual(seen.length, 1);
});

test('an app that refuses a change says why, and the values stay', async () => {
  const fetch = async () => ({ ok: false, status: 400, json: async () => ({ ok: false, error: 'lostAfterMs out of range' }) });
  const m = new S.ModelSettings({ fetch });
  await assert.rejects(m.set({ lostAfterMs: -1 }), /out of range/);
  assert.strictEqual(m.values.lostAfterMs, S.MODEL_DEFAULTS.lostAfterMs);
});
