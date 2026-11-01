# Design guide

Identity: instrument software for the whole ChipSat, from the bench it is flashed on to the ground station that hears it. A warm paper ground, terracotta for action and alarm, sage for good, one display face for names and one text face for everything else. Numbers first. Colour only means state.

The tokens are in `web/css/style.css`, and the rules for each screen are in `live.css`, `tbeams.css`, `data.css` and `chipsats.css`, each only under its own screen's id. A class two screens need goes in `style.css`. This guide is enough to add a screen without reading them.

## Colour

Names are the design handoff's. Dark swaps the values below, and the plain-hex aliases to match; a token with no dark value is the same in both. Dark redefines the whole neutral ramp but, of the two accent ramps, only the steps in this table. The rest (terracotta 300 to 600 and 900, sage 100, 300 to 600 and 900) keep their light values.

| Token | Light | Dark | Meaning (only this) |
|---|---|---|---|
| `--color-bg` | `#faf8f5` | `#1a1817` | The page ground |
| `--color-surface` | `#f1eee8` | `#262322` | A panel (`.pane`), a sub-card |
| `--color-control` | surface | `#3a3736` | The fill of a select, a text input, a segmented control, a toggle track: one step off the card it sits on |
| `--color-control-on-surface` | neutral-100 | `#3a3736` | The same control inside a surface box (a phase card), where it takes the card colour |
| `--color-neutral-100` | `#ffffff` | `#2d2b2b` | A card inside a panel, a raised pane, the picked row |
| `--color-neutral-200` | `#f3f0eb` | `#333030` | A hairline between rows inside a card |
| `--color-neutral-300` | `#e3dfd8` | `#444141` | A track, the fill of an `off` mark, a filter pill's border |
| `--color-neutral-700` / `-800` | `#645c50` / `#474238` | `#b8b2ae` / `#d7d3d3` | Quiet control text / text on an `off` mark |
| `--color-text` | `#201e1d` | `#f3f2f2` | Values, headings, anything read |
| `--color-text-70` | text at 70 % | same | Labels, units, notes, stale values |
| `--color-divider` | text at 16 % | same | The border of a control |
| `--color-accent` | `#c67139` | same | Terracotta: the focus ring, the picked ring, the chart line, a bar that has run out. Never a fill under white text and never text |
| `--color-accent-100` / `-200` | `#fff2eb` / `#ffe1d0` | `#4d170e` / `#5c2a1a` | A soft alarm: `warn` mark, a Data row's action |
| `--color-accent-700` / `-800` | `#8c491a` / `#643312` | `#f2a477` / `#ffc4b8` | Alarm text, a ghost button / text on accent-100 or -200 |
| `--color-fill`, `--color-fill-hover` | `#8c491a`, `#643312` | same | A solid under white text: primary button, `bad` mark, step number |
| `--color-accent-2` | `#7a8a5e` | same | Sage: the bar of a unit that is OK |
| `--color-accent-2-200` / `-800` | `#e1eecc` / `#3d472b` | `#272e1b` / `#ccdbb2` | Good: `good` mark, selected tab, selected filter |
| `--color-accent-2-700` | `#56633f` | `#b3c398` | Kicker text and the brand's "ground" |
| `--color-fill-2` | `#56633f` | same | A sage solid under white text: the selected half of a segmented control, a checkbox |
| `--series-1..8` | see `style.css` | | Chart lines in fixed order (X Y Z, or receivers in connect order), drawn at 70 % opacity |

- Contrast: solid fills under white text are accent-700 (6.8:1) and sage-700 (6.5:1). `#c67139` is 3.6:1 under white and accent-600 is 4.49:1, so neither carries text. Muted text is the text colour at 70 % (5.7:1 or better); at 55 %, as the handoff drew it, it was 3.6:1. The dark values are chosen to pass the same 4.5:1.
- Status is never colour alone: a word ("Lost"), or a letter (L G M Q P S E F) that is struck through when its bit is not valid.
- A group or field that is not valid says so in its kicker, and its values go to the muted ink at normal weight, or `—`. They are not faded further, which would drop them under the floor.
- Values are never judged: the page shows what the firmware reported and the firmware's own ok and valid flags. The only colour on a value is a limit the operator set in Settings (low battery, weak RF), and its title names the limit.
- A receiver's spreading factor is a setting, not a status: text ink, no state colour. Whether the board has taken a change that was asked of it is a state, and gets a mark (see the receivers rules below).

### Aliases JavaScript reads

`charts.js` reads colours through `getComputedStyle`, which hands back a `color-mix()` unresolved. These names are therefore plain hex, never `color-mix()`, in the light block and in both dark blocks (the media query and `[data-theme="dark"]`):

- `--accent`, `--critical`, `--ink`, `--ink-2`, `--panel`, `--rule`, `--sunk`, `--sans`, `--series-1` to `--series-8`
- kept beside them for old call sites: `--bg`, `--rule-strong`, `--ink-3`, `--mono`

`test/assets.test.js` fails if a name JS reads is missing from `:root` or is not a plain value. A new colour JS needs gets a plain-hex alias, in all three places.

### The `.mark`

A state is a word in a mark. The colour repeats the word and never replaces it. Which class says which word (this is the table in `style.css`):

| Class | Look | Words |
|---|---|---|
| `good` | sage-200 on sage-800 | OK, live, complete, reading, reading it, being read, this one, the one you replugged, finished, ok=1, being written now, transmitting now, the module (LE, HP) on the ChipSats list |
| `off` | neutral-300 on neutral-800 | stale, quiet, quiet for N, not read, not read yet, unconfigured, not touched, left alone, disconnected by you, as recorded, no receiver being read, no line for N s, the port went quiet, last on the probe, earlier run |
| `warn` | accent-200 on accent-800 | incomplete, newer, on the ST-Link, module never set, dump firmware, sensor bench, being flashed, pulling, flashing, never pulled, nothing heard, not logging to a file, started, started again (a restart is worth noticing, not a failure), running … not flight software, left on the dump firmware, saturated, times not from GPS |
| `bad` | solid accent-700, white | Lost, failed, ok=0, no permission, a port that will not open |

A unit's state is a mark too: `.mark good` for OK, `off` for Stale, `bad` for Lost. Nothing else is tinted.

## Type

- Caprasimo 400 (`--font-heading`) for names and titles: the brand, a screen's title, a board's name, a card's title, a group's title.
- Figtree 400/600/700 (`--font-body`) for everything else, numerals included, tabular on the whole page. Big readouts are 700 with a slightly negative letter-spacing.
- Plex Mono (`--font-mono`) only for hex: CRC, validity mask, a board's UID, the raw console.
- Fonts are in `web/vendor/fonts/`, so the page works offline.
- Sentence case. The one exception is the kicker: 11 px, 600, uppercase, letter-spaced, in sage-700, over a list or a panel.

| Role | Size and face |
|---|---|
| Brand, screen title (`.pane-head h2`) | 22 px Caprasimo |
| Board name on a sheet, name input | 28 px Caprasimo |
| Drawer title | 26 px Caprasimo |
| Card and pane titles, a list row's board name | 17-18 px Caprasimo |
| Group titles on Live, phase names | 16 px Caprasimo |
| CSID in the detail head / on a unit card | 40 px / 22 px Figtree 700 |
| Summary count, stat tile | 24 px / 26 px Figtree 700 |
| Chart readout | 18 px Figtree 700 |
| Body on the sheets, Data and the drawer (`--t-body`) | 14 px |
| Labels, controls (`--t-label`, `--cf`) | 13 px (13.5 px on the sheets and the drawer) |
| Notes and hints (`--t-note`) | 12.5 px, muted |
| Status pills (`.mark`) | 12 px 700 |
| Kickers, table heads, validity letters, axis text, units | 11 px |

The floors: nothing is under 12.5 px except the 11 px group (kickers and table heads, validity letters, chart axis text, the unit after a value) and the 12 px pills. The operator is outdoors on a laptop, and anything smaller stops being readable in sun. A value carries the weight and its label does not.

## Controls

- Pills, all of them: `border-radius: 999px`, `--ch` high (36 px; 26 to 30 px inside a row, a receiver or a table). A button is transparent with a 1 px divider border, a select or text input is the same on `--color-control` (`--color-control-on-surface` inside a surface box), never on the colour it sits on. Hover tints the text colour at 7 % and pressing at 12 %. Disabled is 45 % opacity.
- Primary is solid `--color-fill` (accent-700) with white text, hover accent-800.
- Ghost is no border, accent-700 text, underlined on hover. It is for a link-like action: Rescan, New file, Show hidden units, Look for a probe.
- A selected `button.on` is sage-200 on sage-800. A filter is a pill with its count (`.fpill`), the same sage when on. An open-in-place list is an `.expander` with a ▾.
- The segmented control (`.seg`): a pill of buttons, the selected one filled sage-700 with white text. It is for two or three exclusive choices, like LE | HP.
- Settings' On | Off (`.toggle`) is one real checkbox laid invisibly over two faces. The checkbox is the input, so keyboard, label and form behaviour are the browser's; the faces are sage-200 for On and neutral-300 for Off.
- Focus is a 2 px accent outline with 2 px offset, on everything.
- Inputs and selects share border, radius, height and focus; selects take a chevron. A board's name is an input at title size with no border until hover (`.nameit`).
- Every click answers within a second, with a change on screen or a toast that states the outcome with its number or name: "3 boards, 1 probe", "Saved descent-….log", "New file descent-….log".

## Copy

- Facts, instructions, states, and a consequence the operator has to know before acting, said once where the action is. Nothing that comforts, promises or explains why the design is safe.
- Buttons are verbs in sentence case: "Pull the flight log", "Read the board", "Stop reading it".

## Layout

The chrome, on every screen:

- The bar is 68 px. Left: the brand, then the screens as pill tabs, each one plain word: ChipSats, T-Beams, Live, Replay, Data (the boards first, 2026-10-31). The picked one is sage-200 on sage-800. Replay is disabled until a log is open. The app serves the page, so all five screens are always there.
- Right of the bar: the strip (the receivers button, the file being logged to in bold with Save a copy and New file, a chip for a running job that goes to its screen, the build and release notices), then Open a file, Settings and the sun and moon button. Under 1640 px the bar drops two things it says elsewhere, so it keeps one row at 1440: the replayed file's name in the Replay tab (the replay bar names it) and "Logging to" before the log's name (its title says it). Under 1420 px it also drops Save a copy (the Data screen's row for the log being written saves it), so Replay keeps one row on a Surface. Two or three receivers stack inside their button. A job, build or release chip takes a second row.
- Under the bar, on Replay, a white pill bar with Play, speed, the scrubber, the time and Save log, Save CSV, Save raw, Close log. The toast is a dark pill at the bottom centre.
- Frame padding is 20 px and the gap between panels 16 px. Panels are `--radius-lg` (28 px), cards `--radius-md` (16 px). There are no rules between panels; a hairline is allowed inside a card between rows.

Live and Replay (`live.css`), a 352 px rail and the main column:

- Rail, top: the units panel. The kicker, "by attention" as a quiet select, the unit count over the issues line (no issues is "No issues"), then one card per unit. A card is the CSID, the state mark, a runs pill (opens the Runs tab), the time since the last packet, the stale bar, cell volts, dBm, the spreading factor the last packet came in on as plain "SF7", and packets with "n of m rx", and the eight validity letters. The picked card is white. "Show hidden units" is a ghost button under the list.
- Rail, bottom: the receivers panel. One row per receiver: a swatch in its series colour, its name, spreading factor and packet count, a state mark. The picked row is white and opens its controls in place.
- Main, top: the detail pane (white). The CSID at 40 px, the state mark, "Last packet" with its time and run; Pause, Hide unit and Clear… at the right. Clear opens its choices in an accent-100 row under the head, in place. The readings are four columns of groups (Link, GPS, Environment, Accelerometer, Gyroscope, Magnetometer, Orientation), two columns below 820 px of the pane's width, one below 380 px.
- Main, bottom: the history panel. Tabs as pills, the time window as a quiet select, and two columns of white chart cards whose plots fill the panel, never under 110 px (190 px where the panel grows with them). The history panel keeps room for two rows of cards at that floor, and the detail pane scrolls on its own when the window is shorter. Single-series charts have no legend; the current value sits beside the title.

ChipSats and T-Beams (the sheets), a 340 px list and a sheet:

- The list is a panel of rows, one per board: its name, then its state in words. The picked row is a white card with a 2 px terracotta ring. On ChipSats the card at the top is the probe's own: "The board on the ST-Link", what the probe sees now, Read the board, and anything the probe or a read said (an attach error, "Nothing has answered on the ST-Link since 14:02."). The sheet goes there, off any named board, when the board shown stops answering three id checks in a row (about 6 s, so one missed check does not flicker), when another board answers under the clip (before its read, which can fail), and when any read fails; not while a job runs. A board that reads gets its own sheet. A named board's sheet never carries a read error: what was said about the probe is said on the probe's view (the user, 2026-10-31: a swapped board kept the old board's sheet and its ST-Link errors).
- The sheet is a column of white cards that scrolls on its own. Its head is the name input, the state, the identity line and the actions for the whole board.
- T-Beams: the head (Listen 5 s; Stop reading it, Read this one or Leave it alone) and three cards, Radio, This session and Receiver firmware, three across from 900 px of sheet width and stacked below. Under the list, a "Which board is which?" card.
- ChipSats follows the handoff's Screen 3 (`chipsats.css`). The head card is the name input, the ST-Link pill and the identity line. Under it, while a pull, clear or flash runs or once one has ended, one card (`.now`): the job's steps and bar, where it saves and Stop, else how it ended as a title and one tight block (`.result`). It is the one place that says what is happening to the board; the other cards only switch their buttons off. Under it three step cards, each with a numbered circle: 1 Identity and radio (board id, module as LE | HP, spreading factor, transmit power), 2 Phases (one table per phase, stacked: the switch "Set by battery level (changes both columns)", then Battery, Log every, Transmit every and, once a board is read, On the board. With the switch on, five band rows with the thresholds in their first column and a Gauge unread row; with it off, one Always row using the same two boxes as Gauge unread. A box holding 0 says what that means under it), 3 Write (what changes from the board row by row, as "Air, below 20 %, log 0.05 s → 5 s", one line naming the board id, module, SF and power, and the one primary button, which writes on the first press). Below those, the Flight logs card, top down: the latest pull with Open latest and All n ▾, then rows for the console port, the keep box, Pull the flight log and Clear flash, each with what it does beside it, and the clear's confirm under them. Beside it the Other images card, always open: put back the last saved flight software, flash a file, write the sensor bench, each a row with its label above it. The sensor bench pane is a further full-width card when the board is on the bench firmware. The step cards go three across from 1240 px of sheet width, the first two side by side with Write under them from 620 px, and stack below that. A phase table scrolls inside its card when it is wider than the card.

Data (`data.css`):

- One white pane. The head: "Data", where the folder is, a search box, Open a file…, Look again. Then the filter row: groups of pills (kind, from, when), each with a kicker and counts that follow the other filters.
- The table, newest first, each row a rounded bar: what it is, where it came from, when, size, then its actions as small accent-100 pills at the end. The picked row is surface-coloured. Below 700 px each row is a small card with its actions under it.
- A pull's report opens under the table in a surface panel: the boot table on the left and where it is on disk with its actions on the right. With no report the panel is one line.

The settings drawer: 560 px from the right edge with `--shadow-lg`, over whichever screen is up. A white card per section with its title; a field is its label with the hint under it on the left and its control in one 150 px column on the right. The Status section says its settings are kept in the app, the same on every screen. The foot says where the display settings are kept and holds Reset to defaults.

Scrolling: above 1100 px the page itself does not scroll, each pane does. Below 1100 px the layouts become one column and the page scrolls down. The page never scrolls sideways at any width; below 700 px the screen tabs scroll inside the bar and a table row becomes a card.

## Recipes

- Anything that writes to a board takes a second click in an `.confirm` row (accent-100) that says what will be overwritten, with a primary button and Cancel. No dialogs. Two exceptions. The user's call on 2026-10-31: the flight write from the Write step (and from the Sensor bench card) goes on the first press of Build and flash. The user's call on 2026-11-01: the receiver firmware on the T-Beams screen goes on the first press of Flash <name>, and what stands in the way is said in its place (the board not yet claimed with Read this one, a job running, a build that carries no firmware); the card's sentence says the board is erased first, and the save box is beside it. What stands between the press and the board are the checks that refuse a bad write, each said beside the button: no module chosen, board off the probe, a job already running, a box on the sheet left empty ("Fill in the board id, the spreading factor, the transmit power and 4 boxes in Phases first."), the id 255 every board is built with ("Set a board id first."), a power the module has no row for, a battery table the firmware would refuse.
- The draft has no defaults (the user, 2026-10-31, after a one-press write put 255, SF7 and 14 dBm on a real board): the boxes start from the block read off the board, else from what this browser remembers of that board (the block last read off it, the draft last written to it, kept in localStorage per uid), else empty, with the SF and power pickers showing nothing. A blank never becomes 0 or a default on the way to the board. The one table the app insists on that the sheet does not show, the battery bands behind a switch that is off, is written as the typed gap in every band with the firmware's own thresholds, which it never reads while the switch is off.
- How a flash, a pull or a clear ended is the one thing that opens a box over the page. Its head is one line: the board's name, the title ("Flight log pulled", "Flash cleared", "Flashed and checked", or the failure) and the state mark, with the finish time small at the right. Under it a label and value a line (`.kv`: Records, Boots, GPS time, Board id, Folder, Saved copy, Flight software), then what the operator must act on, standing alone at the bottom in its colour (`.outcome.bad`: the board left on the dump firmware, an id that disagrees with the board's). No sentences, no buttons. It stays until OK, so someone who stepped away during a write still sees the result. The job card on the sheet (`.now`) is the same list plus what only the sheet needs: which dump firmware read the log, what was on the board, the steps of a failed job, Open and Save, and a Close at the right of its title. Closing is remembered for that result (across repaints and reloads); the next job opens the card again. A running job has Stop, not Close.
- On the board and what will be written: each control in the step cards has the value read off the board under it. When the control would replace it, that value is struck through (`.was`), the control gets a ring (`.changed`), and the Write step lists the change as a row, the board's value struck beside the new one.
- Step list of a running job (`.steps`): the steps as the app reports them, finished ones muted, the current one bold with its latest note on its own line under it (a note can be as long as the image's name, and must not squeeze the step into a column of single words), then a 4 px bar (`.track` and `.bar-fill`). A step it failed on is marked and stays on screen.
- Folded pane: a `details.fold` whose summary is the pane's name and one note saying why it is folded. It is for a pane a board has no use for in its current state (Radio and identity, Flight log on a bench board). It opens in place, with the browser's own triangle.
- Sensor bench: a pane on a ChipSat's sheet, shown when the app last found the bench on that board (never because a console line arrived). Its head holds the one console chooser on the sheet and a way back to flight software. Under it a grid of part cards in the order the board starts them, then the tally of any board on the bench (254), the starts and the raw console, redrawn on their own so the rest of the sheet holds still. A card's head is the part and the last `ok=` the board printed, in a mark. A part with no line for a while gets an `off` mark saying how long. Event tags (the I2C bus) show their newest lines as printed, not parsed. Values are shown as the board gave them. When the console's start line disagrees with what was read, an `.outcome.bad` line names both.
- A sheet is a container (`container-type: inline-size`), so its columns follow its own width, not the window's: the ChipSats step cards go three across from 1240 px of it.
- Each pane scrolls on its own, and a pane inside a grid has `min-height: 0` and `min-width: 0` or it pushes the page.
- A receiver is called by the name it reports, else the name kept for its id, else `T-Beam` and its id. The port order (rx1, rx2) is shown only when nothing else is known.
- A receiver's spreading factor is the board's own word (the user, 2026-11-01). Its select shows what the board last reported, never what was asked. A change is asked of the board (`#SET,sf`), and the row carries `asked for SFn` (warn) until the board's header line (`#DG,RX,… sf=n`) says the new value: then `SFn confirmed` (good) and a toast "<name> is listening on SFn". `#DG,ERR,sf` is `refused SFn` (bad); no header within 4 s is `SFn not confirmed` (bad); a request the app would not send is `SFn not sent` (bad). A header that still says the old value decides nothing by itself, since one may have been printed before the command landed. The T-Beams Radio card says the same under its select, with the time, in one line.

## The one distinctive element

The unit card's stale bar: a 4 px bar under the CSID that fills toward that unit's own stale limit, as time since the last packet over the limit. Sage while it is OK, grey once stale, terracotta once lost, and the word on the card says the same. A bar nearly full is a unit about to go quiet.

## Motion

None, apart from the 120 ms hover tint on buttons, pills and cards and the 300 ms width change of a progress bar. A toggle changes at once. `prefers-reduced-motion` turns both off.

## Sound

Two sounds, both synthesised in `app.js` and both behind the one setting "Sound when a unit goes lost or a run starts" (`alarmSound`), never in a replay:

- Lost: three low notes (660 Hz, 160 ms, 220 ms apart) when a unit the app was hearing goes to Lost, with a toast.
- A run starting: one plain beep (a 1000 Hz square wave for 150 ms, switched hard on and off, no slide; the user's call on 2026-11-01 over a rising chirp) when a board is heard for the first time or its counter starts again, by the app's own rows (a csid new to the live list, or a live row whose first-packet time moved), not by a counter of 0, so a missed first packet still counts. Nothing for a packet, a repeat from a second receiver, or the rows a snapshot brings on load or after a reconnect. Several boards up in one pass or within two seconds are one beep.

Browser limits, said plainly: the browser plays nothing until the page has been clicked or typed on once (an audio context made before that is suspended; the page resumes it on the first click, so the first sound is the one after that click). In a background tab the redraw that listens for runs is throttled to once a second, and to once a minute after five minutes hidden, so a sound can come that late; it does play in the background once the context is running. A tab the browser has discarded plays nothing.
