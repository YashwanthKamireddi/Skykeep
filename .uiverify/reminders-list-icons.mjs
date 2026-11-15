// Each list shows the icon chosen on the iPhone (#40): an emoji as the emoji itself, Apple's
// named icons each as their own glyph, and only an unknown one as the plain list icon.
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const ctx = await b.newContext({ viewport: { width: 1180, height: 800 } });
await ctx.addInitScript(() => { window.__listEmblems = { l1: '{"Emoji":"🛒"}', l2: "fitness", l3: "education2" }; });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, remindersFixture: true });
const page = await ctx.newPage();
page.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await page.goto(`${process.env.URL ?? "http://localhost:4173"}/?app=reminders`, { waitUntil: "networkidle" });
await page.waitForTimeout(600);
const badge = (name) => page.locator(".rx-list-row, .source-row", { hasText: name }).first().locator(".rx-badge");
ok((await badge("Daily").locator(".list-emoji").innerText()) === "🛒", "an emoji icon shows as the emoji");
const svg = async (name) => (await badge(name).locator("svg").getAttribute("class")) ?? "";
const gym = await svg("GYM"), goals = await svg("Life Goals");
ok(/dumbbell/.test(gym), `fitness shows its own glyph  [${gym}]`);
ok(/graduation-cap/.test(goals), `education2 shows the family's second glyph  [${goals}]`);
const box = await badge("Daily").boundingBox();
const emo = await badge("Daily").locator(".list-emoji").boundingBox();
ok(box && emo && emo.width <= box.width && emo.height <= box.height, `the emoji fits its badge  ${JSON.stringify([box, emo])}`);
await b.close();
process.exitCode = failures ? 1 : 0;
