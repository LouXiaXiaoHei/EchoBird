import type { ComponentProps } from 'react';
import type { LucideIcon } from 'lucide-react';

export function PageActionButton({
  icon: Icon,
  spinning = false,
  children,
  className = '',
  ...props
}: ComponentProps<'button'> & { icon: LucideIcon; spinning?: boolean }) {
  return (
    <button
      type="button"
      {...props}
      className={`flex h-9 shrink-0 items-center justify-center rounded-button border border-cyber-border text-sm text-cyber-text transition-colors enabled:hover:bg-cyber-text/10 disabled:text-cyber-text-muted disabled:opacity-50 outline-none ${children ? 'gap-1.5 px-3' : 'w-9'} ${className}`}
    >
      <Icon size={16} className={`shrink-0 ${spinning ? 'animate-spin' : ''}`} aria-hidden="true" />
      {children}
    </button>
  );
}
