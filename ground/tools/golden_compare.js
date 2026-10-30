// Holds the Rust decoder's lines against lines.js's, field by field, and counts.
// node golden_compare.js <js.jsonl> <rust.jsonl>
// Prints the counts and up to 3 examples of each kind of mismatch; exits 1 on any.
const fs = require('node:fs');
const readline = require('node:readline');

// Paths where a and b differ, as 'decoded.values.lat'. Numbers must be the same double.
function diff(a, b, at, out) {
  if (a !== null && b !== null && typeof a === 'object' && typeof b === 'object') {
    const ka = Object.keys(a); const kb = Object.keys(b);
    if (ka.join() !== kb.join()) out.push((at || '.') + ' keys');
    for (const k of ka) if (k in b) diff(a[k], b[k], at ? at + '.' + k : k, out);
  } else if (!Object.is(a, b)) {
    out.push(at || '.');
  }
  return out;
}

async function main() {
  const [jsPath, rustPath] = process.argv.slice(2);
  const open = (p) => readline.createInterface({ input: fs.createReadStream(p), crlfDelay: Infinity })[Symbol.asyncIterator]();
  const js = open(jsPath); const rs = open(rustPath);
  const n = { files: 0, lines: 0, packets: 0, packetsCrcOk: 0, sameText: 0, mismatches: 0 };
  const kinds = {}; const classes = new Map();
  let file = '';
  for (;;) {
    const [x, y] = await Promise.all([js.next(), rs.next()]);
    if (x.done || y.done) {
      if (x.done !== y.done) { n.mismatches++; classes.set('one output is longer', [{ file, js: x.value, rust: y.value }]); }
      break;
    }
    if (x.value === y.value) n.sameText++;
    const a = JSON.parse(x.value); const b = JSON.parse(y.value);
    if ('file' in a) { file = a.file; n.files++; if (a.file !== b.file) { n.mismatches++; classes.set('file', [{ file, js: a.file, rust: b.file }]); } continue; }
    n.lines++;
    kinds[a.kind] = (kinds[a.kind] || 0) + 1;
    if (a.kind === 'packet') { n.packets++; if (a.decoded.crcOk) n.packetsCrcOk++; }
    const d = diff(a, b, '', []);
    if (d.length) {
      n.mismatches++;
      const key = a.kind + ': ' + d.join(' ');
      if (!classes.has(key)) classes.set(key, []);
      if (classes.get(key).length < 3) classes.get(key).push({ file, js: x.value, rust: y.value });
    }
  }
  console.log(JSON.stringify(n));
  console.log('kinds', JSON.stringify(kinds));
  for (const [key, ex] of classes) {
    console.log('\nMISMATCH ' + key);
    for (const e of ex) console.log('  ' + e.file + '\n    js   ' + e.js + '\n    rust ' + e.rust);
  }
  process.exit(n.mismatches ? 1 : 0);
}

main();
