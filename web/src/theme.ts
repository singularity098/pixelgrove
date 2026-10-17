export type Theme = 'dark' | 'light';

const STORAGE_KEY = 'pixelgrove-theme';

export function currentTheme(): Theme {
  return document.documentElement.dataset.theme === 'light' ? 'light' : 'dark';
}

export function setTheme(theme: Theme): void {
  document.documentElement.dataset.theme = theme;
  // Match the installed app's title bar to the theme.
  document
    .querySelector('meta[name="theme-color"]')
    ?.setAttribute('content', theme === 'light' ? '#f7f8f9' : '#0f1113');
  try {
    localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    // Storage can be unavailable (private mode); the theme still applies for this visit.
  }
}
