/** A parsed semver version: numeric core plus optional prerelease identifiers. */
interface ParsedVersion {
  major: number;
  minor: number;
  patch: number;
  /** Empty for a release; e.g. ['lxxh', 'preview'] for `6.0.7-lxxh-preview`. */
  prerelease: string[];
}

// Strict semver. Build metadata (`+sha.abc`) is allowed but ignored, exactly as
// the spec requires — it never affects precedence.
const SEMVER = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/;

function parseVersion(value: string): ParsedVersion | null {
  const match = SEMVER.exec(value.trim());
  if (!match) return null;
  const [, major, minor, patch, pre] = match;
  return {
    major: Number(major),
    minor: Number(minor),
    patch: Number(patch),
    prerelease: pre ? pre.split('.') : [],
  };
}

/**
 * Compare prerelease identifiers per semver §11: numeric identifiers compare
 * numerically, alphanumerics compare lexically, numeric sorts below
 * alphanumeric, and a shorter identifier set sorts below a longer one when all
 * preceding identifiers are equal.
 */
function comparePrerelease(a: string[], b: string[]): number {
  if (!a.length && !b.length) return 0;
  // A version WITHOUT a prerelease outranks any prerelease of the same core.
  if (!a.length) return 1;
  if (!b.length) return -1;
  const shared = Math.min(a.length, b.length);
  for (let i = 0; i < shared; i++) {
    const left = a[i];
    const right = b[i];
    const leftNumeric = /^\d+$/.test(left);
    const rightNumeric = /^\d+$/.test(right);
    if (leftNumeric && rightNumeric) {
      const diff = Number(left) - Number(right);
      if (diff !== 0) return diff > 0 ? 1 : -1;
    } else if (leftNumeric || rightNumeric) {
      // Numeric identifiers always have lower precedence than alphanumeric.
      return leftNumeric ? -1 : 1;
    } else if (left !== right) {
      return left < right ? -1 : 1;
    }
  }
  return a.length === b.length ? 0 : a.length > b.length ? 1 : -1;
}

/** Returns true only if `remote` is strictly newer than `local` (semver).
 *  Returns false when either side fails to parse — avoids false "update
 *  available" prompts from a malformed manifest.
 *
 *  Prerelease suffixes are supported because this fork numbers its builds like
 *  `6.0.7-lxxh-preview`: a bare `split('.').map(Number)` used to yield NaN on
 *  the suffix and silently disable every update prompt. Precedence follows
 *  semver: `6.0.6` < `6.0.7-preview` < `6.0.7-preview.2` < `6.0.7`. */
export function isNewerVersion(remote: string, local: string): boolean {
  const r = parseVersion(remote);
  const l = parseVersion(local);
  if (!r || !l) return false;
  if (r.major !== l.major) return r.major > l.major;
  if (r.minor !== l.minor) return r.minor > l.minor;
  if (r.patch !== l.patch) return r.patch > l.patch;
  return comparePrerelease(r.prerelease, l.prerelease) > 0;
}
