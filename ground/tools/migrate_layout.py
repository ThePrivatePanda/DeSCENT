#!/usr/bin/env python3
"""Move what older DeSCENT Ground builds saved flat in the data folder into the day folders
the app uses now (ground/app/src/layout.rs):

  descent-ground-2026-10-17T18-02-58.log            -> 2026-10-17/logs/1402-ground.log
  descent-pull-2026-10-18T18-32-04-9/               -> 2026-10-18/pulls/1432-board9/
  descent-clear-2026-10-17T15-25-37-9/              -> 2026-10-17/clears/1125-board9/
  descent-image-0027003A-9-2026-10-17T15-01-23.bin  -> 2026-10-17/images/1101-board9-0027003A.bin
  descent-ground-backup-ttyACM0-...bin              -> <day>/receiver-images/<HHMM>-ttyACM0.bin

The old names carry a UTC stamp; the new ones are in this computer's local time. A firmware image
inside a pull or clear folder is renamed the same way. Anything the script does not recognise
is left where it is. Nothing is deleted or overwritten: a name that is taken gets a count after
its time, 1432-2-board9, as the app does. Stop the app first: it is writing one of these logs.

  python3 migrate_layout.py [DATA]           print the plan, change nothing (DATA is ~/DeSCENT)
  python3 migrate_layout.py [DATA] --apply   do it, and write the moves to DATA/migrated-<stamp>.tsv
"""
import datetime as dt
import os
import re
import sys

STAMP = r"(\d{4}-\d{2}-\d{2})T(\d{2})-(\d{2})-(\d{2})"
EPOCH13 = r"(\d{13})"


def local(utc_text):
    m = re.fullmatch(STAMP, utc_text)
    t = dt.datetime(*map(int, m.group(1).split("-")), int(m.group(2)), int(m.group(3)), int(m.group(4)),
                    tzinfo=dt.timezone.utc)
    return t.astimezone()


def from_ms(ms):
    return dt.datetime.fromtimestamp(int(ms) / 1000, dt.timezone.utc).astimezone()


def tidy(hint):
    last = [p for p in hint.split("/") if p]
    last = last[-1] if last else ""
    out = "".join(c if c.isalnum() and c.isascii() or c in "-_." else "-" for c in last)
    return out or "board"


def board_word(name):
    t = tidy(name)
    return "board" + t if t.isdigit() else t


def day_hm(t):
    return t.strftime("%Y-%m-%d"), t.strftime("%H%M")


def image_target(file):
    """(local time, new file name, kind) for an image's old name, or None."""
    m = re.fullmatch(r"descent-(?:image|fsw)-([0-9A-Fa-f]{8})-(.*)-" + STAMP + r"\.bin", file)
    if m:
        t = local("%sT%s-%s-%s" % m.group(3, 4, 5, 6))
        return t, "%s-%s-%s.bin" % (day_hm(t)[1], board_word(m.group(2)), m.group(1).upper()), "images"
    m = re.fullmatch(r"descent-ground-fsw-(.*)-" + STAMP + r"\.bin", file)
    if m:
        t = local("%sT%s-%s-%s" % m.group(2, 3, 4, 5))
        return t, "%s-%s.bin" % (day_hm(t)[1], board_word(m.group(1))), "images"
    m = re.fullmatch(r"descent-ground-backup-(.*)-" + STAMP + r"\.bin", file)
    if m:
        t = local("%sT%s-%s-%s" % m.group(2, 3, 4, 5))
        return t, "%s-%s.bin" % (day_hm(t)[1], board_word(m.group(1))), "receiver-images"
    m = re.fullmatch(r"descent-ground-backup-(.*)-" + EPOCH13 + r"\.bin", file)
    if m:
        t = from_ms(m.group(2))
        return t, "%s-%s.bin" % (day_hm(t)[1], board_word(m.group(1))), "receiver-images"
    return None


def plan(data):
    moves = []      # (old path, new path)
    taken = set()

    def free(path):
        # <dir>/<HHMM>-<rest>, and <dir>/<HHMM>-<n>-<rest> when that is taken
        head, leaf = os.path.split(path)
        hm, rest = leaf[:4], leaf[5:]
        n, cand = 1, path
        while os.path.exists(cand) or cand in taken:
            n += 1
            cand = os.path.join(head, "%s-%d-%s" % (hm, n, rest))
        taken.add(cand)
        return cand

    for name in sorted(os.listdir(data)):
        old = os.path.join(data, name)
        m = re.fullmatch(r"descent-ground-" + STAMP + r"(?:-(\d+))?\.log", name)
        if m and os.path.isfile(old):
            t = local("%sT%s-%s-%s" % m.group(1, 2, 3, 4))
            day, hm = day_hm(t)
            moves.append((old, free(os.path.join(data, day, "logs", hm + "-ground.log"))))
            continue
        m = re.fullmatch(r"descent-pull-" + STAMP + r"-(.*)-ground\.log", name)
        if m and os.path.isfile(old):
            # An older build kept a pull's log beside its folder. It stays a log of its own.
            t = local("%sT%s-%s-%s" % m.group(1, 2, 3, 4))
            day, hm = day_hm(t)
            moves.append((old, free(os.path.join(data, day, "logs", "%s-pull-%s-ground.log" % (hm, board_word(m.group(5)))))))
            continue
        m = re.fullmatch(r"descent-(pull|clear)-" + STAMP + r"-(.*)", name)
        if m and os.path.isdir(old):
            t = local("%sT%s-%s-%s" % m.group(2, 3, 4, 5))
            day, hm = day_hm(t)
            kind = "pulls" if m.group(1) == "pull" else "clears"
            new = free(os.path.join(data, day, kind, "%s-%s" % (hm, board_word(m.group(6)))))
            moves.append((old, new))
            # Images inside are renamed where they will be, after the folder has moved.
            for inside in sorted(os.listdir(old)):
                got = image_target(inside)
                if got:
                    moves.append((os.path.join(new, inside), free(os.path.join(new, got[1]))))
            continue
        got = image_target(name)
        if got and os.path.isfile(old):
            t, file, kind = got
            moves.append((old, free(os.path.join(data, day_hm(t)[0], kind, file))))
    return moves


def main(argv):
    apply = "--apply" in argv
    rest = [a for a in argv if a != "--apply"]
    data = os.path.abspath(os.path.expanduser(rest[0] if rest else "~/DeSCENT"))
    moves = plan(data)
    left = sorted(set(os.listdir(data)) - {os.path.relpath(o, data) for o, _ in moves if os.path.dirname(o) == data})
    for old, new in moves:
        print("%s\t->\t%s" % (os.path.relpath(old, data), os.path.relpath(new, data)))
    print("\n%d moves. Left where they are: %s" % (len(moves), ", ".join(left) or "nothing"), file=sys.stderr)
    if not apply:
        print("Nothing changed. Run again with --apply to do it.", file=sys.stderr)
        return 0
    # Every target is checked before the first move, so a clash stops it before anything moved.
    # Targets inside a moved folder exist only once that folder has moved.
    moved_dirs = {new for _, new in moves}
    clash = [new for old, new in moves if os.path.exists(new) and os.path.dirname(old) not in moved_dirs]
    if clash:
        sys.exit("refusing, nothing moved: these exist already: %s" % ", ".join(clash))
    record = os.path.join(data, "migrated-%s.tsv" % dt.datetime.now().strftime("%Y-%m-%dT%H-%M-%S"))
    with open(record, "w") as out:
        out.write("# old\tnew, relative to %s\n" % data)
        for old, new in moves:
            if os.path.exists(new):
                sys.exit("refusing: %s exists" % new)
            os.makedirs(os.path.dirname(new), exist_ok=True)
            os.rename(old, new)   # same file system, so modification times stay
            out.write("%s\t%s\n" % (os.path.relpath(old, data), os.path.relpath(new, data)))
            out.flush()
    print("Done. The moves are listed in %s" % record, file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
