/** How many columns photo and folder grids have on phones (2, 3 or 4), remembered per device. */
export const COLUMN_CHOICES = [2, 3, 4] as const;
export type Columns = (typeof COLUMN_CHOICES)[number];

const STORAGE_KEY = 'pixelgrove-columns';

export function currentColumns(): Columns {
  const n = Number(document.documentElement.style.getPropertyValue('--phone-columns'));
  return (COLUMN_CHOICES as readonly number[]).includes(n) ? (n as Columns) : 3;
}

/** Sets the column count; grids read it from the --phone-columns CSS variable. */
export function setColumns(n: Columns): void {
  document.documentElement.style.setProperty('--phone-columns', String(n));
  try {
    localStorage.setItem(STORAGE_KEY, String(n));
  } catch {
    // Storage can be unavailable (private mode); the choice still applies for this visit.
  }
}
