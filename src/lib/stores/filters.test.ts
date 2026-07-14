import { describe, expect, test } from "vitest";
import { migrateLegacyFilterCache } from "./filterMigration";

describe("filter persistence migration", () => {
  test("retains valid v1 simple filters and discards the incompatible Generation facet", () => {
    const migrated = migrateLegacyFilterCache({
      gameDir: "C:\\Games\\BG2",
      byScreen: {
        attribution: { search: "token", facets: { reason: "token" } },
        harvest: { search: "Xzar", facets: {} },
        generation: { search: "22570", facets: { speaker: "1" } },
      },
    });

    expect(migrated.gameDir).toBe("C:\\Games\\BG2");
    expect(migrated.byScreen.attribution).toEqual({ search: "token", facets: { reason: "token" } });
    expect(migrated.byScreen.harvest).toEqual({ search: "Xzar", facets: {} });
    expect("generation" in migrated.byScreen).toBe(false);
  });

  test("corrupt legacy data becomes a clean cache", () => {
    expect(migrateLegacyFilterCache("broken")).toEqual({ gameDir: null, byScreen: {} });
  });
});
