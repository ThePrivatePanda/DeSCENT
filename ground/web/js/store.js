// Settings, in two kinds. Display settings are this browser's (localStorage). Model settings
// are the app's, the same for every screen on it: read with GET /api/model-settings, changed
// one at a time with POST, and announced to every viewer on the stream. Never sent on load,
// so two viewers opening the page do not overwrite each other.
(function (root, factory) {
  const S = typeof require === 'function' ? require('./stream.js') : root.DG.stream;
  const mod = factory(S);
  if (typeof module === 'object' && module.exports) module.exports = mod;
  else (root.DG = root.DG || {}).store = mod;
})(typeof self !== 'undefined' ? self : this, function (S) {
  'use strict';

  const SETTINGS_KEY = 'dg-settings-v1';
  const MODEL_URL = '/api/model-settings';

  const UI_DEFAULTS = {
    graphWindowMin: 0,        // 0 = show everything kept
    retainMin: 0,             // drop graph points older than this (0 = keep everything)
    maxPoints: 20000,         // graph points kept per unit
    lineWidth: 1.25,
    fileTag: 'descent',       // prefix for saved file names
    alarmSound: true,         // three notes when a unit that was being heard goes lost, one beep when a run starts
  };
  const MODEL_DEFAULTS = S.MODEL_DEFAULTS;

  // The drawer. A field with model: is the app's setting of that name, shown divided by
  // scale (the app keeps milliseconds, the drawer says minutes and seconds).
  const MIN = (n) => [n, n ? 'Last ' + n + ' min' : 'All data kept'];
  const FIELDS = [
    { section: 'Graphs' },
    { key: 'graphWindowMin', label: 'Time shown', type: 'select', options: [0, 1, 5, 15, 30, 60].map(MIN),
      hint: 'Counted back from the newest packet.' },
    { key: 'retainMin', label: 'Drop graph data older than', type: 'select',
      options: [[0, 'Never'], [5, '5 min'], [15, '15 min'], [30, '30 min'], [60, '1 hour'], [180, '3 hours']],
      hint: 'Graph history only. Packet counts, misses and resets are kept.' },
    { key: 'maxPoints', label: 'Max graph points per unit', type: 'number', min: 100, step: 100 },
    { key: 'lineWidth', label: 'Line width', type: 'select', options: [[1, 'Thin'], [1.25, 'Normal'], [2, 'Thick']] },
    { section: 'Status', note: 'Kept in the app, the same on every screen.' },
    { key: 'lostMin', model: 'lostAfterMs', scale: 60000, label: 'Lost after silence of (min)', type: 'number', min: 1, step: 1 },
    { key: 'staleFactor', model: 'staleIntervals', scale: 1, label: 'Stale after missed intervals', type: 'number', min: 1, step: 0.5 },
    { key: 'staleMinS', model: 'staleMinMs', scale: 1000, label: 'Stale no sooner than (s)', type: 'number', min: 1, step: 1 },
    { key: 'lowBatteryV', model: 'lowBatteryV', scale: 1, label: 'Low battery below (V)', type: 'number', min: 2.5, max: 4.5, step: 0.05 },
    { key: 'weakRssiDbm', model: 'weakRssiDbm', scale: 1, label: 'Weak RF below (dBm)', type: 'number', max: 0, step: 1 },
    { key: 'saturationMps2', model: 'accelSatMs2', scale: 1, label: 'Accel saturation at (m/s²)', type: 'number', min: 1, step: 1,
      hint: 'BNO085 range is ±8 g ≈ 78 m/s².' },
    { key: 'dedupeMs', model: 'dedupeWindowMs', scale: 1, label: 'Same packet across receivers within (ms)', type: 'number', min: 100, step: 100 },
    { key: 'alarmSound', label: 'Sound when a unit goes lost or a run starts', type: 'checkbox',
      hint: 'Three low notes for lost, one short beep for a run. The browser plays nothing until the page has been clicked once.' },
    { section: 'Saving' },
    { key: 'fileTag', label: 'File name prefix', type: 'text' },
  ];
  const field = (key) => FIELDS.find((f) => f.key === key) || null;

  function defaults() { return Object.assign({}, UI_DEFAULTS); }

  function loadSettings(storage) {
    let saved = {};
    try { saved = JSON.parse((storage || localStorage).getItem(SETTINGS_KEY) || '{}'); } catch (e) { /* blocked or corrupt */ }
    const out = defaults();
    for (const k in out) if (typeof saved[k] === typeof out[k]) out[k] = saved[k];
    return out;
  }

  function saveSettings(s, storage) {
    const keep = {};
    for (const k in UI_DEFAULTS) if (k in s) keep[k] = s[k];
    try { (storage || localStorage).setItem(SETTINGS_KEY, JSON.stringify(keep)); return true; } catch (e) { return false; }
  }

  // What a field's input shows
  function shown(f, display, model) {
    if (f.model) return model[f.model] / f.scale;
    return display[f.key];
  }

  // What changing a field's input to raw asks for: {display: {...}} or {model: {...}}, or
  // null when it is not a number where one is wanted.
  function change(key, raw) {
    const f = field(key);
    if (!f) return null;
    if (f.model) {
      const v = Number(raw);
      if (raw === '' || !Number.isFinite(v)) return null;
      return { model: { [f.model]: v * f.scale } };
    }
    let v = raw;
    if (typeof UI_DEFAULTS[key] === 'number') { v = Number(raw); if (raw === '' || !Number.isFinite(v)) return null; }
    if (typeof UI_DEFAULTS[key] === 'boolean') v = !!raw;
    return { display: { [key]: v } };
  }

  // The app's model settings as this page last heard them
  class ModelSettings {
    // o: {fetch, onChange(values)}
    constructor(o) {
      this.o = o || {};
      this.values = Object.assign({}, MODEL_DEFAULTS);
      this.loaded = false;
    }

    take(j) {
      let changed = false;
      for (const k of Object.keys(MODEL_DEFAULTS)) {
        if (j && typeof j[k] === 'number' && j[k] !== this.values[k]) { this.values[k] = j[k]; changed = true; }
      }
      if (changed && this.o.onChange) this.o.onChange(this.values);
      return changed;
    }

    async load() {
      const r = await this.o.fetch(MODEL_URL);
      if (!r.ok) throw new Error('HTTP ' + r.status);
      this.take(await r.json());
      this.loaded = true;
      return this.values;
    }

    // Only the keys given go: another screen may be changing another one
    async set(partial) {
      const body = {};
      for (const k of Object.keys(partial)) if (k in MODEL_DEFAULTS) body[k] = partial[k];
      const r = await this.o.fetch(MODEL_URL, { method: 'POST', body: JSON.stringify(body) });
      const j = await r.json().catch(() => null);
      if (!r.ok || (j && j.ok === false)) throw new Error((j && j.error) || 'HTTP ' + r.status);
      // the app's answer, or what was asked for when it only says ok
      this.take(j && Object.keys(MODEL_DEFAULTS).some((k) => typeof j[k] === 'number') ? j : body);
      return this.values;
    }

    // {kind: "settings", ...} off the stream
    fromStream(m) { return this.take(m); }
  }

  return { UI_DEFAULTS, MODEL_DEFAULTS, FIELDS, MIN, field, defaults, loadSettings, saveSettings, shown, change, ModelSettings };
});
