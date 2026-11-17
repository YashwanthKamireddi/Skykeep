// Calendar accounts other than iCloud: one Add CalDAV Account, by address and password, with a
// Nextcloud server preconfigured under it, from nothing but its address and the person's approval
// in their browser; each listed under its own heading, and let go with Remove Account.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1300, height: 850 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
const page = await ctx.newPage();
await page.goto(BASE, { waitUntil: "networkidle" });
await openScreen(page, "Calendar");
await page.keyboard.press("Control+2");
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const writes = () => page.evaluate(() => window.__davWrites ?? []);
const side = page.getByRole("complementary", { name: "Calendars" });
const addMenu = async (item) => {
  await side.getByRole("button", { name: "Add Calendar" }).click();
  await page.getByRole("menuitem", { name: item }).click();
};

ok(await side.getByRole("button", { name: "Nextcloud" }).count() === 0, "no Nextcloud group before there is one");

// A wrong address is said in the panel.
await page.evaluate(() => { window.__davFails = "There’s no Nextcloud at that address. Check it and try again."; });
await side.getByRole("button", { name: "Add Calendar" }).click();
ok(await page.getByRole("menuitem", { name: /Nextcloud/ }).count() === 0 && await page.getByRole("menuitem", { name: /Add CalDAV Account/ }).count() === 1,
  "the menu offers one way to add an account: CalDAV");
await page.getByRole("menuitem", { name: /Add CalDAV Account/ }).click();
const caldav = page.getByRole("form", { name: "Add CalDAV Account" });
const form = page.getByRole("form", { name: "Set Up Nextcloud Server" });
const preset = caldav.getByRole("button", { name: "Nextcloud Server" });
ok(await caldav.isVisible() && await preset.isVisible(), "the CalDAV panel offers a Nextcloud server under it");
await preset.click();
await page.waitForTimeout(100);
ok(await form.isVisible() && await caldav.count() === 0, "the preset opens the Nextcloud panel in its place");
await form.getByRole("button", { name: "Back" }).click();
await page.waitForTimeout(100);
ok(await caldav.isVisible() && await form.count() === 0, "Back returns to the CalDAV panel");
await preset.click();
await page.waitForTimeout(100);
ok(await form.getByRole("textbox").count() === 1 && await form.locator('input[type="password"]').count() === 0,
  "only the server is asked for: no username, no password");
await form.getByRole("textbox").fill("nope.example.org");
await form.getByRole("button", { name: "Continue" }).click();
await page.waitForTimeout(200);
ok(/no Nextcloud at that address/.test(await form.getByRole("alert").innerText().catch(() => "")), "a refusal is said where the address was typed");

// Approved in the browser: the panel waits, then the calendars are there by themselves.
await page.evaluate(() => { window.__davFails = null; });
await form.getByRole("textbox").fill("cloud.example.org");
await page.keyboard.press("Enter");
await page.waitForTimeout(300);
ok((await writes()).some((w) => w.begin === "cloud.example.org"), "the sign-in starts on that server");
ok(/in your browser/.test(await form.getByRole("status").innerText().catch(() => "")), "the panel says to approve it in the browser");
ok(await form.evaluate((f) => {
  const panel = f.getBoundingClientRect();
  return [...f.querySelectorAll(".cx-subscribe-actions button")].every((b) => {
    const r = b.getBoundingClientRect();
    return r.left >= panel.left && r.right <= panel.right;
  });
}), "both waiting buttons stay inside the panel");
await form.getByRole("button", { name: "Open Page Again" }).click();
await page.waitForTimeout(100);
ok((await writes()).some((w) => w.reopen), "the page can be opened again");
await page.evaluate(() => { window.__davApproved = true; });
await page.waitForTimeout(500);
ok(await form.count() === 0, "approved, the panel closes by itself");
ok(await side.getByRole("button", { name: "Nextcloud" }).isVisible(), "the account has its own heading");
ok(await side.getByText("Familie").isVisible() && await side.getByText("Team (shared)").isVisible(), "its calendars are listed, found without being asked for");
const headings = await side.locator(".cx-side-heading").allInnerTexts();
ok(headings.indexOf("Nextcloud") > headings.indexOf("iCloud"), `listed after iCloud: ${headings.join(", ")}`);
const inIcloud = await page.evaluate(() => (window.__cals ?? []).filter((c) => c.guid.startsWith("quince:dav:")).length);
ok(inIcloud === 2, "both calendars arrive");
const listed = await side.locator(".cx-cals").innerText();
ok(listed.indexOf("Team (shared)") < listed.indexOf("Other"), "a read-only shared calendar is under its account, not Other");

// Cancelling a sign-in leaves nothing.
await addMenu(/Add CalDAV Account/);
await preset.click();
await page.waitForTimeout(100);
await form.getByRole("textbox").fill("other.example.org");
await page.keyboard.press("Enter");
await page.waitForTimeout(200);
await form.getByRole("button", { name: "Cancel" }).click();
await page.waitForTimeout(200);
ok(await form.count() === 0 && (await writes()).some((w) => w.cancel), "Cancel stops waiting on the browser");

// A CalDAV server, by address and password.
await addMenu(/Add CalDAV Account/);
ok(await caldav.isVisible(), "the menu opens the CalDAV panel");
ok(await caldav.getByRole("button", { name: "Add" }).isDisabled(), "nothing to add until all three are given");
await caldav.getByRole("textbox", { name: "Server" }).fill("dav.example.net");
await caldav.getByRole("textbox", { name: "Username" }).fill("felix");
await caldav.getByLabel("Password").fill("hunter2");
await caldav.getByRole("button", { name: "Add" }).click();
await page.waitForTimeout(300);
ok(await caldav.count() === 0, "added, the panel closes");
ok(await side.getByRole("button", { name: "dav.example.net" }).isVisible(), "it has its own heading, by its server");
ok(!JSON.stringify(await writes()).includes("hunter2"), "(the fixture records no password)");

// A problem is shown on the heading, and Sign In Again is offered when the server refused.
await page.evaluate(() => {
  window.__davAccounts[0].problem = "The server no longer accepts this sign-in.";
  window.__davAccounts[0].needsSignIn = true;
});
await page.evaluate(() => window.__emit("quince://synced", { services: [], asOf: Math.floor(Date.now() / 1000), tookMs: 5, changed: ["calendar"] }));
await page.waitForTimeout(500);
const heading = side.getByRole("button", { name: /Nextcloud/ });
ok(await heading.locator(".cx-account-problem").count() === 1, "a refused sign-in is marked on the account");
await heading.click({ button: "right" });
await page.waitForTimeout(100);
ok(await page.getByRole("menuitem", { name: "Sign In Again" }).isVisible(), "Sign In Again is offered");
ok(await page.getByRole("menuitem", { name: "Refresh Now" }).count() === 0, "and Refresh Now is not, while the password is refused");
await page.getByRole("menuitem", { name: "Sign In Again" }).click();
await page.waitForTimeout(150);
ok(await form.getByRole("textbox").inputValue() === "https://cloud.example.org", "the panel opens on that server");
await form.getByRole("button", { name: "Cancel" }).click();
await page.waitForTimeout(100);
ok(await form.count() === 0 && await caldav.count() === 0, "signing in again, Cancel just closes it");

// An event in such a calendar stays in it; a new one can be made in it.
const calendarsOffered = await page.evaluate(() => (window.__cals ?? []).filter((c) => c.writable).map((c) => c.title));
ok(calendarsOffered.includes("Familie"), "a writable account calendar is offered for new events");

// Let go.
await heading.click({ button: "right" });
await page.getByRole("menuitem", { name: "Remove Account" }).click();
await page.waitForTimeout(150);
const sure = page.getByRole("alertdialog");
ok(/Nothing is deleted on the server/.test(await sure.innerText()), "the question says what removing does and doesn't do");
await sure.getByRole("button", { name: "Remove" }).click();
await page.waitForTimeout(300);
ok((await writes()).some((w) => w.remove === "n1"), "removed");
ok(await side.getByRole("button", { name: /Nextcloud/ }).count() === 0 && await side.getByText("Familie").count() === 0, "its heading and calendars are gone");

await b.close();
if (failures) { console.error(`${failures} failed`); process.exit(1); }
console.log("calendar accounts: all passed");
