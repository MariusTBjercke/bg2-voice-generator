/** Shared client-side sort helpers for filtered list screens. */

/** One entry in a Sort dropdown (UI label + stable key). */
export interface SortOption {
  key: string;
  label: string;
}

/** A named comparator used by `sortItems` / `resolveSort`. */
export interface SortSpec<T> {
  key: string;
  /** Human label for the Sort dropdown; when omitted the key is shown. */
  label?: string;
  compare: (a: T, b: T) => number;
}

/** Case-insensitive locale-aware text compare (null/undefined sort last). */
export function localeText(a: string | null | undefined, b: string | null | undefined): number {
  const left = a ?? "";
  const right = b ?? "";
  return left.localeCompare(right, undefined, { sensitivity: "base" });
}

/** Ascending numeric compare (null/undefined treated as +Infinity). */
export function numberAsc(a: number | null | undefined, b: number | null | undefined): number {
  const left = a ?? Number.POSITIVE_INFINITY;
  const right = b ?? Number.POSITIVE_INFINITY;
  return left - right;
}

/** Descending numeric compare (null/undefined treated as −Infinity). */
export function numberDesc(a: number | null | undefined, b: number | null | undefined): number {
  const left = a ?? Number.NEGATIVE_INFINITY;
  const right = b ?? Number.NEGATIVE_INFINITY;
  return right - left;
}

/** Chain comparators: first non-zero result wins. */
export function thenBy<T>(
  ...comparators: Array<(a: T, b: T) => number>
): (a: T, b: T) => number {
  return (a, b) => {
    for (const compare of comparators) {
      const result = compare(a, b);
      if (result !== 0) return result;
    }
    return 0;
  };
}

/** Resolve a sort key against available specs; falls back to the first spec. */
export function resolveSort<T>(
  options: SortSpec<T>[],
  key: string | undefined | null,
): SortSpec<T> | null {
  if (options.length === 0) return null;
  if (key) {
    const match = options.find((option) => option.key === key);
    if (match) return match;
  }
  return options[0] ?? null;
}

/** Stable-ish sort: returns a new array ordered by `spec` (or the input when null). */
export function sortItems<T>(items: T[], spec: SortSpec<T> | null): T[] {
  if (!spec) return items;
  return items.slice().sort(spec.compare);
}

/** Map SortSpec list to SortOption list for SearchFilterBar. */
export function sortOptionsFromSpecs<T>(specs: SortSpec<T>[]): SortOption[] {
  return specs.map((spec) => ({ key: spec.key, label: spec.label ?? spec.key }));
}
