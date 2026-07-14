import { describe, expect, it } from "vitest";
import {
  PLACEHOLDER_SPECS,
  PROFILE_PRO_DEFAULTS,
  decodeTokenMask,
  previewReplacement,
  previewProfileToken,
} from "./placeholderTokens";

describe("placeholderTokens", () => {
  it("PLACEHOLDER_SPECS keys are unique", () => {
    const keys = PLACEHOLDER_SPECS.map((s) => s.key);
    expect(new Set(keys).size).toBe(keys.length);
  });

  it("previewReplacement handles vocative comma-before CHARNAME", () => {
    expect(
      previewReplacement(
        "Greetings, <CHARNAME>. It is good to see you.",
        "<CHARNAME>",
        "friend",
      ),
    ).toBe("Greetings, friend. It is good to see you.");
  });

  it("previewReplacement fixes article before stand-in", () => {
    expect(
      previewReplacement("You are a <PRO_LADYLORD> now.", "<PRO_LADYLORD>", "Lord"),
    ).toBe("You are a Lord now.");
  });

  it("profile defaults match Rust neutral PRO_HISHER", () => {
    expect(previewProfileToken("neutral", "PRO_HISHER")).toBe("their");
    expect(PROFILE_PRO_DEFAULTS.male.PRO_LADYLORD).toBe("Lord");
    expect(PROFILE_PRO_DEFAULTS.female.PRO_LADYLORD).toBe("Lady");
  });

  it("decodeTokenMask returns labels for known bits", () => {
    const labels = decodeTokenMask(1 | 4);
    expect(labels).toContain("CHARNAME");
    expect(labels).toContain("PRO_HISHER");
  });
});
