/** Moves the tab `fromId` before the tab `beforeId` (`null` = to the end). Unknown ids return the input. */
export function reorder<T extends { id: string }>(
  tabs: T[],
  fromId: string,
  beforeId: string | null,
): T[] {
  const from = tabs.find((t) => t.id === fromId);
  if (!from) return tabs;
  if (beforeId !== null && !tabs.some((t) => t.id === beforeId)) return tabs;
  if (beforeId === fromId) return tabs;
  const rest = tabs.filter((t) => t.id !== fromId);
  const at = beforeId === null ? rest.length : rest.findIndex((t) => t.id === beforeId);
  return [...rest.slice(0, at), from, ...rest.slice(at)];
}
