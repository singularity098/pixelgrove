// Takes the README screenshots (docs/screenshots) against a demo server. Usage:
//   python3 scripts/screenshots/demo_library.py "/tmp/demo/Family Library"
//   PIXELGROVE_MEDIA_ROOTS="/tmp/demo/Family Library" PIXELGROVE_BIND=127.0.0.1:8092 \
//     PIXELGROVE_ADMIN_USER=admin PIXELGROVE_ADMIN_PASSWORD=demo-password \
//     PIXELGROVE_CONFIG_DIR=/tmp/demo/config target/release/pixelgrove
//   PW=$(npm root -g)/playwright VIEWER_PHOTO=DSC01001.jpg node scripts/screenshots/shoot.mjs /tmp/demo/out
// then convert the PNGs to WebP (see the README in this folder).
import { createRequire } from 'module';
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PW);
const base = 'http://localhost:8092';
const out = process.argv[2];
const browser = await chromium.launch();
const errors = [];

async function waitImages(page) {
  await page.waitForFunction(
    () =>
      [...document.querySelectorAll('img')]
        .filter((img) => {
          const r = img.getBoundingClientRect();
          return r.bottom > 0 && r.top < innerHeight && r.right > 0 && r.left < innerWidth && r.width > 0;
        })
        .every((img) => img.complete && img.naturalWidth > 0),
    null,
    { timeout: 30000, polling: 250 },
  );
  await page.waitForTimeout(400);
}

async function signIn(page) {
  await page.goto(base);
  await page.getByLabel('Username').fill('admin');
  await page.getByLabel('Password').fill('demo-password');
  await page.getByRole('button', { name: 'Sign in' }).click();
  await page.locator('.album').first().waitFor();
}

async function context(opts, theme = 'dark') {
  const ctx = await browser.newContext(opts);
  await ctx.addInitScript((t) => localStorage.setItem('pixelgrove-theme', t), theme);
  const page = await ctx.newPage();
  page.on('pageerror', (e) => errors.push(String(e)));
  return page;
}

const desktop = { viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2 };
const albums = async (page) => (await (await page.request.get(`${base}/api/albums?limit=100`)).json()).items;

// 1. Photos (dark)
const page = await context(desktop);
await signIn(page);
await waitImages(page);
await page.screenshot({ path: `${out}/photos.png` });

// 2. A folder
const list = await albums(page);
const iceland = list.find((a) => a.name === 'Iceland Ring Road');
await page.goto(`${base}/folder/${iceland.id}`);
await page.locator('.tile').first().waitFor();
await waitImages(page);
await page.screenshot({ path: `${out}/folder.png` });

// 3. Viewer with info, on an aurora photo
const items = (await (await page.request.get(`${base}/api/folders/${iceland.id}/media?limit=200`)).json()).items;
const pick = items.find((m) => m.name === process.env.VIEWER_PHOTO) ?? items[3];
await page.goto(`${base}/folder/${iceland.id}?m=${pick.id}`);
await page.locator('.viewer img').waitFor();
await page.keyboard.press('i');
await page.locator('.info dl').waitFor();
await waitImages(page);
await page.mouse.move(700, 450);
await page.waitForTimeout(600);
await page.screenshot({ path: `${out}/viewer.png` });
await page.keyboard.press('Escape');

// 4. Explore
await page.goto(`${base}/explore`);
await page.getByRole('heading', { name: 'Places' }).waitFor();
await waitImages(page);
await page.screenshot({ path: `${out}/explore.png` });

// 5. A folder in the light theme
const light = await context(desktop, 'light');
await signIn(light);
const alps = list.find((a) => a.name === 'Kyoto in Spring');
await light.goto(`${base}/folder/${alps.id}`);
await light.locator('.tile').first().waitFor();
await waitImages(light);
await light.screenshot({ path: `${out}/folder-light.png` });

// 6. Shared folder, as a visitor
const share = await (
  await page.request.post(`${base}/api/shares`, { headers: { 'x-pixelgrove': '1' }, data: { folder_id: iceland.id } })
).json();
const visitor = await context(desktop);
await visitor.goto(base + share.url);
await visitor.locator('.tile').first().waitFor();
await waitImages(visitor);
await visitor.screenshot({ path: `${out}/share.png` });

// 7. Phone: photos, folder, viewer
const phoneOpts = { viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true };
const phone = await context(phoneOpts);
await signIn(phone);
await waitImages(phone);
await phone.screenshot({ path: `${out}/phone-photos.png` });
const kyoto = list.find((a) => a.name === 'Kyoto in Spring');
await phone.goto(`${base}/folder/${kyoto.id}`);
await phone.locator('.tile').first().waitFor();
await waitImages(phone);
await phone.screenshot({ path: `${out}/phone-folder.png` });
const kyotoItems = (await (await phone.request.get(`${base}/api/folders/${kyoto.id}/media?limit=200`)).json()).items;
const portrait = kyotoItems.find((m) => m.height > m.width) ?? kyotoItems[0];
await phone.goto(`${base}/folder/${kyoto.id}?m=${portrait.id}`);
await phone.locator('.viewer img').waitFor();
await waitImages(phone);
await phone.screenshot({ path: `${out}/phone-viewer.png` });

console.log('errors:', errors);
await browser.close();
