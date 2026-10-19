// Takes the README screenshots (docs/screenshots) against a demo server. Usage:
//   python3 scripts/screenshots/demo_photos.py <Demo_Images dir> /tmp/demo/lib
//   ln -s /tmp/demo/lib /photos   # the path the viewer's info panel shows
//   PIXELGROVE_MEDIA_ROOTS=/photos PIXELGROVE_BIND=127.0.0.1:8092 \
//     PIXELGROVE_ADMIN_USER=admin PIXELGROVE_ADMIN_PASSWORD=demo-password \
//     PIXELGROVE_CONFIG_DIR=/tmp/demo/config target/release/pixelgrove
//   PW=$(npm root -g)/playwright node scripts/screenshots/shoot.mjs /tmp/demo/out
// then convert the PNGs to WebP (see the README in this folder).
import { createRequire } from 'module';
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PW);
const base = 'http://localhost:8092';
const out = process.argv[2];
// The aurora over cabins, and the toucan (portrait), as demo_photos.py names them.
const VIEWER_PHOTO = process.env.VIEWER_PHOTO ?? 'IMG_4107.jpg';
const PHONE_PHOTO = process.env.PHONE_PHOTO ?? 'DSCF4177.jpg';
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

const json = async (page, path) => (await page.request.get(base + path)).json();
const album = (list, name) => list.find((a) => a.name === name);
const mediaIn = async (page, a) => (await json(page, `/api/folders/${a.id}/media?limit=200`)).items;

const desktop = { viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2 };

// 1. Photos (dark)
const page = await context(desktop);
await signIn(page);
await waitImages(page);
await page.screenshot({ path: `${out}/photos.png` });
const list = (await json(page, '/api/albums?limit=100')).items;

// 2. Timeline
await page.goto(`${base}/timeline`);
await page.locator('.tile').first().waitFor();
await waitImages(page);
await page.screenshot({ path: `${out}/timeline.png` });

// 3. Viewer with info, on the aurora
const lapland = album(list, 'Lapland & Svalbard');
const pick = (await mediaIn(page, lapland)).find((m) => m.name === VIEWER_PHOTO);
await page.goto(`${base}/folder/${lapland.id}?m=${pick.id}`);
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
await page.mouse.move(1430, 890); // Away from the cards, so none is highlighted.
await waitImages(page);
await page.screenshot({ path: `${out}/explore.png` });

// 5. The timeline in the light theme, at the desert and the sea
const light = await context(desktop, 'light');
await signIn(light);
await light.goto(`${base}/timeline`);
await light.locator('.tile').first().waitFor();
await light.evaluate(async () => {
  for (let i = 0; i < 20; i++) {
    const day = [...document.querySelectorAll('h3.day')].find((h) => /Oct.*2024/.test(h.textContent));
    if (day) {
      scrollTo(0, day.getBoundingClientRect().top + scrollY - 70);
      return;
    }
    scrollTo(0, document.documentElement.scrollHeight);
    await new Promise((r) => setTimeout(r, 300));
  }
});
await light.waitForTimeout(500);
await waitImages(light);
await light.screenshot({ path: `${out}/light.png` });

// 6. Shared folder, as a visitor
const share = await (
  await page.request.post(`${base}/api/shares`, { headers: { 'x-pixelgrove': '1' }, data: { folder_id: lapland.id } })
).json();
// A smaller window, so the folder's nine items fill it.
const visitor = await context({ viewport: { width: 1024, height: 640 }, deviceScaleFactor: 2 });
await visitor.goto(base + share.url);
await visitor.locator('.tile').first().waitFor();
await waitImages(visitor);
await visitor.screenshot({ path: `${out}/share.png` });

// 7. Phone: photos, timeline, viewer
const phoneOpts = { viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true };
const phone = await context(phoneOpts);
await signIn(phone);
await waitImages(phone);
await phone.screenshot({ path: `${out}/phone-photos.png` });
await phone.goto(`${base}/timeline`);
await phone.locator('.tile').first().waitFor();
await waitImages(phone);
await phone.screenshot({ path: `${out}/phone-timeline.png` });
const monteverde = album(list, 'Monteverde');
const toucan = (await mediaIn(phone, monteverde)).find((m) => m.name === PHONE_PHOTO);
await phone.goto(`${base}/folder/${monteverde.id}?m=${toucan.id}`);
await phone.locator('.viewer img').waitFor();
await waitImages(phone);
await phone.screenshot({ path: `${out}/phone-viewer.png` });

console.log('errors:', errors);
await browser.close();
