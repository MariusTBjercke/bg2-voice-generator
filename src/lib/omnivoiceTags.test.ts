import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  findUnknownInlineTags,
  isSupportedInlineTag,
  SUPPORTED_INLINE_TAGS,
} from "./omnivoiceTags";

describe("omnivoiceTags", () => {
  it("mirrors Rust SUPPORTED_INLINE_TAGS", () => {
    const rust = readFileSync(
      join(process.cwd(), "src-tauri/src/omnivoice_tags.rs"),
      "utf8",
    );
    const block = rust.match(
      /pub const SUPPORTED_INLINE_TAGS: &\[&str\] = &\[([\s\S]*?)\];/,
    );
    expect(block).toBeTruthy();
    const rustTags = [...block![1].matchAll(/"(\[[^\"]+\])"/g)].map((m) => m[1]);
    expect(rustTags).toEqual([...SUPPORTED_INLINE_TAGS]);
  });

  it("recognizes supported tags and soft-flags unknown ones", () => {
    expect(isSupportedInlineTag("[sigh]")).toBe(true);
    expect(isSupportedInlineTag("[angry]")).toBe(false);
    expect(findUnknownInlineTags("Hello [sigh] and [angry] there.")).toEqual([
      "[angry]",
    ]);
  });
});
