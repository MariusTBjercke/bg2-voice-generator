import { describe, expect, test } from "vitest";
import {
  IDENTITY_PARAM,
  findGroupByIdentityParam,
  identityHref,
  pathWithoutIdentity,
  readIdentityParam,
} from "./speakerDeepLink";

describe("speakerDeepLink", () => {
  test("identityHref encodes named and ungrouped keys", () => {
    expect(identityHref("/binding", "22570:1")).toBe("/binding?identity=22570%3A1");
    expect(identityHref("/harvest", "ungrouped:12")).toBe(
      "/harvest?identity=ungrouped%3A12",
    );
    expect(identityHref("/generation?x=1#y", "33001:1")).toBe(
      "/generation?identity=33001%3A1",
    );
  });

  test("readIdentityParam returns trimmed keys and ignores empties", () => {
    expect(readIdentityParam(new URL("http://x/binding?identity=22570%3A1"))).toBe("22570:1");
    expect(
      readIdentityParam(new URL("http://x/harvest?identity=ungrouped%3A12")),
    ).toBe("ungrouped:12");
    expect(readIdentityParam(new URL("http://x/binding?identity=%20"))).toBeNull();
    expect(readIdentityParam(new URL("http://x/binding"))).toBeNull();
  });

  test("pathWithoutIdentity strips only the identity param", () => {
    const url = new URL(`http://x/binding?foo=1&${IDENTITY_PARAM}=22570%3A1#panel`);
    expect(pathWithoutIdentity(url)).toBe("/binding?foo=1#panel");
    expect(pathWithoutIdentity(new URL("http://x/harvest?identity=1"))).toBe("/harvest");
  });

  test("findGroupByIdentityParam accepts sex-scoped and legacy plain strrefs", () => {
    const groups = [
      { identity_key: "15855:1", long_name_strref: 15855 },
      { identity_key: "15855:2", long_name_strref: 15855 },
    ];
    expect(findGroupByIdentityParam(groups, "15855:2")?.identity_key).toBe("15855:2");
    expect(findGroupByIdentityParam(groups, "15855")?.identity_key).toBe("15855:1");
    expect(findGroupByIdentityParam(groups, "999")).toBeUndefined();
  });
});
