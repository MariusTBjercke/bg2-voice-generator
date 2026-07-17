//! Built-in stage-cue and spoken-word → OmniVoice tag mappings.

/// `(find_text, tag)` pairs inserted as non-deletable `stage_cue` defaults.
/// One row per cue alias so each can be toggled independently.
pub const DEFAULT_STAGE_CUE_TAG_RULES: &[(&str, &str)] = &[
    ("sigh", "[sigh]"),
    ("sighs", "[sigh]"),
    ("sighing", "[sigh]"),
    ("laugh", "[laughter]"),
    ("laughs", "[laughter]"),
    ("laughing", "[laughter]"),
    ("laughter", "[laughter]"),
    ("chuckle", "[laughter]"),
    ("chuckles", "[laughter]"),
    ("chuckling", "[laughter]"),
    ("giggle", "[laughter]"),
    ("giggles", "[laughter]"),
    ("giggling", "[laughter]"),
    ("cackle", "[laughter]"),
    ("cackles", "[laughter]"),
    ("cackling", "[laughter]"),
    ("grin", "[laughter]"),
    ("grins", "[laughter]"),
    ("grinning", "[laughter]"),
    ("gasp", "[surprise-ah]"),
    ("gasps", "[surprise-ah]"),
    ("gasping", "[surprise-ah]"),
    ("surprised", "[surprise-oh]"),
    ("surprise", "[surprise-oh]"),
    ("hmm", "[dissatisfaction-hnn]"),
    ("hmph", "[dissatisfaction-hnn]"),
    ("hnn", "[dissatisfaction-hnn]"),
    ("grumble", "[dissatisfaction-hnn]"),
    ("grumbles", "[dissatisfaction-hnn]"),
];

/// `(find_text, tag)` pairs inserted as non-deletable `whole_word` defaults.
/// Whole-word only (same boundary rules as Pronunciation), so e.g. `Bah` does not
/// match inside names like `Bahardor`.
pub const DEFAULT_SPOKEN_WORD_TAG_RULES: &[(&str, &str)] = &[
    ("Bah", "[dissatisfaction-hnn]"),
];
