// Placeholder stand-ins for BG2 `<TOKEN>` dialogue substitutions. The engine fills
// these at runtime from the live party/protagonist; we replace them with spoken
// stand-ins at attribution so lines can be voiced. Keys mirror `token_resolve.rs`.

export type PcProfile = "male" | "female" | "neutral";

export const KEY_PC_PROFILE = "placeholder_pc_profile";

export const PC_PROFILE_OPTIONS: { value: PcProfile; label: string }[] = [
  { value: "neutral", label: "Neutral (recommended)" },
  { value: "male", label: "Male PC" },
  { value: "female", label: "Female PC" },
];

/** PRO_* defaults per profile (mirrors Rust `TokenReplacements`). */
export const PROFILE_PRO_DEFAULTS: Record<
  PcProfile,
  Record<string, string>
> = {
  male: {
    PRO_HISHER: "his",
    PRO_HIMHER: "him",
    PRO_HESHE: "he",
    PRO_LADYLORD: "Lord",
    PRO_SIRMAAM: "sir",
  },
  female: {
    PRO_HISHER: "her",
    PRO_HIMHER: "her",
    PRO_HESHE: "she",
    PRO_LADYLORD: "Lady",
    PRO_SIRMAAM: "ma'am",
  },
  neutral: {
    PRO_HISHER: "their",
    PRO_HIMHER: "them",
    PRO_HESHE: "they",
    PRO_LADYLORD: "friend",
    PRO_SIRMAAM: "friend",
  },
};

export interface PlaceholderSpec {
  key: string;
  token: string;
  context?: string;
  description: string;
  suggestion: string;
  example: string;
  exampleToken: string;
  fallback: string;
}

export const PLACEHOLDER_SPECS: PlaceholderSpec[] = [
  {
    key: "placeholder_charname_vocative",
    token: "<CHARNAME>",
    context: "direct address",
    description:
      "The protagonist's name when spoken to directly (after a comma). Empty drops the name and tidies the comma.",
    suggestion: "friend",
    example: "Greetings, <CHARNAME>. It is good to see you.",
    exampleToken: "<CHARNAME>",
    fallback: "friend",
  },
  {
    key: "placeholder_charname",
    token: "<CHARNAME>",
    context: "mid-sentence",
    description: "The protagonist's name inside a sentence. Empty cuts the token out.",
    suggestion: "Hero",
    example: "Tell <CHARNAME> the truth.",
    exampleToken: "<CHARNAME>",
    fallback: "Hero",
  },
  {
    key: "placeholder_gabber",
    token: "<GABBER>",
    description: "Who initiated the conversation (often the PC or a party member).",
    suggestion: "friend",
    example: "<GABBER> speaks first.",
    exampleToken: "<GABBER>",
    fallback: "friend",
  },
  {
    key: "placeholder_pro_race",
    token: "<PRO_RACE>",
    description: "The protagonist's race. Empty uses the PC profile default.",
    suggestion: "traveler",
    example: "A <PRO_RACE> like you knows better.",
    exampleToken: "<PRO_RACE>",
    fallback: "traveler",
  },
  {
    key: "placeholder_daytime",
    token: "<DAYTIME>",
    description: "Time of day (morning/afternoon/evening/night).",
    suggestion: "morning",
    example: "It is <DAYTIME> and we must leave.",
    exampleToken: "<DAYTIME>",
    fallback: "morning",
  },
  {
    key: "placeholder_daynight",
    token: "<DAYNIGHT>",
    description: "Whether it is day or night in the game world.",
    suggestion: "day",
    example: "Travel by <DAYNIGHT> if you can.",
    exampleToken: "<DAYNIGHT>",
    fallback: "day",
  },
  {
    key: "placeholder_day",
    token: "<DAY>",
    description: "Current in-game day reference.",
    suggestion: "today",
    example: "We leave <DAY>.",
    exampleToken: "<DAY>",
    fallback: "today",
  },
  {
    key: "placeholder_month",
    token: "<MONTH>",
    description: "Current month as a number or phrase.",
    suggestion: "this month",
    example: "It is <MONTH> already.",
    exampleToken: "<MONTH>",
    fallback: "this month",
  },
  {
    key: "placeholder_monthname",
    token: "<MONTHNAME>",
    description: "Current month by name (e.g. Mirtul).",
    suggestion: "Mirtul",
    example: "The month of <MONTHNAME> is cruel.",
    exampleToken: "<MONTHNAME>",
    fallback: "Mirtul",
  },
  {
    key: "placeholder_year",
    token: "<YEAR>",
    description: "Current in-game year.",
    suggestion: "1369",
    example: "It is <YEAR> in the Realms.",
    exampleToken: "<YEAR>",
    fallback: "1369",
  },
  {
    key: "placeholder_global",
    token: "<AnyToken>",
    description: "Any other `<IDENT>` token (mod tokens, PLAYER slots, etc.).",
    suggestion: "friend",
    example: "You would get <MOD_PAYOUT> gold.",
    exampleToken: "<MOD_PAYOUT>",
    fallback: "friend",
  },
];

/** Mask bit for `<CHARNAME>` (mirrors Rust `MASK_CHARNAME`). */
export const MASK_CHARNAME = 1;

/** Default spoken stand-in when the Placeholders screen leaves the field blank. */
export const DEFAULT_CHARNAME_STANDIN = "Hero";

/** Mask bit labels mirrored from Rust `TOKEN_MASK_LABELS`. */
export const TOKEN_MASK_LABELS: Record<number, string> = {
  [MASK_CHARNAME]: "CHARNAME",
  2: "GABBER",
  4: "PRO_HISHER",
  8: "PRO_HIMHER",
  16: "PRO_HESHE",
  32: "PRO_LADYLORD",
  64: "PRO_SIRMAAM",
  128: "PRO_BROTHERSISTER",
  256: "PRO_SONDAUGHTER",
  512: "PRO_GIRLBOY",
  1024: "PRO_MANWOMAN",
  2048: "PRO_MALEFEMALE",
  4096: "PRO_RACE",
  8192: "speaker pronoun",
  16384: "time",
  32768: "other",
};

export function lineUsesCharname(mask: number): boolean {
  return (mask & MASK_CHARNAME) !== 0;
}

/** Decode a persisted `token_mask` into human-readable labels. */
export function decodeTokenMask(mask: number): string[] {
  const out: string[] = [];
  for (const [bit, label] of Object.entries(TOKEN_MASK_LABELS)) {
    if (mask & Number(bit)) out.push(label);
  }
  return out;
}

/**
 * Preview one spec's example with a stand-in applied — UI mirror of Rust
 * `token_resolve` for single-token examples.
 */
export function previewReplacement(
  example: string,
  token: string,
  replacement: string,
): string {
  const pos = example.toLowerCase().indexOf(token.toLowerCase());
  if (pos < 0) return example;
  const before = example.slice(0, pos);
  const after = example.slice(pos + token.length);
  let out: string;
  if (before.endsWith(", ")) {
    out = replacement
      ? `${before.slice(0, -2)}, ${replacement}${after}`
      : before.slice(0, -2) + after;
  } else if (after.startsWith(",")) {
    out = before + replacement + after;
  } else {
    out = fixArticle(before, replacement) + replacement + after;
  }
  out = out
    .replace(/\s+/g, " ")
    .replace(/ ([.,!?;:])/g, "$1")
    .replace(/^[,;\s]+/, "")
    .trimEnd();
  if (!/[a-z0-9]/i.test(out)) return "";
  return out;
}

function fixArticle(before: string, replacement: string): string {
  if (!replacement) return before;
  const wantsAn = /^[aeiou]/i.test(replacement);
  const m = before.match(/(^|\s)(an?)( +)$/i);
  if (!m) return before;
  const isAn = m[2].length === 2;
  if (wantsAn === isAn) return before;
  const cap = m[2][0] === "A";
  const fixed = wantsAn ? (cap ? "An" : "an") : cap ? "A" : "a";
  return before.slice(0, before.length - m[2].length - m[3].length) + fixed + m[3];
}

/** Preview a PRO_* token with the active PC profile. */
export function previewProfileToken(
  profile: PcProfile,
  token: string,
): string {
  return PROFILE_PRO_DEFAULTS[profile][token] ?? "…";
}
