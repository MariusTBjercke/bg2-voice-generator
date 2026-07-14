---
name: set-synthesis
description: Review BG2 dialogue and author generation-only OmniVoice synthesis text.
---

# Set synthesis text

The `bg2-synthesis` CLI path, database path, and project id are in `AGENTS.md` in this
workspace. Use `bg2-synthesis` for all corpus reads and writes. Never edit `bg2vg.db`
directly.

Run `bg2-synthesis catalog` for the full allowed inline tag list.

## Named pacing presets (rare)

For a line whose pacing is clearly wrong in context, use the bounded presets in
`bg2-synthesis preset list`. Keep **`inherit`** unless the text and surrounding dialogue give
you a strong reason to choose a named pacing change. The only allowed writes are:

- `preset show --line <id>` — inspect the effective named state and diagnostics.
- `preset set --line <id> --preset <inherit|auto_pace|deliberate|natural|brisk|very_brisk>`.
- `preset clear --line <id>` — equivalent to `inherit`.
- `preset set --batch <file|->` with `[{"line": 42, "preset": "brisk"}]` JSON.

`inherit` clears only the line's pacing layer and preserves any manual UI render settings.
Preset changes reset only that line's accepted generation and discard only its stale candidate.
Batch failures report every failed line while retaining successful lines.

Never set steps, seeds, guidance, temperatures, speeds, or any other raw render field. Never
render candidate audio, audition audio, or accept a candidate: agents cannot hear the result.

## Workflow (start here)

1. `audit-corpus --project <id>` — corpus shape and how many strings still need judgment.
2. `auto-review-plain --project <id>` — bulk `review` every undecided plain dialogue string
   (no `*...*` cues; mapper output equals plain strip).
3. `list-flagged --project <id> --limit 500 --after <last-id>` — **primary work queue**.
   Each entry shows `original`, `mapped`, and `flags` explaining why it was flagged.
4. Per flagged entry, compare `original` vs `mapped`:
   - **`tag`** (prefer batch JSON) when tag placement should move, a stripped unknown cue
     clearly matches a catalog tag, or a rare interpretive catalog tag is unambiguous.
   - **`review`** when mapper output is acceptable.
5. **Never** bulk `review` lines with `*` cues without comparing columns.
6. Run `progress --project <id>` before finishing.

`list-untagged` remains available for legacy paging, but **`list-flagged` is the main queue**.

## Overrides are final generation text

Overrides are stored **as-is** — they do **not** pass through the mapper again. Write spoken
words plus `[catalog tags]` in their final positions. Do **not** put `*asterisk*` stage
directions in overrides; those are subtitle-only.

Non-overridden lines still use the mapper at generation time (`*sigh*` → `[sigh]`, etc.).
Unsupported cues such as `*sniff*` and `*breath*` are stripped; never reintroduce them
as bracket tags.

## Default: review

On a flagged line, `review --line <id>` means the mapper output (shown in `mapped`) is good
enough. Do not override just because you *could* improve delivery.

## Allowed inline tags (only these)

The CLI rejects unknown `[...]` tags. Use **only** tags from `bg2-synthesis catalog`:

**Body / non-verbal:** `[laughter]`, `[sigh]`

**English intonation / delivery** (use sparingly for obvious cases):
`[confirmation-en]`, `[question-en]`, `[question-ah]`, `[question-oh]`, `[question-ei]`,
`[question-yi]`, `[surprise-ah]`, `[surprise-oh]`, `[surprise-wa]`, `[surprise-yo]`,
`[dissatisfaction-hnn]`

Do **not** use `[sniff]`, `[breath]`, `[angry]`, `[sad]`, `[happy]`, `[whisper]`, `[fear]`, or any tag not in
`catalog` — the shipped base OmniVoice checkpoint does not support them.

For English, attach an inline tag without whitespace before it when it follows punctuation:
`What?[question-en]`, not `What? [question-en]`.

## Flag guide

| Flag | Meaning | Typical action |
|------|---------|----------------|
| `plain_ok` | No cues; plain strip is fine | Already handled by `auto-review-plain` |
| `mapped_ok` | Cues handled cleanly (mapped or deliberately stripped) | `review` if you agree |
| `stripped_unknown_cue` | Unknown `*...*` stripped | `tag` if a catalog tag fits; else `review` |
| `unterminated_asterisk` | Unclosed `*` in source | Inspect; `tag` fix or `review` |
| `placement_candidate` | Tag spacing after `.?!…` looks suboptimal | Compare columns; `tag` if clearly better |
| `interpretive_candidate` | Narrow spoken cue without stage direction | `tag` only if unambiguous; else `review` |
| `tts_unfriendly_spelling` | Dictionary + mapper output still contains a stutter, elongated spelling, scream, or comic phonetic token | Rewrite only the difficult token to a short speakable form |
| `non_speakable` | No speakable content after strip | `review` (generation already skips) |

## Phonetic and scream spellings

`tts_unfriendly_spelling` means the global Dictionary did not fully normalize text that is
likely to sound wrong in OmniVoice. Use a generation-only override and change only the flagged
phonetic tokens:

- Collapse stutters: `B-b-b-but` → `But`.
- Collapse elongated ordinary words: `Nooooo` → `No`, `sooooo` → `so`.
- Replace written screams with a short speakable interjection: `wwaaAAAAHHHH` → `Wah`.
- Normalize obvious drunk/comic spellings to the nearest ordinary English pronunciation.

Preserve every ordinary dialogue word and its order. A write is rejected if it changes
unflagged words. You may add one supported surprise tag when a cry is unambiguous, for example
`Wah![surprise-wa]` or `Ah![surprise-ah]`, but first run `bg2-synthesis catalog`. Never invent
`[scream]`, `[angry]`, or another unsupported tag. Prefer a speakable word without a tag when
uncertain.

Do not mark a `tts_unfriendly_spelling` line reviewed while its mapped text still contains the
difficult spelling unless you are confident OmniVoice supports it.

## When to `tag` (rare)

Use `tag --line <id> --text "<text>"` only when a generation-only override genuinely
improves delivery. Preserve every spoken word and its order. Overrides change generated
audio only, but must still match the subtitle.

### Two override reasons

1. **Fix mapper output** — awkward tag placement, a cue the mapper mishandled, or a line
   with multiple cues that reads better with manual placement. Start from the `mapped`
   column and edit lightly.

2. **Interpretive delivery** (very sparingly) — the subtitle has **no** matching stage
   direction, but delivery is unambiguous and one allowed tag would noticeably help.
   If you are unsure, **`review`**.

### Decision tree (every flagged entry)

```
Does mapped output contain a TTS-unfriendly spelling?
  YES → tag (phonetic rewrite); change only difficult tokens
  NO  ↓
Does mapped output look acceptable?
  YES → review
  NO  → Can you fix it using only catalog tags, same spoken words?
          YES → tag (fix)
          NO  → Is delivery unambiguous with no stage direction, and one catalog tag
                would clearly help (not "nice to have")?
                  YES → tag (interpret) — rare
                  NO  → review
```

Global Dictionary curation is a separate corpus-maintenance task. Do not add or edit global
rules during this review workflow; use the documented `bg2-synthesis dict` commands outside
the staged review workspace.

### Hard limits

- At most **one** interpretive tag per line unless the source already had multiple cues.
- Never add tags to neutral exposition, shopkeeper bark, or combat callouts unless the
  source text already cues it.
- Never tag on punctuation alone (`!`, `?`, `...`) or NPC stereotype.
- Unknown `*...*` cues the mapper stripped → **review** unless a catalog tag exactly
  matches the cue meaning (check mapper mappings in `catalog`).

## Writes: prefer batch JSON, not shell `--text`

For any line with spaces, apostrophes, or quotation marks in the subtitle, **never**
use `tag --line <id> --text "..."` in a shell. PowerShell and nested orchestration
easily corrupt quoting and append CLI tails such as `--db C:\...\bg2vg.db` into the
stored override.

Instead:

1. Build a JSON array: `[{"line": 42, "text": "Please.[sigh] Leave me."}]`
2. Run `tag --batch -` and pipe the JSON on stdin, **or** write a `.json` file and pass
   its path to `--batch`.

Use `review --batch` the same way for bulk default-mapper lines. The CLI rejects
overrides that contain CLI markers, filesystem paths, broken `\\\"` escapes, or spoken
words that do not match the subtitle. Run `audit --project <id>` to list suspicious
overrides already in the database.
