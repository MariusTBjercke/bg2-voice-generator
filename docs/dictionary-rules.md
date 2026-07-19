# Dictionary rule curation

Dictionary rules replace difficult spellings immediately before BG2 Voice Generator sends
generation text to OmniVoice. They never change TLK subtitles or exported dialogue strings.
Rules are **profile-scoped** (stored in the active profile database, applied to every line
in that profile — not per-line) and travel with Profile Transfer backups, like placeholder
settings and tag rules.

The Dictionary screen has three tabs:

- **Placeholders** — spoken stand-ins for dynamic TLK tokens (`<CHARNAME>`, etc.).
- **Pronunciation** — find → speak-as whole-word substitutions (no OmniVoice tags).
- **Tag rules** — find → supported OmniVoice tag (`*sigh*` cues and optional spoken words
  like `Bah` → `[dissatisfaction-hnn]`). Defaults seed the former hardcoded mapper table
  plus the spoken-word `Bah` → `[dissatisfaction-hnn]` rule.

Pipeline order (overrides still win and skip everything):

1. Pronunciation rules
2. Stage-cue `*...*` tag rules (+ strip/emphasis for unknown cues)
3. Spoken-word tag rules (so inserted `[tags]` are not stripped as game brackets)

Use the Dictionary screen for individual rules. For corpus maintenance in Cursor or another
external agent, use the companion CLI with the database path shown by the app:

```powershell
bg2-synthesis --db "C:\path\to\bg2vg.db" dict list
bg2-synthesis --db "C:\path\to\bg2vg.db" dict scan --project 1
bg2-synthesis --db "C:\path\to\bg2vg.db" dict test --text "B-b-b-but... wwaaAAAAHHHH!"
bg2-synthesis --db "C:\path\to\bg2vg.db" dict export --file rules.json
bg2-synthesis --db "C:\path\to\bg2vg.db" dict import --file rules.json
bg2-synthesis --db "C:\path\to\bg2vg.db" tag-rule list
bg2-synthesis --db "C:\path\to\bg2vg.db" tag-rule add --find Bah --tag "[dissatisfaction-hnn]" --match whole_word
bg2-synthesis --db "C:\path\to\bg2vg.db" tag-rule test --text "Bah! *sigh* Fine."
```

Import files for pronunciation contain a JSON array:

```json
[
  {
    "find": "Cyrodiil",
    "speak_as": "Searohdiil",
    "match": "whole_word",
    "enabled": true
  }
]
```

Keep pronunciation rules narrow and repeatable. Prefer exact lore words and recurring
phonetic spellings. Do **not** put OmniVoice bracket tags in Pronunciation speak-as;
use **Tag rules** (or a line override) for delivery tags.

Prefer tag rules for recurring spoken→tag patterns (`Bah`); use generation-text overrides
for one-off line delivery. Changing Pronunciation rules, Tag rules, generation-text
overrides, clearing overrides, resetting agent synthesis state, or re-applying token
stand-ins (for example `<CHARNAME>`) marks matching completed clips as **text changed**.
They stay previewable; filter and regenerate on Generation when you want the new transcript.
