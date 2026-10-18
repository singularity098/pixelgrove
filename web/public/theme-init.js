// Apply the saved theme and phone grid columns before first paint to avoid a flash
// (src/theme.ts then sets the light/dark mode and title bar colour).
try {
  const theme = localStorage.getItem('pixelgrove-theme');
  if (theme && /^[a-z-]+$/.test(theme)) document.documentElement.dataset.theme = theme;
  const columns = localStorage.getItem('pixelgrove-columns');
  if (columns && /^[234]$/.test(columns)) document.documentElement.style.setProperty('--phone-columns', columns);
} catch {}
