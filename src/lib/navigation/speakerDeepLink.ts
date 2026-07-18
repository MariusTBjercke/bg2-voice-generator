/** Query param used to deep-link a speaker identity across pipeline screens. */
export const IDENTITY_PARAM = "identity";

/**
 * Build a same-origin href that opens `path` with the given speaker identity selected.
 * `path` should be a pipeline route such as `/binding` (query/hash on `path` are dropped).
 */
export function identityHref(path: string, identityKey: string): string {
  const pathname = path.split(/[?#]/, 1)[0] || "/";
  return `${pathname}?${IDENTITY_PARAM}=${encodeURIComponent(identityKey)}`;
}

/** Read a non-empty `identity` query param from a URL, or null. */
export function readIdentityParam(url: Pick<URL, "searchParams">): string | null {
  const raw = url.searchParams.get(IDENTITY_PARAM);
  if (raw === null) return null;
  const trimmed = raw.trim();
  return trimmed.length > 0 ? trimmed : null;
}

/** Same path/hash with the `identity` query param removed (other params kept). */
export function pathWithoutIdentity(url: URL): string {
  const next = new URL(url.href);
  next.searchParams.delete(IDENTITY_PARAM);
  const search = next.searchParams.toString();
  return `${next.pathname}${search ? `?${search}` : ""}${next.hash}`;
}

type IdentityGroup = { identity_key: string; long_name_strref: number | null };

/**
 * Resolve a deep-link / saved identity key against current speaker groups.
 * Accepts sex-scoped keys (`15855:2`) and legacy plain strrefs (`15855`).
 */
export function findGroupByIdentityParam<T extends IdentityGroup>(
  groups: T[],
  key: string,
): T | undefined {
  const exact = groups.find((g) => g.identity_key === key);
  if (exact) return exact;
  if (/^\d+$/.test(key)) {
    const strref = Number(key);
    return groups.find((g) => g.long_name_strref === strref);
  }
  return undefined;
}
