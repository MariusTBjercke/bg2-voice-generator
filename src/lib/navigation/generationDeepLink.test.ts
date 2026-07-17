import { describe, expect, test } from "vitest";
import {
  GENERATION_FOCUS_ORPHANS,
  GENERATION_FOCUS_PARAM,
  GENERATION_FOCUS_VOICE_CHANGED,
  generationFocusHref,
  pathWithoutGenerationFocus,
  readGenerationFocusParam,
} from "./generationDeepLink";

describe("generationFocusHref", () => {
  test("builds a focus query on the generation path", () => {
    expect(generationFocusHref("/generation", GENERATION_FOCUS_ORPHANS)).toBe(
      `/generation?${GENERATION_FOCUS_PARAM}=orphans`,
    );
    expect(generationFocusHref("/generation?x=1#h", GENERATION_FOCUS_VOICE_CHANGED)).toBe(
      `/generation?${GENERATION_FOCUS_PARAM}=voice_changed`,
    );
  });
});

describe("readGenerationFocusParam", () => {
  test("accepts known focus tokens and rejects others", () => {
    expect(readGenerationFocusParam(new URL("https://x/generation?focus=orphans"))).toBe("orphans");
    expect(readGenerationFocusParam(new URL("https://x/generation?focus=voice_changed"))).toBe(
      "voice_changed",
    );
    expect(readGenerationFocusParam(new URL("https://x/generation?focus=nope"))).toBeNull();
    expect(readGenerationFocusParam(new URL("https://x/generation"))).toBeNull();
  });
});

describe("pathWithoutGenerationFocus", () => {
  test("removes focus while keeping other params", () => {
    const url = new URL("https://x/generation?focus=orphans&identity=1#list");
    expect(pathWithoutGenerationFocus(url)).toBe("/generation?identity=1#list");
  });
});
