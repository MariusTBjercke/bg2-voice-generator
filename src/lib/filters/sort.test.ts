import { describe, expect, test } from "vitest";
import {
  localeText,
  numberAsc,
  numberDesc,
  resolveSort,
  sortItems,
  sortOptionsFromSpecs,
  thenBy,
  type SortSpec,
} from "./sort";

interface Row {
  id: number;
  name: string;
  count: number;
}

const rows: Row[] = [
  { id: 3, name: "Xzar", count: 2 },
  { id: 1, name: "Anomen", count: 10 },
  { id: 2, name: "Jaheira", count: 2 },
];

const byName: SortSpec<Row> = {
  key: "name_asc",
  label: "Name A–Z",
  compare: (a, b) => localeText(a.name, b.name),
};

const byCount: SortSpec<Row> = {
  key: "count_desc",
  label: "Count",
  compare: thenBy(
    (a, b) => numberDesc(a.count, b.count),
    (a, b) => localeText(a.name, b.name),
  ),
};

describe("sort helpers", () => {
  test("localeText is case-insensitive and null-safe", () => {
    expect(localeText("anomen", "Jaheira")).toBeLessThan(0);
    expect(localeText(null, "a")).toBeLessThan(0);
    expect(localeText("a", undefined)).toBeGreaterThan(0);
  });

  test("numberAsc and numberDesc treat missing as extreme", () => {
    expect(numberAsc(1, 2)).toBeLessThan(0);
    expect(numberAsc(null, 1)).toBeGreaterThan(0);
    expect(numberDesc(1, 2)).toBeGreaterThan(0);
    expect(numberDesc(null, 1)).toBeGreaterThan(0);
  });

  test("thenBy falls through on ties", () => {
    const sorted = sortItems(rows, byCount);
    expect(sorted.map((row) => row.name)).toEqual(["Anomen", "Jaheira", "Xzar"]);
  });

  test("resolveSort matches a key or falls back to the first spec", () => {
    expect(resolveSort([byName, byCount], "count_desc")?.key).toBe("count_desc");
    expect(resolveSort([byName, byCount], "missing")?.key).toBe("name_asc");
    expect(resolveSort([byName, byCount], null)?.key).toBe("name_asc");
    expect(resolveSort([], "name_asc")).toBeNull();
  });

  test("sortItems returns a new array and leaves the source untouched", () => {
    const sorted = sortItems(rows, byName);
    expect(sorted.map((row) => row.name)).toEqual(["Anomen", "Jaheira", "Xzar"]);
    expect(rows.map((row) => row.name)).toEqual(["Xzar", "Anomen", "Jaheira"]);
  });

  test("sortOptionsFromSpecs uses label when present", () => {
    expect(sortOptionsFromSpecs([byName, byCount])).toEqual([
      { key: "name_asc", label: "Name A–Z" },
      { key: "count_desc", label: "Count" },
    ]);
  });
});
