import { Suspense, useState, type ReactNode } from 'react';

// Keep visited display layers mounted. Providers and background work live outside.
export function OnFirstVisit({ active, children }: { active: boolean; children: ReactNode }) {
  const [visited, setVisited] = useState(active);
  if (active && !visited) setVisited(true);
  return active || visited ? <Suspense fallback={null}>{children}</Suspense> : null;
}
