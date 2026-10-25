// Release endpoints for THIS fork.
//
// Upstream EchoBird reads its update manifest from echobird.ai and hard-codes
// the `edison7009/EchoBird` repository in the self-updater. A fork publishes
// builds of its own, so the version check, the changelog and the in-app update
// all have to point at the fork's releases — otherwise users are told a newer
// version exists that they cannot install.
//
// The manifest itself is committed to this repo by
// `.github/workflows/sync-version-manifest.yml` (docs/api/version/index.json)
// and also uploaded as a release asset, so there are three interchangeable
// sources. They are tried in order:
//
//  1. the release asset — published in lockstep with the binaries, never stale;
//  2. jsDelivr's copy of the branch — usually reachable from mainland China
//     where raw.githubusercontent.com is unreliable;
//  3. raw.githubusercontent.com — the authoritative branch copy.

export const RELEASE_REPO = 'LouXiaXiaoHei/EchoBird';

export const RELEASE_PAGE_URL = `https://github.com/${RELEASE_REPO}/releases/latest`;

const MANIFEST_PATH = 'docs/api/version/index.json';

export const VERSION_MANIFEST_URLS = [
  `${RELEASE_PAGE_URL}/download/index.json`,
  `https://cdn.jsdelivr.net/gh/${RELEASE_REPO}@main/${MANIFEST_PATH}`,
  `https://raw.githubusercontent.com/${RELEASE_REPO}/main/${MANIFEST_PATH}`,
];

/** Shape of the version manifest shared by the app (see sync-version-manifest.yml). */
export interface VersionManifest {
  version?: string;
  releaseDate?: string;
  /** Legacy single blob with "## 中文 / ## English / ## 日本語" sections. */
  releaseNotes?: string;
  /** Preferred: notes per language, keyed "en" | "zh-cn" | "zh-tw" | "ja". */
  releaseNotesI18n?: Record<string, string>;
}

/**
 * Fetch the manifest from the first source that answers.
 *
 * Returns `null` when every source fails — callers treat that as "no update
 * information" and stay quiet, which is the behaviour users expect offline.
 */
export async function fetchVersionManifest(): Promise<VersionManifest | null> {
  for (const url of VERSION_MANIFEST_URLS) {
    try {
      const res = await fetch(url, { cache: 'no-store' });
      if (!res.ok) continue;
      return (await res.json()) as VersionManifest;
    } catch {
      /* unreachable — try the next source */
    }
  }
  return null;
}
