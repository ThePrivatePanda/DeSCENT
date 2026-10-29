# Design guide

**Identity:** instrument software for the whole ChipSat, from the bench it is flashed on to the ground station that hears it. Cool slate surfaces, one engineering typeface, numbers first. Colour only means state.

## Colour

| Token | Light | Dark | Meaning (only this) |
|---|---|---|---|
| `--bg` | `#edf0f3` | `#0e151c` | App background |
| `--panel` | `#ffffff` | `#151f29` | Panes |
| `--ink` / `--ink-2` / `--ink-3` | `#18222c` / `#414e5b` / `#667380` | `#e4eaf0` / `#b4c0cc` / `#8b99a7` | Values and headings / labels and notes, anything that has to be read / units, stale values, incidental text |
| `--edge` | `#98a2ad` | `#586878` | The border of a control |
| `--accent` | `#2f6fdb` | `#5b92ea` | Selection, primary action |
| `--good` | `#0c7a35` | `#3cbf6a` | OK, valid bit, CRC pass |
| `--warning` | `#96610f` | `#e0a53a` | Stale, near stale, saturation |
| `--serious` | `#c2572b` | `#e5794d` | Low battery, weak RF, resets |
| `--critical` | `#c93434` | `#ec6464` | Lost, invalid bit, CRC fail |
| `--series-1..8` | validated palette | | Chart lines in fixed order (X Y Z, or receivers in connect order), drawn at 70% opacity |

- Status is never colour alone: a word ("Lost"), a letter (L G M Q P S E F) or a shape (hollow dot = lost).
- An invalid field shows its last trusted value in `--ink-3`, or `—`.
- Text has to pass about 4.5:1 on `--panel` and `--sunk` in both themes. `--ink-3` only just does, so it is never used for a sentence.
- The spreading-factor select on a receiver chip is a control, not a status: `--ink` and no state colour.
- Off the dashboard a state is a word in a mark (`.mark good | warn | bad | off`): the state colour at low alpha behind the word. The word is never left out, and nothing else is tinted.

## Type

- IBM Plex Sans (400/500/600) for everything; tabular figures on the whole page.
- IBM Plex Mono only for hex (CRC, validity mask, a board's UID, a receiver's id).
- Fonts are stored in `web/vendor/fonts/`, so the page works offline.
- Sentence case everywhere. No all-caps labels, no middle-dot separators.
- Live is dense on purpose: 13 px, pane headings 14 px. Every other screen and the drawer use one scale, and hierarchy comes from size and weight before colour:

| Role | Size / weight / ink |
|---|---|
| Sheet or drawer title, a board's name | 22 px / 600 / `--ink` |
| Screen name at the head of a list or table | 17 px / 600 / `--ink` |
| Pane and section headings, a row's name in a list or table | 15 px / 600 / `--ink`, over a `--rule-strong` rule |
| Values and sentences | 14 px / 600 for a value, 400 for a sentence / `--ink` |
| Field labels, controls | 13 px / 400 (500 on a control) / `--ink-2` |
| Group labels (table and column heads), notes and hints | 12.5 px / 600 for a head, 400 for a note / `--ink-2` |

- A value carries the weight and its label does not. Numbers that change stay tabular.

## Controls

- Anything that does something when clicked looks like a button: a 1 px `--edge` border at rest, 5 px radius, `--sunk` fill, one height (28 px on Live and in the bar, 32 px elsewhere). Primary is the same shape filled with `--accent`. There is no borderless button.
- The only clickable text without a border is a tab. A row of a list or a table selects; it is not a button.
- Text and number inputs and selects are the same family: same border, radius, height and focus ring, on `--panel`. No browser-default inputs.
- Every click answers within a second, with a change on screen or a toast that states the outcome with its number or name: "3 boards, 1 probe", "Saved descent-….log", "New file descent-….log".

## Copy

- Facts, instructions, states, and a consequence the operator has to know before acting, said once where the action is. Nothing that comforts, promises or explains why the design is safe.

## Layout

- Screens are tabs in the top bar: ChipSats, T-Beams, Live, Data, and Replay once a log is open. All but Live and Replay need the app; a page from anywhere else has only those two. Settings is a drawer over whichever screen is up.
- The strip sits at the right of the bar on every screen: the receivers being read with their spreading factor and a state word, the file being logged to with `Save a copy` and `New file`, and a button for a running pull or flash that goes to it. On Live the receiver chips with their controls stand in for the first part.
- Each pane scrolls on its own. The page never scrolls sideways.

Live and Replay, the dashboard:

- Left: the selected unit. Header (CSID, state, time), validity row, then lists by area (Link, GPS, Environment, Accelerometer, Gyroscope, Magnetometer, Orientation), flowing into two columns. Scrolls only when the window is too short to show every list.
- Right top: fleet table. The header shows unit and packet counts; issue counts appear only when non-zero.
- Right bottom: two wide chart columns, 190 px plots, no boxes. Single-series charts have no legend; the hover value appears beside the title.

## Recipes

- Tab bar: plain text buttons. The screen that is up is `--ink` on `--accent-soft` with an `--accent` underline; a tab that cannot be opened yet is `--ink-3`.
- List and sheet (ChipSats, T-Beams): a 290 px list on the left, one row per board with its name, then its state in words. The picked row has the accent edge. Its sheet is on the right: a head with the name in an input at title size, state and identity, then one pane per job. Anything that writes to a board takes a second click in an `--accent-soft` row that says what will be overwritten. No dialogs.
- On the board / write table (`.kv`): three columns, the setting, what was read off the board, and the control for what will be written. A control that cannot write yet is disabled under one plain line saying why.
- Step list of a running job (`.steps`): the steps as the app reports them, finished ones ticked in `--ink-2`, the current one bold with its latest note at the right, then a 4 px bar. A step it failed on is marked and stays on screen.
- Sensor bench: a pane on a ChipSat's sheet, shown when the app last found the bench on that board (never because a console line arrived). Its head holds the one console chooser on the sheet and a way back to flight software; under it a grid of part cards in the order the board starts them, then the tally of any board on the bench (254), the starts and the raw console, redrawn on their own so the rest of the sheet holds still. A card's head is the part and the last `ok=` the board printed, in a mark; a part with no line for a while gets a `bad` mark saying how long. Event tags (the I2C bus) show their newest lines as printed, not parsed. Values are shown as the board gave them, never judged. When the console's start line disagrees with what was read, an `outcome bad` line names both.
- Folded pane: a `<details>` whose summary is the pane's name over its usual rule, and one note saying why it is folded. Used for panes a board has no use for in its current state (Radio and identity, Flight log on a bench board). It is not a dialog; it opens in place.
- Radio and identity: the settings table left and the phases right once the sheet is 1240 px wide (a container query on the sheet, not the window), stacked below that.
- Data table: the fleet table's grid. One row per file, newest first: what it is, where it came from, when, size, then its actions as buttons at the end of the row. A pull's report opens under the table. There is no delete.
- Settings drawer: sections are a heading over a rule with space above. A field is its label with the hint under it on the left, and its control in one 150 px column on the right.
- A receiver is called by the name it reports, else the name kept for its id, else `T-Beam` and its id. The port order (rx1, rx2) is shown only when nothing else is known.

## The one distinctive element

The "Last heard" cell: time since the last packet, over a thin bar that fills toward that unit's own stale limit (green, amber past two thirds, red once stale).

## Motion

None.
