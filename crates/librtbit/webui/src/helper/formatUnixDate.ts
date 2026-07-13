/** Format a unix timestamp (seconds) as a compact local date-time string. */
export function formatUnixDate(unixSecs: number | undefined | null): string {
  if (!unixSecs) return "—";
  const d = new Date(unixSecs * 1000);
  const date = d.toLocaleDateString(undefined, {
    year: "2-digit",
    month: "2-digit",
    day: "2-digit",
  });
  const time = d.toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  });
  return `${date} ${time}`;
}
