// Client-side, case-insensitive multi-field matcher shared by every pipeline
// screen (Attribution / Harvest / Binding / Generation). It is UI-only: it never
// touches the backend and only ever narrows an already-loaded list, mirroring the
// Generation screen's original `$derived.by` pattern (see AGENTS.md, ADR 0003).
//
// A screen describes WHAT to search/facet with a `FilterConfig<T>` (pure accessor
// functions over its own row type) and holds the live selections in a
// `FilterValues`; `filterItems` applies them. `<SearchFilterBar>` renders the same
// config so config — not forked markup — is the single source of truth.

/** A selectable facet (a dropdown): equality-match on a value derived per item. */
export interface FacetSpec<T> {
  /** Stable key; also the key into the `FilterValues.facets` map. */
  key: string;
  /** Human label shown above the dropdown. */
  label: string;
  /** Label for the "no selection" option (defaults to `All`). */
  allLabel?: string;
  /** The facet value for one item, as a string token (or null = "no value"). */
  value: (item: T) => string | null;
  /** The options offered; if omitted, they are derived from the data. */
  options?: FacetOption<T>[];
}

/** One dropdown option: the stored token + its human label. */
export interface FacetOption<T = unknown> {
  value: string;
  label: string;
  /** Optional matcher for compound options that cannot be expressed as equality. */
  predicate?: (item: T) => boolean;
}

/** A screen's full filter description: free-text fields + optional facets. */
export interface FilterConfig<T> {
  /** Per-item strings the free-text query matches against (case-insensitive). */
  text: (item: T) => Array<string | number | null | undefined>;
  /** Placeholder for the text input. */
  textPlaceholder?: string;
  /** Optional facet dropdowns. */
  facets?: FacetSpec<T>[];
}

/** The live selections, owned (and `$bindable`) by the screen. */
export interface FilterValues {
  search: string;
  /** facet key -> selected token, or "all" for no selection. */
  facets: Record<string, string>;
  /** Optional sort key for screens that expose a Sort dropdown. */
  sort?: string;
}

/** The sentinel a facet holds when nothing is selected. */
export const FACET_ALL = "all";

/** An empty value map for `config`, so a screen can initialize its `$state`. */
export function emptyValues<T>(config: FilterConfig<T>): FilterValues {
  const facets: Record<string, string> = {};
  for (const f of config.facets ?? []) facets[f.key] = FACET_ALL;
  return { search: "", facets };
}

/** True when nothing is being filtered (no query, every facet = all). */
export function isEmpty(values: FilterValues): boolean {
  if (values.search.trim()) return false;
  return Object.values(values.facets).every((v) => v === FACET_ALL);
}

/** True if `query` (already lowercased/trimmed) matches any of the item's fields. */
function textMatches(fields: Array<string | number | null | undefined>, query: string): boolean {
  if (!query) return true;
  for (const f of fields) {
    if (f === null || f === undefined) continue;
    if (String(f).toLowerCase().includes(query)) return true;
  }
  return false;
}

/** Apply the free-text query + every selected facet to `items`. */
export function filterItems<T>(
  items: T[],
  config: FilterConfig<T>,
  values: FilterValues,
): T[] {
  const query = values.search.trim().toLowerCase();
  const facets = config.facets ?? [];
  return items.filter((item) => {
    if (!textMatches(config.text(item), query)) return false;
    for (const facet of facets) {
      const selected = values.facets[facet.key] ?? FACET_ALL;
      if (selected === FACET_ALL) continue;
      const option = facet.options?.find((candidate) => candidate.value === selected);
      if (option?.predicate) {
        if (!option.predicate(item)) return false;
      } else if ((facet.value(item) ?? "") !== selected) {
        return false;
      }
    }
    return true;
  });
}

/** Derive a facet's options from the data when it has no explicit `options`. */
export function facetOptions<T>(facet: FacetSpec<T>, items: T[]): FacetOption<T>[] {
  if (facet.options) return facet.options;
  const seen = new Map<string, string>();
  for (const item of items) {
    const v = facet.value(item);
    if (v === null || v === undefined || v === "") continue;
    if (!seen.has(v)) seen.set(v, v);
  }
  return [...seen.entries()]
    .map(([value, label]) => ({ value, label }))
    .sort((a, b) => a.label.localeCompare(b.label));
}

export {
  localeText,
  numberAsc,
  numberDesc,
  resolveSort,
  sortItems,
  sortOptionsFromSpecs,
  thenBy,
  type SortOption,
  type SortSpec,
} from "./sort";
