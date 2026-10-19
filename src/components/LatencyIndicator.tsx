import { useEffect, useId, useRef, useState } from 'react';
import { useDocumentVisible } from '../hooks/useDocumentVisible';
import { useI18n } from '../hooks/useI18n';

const latencyThreadDuration = (Math.PI * 2) / 1.7;
const latencyThreads = Array.from({ length: 7 }, (_, index) => {
  const frames = Array.from({ length: 25 }, (_, frame) =>
    Array.from({ length: 37 }, (_, point) => {
      const x = point * 2;
      const envelope = Math.pow(Math.sin((x / 72) * Math.PI), 1.2);
      const wave = Math.sin(x * 0.085 - 0.8 * 1.7 - (frame / 24) * Math.PI * 2 + index * 0.19);
      const y = 8 + envelope * (wave * (2.5 + index * 0.48) + (index - 3) * 0.5);
      return `${point === 0 ? 'M' : 'L'}${x},${y.toFixed(2)}`;
    }).join(' ')
  );
  return { path: frames[0], frames: frames.join(';') };
});

export function LatencyIndicator() {
  const { t } = useI18n();
  const gradientId = useId();
  const svgRef = useRef<SVGSVGElement>(null);
  const visible = useDocumentVisible();
  const [reducedMotion, setReducedMotion] = useState(
    () => window.matchMedia('(prefers-reduced-motion: reduce)').matches
  );

  useEffect(() => {
    const preference = window.matchMedia('(prefers-reduced-motion: reduce)');
    const update = () => setReducedMotion(preference.matches);
    preference.addEventListener('change', update);
    return () => preference.removeEventListener('change', update);
  }, []);

  useEffect(() => {
    if (!visible || reducedMotion) return;
    // Start after the SVG timeline has been registered by the browser.
    const frame = requestAnimationFrame(() => {
      svgRef.current?.querySelectorAll('animate').forEach((animation) => animation.beginElement());
    });
    return () => cancelAnimationFrame(frame);
  }, [visible, reducedMotion]);

  return (
    <span className="model-latency-loading" role="status">
      <span className="sr-only">{t('mother.testing')}</span>
      <svg
        ref={svgRef}
        className="model-latency-threads"
        viewBox="0 0 72 16"
        fill="none"
        stroke={`url(#${gradientId})`}
        aria-hidden="true"
      >
        <defs>
          <linearGradient id={gradientId}>
            <stop offset="0%" stopColor="currentColor" stopOpacity="0" />
            <stop offset="15%" stopColor="currentColor" stopOpacity="0.65" />
            <stop offset="50%" stopColor="currentColor" />
            <stop offset="85%" stopColor="currentColor" stopOpacity="0.65" />
            <stop offset="100%" stopColor="currentColor" stopOpacity="0" />
          </linearGradient>
        </defs>
        {latencyThreads.map((thread, index) => (
          <path
            key={index}
            d={thread.path}
            strokeWidth={index === 3 ? 0.9 : 0.55}
            opacity={index === 3 ? 1 : 0.28 + index * 0.055}
            strokeLinecap="round"
          >
            {visible && !reducedMotion && (
              <animate
                attributeName="d"
                values={thread.frames}
                dur={`${latencyThreadDuration}s`}
                begin="indefinite"
                repeatCount="indefinite"
              />
            )}
          </path>
        ))}
      </svg>
    </span>
  );
}
