// lines.js over a list of logs, one JSON line per line parsed, for golden_compare.js to
// hold against app/src/lines.rs. Files are read the way Open log reads them
// (recorder.js parseLogFile). The byte array is left out; hex already says it.
// node golden_decode.js <file of paths> <out.jsonl>
const fs = require('node:fs');
const path = require('node:path');
// the reference JS, which left web/ in 1.4; the .sh scripts take it from git at 1.3.0
const web = process.env.DG_REF_JS || path.join(__dirname, '..', 'web', 'js');
const L = require(path.join(web, 'lines.js'));
const R = require(path.join(web, 'recorder.js'));

const [list, out] = process.argv.slice(2);
const fd = fs.openSync(out, 'w');
let buf = [];
const flush = () => { fs.writeSync(fd, buf.join('')); buf = []; };
for (const p of fs.readFileSync(list, 'utf8').split('\n').filter(Boolean)) {
  buf.push(JSON.stringify({ file: p }) + '\n');
  for (const e of R.parseLogFile(fs.readFileSync(p, 'utf8'), path.basename(p), 0)) {
    const { bytes, ...ev } = L.parseLine(e.text);
    buf.push(JSON.stringify(ev) + '\n');
    if (buf.length > 5000) flush();
  }
}
flush();
fs.closeSync(fd);
