import { describe, expect, it } from 'vitest';
import { isNewerVersion } from './version';

describe('isNewerVersion', () => {
  it('compares plain numeric versions', () => {
    expect(isNewerVersion('6.0.7', '6.0.6')).toBe(true);
    expect(isNewerVersion('6.0.6', '6.0.6')).toBe(false);
    expect(isNewerVersion('6.0.5', '6.0.6')).toBe(false);
    expect(isNewerVersion('6.1.0', '6.0.9')).toBe(true);
    expect(isNewerVersion('7.0.0', '6.9.9')).toBe(true);
    expect(isNewerVersion('6.0.10', '6.0.9')).toBe(true);
  });

  it('ignores a leading v and build metadata', () => {
    expect(isNewerVersion('v6.0.7', '6.0.6')).toBe(true);
    expect(isNewerVersion('6.0.7+sha.abc123', '6.0.6')).toBe(true);
    expect(isNewerVersion('6.0.7+build.9', '6.0.7')).toBe(false);
  });

  // The fork numbers its builds `6.0.7-lxxh-preview`. A prerelease must beat an
  // older core version, otherwise installed users would never be notified.
  it('treats a prerelease of a newer core as newer', () => {
    expect(isNewerVersion('6.0.7-lxxh-preview', '6.0.6')).toBe(true);
    expect(isNewerVersion('6.0.6-lxxh-preview', '6.0.6')).toBe(false);
    expect(isNewerVersion('6.0.6-lxxh-preview', '6.0.5')).toBe(true);
  });

  it('ranks a release above its own prereleases', () => {
    expect(isNewerVersion('6.0.7', '6.0.7-lxxh-preview')).toBe(true);
    expect(isNewerVersion('6.0.7-lxxh-preview', '6.0.7')).toBe(false);
  });

  it('orders prerelease identifiers per semver §11', () => {
    expect(isNewerVersion('6.0.7-lxxh-preview.2', '6.0.7-lxxh-preview')).toBe(true);
    expect(isNewerVersion('6.0.7-lxxh-preview.10', '6.0.7-lxxh-preview.9')).toBe(true);
    // numeric identifiers sort below alphanumeric ones
    expect(isNewerVersion('6.0.7-alpha', '6.0.7-1')).toBe(true);
    // lexical order for two alphanumerics
    expect(isNewerVersion('6.0.7-beta', '6.0.7-alpha')).toBe(true);
  });

  it('returns false for anything unparseable', () => {
    expect(isNewerVersion('not-a-version', '6.0.6')).toBe(false);
    expect(isNewerVersion('6.0.7', 'unknown')).toBe(false);
    expect(isNewerVersion('6.0', '6.0.6')).toBe(false);
    expect(isNewerVersion('', '')).toBe(false);
  });
});
