// The code screen sends the code back the way it came: texted to a number, it is checked as
// that number's; asked for again on the devices after a text, it is checked as a device code
// again — a device code checked as a texted one was refused as wrong, every time.
//
//   URL=http://localhost:4203 ENGINE=webkit node .uiverify/signin-code-route.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const URL = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };
const TWO_FACTOR = { trustedDeviceCount: 2, phones: [{ id: 7, maskedNumber: "+1 (•••) •••-••01" }], tooManyCodesSent: false };

async function codeScreen() {
  const ctx = await b.newContext({ viewport: { width: 1100, height: 800 } });
  await ctx.addInitScript(installStub, { screen: "code", services: SERVICES, twoFactor: TWO_FACTOR });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
  await page.goto(URL, { waitUntil: "networkidle" });
  await page.fill('input[aria-label="Apple Account"]', "someone@example.com");
  await page.fill('input[aria-label="Password"]', "not-a-real-password");
  await page.click('button[type="submit"]');
  await page.locator(".code").waitFor();
  await page.waitForTimeout(200);
  return { ctx, page };
}
const type = async (page, code) => {
  await page.locator('[aria-label="Digit 1"]').click();
  await page.keyboard.type(code);
  await page.waitForTimeout(300);
};

{
  const { ctx, page } = await codeScreen();
  await page.getByRole("button", { name: "Didn't get a code?" }).click();
  await page.getByRole("button", { name: /^Text / }).click();
  await page.waitForTimeout(200);
  ok(/texted a code/.test(await page.locator(".lede").innerText()), "after Text, the screen says where the code went");
  await type(page, "123456");
  ok((await page.evaluate(() => window.__codeVia)) === "sms:7", `a texted code is checked as the number's  [${await page.evaluate(() => window.__codeVia)}]`);
  await ctx.close();
}
{
  const { ctx, page } = await codeScreen();
  await page.getByRole("button", { name: "Didn't get a code?" }).click();
  await page.getByRole("button", { name: /^Text / }).click();
  await page.waitForTimeout(200);
  await page.getByRole("button", { name: "Didn't get a code?" }).click();
  await page.getByRole("button", { name: "Send to my devices again" }).click();
  await page.waitForTimeout(200);
  const lede = await page.locator(".lede").innerText();
  ok(!/texted/.test(lede), `sent to the devices after a text, the screen no longer says it was texted  [${lede}]`);
  await type(page, "654321");
  ok((await page.evaluate(() => window.__codeVia)) === "devices", `and the code is checked as a device code  [${await page.evaluate(() => window.__codeVia)}]`);
  await ctx.close();
}

await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
