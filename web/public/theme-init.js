// Apply the saved theme before first paint to avoid a flash.
try {
  const theme = localStorage.getItem('pixelgrove-theme');
  if (theme === 'light' || theme === 'dark') document.documentElement.dataset.theme = theme;
  if (theme === 'light') document.querySelector('meta[name="theme-color"]').setAttribute('content', '#f7f8f9');
} catch {}
