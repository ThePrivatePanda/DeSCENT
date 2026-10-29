// Offline copy of the hosted dashboard. Network first so updates arrive as
// soon as there is a connection; the cache is used when offline or when the
// network has not answered in 3 s.
const CACHE = 'descent-ground-v15';
const FILES = [
  './', 'index.html', 'css/style.css', 'css/live.css', 'css/chipsats.css', 'css/tbeams.css', 'css/data.css',
  'js/packet.js', 'js/lines.js', 'js/bench.js', 'js/fleet.js', 'js/store.js', 'js/serial.js', 'js/serial_native.js',
  'js/recorder.js', 'js/exportcsv.js', 'js/datamodel.js', 'js/boardmodel.js', 'js/update.js', 'js/charts.js', 'js/app.js',
  'js/screens.js', 'js/tbeams.js', 'js/chipsats.js', 'js/benchpane.js', 'js/datascreen.js',
  'vendor/uPlot.iife.min.js', 'vendor/uPlot.min.css',
  'vendor/fonts/figtree-latin-400-normal.woff2', 'vendor/fonts/figtree-latin-600-normal.woff2',
  'vendor/fonts/figtree-latin-700-normal.woff2', 'vendor/fonts/caprasimo-latin-400-normal.woff2',
  'vendor/fonts/ibm-plex-mono-latin-400-normal.woff2', 'vendor/fonts/ibm-plex-mono-latin-500-normal.woff2',
];

self.addEventListener('install', (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(FILES)).then(() => self.skipWaiting()));
});

self.addEventListener('activate', (e) => {
  e.waitUntil(caches.keys()
    .then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))))
    .then(() => self.clients.claim()));
});

self.addEventListener('fetch', (e) => {
  if (e.request.method !== 'GET' || new URL(e.request.url).origin !== location.origin) return;
  // A stalled hotspot on the field tablet never fails, it just hangs, so the cache gets a turn after 3 s.
  const late = fetch(e.request).then((res) => {
    const copy = res.clone();
    caches.open(CACHE).then((c) => c.put(e.request, copy));
    return res;
  });
  const timer = new Promise((done) => setTimeout(done, 3000, null));
  e.respondWith(
    Promise.race([late, timer])
      .catch(() => null)
      .then((res) => res || caches.match(e.request, { ignoreSearch: true }).then((hit) => hit || late)),
  );
});
