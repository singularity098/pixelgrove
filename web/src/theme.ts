/** Colour themes. Their colours live in app.css under `[data-theme='<id>']`. */
export interface ThemeInfo {
  id: string;
  name: string;
  /** Whether it's a dark or light theme (for the map and the browser's own controls). */
  mode: 'dark' | 'light';
  /** Background, card and accent colours, for previews and the app's title bar. */
  swatch: [string, string, string];
}

export const THEMES: ThemeInfo[] = [
  { id: 'dark', name: 'Dark', mode: 'dark', swatch: ['#0f1113', '#22262b', '#4fc184'] },
  { id: 'light', name: 'Light', mode: 'light', swatch: ['#f7f8f9', '#ffffff', '#1f8a55'] },
  { id: 'amoled', name: 'AMOLED Black', mode: 'dark', swatch: ['#000000', '#161616', '#4fc184'] },
  { id: 'dracula', name: 'Dracula', mode: 'dark', swatch: ['#21222c', '#343746', '#bd93f9'] },
  { id: 'tokyo-night', name: 'Tokyo Night', mode: 'dark', swatch: ['#1a1b26', '#292e42', '#7aa2f7'] },
  { id: 'catppuccin-mocha', name: 'Catppuccin Mocha', mode: 'dark', swatch: ['#181825', '#313244', '#cba6f7'] },
  { id: 'catppuccin-macchiato', name: 'Catppuccin Macchiato', mode: 'dark', swatch: ['#1e2030', '#363a4f', '#c6a0f6'] },
  { id: 'catppuccin-frappe', name: 'Catppuccin Frappé', mode: 'dark', swatch: ['#292c3c', '#414559', '#ca9ee6'] },
  { id: 'catppuccin-latte', name: 'Catppuccin Latte', mode: 'light', swatch: ['#e6e9ef', '#dce0e8', '#8839ef'] },
];

const STORAGE_KEY = 'pixelgrove-theme';

export function themeInfo(id: string): ThemeInfo {
  return THEMES.find((t) => t.id === id) ?? THEMES[0];
}

/** The theme in use (public/theme-init.js applies the saved one before first paint). */
export function currentTheme(): string {
  return themeInfo(document.documentElement.dataset.theme ?? 'dark').id;
}

/** Applies a theme to the page: colours, light/dark mode and the title bar colour. */
export function applyTheme(id: string): void {
  const theme = themeInfo(id);
  const root = document.documentElement;
  root.dataset.theme = theme.id;
  root.dataset.mode = theme.mode;
  // Match the installed app's title bar to the theme.
  document.querySelector('meta[name="theme-color"]')?.setAttribute('content', theme.swatch[0]);
}

export function setTheme(id: string): void {
  applyTheme(id);
  try {
    localStorage.setItem(STORAGE_KEY, themeInfo(id).id);
  } catch {
    // Storage can be unavailable (private mode); the theme still applies for this visit.
  }
}
