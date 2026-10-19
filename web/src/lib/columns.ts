/** How many columns grids have on phones (2, 3 or 4), remembered per device: one count
 *  for folder cards (the Photos page) and one for photos. */
export const COLUMN_CHOICES = [2, 3, 4] as const;
export type Columns = (typeof COLUMN_CHOICES)[number];
export type GridKind = 'folders' | 'photos';

// public/theme-init.js applies the saved counts before first paint.
const SETTINGS: Record<GridKind, { key: string; variable: string; fallback: Columns }> = {
  folders: { key: 'pixelgrove-folder-columns', variable: '--folder-columns', fallback: 2 },
  photos: { key: 'pixelgrove-columns', variable: '--phone-columns', fallback: 3 },
};

export function currentColumns(kind: GridKind): Columns {
  const { variable, fallback } = SETTINGS[kind];
  const n = Number(document.documentElement.style.getPropertyValue(variable));
  return (COLUMN_CHOICES as readonly number[]).includes(n) ? (n as Columns) : fallback;
}

/** Sets the column count; grids read it from a CSS variable. */
export function setColumns(kind: GridKind, n: Columns): void {
  const { key, variable } = SETTINGS[kind];
  document.documentElement.style.setProperty(variable, String(n));
  try {
    localStorage.setItem(key, String(n));
  } catch {
    // Storage can be unavailable (private mode); the choice still applies for this visit.
  }
}
