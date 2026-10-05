// The team's lab captures are not in this repo. The checks against them run when
// DESCENT_LAB_LOGS is the folder that holds V2_5_X and V2_6_X (Software/ in the team's
// repo). Without it they are skipped and say so; test/data holds enough to run the rest.
const fs = require('node:fs');
const path = require('node:path');

const ROOT = process.env.DESCENT_LAB_LOGS || '';
const at = (...parts) => path.join(ROOT, ...parts);

// Why a test that needs these files is skipped, or false when they are there.
function missing(...parts) {
  if (!ROOT) return 'lab logs not present, set DESCENT_LAB_LOGS to the folder';
  return fs.existsSync(at(...parts)) ? false : 'DESCENT_LAB_LOGS is set, but ' + at(...parts) + ' is not there';
}

module.exports = { at, missing, DATA: path.join(__dirname, 'data') };
