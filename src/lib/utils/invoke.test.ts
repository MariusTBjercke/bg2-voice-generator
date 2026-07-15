import { describe, expect, test } from "vitest";
import { appendCacheBust } from "./invoke";

describe("appendCacheBust", () => {
  test("leaves url alone when no token is provided", () => {
    expect(appendCacheBust("asset://localhost/x.ogg")).toBe("asset://localhost/x.ogg");
    expect(appendCacheBust("asset://localhost/x.ogg", "")).toBe("asset://localhost/x.ogg");
  });

  test("appends ?v= when the url has no query string", () => {
    expect(appendCacheBust("asset://localhost/x.ogg", 42)).toBe(
      "asset://localhost/x.ogg?v=42",
    );
  });

  test("appends &v= when the url already has a query string", () => {
    expect(appendCacheBust("asset://localhost/x.ogg?foo=1", "abc")).toBe(
      "asset://localhost/x.ogg?foo=1&v=abc",
    );
  });
});
