/** Query param used to deep-link Generation filter presets (e.g. from Export). */
export const GENERATION_FOCUS_PARAM = "focus";

/** Show blocked/skipped lines that still have generated clips. */
export const GENERATION_FOCUS_ORPHANS = "orphans";

/** Show voice-changed clips (including orphans that Export warned about). */
export const GENERATION_FOCUS_VOICE_CHANGED = "voice_changed";

export type GenerationFocus =
  | typeof GENERATION_FOCUS_ORPHANS
  | typeof GENERATION_FOCUS_VOICE_CHANGED;

/**
 * Build a same-origin href that opens Generation with a focus preset applied.
 * `path` should be `/generation` (query/hash on `path` are dropped).
 */
export function generationFocusHref(path: string, focus: GenerationFocus): string {
  const pathname = path.split(/[?#]/, 1)[0] || "/";
  return `${pathname}?${GENERATION_FOCUS_PARAM}=${encodeURIComponent(focus)}`;
}

/** Read a supported `focus` query param from a URL, or null. */
export function readGenerationFocusParam(url: Pick<URL, "searchParams">): GenerationFocus | null {
  const raw = url.searchParams.get(GENERATION_FOCUS_PARAM);
  if (raw === null) return null;
  const trimmed = raw.trim();
  if (trimmed === GENERATION_FOCUS_ORPHANS || trimmed === GENERATION_FOCUS_VOICE_CHANGED) {
    return trimmed;
  }
  return null;
}

/** Same path/hash with the `focus` query param removed (other params kept). */
export function pathWithoutGenerationFocus(url: URL): string {
  const next = new URL(url.href);
  next.searchParams.delete(GENERATION_FOCUS_PARAM);
  const search = next.searchParams.toString();
  return `${next.pathname}${search ? `?${search}` : ""}${next.hash}`;
}
