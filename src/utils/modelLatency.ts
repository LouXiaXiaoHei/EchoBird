export function modelLatencyColor(latency: number): string {
  return latency < 200 ? 'text-green-500' : latency < 500 ? 'text-yellow-500' : 'text-red-500';
}
