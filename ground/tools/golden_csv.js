// exportcsv.js over a list of logs, for holding against app/src/csv.rs
// (csv::tests::golden writes <n>.rust.csv the same way).
//   node golden_csv.js <file of paths> <out folder>
// Each file goes through recorder.js parseLogFile and mergeFiles, as Open log reads one,
// then toCsv, into <out>/<n>.js.csv where n is its line in the list.
const fs = require('node:fs');
const path = require('node:path');
const web = path.join(__dirname, '..', 'web', 'js');
const R = require(path.join(web, 'recorder.js'));
const X = require(path.join(web, 'exportcsv.js'));

// Midnight on a fixed day, so a file without times gets the same ones both sides
const BASE_MS = 1793232000000;

const [list, out] = process.argv.slice(2);
const files = fs.readFileSync(list, 'utf8').split('\n').filter(Boolean);
files.forEach((p, n) => {
  const events = R.mergeFiles([R.parseLogFile(fs.readFileSync(p, 'utf8'), path.basename(p), BASE_MS)]);
  fs.writeFileSync(path.join(out, n + '.js.csv'), X.toCsv(events));
});
console.log(files.length + ' files');
