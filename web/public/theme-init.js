// Apply the saved theme and phone grid columns before first paint to avoid a flash
// (src/theme.ts then sets the light/dark mode and title bar colour).
try {
  const theme = localStorage.getItem('pixelgrove-theme');
  if (theme && /^[a-z-]+$/.test(theme)) document.documentElement.dataset.theme = theme;
  for (const [key, variable] of [
    ['pixelgrove-columns', '--phone-columns'],
    ['pixelgrove-folder-columns', '--folder-columns'],
  ]) {
    const columns = localStorage.getItem(key);
    if (columns && /^[234]$/.test(columns)) document.documentElement.style.setProperty(variable, columns);
  }
} catch {}
