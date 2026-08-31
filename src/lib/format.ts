/** Human-readable byte size. */
export function humanSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  const units = ["o", "Kio", "Mio", "Gio", "Tio"];
  let i = 0;
  let n = bytes;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return `${n.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

/** Normalise a user-typed server URL to a bare origin with no trailing slash. */
export function normaliseServerUrl(input: string): string {
  let s = input.trim();
  if (!s) return s;
  if (!/^https?:\/\//i.test(s)) s = `https://${s}`;
  try {
    return new URL(s).origin;
  } catch {
    return s.replace(/\/+$/, "");
  }
}
