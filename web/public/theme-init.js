// Apply the saved theme before first paint to avoid a flash (src/theme.ts then sets the
// light/dark mode and title bar colour).
try {
  const theme = localStorage.getItem('pixelgrove-theme');
  if (theme && /^[a-z-]+$/.test(theme)) document.documentElement.dataset.theme = theme;
} catch {}
