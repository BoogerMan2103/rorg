# rorg — agent-friendly org-mode CLI

`rorg` lets a human or an LLM agent navigate and edit org-mode files **piece by piece**:
find headings by keyword/text/tags/dates, look at one entry, change one field — without
ever loading a whole (possibly 3000-line, possibly secret-laden) file into context.

- **Lossless**: the file is kept as lines; edits splice only the affected lines. Everything else stays byte-for-byte identical.
- **Compact output**: one line per heading by default, `--json` when structure is needed.
- **Every write prints a diff** of exactly what changed; `--dry-run` previews without saving; writes are atomic (temp file + rename).
- **Protected entries**: bodies of headings tagged `crypt` / `secret` (or inheriting them) are hidden and skipped by `grep` unless `--reveal`.

## Install

```bash
cargo install --path .
```

## Reading

```bash
rorg info ~/org/                       # sizes, TODO keywords in use, counts, tags
rorg outline work.org --depth 2        # heading tree, no bodies
rorg find ~/org --status WAIT          # all WAIT items across files
rorg find work.org --todo --deadline ..+7d      # open items due within a week (incl. overdue)
rorg find work.org --date 2026-11-10   # scheduled/deadline/active timestamp on that day
rorg find work.org --text vendor --in-body --tag client
rorg find work.org --scheduled none --todo      # unscheduled open tasks
rorg find work.org --todo --scheduled --sort scheduled   # planned open tasks, by date (like agenda)
rorg show work.org 2.3                 # metadata + children, body size only
rorg show work.org 2.3 --body          # plus body text
rorg show work.org 2.3 --context       # body plus bodies of all parent headings
rorg show work.org "** WAIT Ping vendor :client:" --body   # heading pasted from Emacs
rorg grep 'invoice' work.org           # matching lines with owning heading id
rorg agenda ~/org -r today..+6d        # by day, plus overdue
rorg clock work.org -r 2026-11-01..    # clocked time per heading
```

Output line format: `ID  L<line>  KEYWORD [#P] Title  :tags:  sched:… dl:… closed:…  (in: Parent > …)`

## Selectors

| form       | meaning                                         |
|------------|-------------------------------------------------|
| `2.3.1`    | positional path, as printed by outline/find     |
| `L120`     | heading whose section contains line 120         |
| `id:abc`   | `:ID:` or `:CUSTOM_ID:` property                |
| `@vendor`  | unique case-insensitive title substring         |
| `@=Exact`  | exact title                                     |
| `vendor`, `** WAIT Ping vendor :x:` | anything else: title substring; a pasted heading line is reduced to its title |

Positional ids shift after `add`/`move`/`delete`. Every write prints the new id; for
safety add `--expect TEXT` (aborts unless the target title contains TEXT).

## Dates

`YYYY-MM-DD`, `today`, `tomorrow`, `yesterday`, `+3d`, `-1w`, `+1m`, `fri` (next Friday, today included).
Ranges: `A..B`, `..B`, `A..`, `overdue`, `week`. Planning filters (`--scheduled`, `--deadline`,
`--closed`) also take `any` / `none`; given without a value they mean `any` — put them after the
file paths (or write `--scheduled=any`), otherwise the path is taken as the value.
Repeaters (`+1w`, `++1d`, `.+1m`) are expanded when matching ranges.

## Writing

```bash
rorg status work.org 2.3 DONE            # adds CLOSED; repeating tasks get dates shifted instead
rorg status work.org 2.3 TODO            # removes CLOSED
rorg schedule work.org @vendor "fri 10:00"
rorg deadline work.org 2.3 none
rorg tag work.org 2.3 --add urgent --rm someday
rorg priority work.org 2.3 A
rorg title work.org 2.3 "New title"
rorg prop work.org 2.3 OWNER me
rorg body work.org 2.3 "Called them, waiting for reply"   # append (default)
make test 2>&1 | tail -5 | rorg body work.org 2.3 --append   # no TEXT: read stdin
echo "full new body" | rorg body work.org 2.3 --set
rorg note work.org 2.3 "Deployed, no errors"   # "- Note taken on [now] \\" entry, newest first
rorg add work.org --under 2 --title "Ping vendor" --status WAIT --scheduled +2d
rorg move work.org 2.3 --under @Archive
rorg delete work.org 2.3 --expect "Ping vendor"
```

## TODO keywords

Taken from the file's `#+TODO:` / `#+SEQ_TODO:` lines; otherwise `$RORG_TODO`
(e.g. `export RORG_TODO="TODO NEXT WAIT | DONE CANCELLED"`); otherwise a broad default
(`TODO NEXT STARTED IN-PROGRESS WAIT WAITING HOLD SOMEDAY | DONE CANCELLED CANCELED`).
A word is only treated as a keyword if it is in that list. Check with `rorg info`.

The `|` split drives org semantics (CLOSED on done, hiding done items in agenda).
`find --todo` / `--done` match the same split unless narrowed with
`$RORG_FIND_TODO` / `$RORG_FIND_DONE`, e.g. `RORG_FIND_TODO="TODO WAIT"` to leave
projects out of `--todo`, `RORG_FIND_DONE="DONE OKAY"` to leave cancelled items out of `--done`.

Notes go directly under the heading's metadata (org default, `org-log-into-drawer` nil);
pass `--drawer` or set `RORG_LOG_INTO_DRAWER=1` to put them into `:LOGBOOK:`.

Protected tags are configurable via `$RORG_PROTECT_TAGS` (default `crypt,secret`).

## Instructions for agents

Paste into `CLAUDE.md` / `AGENTS.md` of the directory with org files:

```markdown
Org files here are managed through the `rorg` CLI. Do NOT cat/Read whole .org files.
- Orient: `rorg info <file>`, `rorg outline <file> --depth 2`.
- Locate: `rorg find` (filters: --status, --todo, --text, --in-body, --tag, --date, --scheduled, --deadline), `rorg grep`.
- Inspect one entry: `rorg show <file> <id>`; add `--body` only when the body is needed, `--context` when parent headings' bodies matter too.
- Edit only via rorg write commands; pass `--expect "<title words>"` on writes; ids shift after add/move/delete, so re-query.
- Never pass `--reveal` unless the user explicitly asks to see a protected entry.
```
