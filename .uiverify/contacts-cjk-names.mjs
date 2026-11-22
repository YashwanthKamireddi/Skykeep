// Chinese, Japanese and Korean names read family name first, nothing between, as Apple's
// Contacts shows them (#232): in the list (the family name in bold) and at the top of the
// card. Every other name keeps given name first.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1200, height: 800 } });
await ctx.addInitScript(() => {
  const card = (guid, firstName, lastName) => ({ guid, firstName, lastName, company: null, isCompany: false,
    birthday: null, notes: null, etag: null, methods: [{ kind: "phone", label: null, value: "+86 10 0000 0000" }] });
  window.__contactsExtra = [card("z1", "小明", "王"), card("j1", "駿", "宮崎"), card("k1", "민준", "김")];
});
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, contactsFixture: true });
const page = await ctx.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Contacts");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const rowOf = (text) => page.locator(".contact-rows > li", { hasText: text });
/** Every row's name as the row writes it, avatar left out. */
const names = () => page.locator(".contact-rows .contact-row-name").allInnerTexts();

const listed = await names();
for (const [full, family] of [["王小明", "王"], ["宮崎駿", "宮崎"], ["김민준", "김"]]) {
  ok(listed.includes(full), `${full} is listed family name first  ${JSON.stringify(listed)}`);
  const row = page.locator(".contact-row-name", { hasText: full });
  ok((await row.locator("b").first().innerText().catch(() => "")) === family, `with ${family} in bold`);
}
ok(listed.includes("Zoya Ahuja"), "a Latin name keeps given name first");
ok((await rowOf("Zoya Ahuja").locator("b").innerText()) === "Ahuja", "with its last name in bold");

await page.locator(".contact-row-name", { hasText: "王小明" }).click();
await page.waitForTimeout(300);
const heading = await page.locator("h2").filter({ hasText: "王" }).first().innerText().catch(() => "");
ok(heading === "王小明", `the card's name reads family name first  ${JSON.stringify(heading)}`);

ok(errors.length === 0, `no page errors  ${errors.join(" | ")}`);
await b.close();
process.exitCode = failures ? 1 : 0;
