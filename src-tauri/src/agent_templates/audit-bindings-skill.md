---
name: audit-bindings
description: Audit personal voice bindings for wrong-character reference clips.
---

# Audit voice bindings

The `bg2-synthesis` CLI path, database path, and project id are in `AGENTS.md` in this
workspace. Use `bg2-synthesis binding …` for every read and write. Never edit `bg2vg.db`
directly. Never audition audio, approve samples, or run auto-bind.

Focus on **personal** clones (`default` / `override`). Demographic (`generic`) binds
intentionally reuse another character’s voice — skip them unless the user asks.

## Workflow (start here)

1. `binding progress --project <id>` — personal ready / flagged / reviewed / remaining.
2. Work `binding list-suspicious --project <id> --limit 100` first (heuristics + agent flags).
3. Page `binding list-personal --project <id> --limit 100 --after <speaker-id>` for the rest.
4. For each suspect: `binding show --project <id> --cre <RESREF>` (or `--speaker <id>`).
5. Compare **display name**, **member CRE resrefs**, **sample sound resref**, **transcript**,
   and **display-group siblings**.
6. If wrong-character VO is likely:
   - `binding flag --project <id> --cre <RESREF> --reason "<short why>"`
   - `binding reject-sample --project <id> --sample <id>` when that clip should not bind again
   - `binding clear-personal --project <id> --cre <RESREF>` when the personal clone must go
7. If clearly self-voice (or verified companion identity sharing) →
   `binding review --project <id> --cre <RESREF>`
8. Re-run `binding progress --project <id>` before finishing.

## What to look for

- Sound/CRE stem that belongs to another named character under an unrelated display name
  (companion prefixes like `jaheir*`, `minsc*`, `aerie*` on crowd NPCs).
- Crowd / short generic names (**Boy**, Guard, Slave, Beggar, …) with a companion-like stem.
- Approved `manual_only` or high `shared_source_count` used as the personal primary.
- Personal sample shared across many CREs in a **non-companion** display group.
- Transcript that clearly belongs to a different character than the display name.

Game files sometimes attach foreign VO to a CRE tree. Harvest may attribute it; that does
**not** make it a valid personal clone for that display name.

## Worked example

Display group **Boy** with an approved / auto-bind pick whose sound is **`jaheir62`** and
transcript talks about druids / nature’s cause → Jaheira’s VO mis-attached via game data.
Reject that sample for Boy; clear the personal bind if it was using that clip. Prefer
same-identity child VO (e.g. `chiln*`) or leave unbound for demographic fallback — never
Jaheira’s voice on Boy.

## Allowed writes only

- `binding flag` / `binding review` / `binding unreview` / `binding clear-flag`
- `binding clear-personal`
- `binding reject-sample`

Do not approve samples, edit demographic pools, call `auto_bind_all`, or re-parse BIF/DLG
archives from this workspace.
