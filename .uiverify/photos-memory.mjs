import assert from 'node:assert/strict';
import { engine } from './engine.mjs';
import { SERVICES, installStub, openScreen } from './stub.mjs';
const browser = await engine.launch();
try {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  await ctx.addInitScript(installStub, { screen: 'library', services: SERVICES, twoFactor: null });
  await ctx.addInitScript(() => {
    window.__picBase = 'http://localhost:4173/test-pictures';
    window.__photos = Array.from({ length: 1800 }, (_, i) => ({
      id: `q${i}`, master: `m${i}`, takenAt: 1792600000 - i * 600, addedAt: 1792600000 - i * 600,
      kind: 'image', width: 4032, height: 3024, filename: `${i}.JPG`, size: 2400000,
      favorite: false, hidden: false, deleted: false, deletedAt: null, subtype: 0, changeTag: null,
    }));
  });
  const page = await ctx.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  const requests = [];
  await page.route('**/test-pictures/**', async (route) => {
    const path = decodeURIComponent(route.request().url());
    requests.push(path);
    const edge = Number(path.match(/fit(\d+)/)?.[1] ?? 128);
    await route.fulfill({ contentType: 'image/svg+xml', body: `<svg xmlns="http://www.w3.org/2000/svg" width="${edge}" height="${Math.round(edge * .75)}"><rect width="100%" height="100%" fill="#829da1"/></svg>` });
  });
  await page.goto(process.env.URL ?? 'http://localhost:4173', { waitUntil: 'networkidle' });
  await openScreen(page, 'Photos');
  await page.locator('.px-tile img[src]').first().waitFor();
  await page.waitForTimeout(600);
  const budget = await page.locator('.px-tile img[src]').evaluateAll((images) => ({
    count: images.length,
    bytes: images.reduce((n, img) => n + Number(decodeURIComponent(img.src).match(/fit(\d+)/)?.[1] ?? 0) ** 2 * 4, 0),
  }));
  assert(budget.count > 0 && budget.count < 150, JSON.stringify(budget));
  assert(budget.bytes <= 32 * 1024 * 1024, JSON.stringify(budget));
  console.log('PASS 1800 photos: only nearby images receive sources; requested grid pixels stay within 32 MiB', budget);
  requests.length = 0;
  const first = page.locator('.px-tile').first();
  const firstId = await first.getAttribute('data-id');
  await first.hover();
  await page.waitForTimeout(500);
  assert(!requests.some((url) => Number(url.match(/fit(\d+)/)?.[1] ?? 0) > 512));
  console.log('PASS hovering a thumbnail does not decode a large image');
  await first.dblclick();
  await page.locator('.px-viewer').waitFor();
  await page.waitForTimeout(600);
  const large = requests.filter((url) => Number(url.match(/fit(\d+)/)?.[1] ?? 0) > 512);
  assert(large.length > 0 && large.every((url) => url.includes(`pic/${firstId}~`)), JSON.stringify(large));
  console.log('PASS opening a photo requests its large image without decoding four neighbours');
  await page.keyboard.press('Escape');
  await page.locator('.px-scroll').evaluate((el) => { el.scrollTop = el.scrollHeight; });
  await page.waitForTimeout(800);
  const oldSrc = await page.locator(`.px-tile[data-id="${firstId}"] img`).evaluateAll((els) => els.some((el) => el.hasAttribute('src')));
  assert(!oldSrc);
  console.log('PASS scrolling away removes the old image source');
  await openScreen(page, 'Notes');
  assert.equal(await page.locator('.px-tile').count(), 0);
  assert.equal(await page.locator('.px-viewer').count(), 0);
  console.log('PASS changing section unmounts the Photos image tree');
  await openScreen(page, 'Photos');
  await page.locator('.px-tile img[src]').first().waitFor();
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { configurable: true, value: true });
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await page.waitForTimeout(100);
  assert.equal(await page.locator('.px-tile').count(), 0);
  await page.evaluate(() => {
    delete document.hidden;
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await page.locator('.px-tile img[src]').first().waitFor();
  console.log('PASS hiding the window releases Photos; showing it restores the screen');
  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
