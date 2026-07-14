# Dictionary rule curation

Dictionary rules replace difficult spellings immediately before BG2 Voice Generator sends
generation text to OmniVoice. They never change TLK subtitles or exported dialogue strings.
Rules are machine-wide and stay local, like placeholder settings.

Use the Dictionary screen for individual rules. For corpus maintenance in Cursor or another
external agent, use the companion CLI with the database path shown by the app:

```powershell
bg2-synthesis --db "C:\path\to\bg2vg.db" dict list
bg2-synthesis --db "C:\path\to\bg2vg.db" dict scan --project 1
bg2-synthesis --db "C:\path\to\bg2vg.db" dict test --text "B-b-b-but... wwaaAAAAHHHH!"
bg2-synthesis --db "C:\path\to\bg2vg.db" dict export --file rules.json
bg2-synthesis --db "C:\path\to\bg2vg.db" dict import --file rules.json
```

Import files contain a JSON array:

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

Keep rules narrow and repeatable. Prefer exact lore words and recurring phonetic spellings.
Do not add OmniVoice bracket tags to Dictionary rules; use generation-text overrides for
line-specific delivery. Test rules before a batch generation, because changing the Dictionary
invalidates completed clips whose synthesis input may have changed.
