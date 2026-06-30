// Shared formatting helpers. Previously each view had its own (often divergent)
// copy of these, so the same number could render differently per page.

/** Human-readable byte size, e.g. 1536 -> "1.50 KB". Bytes are shown as integers,
 *  everything else with 2 decimals. Non-positive / nullish input yields "0 B". */
export function formatBytes(bytes?: number | null): string {
  if (bytes == null || !Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / Math.pow(1024, i);
  return `${value.toFixed(i === 0 ? 0 : 2)} ${units[i]}`;
}
