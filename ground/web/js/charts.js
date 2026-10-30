// History charts (uPlot). One set of panels per tab; each panel has its own
// y axis so no chart ever mixes units. The x axis spans exactly the data
// shown: from the first to the last packet kept (or the chosen window,
// counted back from the newest packet).
(function (root) {
  'use strict';

  const css = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

  // A token's hex at some opacity. Anything not a 6-digit hex comes back as it is.
  function alpha(value, a) {
    const hex = value.replace('#', '');
    if (!/^[0-9a-f]{6}$/i.test(hex)) return value;
    const n = parseInt(hex, 16);
    return 'rgba(' + (n >> 16 & 255) + ',' + (n >> 8 & 255) + ',' + (n & 255) + ',' + a + ')';
  }
  // Series colour at 70% opacity: noisy data reads calmer.
  const seriesColor = (i) => alpha(css('--series-' + (i + 1)), 0.7);
  const readoutFormats = new Map();
  function readoutNum(v, d) {
    if (!readoutFormats.has(d)) readoutFormats.set(d, new Intl.NumberFormat('en-US', { minimumFractionDigits: d, maximumFractionDigits: d }));
    return readoutFormats.get(d).format(v);
  }

  // Tab -> panels -> series. `key` reads unit.history[key]; `rx` = per receiver.
  // A tab marked custom draws no plots: the page fills the panel itself. Runs is one,
  // because a board's earlier runs are a list to act on, not a graph.
  const TABS = [
    { id: 'overview', label: 'Overview', panels: [
      { title: 'Cell voltage', unit: 'V', dec: 2, series: [{ key: 'battery', label: 'Cell voltage' }] },
      { title: 'Pressure altitude', unit: 'm', dec: 1, series: [{ key: 'envAlt', label: 'Altitude' }] },
      { title: 'RSSI, best receiver', unit: 'dBm', dec: 1, series: [{ key: 'rssi', label: 'RSSI' }] },
      { title: 'SNR, best receiver', unit: 'dB', dec: 1, series: [{ key: 'snr', label: 'SNR' }] },
    ] },
    { id: 'imu', label: 'IMU', panels: [
      { title: 'Linear acceleration', unit: 'm/s²', band: 'saturationMps2', series: [{ key: 'ax', label: 'X' }, { key: 'ay', label: 'Y' }, { key: 'az', label: 'Z' }] },
      { title: 'Gyroscope', unit: 'deg/s', series: [{ key: 'gx', label: 'X' }, { key: 'gy', label: 'Y' }, { key: 'gz', label: 'Z' }] },
      { title: 'Magnetometer', unit: 'µT', series: [{ key: 'mx', label: 'X' }, { key: 'my', label: 'Y' }, { key: 'mz', label: 'Z' }] },
      { title: 'Quaternion', series: [{ key: 'qi', label: 'i' }, { key: 'qj', label: 'j' }, { key: 'qk', label: 'k' }, { key: 'qr', label: 'real' }] },
    ] },
    { id: 'env', label: 'Environment', panels: [
      { title: 'Temperature', unit: '°C', dec: 2, series: [{ key: 'temp', label: 'Temperature' }] },
      { title: 'Pressure', unit: 'hPa', dec: 1, series: [{ key: 'pressure', label: 'Pressure' }] },
      { title: 'Humidity', unit: '%RH', dec: 1, series: [{ key: 'humidity', label: 'Humidity' }] },
      { title: 'Pressure altitude', unit: 'm', dec: 1, series: [{ key: 'envAlt', label: 'Altitude' }] },
    ] },
    { id: 'gps', label: 'GPS', panels: [
      { title: 'GPS altitude MSL', unit: 'm', dec: 1, series: [{ key: 'gpsAlt', label: 'Altitude' }] },
      { title: 'Latitude', unit: 'deg', dec: 6, series: [{ key: 'lat', label: 'Latitude' }] },
      { title: 'Longitude', unit: 'deg', dec: 6, series: [{ key: 'lon', label: 'Longitude' }] },
    ] },
    { id: 'radio', label: 'Radio', panels: [
      { title: 'RSSI per receiver', unit: 'dBm', dec: 1, rx: 'rssi' },
      { title: 'SNR per receiver', unit: 'dB', dec: 2, rx: 'snr' },
    ] },
    { id: 'runs', label: 'Runs', custom: true },
  ];

  const clock = (s) => new Date(s * 1000).toTimeString().slice(0, 8);
  const PLOT_H = 190;
  const CARD_HEAD = 50;   // a chart card's title row and padding, around the plot

  class Charts {
    constructor(container, tabsEl, rxLabel) {
      this.el = container;
      this.tabsEl = tabsEl;
      this.rxLabel = rxLabel;     // rxKey -> {label, index}
      const wanted = new URLSearchParams(location.search).get('tab');
      this.tab = TABS.some((t) => t.id === wanted) ? wanted : 'overview';
      this.plots = [];
      this.sig = '';
      for (const t of TABS) {
        const b = document.createElement('button');
        b.textContent = t.label; b.dataset.tab = t.id;
        b.addEventListener('click', () => { this.tab = t.id; this.sig = ''; this.markTabs(); if (this.last) this.update(...this.last); });
        tabsEl.appendChild(b);
      }
      this.markTabs();
      new ResizeObserver(() => this.resize()).observe(container);
    }

    // Another part of the page asks for a tab, like the runs pill on a unit card
    showTab(id) {
      if (!TABS.some((t) => t.id === id) || id === this.tab) return;
      this.tab = id; this.sig = ''; this.markTabs();
    }

    markTabs() {
      for (const b of this.tabsEl.children) b.classList.toggle('on', b.dataset.tab === this.tab);
    }

    // The cards fill the panel, as many rows as the tab needs. Where the panel grows with
    // what is in it (stacked, narrow windows) a plot sized to it would grow it again, so
    // there it stays at PLOT_H.
    plotSize(box) {
      const width = Math.max(240, box.clientWidth);
      const st = getComputedStyle(this.el);
      if (st.overflowY === 'visible' || !this.el.clientHeight) return { width, height: PLOT_H };
      const cols = st.gridTemplateColumns.split(' ').length;
      const rows = Math.max(1, Math.ceil((this.count || 1) / cols));
      const gap = parseFloat(st.rowGap) || 0;
      const each = (this.el.clientHeight - gap * (rows - 1)) / rows - CARD_HEAD;
      return { width, height: Math.round(Math.max(110, Math.min(320, each))) };
    }

    resize() {
      for (const p of this.plots) {
        const size = this.plotSize(p.box);
        if (Math.abs(size.width - p.u.width) > 2 || Math.abs(size.height - p.u.height) > 2) p.u.setSize(size);
      }
    }

    // Column arrays for one panel, from t0 (seconds) onwards.
    panelData(panel, unit, t0) {
      if (!unit) return null;
      if (panel.rx) {
        const keys = Object.keys(unit.rxHistory);
        if (!keys.length) return null;
        const times = [...new Set(keys.flatMap((k) => unit.rxHistory[k].t))].filter((t) => t >= t0).sort((a, b) => a - b);
        const idx = new Map(times.map((t, i) => [t, i]));
        const cols = keys.map((k) => {
          const col = new Array(times.length).fill(null);
          const h = unit.rxHistory[k];
          h.t.forEach((t, i) => { if (idx.has(t) && Number.isFinite(h[panel.rx][i])) col[idx.get(t)] = h[panel.rx][i]; });
          return col;
        });
        return { data: [times].concat(cols), labels: keys.map((k) => this.rxLabel(k)) };
      }
      const h = unit.history;
      let start = 0;
      while (start < h.t.length && h.t[start] < t0) start++;
      // Break the line where packets are missing (gap > 3 intervals) so a
      // straight segment never pretends there was data.
      const gapS = Number.isFinite(unit.intervalMs) ? 3 * unit.intervalMs / 1000 : Infinity;
      const data = [[]].concat(panel.series.map(() => []));
      for (let i = start; i < h.t.length; i++) {
        if (i > start && h.t[i] - h.t[i - 1] > gapS) {
          data[0].push((h.t[i] + h.t[i - 1]) / 2);
          for (let s = 0; s < panel.series.length; s++) data[s + 1].push(null);
        }
        data[0].push(h.t[i]);
        panel.series.forEach((s, j) => data[j + 1].push(h[s.key][i]));
      }
      return { data, labels: panel.series.map((s, j) => ({ label: s.label, index: j })) };
    }

    build(panels, opt) {
      this.el.classList.remove('custom');
      this.el.innerHTML = '';
      this.count = panels.length;
      // The Runs tab paints this box through shell.paint, which skips html it thinks is
      // already there. It is not any more.
      delete this.el.dgHtml;
      this.plots = [];
      const boxes = panels.map(({ panel }) => {
        const box = document.createElement('div');
        box.className = 'chart';
        const h = document.createElement('h3'); h.textContent = panel.title;
        if (panel.unit) { const u = document.createElement('span'); u.textContent = ' ' + panel.unit; h.appendChild(u); }
        const readout = document.createElement('span'); readout.className = 'readout'; h.appendChild(readout);
        box.appendChild(h);
        this.el.appendChild(box);
        return box;
      });
      panels.forEach(({ panel, pd }, i) => {
        const box = boxes[i];
        if (!pd || pd.data[0].length === 0) {
          const n = document.createElement('div'); n.className = 'none';
          n.textContent = panel.rx ? 'No receiver data yet' : 'No valid data yet';
          box.appendChild(n);
          return;
        }
        // The plain-hex aliases: getComputedStyle hands a color-mix() back unresolved.
        // --ink is --color-text, --rule is --color-neutral-200, --accent is --color-accent.
        // A faint horizontal grid only; the design has none, it stays to read values off.
        const axis = { stroke: alpha(css('--ink'), 0.7), ticks: { show: false }, font: '600 11px ' + css('--sans') };
        const grid = { stroke: css('--rule'), width: 1 };
        const few = pd.data[0].length < 40;
        const single = pd.labels.length === 1;
        // One series in the accent; several, and every receiver, in the series palette, so a
        // receiver keeps the colour of its dot in the receivers panel
        const accent = single && !panel.rx;
        const width = accent ? 2.25 * opt.lineWidth / 1.25 : opt.lineWidth;
        const readout = box.querySelector('.readout');
        const opts = Object.assign(this.plotSize(box), {
          // One series: the title names it, so no legend; hover value goes next to the title.
          legend: { show: !single, live: true },
          cursor: { points: { size: 7 }, drag: { x: false, y: false } },
          scales: { x: { time: true } },
          axes: [Object.assign({ values: (u, splits) => splits.map(clock), grid: { show: false } }, axis), Object.assign({ size: 56, grid }, axis)],
          series: [{ value: (u, v) => (v == null ? '—' : clock(v)) }].concat(pd.labels.map((l) => ({
            label: l.label, stroke: accent ? css('--accent') : seriesColor(l.index), width, spanGaps: false,
            points: { show: few, size: 4, width: 1 },
            value: (u, v) => (v == null ? '—' : +v.toFixed(5)),
          }))),
        });
        opts.hooks = {};
        const plot = { box, panel, readout, single };
        if (single) opts.hooks.setCursor = [(u) => this.readout(plot, u.cursor.idx)];
        const lim = panel.band && opt[panel.band];
        if (lim) {
          opts.hooks.draw = [(u) => {
            // Shade beyond the saturation limit: the accelerometer is probably clipping there.
            const ctx = u.ctx; const { left, top, width, height } = u.bbox;
            ctx.save(); ctx.fillStyle = 'rgba(250,178,25,0.10)';
            for (const v of [lim, -lim]) {
              const y = u.valToPos(v, 'y', true);
              const edge = v > 0 ? top : top + height;
              if (y > top && y < top + height) ctx.fillRect(left, Math.min(y, edge), width, Math.abs(edge - y));
            }
            ctx.restore();
          }];
        }
        plot.u = new uPlot(opts, pd.data, box);
        this.plots.push(plot);
        if (single) this.readout(plot, null);
      });
    }

    // One series: its newest value in the card head, or the one under the cursor with its time
    readout(p, idx) {
      if (!p.single || !p.u) return;
      const ys = p.u.data[1];
      let i = idx;
      if (i == null) { i = ys.length - 1; while (i >= 0 && ys[i] == null) i--; }
      const v = i >= 0 ? ys[i] : null;
      const d = p.panel.dec === undefined ? 2 : p.panel.dec;
      p.readout.textContent = v == null ? '' : readoutNum(v, d) + (p.panel.unit ? ' ' + p.panel.unit : '');
      if (v != null && idx != null) {
        const at = document.createElement('small');
        at.textContent = ' at ' + clock(p.u.data[0][i]);
        p.readout.appendChild(at);
      }
    }

    // Redraw for the selected unit. Rebuilds DOM only when the shape changes.
    // opt: {windowMin, lineWidth, saturationMps2}
    update(unit, opt) {
      this.last = [unit, opt];
      const tab = TABS.find((t) => t.id === this.tab);
      if (tab.custom) {
        this.el.classList.add('custom');   // the panel is a list, not a grid of plots
        this.plots = [];
        this.sig = '';           // the plots are gone, so rebuild when a graph tab returns
        if (this.custom) this.custom(this.el, unit);
        return;
      }
      const newest = unit && unit.history.t.length ? unit.history.t[unit.history.t.length - 1] : 0;
      const t0 = opt.windowMin ? newest - opt.windowMin * 60 : -Infinity;
      const panels = tab.panels.map((panel) => ({ panel, pd: this.panelData(panel, unit, t0) }));
      const sig = [this.tab, unit ? unit.csid : '-', opt.lineWidth, opt.saturationMps2]
        .concat(panels.map((p) => (p.pd && p.pd.data[0].length ? p.pd.labels.map((l) => l.label).join(',') + (p.pd.data[0].length < 40 ? 'f' : '') : 'x'))).join('|');
      if (sig !== this.sig) { this.sig = sig; this.build(panels, opt); return; }
      let i = 0;
      for (const { pd } of panels) {
        if (!pd || !pd.data[0].length) continue;
        const p = this.plots[i++];
        p.u.setData(pd.data);   // x and y rescale to the data
        if (p.u.cursor.idx == null) this.readout(p, null);
      }
    }

    // Colours come from CSS; rebuild after a theme switch.
    restyle() { this.sig = ''; if (this.last) this.update(...this.last); }
  }

  (root.DG = root.DG || {}).Charts = Charts;
  root.DG.TABS = TABS;
})(self);
