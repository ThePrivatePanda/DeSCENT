// The flight view: boards over real imagery and terrain, drawn from what they sent (Live, Replay)
// or logged (Analysis). Flat and north up to find a board, tilted with terrain to see a flight. A
// dot is a sample, solid when its flag is set; anything between dots is dashed; a break says what
// was lost. Where no imagery is saved and there is no connection, a metre grid stays under it all.
// The markup is pure and sits first, so node can test it; the map below needs the page.
(function () {
  'use strict';
  const F = typeof module === 'object' && module.exports ? require('./flight.js') : DG.flight;
  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
  const fixed = (v, d) => (v === null || v === undefined || !Number.isFinite(v) ? '—' : v.toFixed(d));
  const clock = (t) => new Date(t * 1000).toTimeString().slice(0, 8);
  const ago = (s) => (s < 90 ? Math.round(s) + ' s' : s < 5400 ? Math.round(s / 60) + ' min' : (s / 3600).toFixed(1) + ' h');
  const coords = (lat, lon) => lat.toFixed(6) + ', ' + lon.toFixed(6);

  // The imagery the prefetch saves is USGS's: Esri's terms do not allow saving it for offline use,
  // so it is only ever fetched while online. USGS has nothing past z16 at the drop site (z17 to 19
  // answer 404, 2026-11-09), so the map stretches z16 rather than asking for tiles that are not there.
  const SOURCES = {
    'imagery-usgs': { name: 'USGS', maxzoom: 16, save: [14, 16],
      url: 'https://basemap.nationalmap.gov/arcgis/rest/services/USGSImageryOnly/MapServer/tile/{z}/{y}/{x}',
      attribution: 'Imagery: USDA, USGS The National Map: Orthoimagery' },
    'imagery-esri': { name: 'Esri', maxzoom: 19, save: null,
      url: 'https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}',
      attribution: 'Imagery: Esri, Vantor, Earthstar Geographics, and the GIS User Community' },
    terrain: { name: 'Terrain', maxzoom: 15, save: [10, 15],
      url: 'https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png',
      attribution: 'Terrain: Mapzen Terrarium on AWS Open Data (SRTM, GMTED2010, USGS 3DEP, ETOPO1)' },
  };
  // What a tile weighs when none of its layer is saved yet to average over: the sizes these
  // servers sent for the drop site
  const TYPICAL_BYTES = { 'imagery-usgs': 22000, terrain: 98000 };
  const SAVE_MOST = 4000;   // tiles one save asks of a public server; a wider view is asked to zoom in

  // Tiles covering a box, per zoom: [{layer, z, x, y}]
  function tilesFor(bounds, layer, from, to) {
    const out = [];
    const tx = (lon, z) => Math.floor((lon + 180) / 360 * 2 ** z);
    const ty = (lat, z) => {
      const r = lat * Math.PI / 180;
      return Math.floor((1 - Math.log(Math.tan(r) + 1 / Math.cos(r)) / Math.PI) / 2 * 2 ** z);
    };
    for (let z = from; z <= to; z++) {
      for (let x = tx(bounds.west, z); x <= tx(bounds.east, z); x++) {
        for (let y = ty(bounds.north, z); y <= ty(bounds.south, z); y++) out.push({ layer, z, x, y });
      }
    }
    return out;
  }

  // The size of a save before it starts: from the average tile of each layer already saved, else
  // from TYPICAL_BYTES, and which of the two it is
  function saveEstimate(bounds, stored) {
    const parts = ['imagery-usgs', 'terrain'].map((layer) => {
      const tiles = tilesFor(bounds, layer, SOURCES[layer].save[0], SOURCES[layer].save[1]);
      const kept = stored && stored.layers && stored.layers[layer];
      const each = kept && kept.tiles ? kept.bytes / kept.tiles : TYPICAL_BYTES[layer];
      return { layer, tiles, bytes: tiles.length * each, measured: !!(kept && kept.tiles) };
    });
    return { parts, tiles: parts[0].tiles.length + parts[1].tiles.length, bytes: parts[0].bytes + parts[1].bytes, measured: parts.every((p) => p.measured) };
  }

  function mb(bytes) {
    return bytes < 1e6 ? Math.round(bytes / 1000) + ' kB' : (bytes / 1e6).toFixed(1) + ' MB';
  }

  // A Find card. c: {key, name, colour, why, td, spot, restart: {t, fixes} | null, station, lastT, now, battery, rx: [{name, rssi, snr}]}
  function findCardHtml(c) {
    const head = '<div class="fv-cardhead"><span class="fv-swatch" style="background:' + esc(c.colour) + '"></span><b>' + esc(c.name) + '</b>' +
      '<span class="mark ' + (c.why === 'down' ? 'good' : 'off') + '">' + (c.why === 'down' ? 'on the ground' : 'silent ' + ago(c.now - c.lastT)) + '</span></div>';
    const rows = [];
    const row = (label, value) => rows.push('<dt>' + esc(label) + '</dt><dd>' + value + '</dd>');
    row('Touchdown', c.td ? '<b>' + clock(c.td.t) + '</b> <span class="note">derived: IMU still (under ' + F.STILL_ACCEL_MPS2 + ' m/s², ' +
      F.STILL_GYRO_DPS + ' deg/s) for ' + F.STILL_FOR_S + ' s</span>' : '<span class="note">none found: no 20 s still after moving</span>');
    if (c.spot) {
      row('Lying at', '<b class="fv-coords">' + coords(c.spot.lat, c.spot.lon) + '</b> <span class="note">' + esc(c.spot.rule) + '; half within ' +
        fixed(c.spot.half, 1) + ' m, all within ' + fixed(c.spot.all, 1) + ' m</span>');
      if (c.station) {
        const d = F.metres(c.station.lat, c.station.lon, c.spot.lat, c.spot.lon);
        const b = F.bearing(c.station.lat, c.station.lon, c.spot.lat, c.spot.lon);
        row('From the station', '<b>' + fixed(d, 0) + ' m at ' + fixed(b, 0) + '° ' + F.compass(b) + '</b> <span class="note">true north; station ' + esc(c.station.from) + '</span>');
      } else {
        row('From the station', '<span class="note">no station set</span>');
      }
    } else {
      row('Lying at', '<span class="note">no GPS fix with its flag set</span>');
    }
    if (c.restart) {
      row('Restarted', '<b>' + clock(c.restart.t) + '</b> <span class="note">after this, in a new run with ' +
        (c.restart.fixes ? c.restart.fixes + ' fixes' : 'no fix') + ' since; the touchdown and fixes above are from before it</span>');
    }
    row('Last packet', '<b>' + clock(c.lastT) + '</b> <span class="note">' + ago(c.now - c.lastT) + ' ago</span>');
    row('Cell', c.battery === null ? '<span class="note">flag clear</span>' : '<b>' + fixed(c.battery, 2) + '</b> V');
    row('Last heard by', c.rx.length ? c.rx.map((r) => esc(r.name) + ' <b>' + fixed(r.rssi, 0) + '</b> dBm, ' + fixed(r.snr, 1) + ' dB').join('<br>') : '—');
    const acts = c.spot ? '<div class="fv-acts"><button data-copy="' + esc(coords(c.spot.lat, c.spot.lon)) + '">Copy coordinates</button>' +
      '<a class="button ghost" href="geo:' + c.spot.lat.toFixed(6) + ',' + c.spot.lon.toFixed(6) + '">Open in a maps app</a>' +
      '<button class="ghost" data-station-from="' + esc(c.key) + '" title="Use this board\'s last fix as the station">Station here</button></div>' : '';
    return '<div class="fv-card">' + head + '<dl class="fv-kv">' + rows.join('') + '</dl>' + acts + '</div>';
  }

  // The readings at the cursor. r: {t, wall, sample, ages: {fix, height, quat}, height, vs, hs, o, gyro, source}
  function readoutHtml(r) {
    if (!r) return '<p class="note">No sample at or before this time.</p>';
    const flag = (ok) => (ok ? '' : ' <span class="note">flag clear</span>');
    const age = (s) => (s > 0.0005 ? ' <span class="note">' + fixed(s, 2) + ' s before</span>' : '');
    const cells = [];
    const cell = (label, value) => cells.push('<div><span class="kicker">' + esc(label) + '</span><span class="fv-val">' + value + '</span></div>');
    cell('Time', '<b>' + esc(r.when) + '</b>');
    cell(r.source === 'gps' ? 'GPS height' : 'Barometer height', r.height === null ? '—'
      : '<b>' + fixed(r.height.v, 1) + '</b> m' + flag(r.height.ok) + age(r.ages.height) + ' <span class="note">above its first reading in the boot</span>');
    cell('Vertical speed', r.vs ? '<b>' + fixed(r.vs.v, 1) + '</b> m/s <span class="note">derived: height difference over ' + fixed(r.vs.dt, 2) + ' s</span>' : '—');
    cell('Ground speed', r.hs ? '<b>' + fixed(r.hs.v, 1) + '</b> m/s <span class="note">derived: fixes ' + fixed(r.hs.dt, 2) + ' s apart</span>' : '—');
    cell('Fix', r.fix ? '<b>' + coords(r.fix.lat, r.fix.lon) + '</b>' + (r.fix.held ? ' <span class="note">held fix</span>' : '') + age(r.ages.fix) : '—');
    cell('Tilt', r.o ? '<b>' + fixed(r.o.tilt, 0) + '</b>° <span class="note">board z from up</span>' + flag(r.o.ok) + age(r.ages.quat) : '—');
    cell('Magnetic heading', r.o && r.o.heading !== null ? '<b>' + fixed(r.o.heading, 0) + '</b>° <span class="note">board x, from the quaternion</span>' + flag(r.o.ok) : '—');
    cell('Rotation', r.gyro ? '<b>' + fixed(r.gyro.v, 0) + '</b> deg/s' + flag(r.gyro.ok) : '—');
    return '<div class="fv-readout">' + cells.join('') + '</div>';
  }

  // Where to look for a board: its last touchdown and its last fixes in anything heard from it,
  // its earlier runs included. A board that browned out on the ground starts a run with no fix,
  // and the landing before it is the one to walk to; the card then says it restarted since.
  function landing(f) {
    const runs = (f.before || []).concat([{ samples: f.samples, firstT: f.firstT }]);
    let tdRun = null;
    let td = null;
    for (let k = runs.length - 1; k >= 0 && !td; k--) {
      td = F.touchdown(runs[k].samples);
      if (td) tdRun = k;
    }
    let fixRun = -1;
    for (let k = runs.length - 1; k >= 0 && fixRun < 0; k--) if (runs[k].samples.some(F.isFix)) fixRun = k;
    const spot = td && fixRun <= tdRun ? F.findSpot(runs[tdRun].samples, td)
      : fixRun >= 0 ? F.findSpot(runs[fixRun].samples, td) : null;
    const from = Math.max(tdRun === null ? -1 : tdRun, fixRun);
    const now = runs.length - 1;
    const restart = from >= 0 && from < now ? { t: runs[now].firstT, fixes: f.samples.filter(F.isFix).length } : null;
    return { td, spot, restart };
  }

  const pure = { SOURCES, tilesFor, saveEstimate, landing, findCardHtml, readoutHtml, mb };
  if (typeof module === 'object' && module.exports) module.exports = pure;
  if (typeof window === 'undefined') return;

  const css = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const colour = (i) => css('--series-' + ((i % 8) + 1));
  const EMPTY = { type: 'FeatureCollection', features: [] };
  const stats = { ok: 0, missing: 0, unsaved: 0 };   // imagery tiles since the view last moved

  // Cache first, then the source while online, posting what it sends to the cache
  async function loadTile(params, ctl) {
    const path = params.url.slice('dgtile://'.length);
    const layer = path.split('/')[0];
    const imagery = layer !== 'terrain';
    const [z, x, y] = path.split('/').slice(1);
    const kept = await fetch('tiles/' + path, { signal: ctl.signal }).catch(() => null);
    if (kept && kept.ok) {
      if (imagery) stats.ok++;
      return { data: await kept.arrayBuffer() };
    }
    const url = SOURCES[layer].url.replace('{z}', z).replace('{x}', x).replace('{y}', y);
    const got = navigator.onLine === false ? null : await fetch(url, { signal: ctl.signal }).catch(() => null);
    if (!got || !got.ok) {
      if (imagery) stats.missing++;
      throw new Error('no ' + layer + ' tile ' + path + (got ? ': HTTP ' + got.status : ''));
    }
    const data = await got.arrayBuffer();
    fetch('api/tile/' + path, { method: 'POST', body: data }).then((r) => r.json())
      .then((j) => { if (!j.ok) stats.unsaved++; }, () => { stats.unsaved++; });
    if (imagery) stats.ok++;
    return { data };
  }
  maplibregl.addProtocol('dgtile', loadTile);

  // Lines every 10, 20, 50 ... m, about eight across, so the grid is a scale where nothing else is
  function grid(map) {
    const b = map.getBounds();
    const lat0 = map.getCenter().lat;
    const mPerLon = 111320 * Math.cos(lat0 * Math.PI / 180);
    const across = (b.getEast() - b.getWest()) * mPerLon;
    const step = [10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000, 50000].find((s) => across / s <= 10) || 100000;
    const features = [];
    const line = (coords) => features.push({ type: 'Feature', properties: {}, geometry: { type: 'LineString', coordinates: coords } });
    for (let m = Math.floor(b.getWest() * mPerLon / step) * step; m <= b.getEast() * mPerLon; m += step) {
      line([[m / mPerLon, b.getSouth()], [m / mPerLon, b.getNorth()]]);
    }
    for (let m = Math.floor(b.getSouth() * 111320 / step) * step; m <= b.getNorth() * 111320; m += step) {
      line([[b.getWest(), m / 111320], [b.getEast(), m / 111320]]);
    }
    return { step, data: { type: 'FeatureCollection', features } };
  }

  function style(imagery) {
    const src = SOURCES[imagery];
    return {
      version: 8,
      sources: {
        imagery: { type: 'raster', tiles: ['dgtile://' + imagery + '/{z}/{x}/{y}'], tileSize: 256, maxzoom: src.maxzoom, attribution: src.attribution },
        terrain: { type: 'raster-dem', tiles: ['dgtile://terrain/{z}/{x}/{y}'], tileSize: 256, maxzoom: 15, encoding: 'terrarium', attribution: SOURCES.terrain.attribution },
        grid: { type: 'geojson', data: EMPTY },
        lines: { type: 'geojson', data: EMPTY },
        dots: { type: 'geojson', data: EMPTY },
        ring: { type: 'geojson', data: EMPTY },
      },
      layers: [
        { id: 'ground', type: 'background', paint: { 'background-color': '#2a2725' } },
        { id: 'grid', type: 'line', source: 'grid', paint: { 'line-color': '#a39d96', 'line-width': 1.5 } },
        { id: 'imagery', type: 'raster', source: 'imagery' },
        { id: 'lines-casing', type: 'line', source: 'lines', paint: { 'line-color': '#ffffff', 'line-width': 7, 'line-opacity': ['get', 'opacity'] } },
        { id: 'lines', type: 'line', source: 'lines', paint: { 'line-color': ['get', 'colour'], 'line-width': 4, 'line-dasharray': [1.5, 1], 'line-opacity': ['get', 'opacity'] } },
        { id: 'ring', type: 'line', source: 'ring', paint: { 'line-color': '#ffffff', 'line-width': 3, 'line-dasharray': [2, 1] } },
        { id: 'dots', type: 'circle', source: 'dots', paint: {
          'circle-radius': ['case', ['get', 'last'], 9, 5.5],
          'circle-color': ['case', ['get', 'hollow'], 'rgba(0,0,0,0)', ['get', 'colour']],
          'circle-stroke-color': ['case', ['get', 'hollow'], ['get', 'colour'], '#ffffff'],
          'circle-stroke-width': ['case', ['get', 'hollow'], 3, 1.5],
          'circle-opacity': ['get', 'opacity'], 'circle-stroke-opacity': ['get', 'opacity'],
        } },
      ],
    };
  }

  function el(tag, cls, html) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (html !== undefined) e.innerHTML = html;
    return e;
  }

  // The three.js scene in metres, x east, y north, z up, from an origin the map places. Holds the
  // trail in the air (3D only), each board's glyph and its linear acceleration arrow.
  class Air {
    constructor(view) {
      this.view = view;
      this.id = 'air';
      this.type = 'custom';
      this.renderingMode = '3d';
    }

    onAdd(map, gl) {
      const T = window.THREE;
      this.map = map;
      this.camera = new T.Camera();
      this.scene = new T.Scene();
      this.renderer = new T.WebGLRenderer({ canvas: map.getCanvas(), context: gl, antialias: true });
      this.renderer.autoClear = false;
      this.unit = new T.SphereGeometry(1, 12, 8);
      this.stick = new T.CylinderGeometry(1, 1, 1, 6);
      this.stick.rotateX(Math.PI / 2);   // 0 to 1 along z: a turn from z and a length place a dash
      this.stick.translate(0, 0, 0.5);
      this.view.airReady();
    }

    // Everything sized in screen pixels at the current zoom, so a dot stays a dot
    pxMetres() {
      const lat = this.origin ? this.origin.lat : this.map.getCenter().lat;
      return 156543.03 * Math.cos(lat * Math.PI / 180) / 2 ** this.map.getZoom();
    }

    // Metres east and north of the origin, and a flight's lift: its heights are above its own
    // start, which sits on the terrain there
    frame() {
      const cosLat = Math.cos(this.origin.lat * Math.PI / 180);
      const ground = (d) => this.map.queryTerrainElevation([d.lon, d.lat]) || 0;
      const base = ground(this.origin);
      return {
        px: this.pxMetres(),
        en: (d) => [(d.lon - this.origin.lon) * 111320 * cosLat, (d.lat - this.origin.lat) * 111320],
        lift: (f) => (f.trail.dots.length ? ground(f.trail.dots[0]) - base : 0),
      };
    }

    build(flights, threeD) {
      const T = window.THREE;
      this.scene.clear();
      this.trail = new T.Group();
      this.glyph = new T.Group();
      this.scene.add(this.trail, this.glyph);
      const first = flights.find((f) => !f.dim && f.trail.dots.length);
      this.origin = first ? first.trail.dots[0] : null;
      if (!this.origin || !threeD) return;
      const { px, en, lift } = this.frame();
      const m = new T.Matrix4();
      const instanced = (geometry, material, matrices) => {
        if (!matrices.length) return;
        const mesh = new T.InstancedMesh(geometry, material, matrices.length);
        matrices.forEach((x, k) => mesh.setMatrixAt(k, x));
        mesh.frustumCulled = false;   // the camera is the map's whole projection, not one three can cull against
        this.trail.add(mesh);
      };
      for (const f of flights) {
        const up = lift(f);
        const opts = { color: f.colour, transparent: f.dim, opacity: f.dim ? 0.35 : 1 };
        const solid = [];
        const hollow = [];
        const dashes = [];
        const dash = 8 * px;
        let prev = null;
        let along = 0;
        for (const d of f.trail.dots) {
          if (d.h === null) { prev = null; continue; }
          const [x, y] = en(d);
          const p = new T.Vector3(x, y, up + d.h);
          (d.hollowH ? hollow : solid).push(m.clone().compose(p, new T.Quaternion(), new T.Vector3(4 * px, 4 * px, 4 * px)));
          // dashes along the path from the previous dot, carrying the pattern over, never across a break
          if (prev && d.joined) {
            const len = p.distanceTo(prev);
            const dir = p.clone().sub(prev).normalize();
            const turn = new T.Quaternion().setFromUnitVectors(new T.Vector3(0, 0, 1), dir);
            for (let s0 = (2 * dash - along % (2 * dash)) % (2 * dash); s0 < len; s0 += 2 * dash) {
              const piece = Math.min(dash, len - s0);
              dashes.push(m.clone().compose(prev.clone().addScaledVector(dir, s0), turn, new T.Vector3(1.5 * px, 1.5 * px, piece)));
            }
            along += len;
          } else {
            along = 0;
          }
          prev = p;
        }
        instanced(this.unit, new T.MeshBasicMaterial(opts), solid);
        instanced(this.unit, new T.MeshBasicMaterial(Object.assign({ wireframe: true }, opts)), hollow);
        instanced(this.stick, new T.MeshBasicMaterial(opts), dashes);
      }
    }

    // Each board's glyph, apart from the trail so moving the cursor does not rebuild the trail
    glyphs(glyphs, threeD) {
      const T = window.THREE;
      if (!this.origin) return;
      this.glyph.clear();
      const { px, en, lift } = this.frame();
      for (const g of glyphs) {
        const [x, y] = en(g.at);
        const plate = new T.Group();
        const s = 26 * px;
        const board = new T.Mesh(new T.BoxGeometry(s, s, s / 6), new T.MeshBasicMaterial({ color: g.colour, wireframe: !g.ok }));
        const arrow = new T.ArrowHelper(new T.Vector3(1, 0, 0), new T.Vector3(0, 0, 0), 1.3 * s, 0xffffff, 0.45 * s, 0.3 * s);
        const stem = new T.ArrowHelper(new T.Vector3(0, 0, 1), new T.Vector3(0, 0, 0), 1.1 * s, g.colour, 0.3 * s, 0.2 * s);
        plate.add(board, arrow, stem);
        const [r, i, j, k] = g.q;
        plate.quaternion.set(i, j, k, r);
        plate.position.set(x, y, threeD && g.h !== null ? lift(g.flight) + g.h : 0);
        this.glyph.add(plate);
        if (g.accel) {
          // linear acceleration in the world frame, 1 m/s2 to 4 px, from the board
          const v = new T.Vector3(...g.accel).applyQuaternion(plate.quaternion);
          if (v.length() > 0.05) this.glyph.add(new T.ArrowHelper(v.clone().normalize(), plate.position, v.length() * 4 * px, 0xffd400, 6 * px, 4 * px));
        }
      }
      this.glyph.traverse((o) => { o.frustumCulled = false; });
    }

    render(gl, args) {
      if (!this.origin) return;
      const T = window.THREE;
      const at = maplibregl.MercatorCoordinate.fromLngLat([this.origin.lon, this.origin.lat], this.map.queryTerrainElevation([this.origin.lon, this.origin.lat]) || 0);
      const s = at.meterInMercatorCoordinateUnits();
      const m = new T.Matrix4().fromArray(args.defaultProjectionData.mainMatrix);
      const l = new T.Matrix4().makeTranslation(at.x, at.y, at.z).scale(new T.Vector3(s, -s, s));
      this.camera.projectionMatrix = m.multiply(l);
      this.renderer.resetState();
      this.renderer.render(this.scene, this.camera);
    }
  }

  // o: {live, toast, onDraw}. Data comes in through show(); the cursor through setTime().
  class FlightView {
    constructor(host, o) {
      this.o = o;
      this.flights = [];
      this.mode = o.live ? 'find' : '3d';
      this.source = 'baro';
      this.follow = !!o.live;
      this.fitted = '';
      this.labels = [];
      this.station = this.loadStation();
      this.imagery = 'imagery-usgs';
      host.classList.add('fv');
      host.innerHTML = '';
      this.bar = el('div', 'fv-bar');
      this.bar.innerHTML =
        '<span class="seg" role="group" aria-label="View"><button data-mode="find">Find, north up</button><button data-mode="3d">3D</button></span>' +
        '<span class="seg" role="group" aria-label="Height from"><button data-src="baro">Barometer</button><button data-src="gps">GPS</button></span>' +
        '<select data-imagery class="quiet" title="Imagery"><option value="imagery-usgs">USGS imagery</option><option value="imagery-esri">Esri imagery, online only</option></select>' +
        '<button data-follow title="Keep the picked board in view">Follow</button>' +
        '<button data-station title="Click the map where the station is">Set station</button>' +
        '<button data-save title="Save imagery and terrain around this view for use offline">Save this area</button>';
      this.ask = el('div', 'confirm fv-ask');
      this.ask.hidden = true;
      const body = el('div', 'fv-body');
      this.box = el('div', 'fv-map');
      this.note = el('div', 'fv-note');
      this.note.hidden = true;
      this.side = el('div', 'fv-side');
      this.box.appendChild(this.note);
      body.append(this.box, this.side);
      host.append(this.bar, this.ask, body);
      this.bar.addEventListener('click', (e) => this.onBar(e));
      this.bar.addEventListener('change', (e) => { if (e.target.dataset.imagery !== undefined) this.setImagery(e.target.value); });
      this.ask.addEventListener('click', (e) => this.onAsk(e));
      this.side.addEventListener('click', (e) => this.onSide(e));
      try {
        this.map = new maplibregl.Map({
          // the station if one is set, else the field every drop so far has been on
          container: this.box, style: style(this.imagery), zoom: 16, maxPitch: 80,
          center: this.station ? [this.station.lon, this.station.lat] : [-76.3847, 42.4577],
          attributionControl: { compact: false }, fadeDuration: 0,
        });
      } catch (err) {
        this.note.hidden = false;
        this.note.textContent = 'No map: the browser gave this page no WebGL (' + (err.message || err) + ').';
        return;
      }
      this.map.addControl(new maplibregl.NavigationControl({ visualizePitch: true }), 'top-right');
      this.map.addControl(new maplibregl.ScaleControl({ unit: 'metric', maxWidth: 160 }), 'bottom-left');
      this.map.on('load', () => {
        this.loaded = true;
        this.air = window.THREE ? new Air(this) : null;
        if (this.air) this.map.addLayer(this.air);
        else window.addEventListener('three', () => { this.air = new Air(this); this.map.addLayer(this.air); }, { once: true });
        this.applyMode();
        this.draw();
      });
      this.map.on('movestart', (e) => {
        stats.ok = 0; stats.missing = 0;
        // a pan or zoom by hand ends following; the map's own easing does not
        if (e.originalEvent && this.follow) { this.follow = false; this.markBar(); }
      });
      this.map.on('moveend', () => { if (this.loaded) this.map.getSource('grid').setData((this.grid = grid(this.map)).data); });
      this.map.on('zoomend', () => this.drawAir());
      this.map.on('idle', () => this.sayCover());
      this.map.on('click', (e) => {
        if (!this.picking) return;
        this.picking = false;
        this.setStation(e.lngLat.lat, e.lngLat.lng, 'set on the map');
      });
      // a missing tile is said by sayCover; anything else still goes to the console
      this.map.on('error', (e) => { if (!e.tile && !e.sourceId) console.error(e.error); });
      new ResizeObserver(() => this.map.resize()).observe(this.box);
      this.markBar();
    }

    loadStation() {
      try { return JSON.parse(localStorage.getItem('dg-station')) || null; } catch (e) { return null; }
    }

    setStation(lat, lon, from) {
      this.station = { lat, lon, from };
      try { localStorage.setItem('dg-station', JSON.stringify(this.station)); } catch (e) { /* storage blocked: kept for this page */ }
      this.o.toast('Station at ' + coords(lat, lon) + ', ' + from);
      this.draw();
    }

    markBar() {
      for (const b of this.bar.querySelectorAll('[data-mode]')) b.classList.toggle('on', b.dataset.mode === this.mode);
      for (const b of this.bar.querySelectorAll('[data-src]')) b.classList.toggle('on', b.dataset.src === this.source);
      const f = this.bar.querySelector('[data-follow]');
      f.hidden = !this.o.live;
      f.classList.toggle('on', this.follow);
      f.textContent = this.follow ? 'Following' : 'Back to live';
      this.bar.querySelector('[data-station]').classList.toggle('on', !!this.picking);
    }

    onBar(e) {
      const b = e.target.closest('button');
      if (!b) return;
      if (b.dataset.mode) { this.mode = b.dataset.mode; this.follow = this.o.live && this.mode === 'find'; this.applyMode(); }
      else if (b.dataset.src) { this.source = b.dataset.src; this.draw(); }
      else if (b.dataset.follow !== undefined) { this.follow = !this.follow; this.draw(); }
      else if (b.dataset.station !== undefined) {
        this.picking = !this.picking;
        this.o.toast(this.picking ? 'Click the map where the station is' : 'Station left as it was');
      } else if (b.dataset.save !== undefined) this.askSave();
      this.markBar();
    }

    onSide(e) {
      const b = e.target.closest('button');
      if (!b) return;
      if (b.dataset.copy) {
        navigator.clipboard.writeText(b.dataset.copy).then(() => this.o.toast('Copied ' + b.dataset.copy), (err) => this.o.toast('Could not copy: ' + (err.message || err)));
      } else if (b.dataset.stationFrom) {
        const f = this.flights.find((x) => x.key === b.dataset.stationFrom);
        const fix = f && f.trail.dots[f.trail.dots.length - 1];
        if (fix) this.setStation(fix.lat, fix.lon, 'from ' + f.name + '\'s fix at ' + clock(fix.t));
      }
    }

    applyMode() {
      if (!this.loaded) return;
      const flat = this.mode === 'find';
      this.map.setTerrain(flat ? null : { source: 'terrain', exaggeration: 1 });
      // a jump, not an ease: an ease cut short by the fit leaves MapLibre's ground height frozen
      // from before the terrain loaded, with the camera under the ground and nothing drawn
      this.map.jumpTo(flat ? { pitch: 0, bearing: 0 } : { pitch: 60 });
      this.map.dragRotate[flat ? 'disable' : 'enable']();
      this.map.touchZoomRotate[flat ? 'disableRotation' : 'enableRotation']();
      this.draw();
    }

    setImagery(id) {
      this.imagery = id;
      if (!this.loaded) return;
      this.map.removeLayer('imagery');
      this.map.removeSource('imagery');
      const src = SOURCES[id];
      this.map.addSource('imagery', { type: 'raster', tiles: ['dgtile://' + id + '/{z}/{x}/{y}'], tileSize: 256, maxzoom: src.maxzoom, attribution: src.attribution });
      this.map.addLayer({ id: 'imagery', type: 'raster', source: 'imagery' }, 'lines-casing');
      this.o.toast(src.name + ' imagery' + (src.save ? '' : ', fetched while online and not saved for offline'));
    }

    // Under a view with imagery missing: say so, and that the grid is the scale
    sayCover() {
      const step = this.grid ? this.grid.step : null;
      const words = !stats.missing ? ''
        : stats.ok ? stats.missing + (stats.missing === 1 ? ' imagery tile' : ' imagery tiles') + ' not saved here' + (navigator.onLine === false ? ' and no connection' : '')
          : 'No imagery saved here' + (navigator.onLine === false ? ', and no connection' : '') + '. Grid lines every ' + step + ' m.';
      if (!this.picking) {
        this.note.hidden = !words;
        this.note.textContent = words;
      }
    }

    // flights: [{key, name, index, samples, base, gaps, dim, sel, before, firstT, card: {lastT, battery, rx, silent} | null}];
    // base is the boot's samples the heights start from, when samples is only part of it; before is
    // a board's earlier runs, oldest first ({samples, firstT}), which its Find card looks back over
    show(flights, opt) {
      this.opt = opt || {};
      const sig = flights.map((f) => [f.key, f.samples.length, f.samples.length && f.samples[f.samples.length - 1].t, f.dim, f.sel, f.card && f.card.silent].join(':')).join('|');
      const same = sig === this.sig && this.loaded;
      this.sig = sig;
      for (const f of this.flights) {
        const now = flights.find((x) => x.key === f.key);
        if (now) f.card = now.card;
      }
      this.input = flights;
      if (same) this.drawFind(); else this.draw();
    }

    draw() {
      if (!this.input || !this.loaded) return;
      this.flights = this.input.map((f) => {
        const h = F.heights(f.base || f.samples, this.source);
        const trail = F.trail(f.samples, f.gaps, h.of);
        // on the ground a dot is hollow on a held fix; in the air also where the height's flag is clear
        for (const d of trail.dots) d.hollowH = d.held || (h.ok ? !h.ok(f.samples[d.i]) : false);
        return Object.assign({}, f, { h, trail, colour: colour(f.index) }, f.card ? landing(f) : {});
      });
      const features = (fn) => ({ type: 'FeatureCollection', features: this.flights.flatMap(fn) });
      const opacity = (f) => (f.dim ? 0.35 : 1);
      this.map.getSource('lines').setData(features((f) => f.trail.lines.map((l) => ({
        type: 'Feature', properties: { colour: f.colour, opacity: opacity(f) }, geometry: { type: 'LineString', coordinates: l.map((d) => [d.lon, d.lat]) },
      }))));
      this.map.getSource('dots').setData(features((f) => f.trail.dots.filter((d, i) => !d.again || i === f.trail.dots.length - 1).map((d) => ({
        type: 'Feature', properties: { colour: f.colour, opacity: opacity(f), hollow: d.held, last: !f.dim && d === f.trail.dots[f.trail.dots.length - 1] },
        geometry: { type: 'Point', coordinates: [d.lon, d.lat] },
      }))));
      this.drawFind();
      this.drawLabels();
      this.drawAir();
      this.place();
      if (this.o.onDraw) this.o.onDraw();
    }

    // Live: a card for each board on the ground or silent, the ring where it lies, the station
    drawFind() {
      const rings = [];
      const cards = [];
      const now = this.opt.now;
      for (const f of this.flights) {
        if (!f.card || f.dim || (!f.td && !f.card.silent)) continue;
        if (f.spot) rings.push(this.circle(f.spot.lat, f.spot.lon, f.spot.half));
        cards.push(findCardHtml(Object.assign({ key: f.key, name: f.name, colour: f.colour, why: f.td ? 'down' : 'silent', td: f.td, spot: f.spot, restart: f.restart, station: this.station, now }, f.card)));
      }
      this.map.getSource('ring').setData({ type: 'FeatureCollection', features: rings });
      const zeros = this.flights.reduce((n, f) => n + (f.dim ? 0 : F.zeroFixes(f.samples)), 0);
      const html = cards.join('') + (zeros ? '<p class="note">' + zeros + ' fixes at 0, 0 with the GPS flag set, not drawn.</p>' : '');
      if (this.side.dgHtml !== html) { this.side.innerHTML = html; this.side.dgHtml = html; }
      this.side.hidden = !html;
    }

    circle(lat, lon, r) {
      const pts = [];
      for (let a = 0; a <= 360; a += 10) {
        const t = a * Math.PI / 180;
        pts.push([lon + Math.sin(t) * Math.max(r, 0.5) / (111320 * Math.cos(lat * Math.PI / 180)), lat + Math.cos(t) * Math.max(r, 0.5) / 111320]);
      }
      return { type: 'Feature', properties: {}, geometry: { type: 'LineString', coordinates: pts } };
    }

    // A board at the cursor, or at its newest sample in Live: the last fix, quaternion and height
    // at or before it. Never anything between samples.
    at(f) {
      const i = F.at(f.samples, this.cursor === undefined ? Infinity : this.cursor);
      const fi = lastIndex(f.samples, i, F.isFix);
      const qi = lastIndex(f.samples, i, (s) => s.q);
      const hi = lastIndex(f.samples, i, (s) => f.h.of(s) !== null);
      return { fix: fi < 0 ? null : f.samples[fi], s: qi < 0 ? null : f.samples[qi], h: hi < 0 ? null : f.h.of(f.samples[hi]) };
    }

    // HTML markers need no font server: a name where each board is, with its rotation rate, what a
    // break lost, the station
    drawLabels() {
      for (const m of this.labels) m.remove();
      this.labels = [];
      const add = (cls, html, lon, lat, anchor) => {
        this.labels.push(new maplibregl.Marker({ element: el('span', cls, html), anchor: anchor || 'left', offset: anchor ? [0, 0] : [12, 0] }).setLngLat([lon, lat]).addTo(this.map));
      };
      for (const f of this.flights) {
        if (f.dim) continue;
        const { fix, s } = this.at(f);
        const gyro = s && s.gyro ? ' <small>' + fixed(Math.hypot(...s.gyro), 0) + ' deg/s</small>' : '';
        if (fix) add('fv-label', esc(f.name) + gyro, fix.lon, fix.lat);
        for (const m of f.trail.marks) add('fv-gap', esc(m.words), m.lon, m.lat, 'center');
      }
      if (this.station) add('fv-station', 'Station', this.station.lon, this.station.lat);
    }

    glyphs() {
      const out = [];
      for (const f of this.flights) {
        if (f.dim) continue;
        const { fix, s, h } = this.at(f);
        const o = s && F.orientation(s.q);
        if (fix && o) out.push({ flight: f, at: fix, q: o.q, ok: s.ok.quat, colour: f.colour, h, accel: s.ok.accel ? s.accel : null });
      }
      return out;
    }

    airReady() { this.drawAir(); }

    drawAir() {
      if (!this.air || !this.air.scene || !this.flights.length) return;
      this.air.build(this.flights, this.mode === '3d');
      this.drawGlyphs();
    }

    drawGlyphs() {
      if (!this.air || !this.air.scene || !this.flights.length) return;
      this.air.glyphs(this.glyphs(), this.mode === '3d');
      this.map.triggerRepaint();
    }

    // Fit to the flights once per new set; follow the picked board in Live find mode
    place() {
      const all = this.flights.flatMap((f) => f.trail.dots);
      const sig = this.flights.filter((f) => f.trail.dots.length).map((f) => f.key).join(',');
      if (!all.length) return;
      if (sig !== this.fitted) {
        this.fitted = sig;
        const lons = all.map((d) => d.lon);
        const lats = all.map((d) => d.lat);
        // fitBounds with terrain on leaves MapLibre 5.24 drawing nothing at all (seen in Chromium,
        // 2026-11-09); a jump to the same camera does not. The jump also stops the ease into 3D,
        // so it carries the mode's pitch itself.
        const cam = this.map.cameraForBounds([[Math.min(...lons), Math.min(...lats)], [Math.max(...lons), Math.max(...lats)]], { padding: 60, maxZoom: 18 });
        this.map.jumpTo(Object.assign(cam, { pitch: this.mode === '3d' ? 60 : 0 }));
        return;
      }
      if (!this.follow) return;
      const f = this.flights.find((x) => x.sel && !x.dim) || this.flights.find((x) => !x.dim);
      const target = f && (f.spot || f.trail.dots[f.trail.dots.length - 1]);
      if (!target) return;
      const c = this.map.getCenter();
      if (F.metres(c.lat, c.lng, target.lat, target.lon) > 2) this.map.easeTo({ center: [target.lon, target.lat], duration: 400 });
    }

    // Analysis: the cursor's time on the samples' own clock
    setTime(t) {
      this.cursor = t;
      this.drawLabels();
      this.drawGlyphs();
    }

    async askSave() {
      if (!this.loaded) return;
      let stored = null;
      try {
        const r = await fetch('api/tiles');
        if (r.ok) stored = await r.json();
      } catch (e) { stored = null; }   // the estimate then says it used typical sizes
      const b = this.map.getBounds();
      const est = saveEstimate({ west: b.getWest(), east: b.getEast(), north: b.getNorth(), south: b.getSouth() }, stored);
      this.pending = est;
      const [im, te] = est.parts;
      this.ask.hidden = false;
      this.ask.innerHTML = est.tiles > SAVE_MOST
        ? '<span>This view needs ' + est.tiles.toLocaleString('en-US') + ' tiles, more than the ' + SAVE_MOST + ' one save asks of the servers. Zoom in.</span><button data-cancel>Close</button>'
        : '<span>Save ' + im.tiles.length + ' USGS imagery tiles (zoom ' + SOURCES['imagery-usgs'].save.join(' to ') + ') and ' + te.tiles.length +
          ' terrain tiles (zoom ' + SOURCES.terrain.save.join(' to ') + '), about ' + mb(est.bytes) + ' (' +
          (est.measured ? 'from the tiles already saved' : 'from typical tile sizes') + '). Tiles already saved are not fetched again. Esri imagery is never saved.</span>' +
          '<button class="primary" data-go>Save</button><button data-cancel>Cancel</button>';
    }

    async onAsk(e) {
      const b = e.target.closest('button');
      if (!b) return;
      if (b.dataset.cancel !== undefined) { this.ask.hidden = true; this.saving = false; return; }
      if (b.dataset.go === undefined || !this.pending) return;
      const list = this.pending.parts.flatMap((p) => p.tiles);
      this.saving = true;
      let done = 0;
      let failed = 0;
      const say = () => { this.ask.innerHTML = '<span>Saving ' + done + ' of ' + list.length + (failed ? ', ' + failed + ' not available' : '') + '</span><button data-cancel>Stop</button>'; };
      say();
      const next = async () => {
        while (this.saving && list.length) {
          const t = list.shift();
          try {
            await loadTile({ url: 'dgtile://' + t.layer + '/' + t.z + '/' + t.x + '/' + t.y }, new AbortController());
          } catch (err) {
            failed++;
          }
          done++;
          say();
        }
      };
      const total = list.length;
      await Promise.all([next(), next(), next(), next()]);
      const stopped = !this.saving;
      this.saving = false;
      this.ask.hidden = true;
      this.o.toast((stopped ? 'Stopped after ' : 'Saved ') + (done - failed) + ' of ' + total + ' tiles' + (failed ? ', ' + failed + ' not available from the server' : '') +
        (stats.unsaved ? ', ' + stats.unsaved + ' the app could not write' : ''));
    }
  }

  function lastIndex(samples, i, test) {
    for (let k = i; k >= 0; k--) if (test(samples[k])) return k;
    return -1;
  }

  Object.assign(window.DG = window.DG || {}, { FlightView, flightmap: pure });
})();
