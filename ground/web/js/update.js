// Whether a newer release of the dashboard is out. GitHub is asked for the repo's
// releases, the answer is kept in this browser, and it is asked again after six hours, or
// after half an hour when it could not be asked. GitHub gives one address 60 answers an
// hour without signing in, so this is never asked per draw. A bench with no network, a
// repo that is private and a refusal all end the same way: nothing is said.
(function (root, factory) {
  const B = typeof module === 'object' && module.exports ? require('./boardmodel.js') : root.DG.boardmodel;
  const mod = factory(B);
  if (typeof module === 'object' && module.exports) module.exports = mod;
  else (root.DG = root.DG || {}).update = mod;
})(typeof self !== 'undefined' ? self : this, function (B) {
  'use strict';

  const REPO = 'ThePrivatePanda/DeSCENT';
  const LIST = 'https://api.github.com/repos/' + REPO + '/releases?per_page=30';
  // The dashboard's releases. The repo's other tags are the flight software's.
  const TAG = /^ground-v(\d+(?:\.\d+)*)$/;
  const KEY = 'dg-releases';
  const AGAIN_MS = 6 * 3600 * 1000;
  const RETRY_MS = 30 * 60 * 1000;

  // What was last learned, kept here as well as in the browser's storage, so a browser
  // with storage switched off still asks no more often.
  let memory = null;

  // The versions GitHub lists as released. Drafts and prereleases are not for everyone.
  function versions(list) {
    const out = [];
    for (const r of Array.isArray(list) ? list : []) {
      const m = TAG.exec((r && r.tag_name) || '');
      if (m && !r.draft && !r.prerelease) out.push(m[1]);
    }
    return out;
  }

  // The newest of them that is newer than what is running, or null.
  function newest(released, running) {
    let best = null;
    for (const v of released || []) if (B.isNewer(v, best || running)) best = v;
    return best;
  }

  function kept(storage) {
    try {
      const k = JSON.parse((storage && storage.getItem(KEY)) || 'null');
      if (k && Number.isFinite(k.at) && Array.isArray(k.versions)) return k;
    } catch (e) { /* storage blocked, or not ours */ }
    return memory;
  }

  function keep(storage, k) {
    memory = k;
    try { if (storage) storage.setItem(KEY, JSON.stringify(k)); } catch (e) { /* storage blocked or full */ }
  }

  // o is {fetch, storage, now, running}. Answers {version, url} when a newer release is
  // out, and null otherwise, including whenever it could not find out. What was learned
  // before a failure still counts: a release seen yesterday is still out with no network.
  async function check(o) {
    let k = kept(o.storage);
    const age = k ? o.now - k.at : Infinity;
    if (!k || age < 0 || age >= (k.ok ? AGAIN_MS : RETRY_MS)) {
      const before = k ? k.versions : [];
      try {
        const r = await o.fetch(LIST, { headers: { Accept: 'application/vnd.github+json' } });
        if (!r.ok) throw new Error('HTTP ' + r.status);
        k = { at: o.now, ok: true, versions: versions(await r.json()) };
      } catch (e) {
        k = { at: o.now, ok: false, versions: before };
      }
      keep(o.storage, k);
    }
    const version = newest(k.versions.filter((v) => typeof v === 'string'), o.running);
    return version ? { version, url: 'https://github.com/' + REPO + '/releases/tag/ground-v' + version } : null;
  }

  return { check, versions, newest, AGAIN_MS, RETRY_MS, LIST, forget() { memory = null; } };
});
