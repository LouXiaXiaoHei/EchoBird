export function devinQuotaLabel(label: string): string {
  return label === '1 day' ? '24h' : label === '7 days' ? '7d' : label;
}
