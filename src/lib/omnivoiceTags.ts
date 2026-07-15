// Mirror of `src-tauri/src/omnivoice_tags.rs` `SUPPORTED_INLINE_TAGS`.
// Keep identical to the pinned OmniVoice base model's non-verbal controls.
// Contract-tested against the Rust list in `omnivoiceTags.test.ts`.

/** Every inline tag overrides may emit (full bracket form). */
export const SUPPORTED_INLINE_TAGS = [
  "[laughter]",
  "[sigh]",
  "[confirmation-en]",
  "[question-en]",
  "[question-ah]",
  "[question-oh]",
  "[question-ei]",
  "[question-yi]",
  "[surprise-ah]",
  "[surprise-oh]",
  "[surprise-wa]",
  "[surprise-yo]",
  "[dissatisfaction-hnn]",
] as const;

export type SupportedInlineTag = (typeof SUPPORTED_INLINE_TAGS)[number];

const SUPPORTED_SET = new Set<string>(SUPPORTED_INLINE_TAGS);

/** True when `tag` is a known OmniVoice inline control (with brackets). */
export function isSupportedInlineTag(tag: string): boolean {
  return SUPPORTED_SET.has(tag);
}

/**
 * Soft-scan for bracket tokens that look like OmniVoice tags but are not in the
 * supported catalog. Does not block save — the backend remains authoritative.
 */
export function findUnknownInlineTags(text: string): string[] {
  const found = new Set<string>();
  const re = /\[[^\]]+\]/g;
  for (const match of text.matchAll(re)) {
    const token = match[0];
    if (!isSupportedInlineTag(token)) found.add(token);
  }
  return [...found];
}
