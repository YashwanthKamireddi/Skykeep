// Settings applies in every window, not only the one it is open in: each app has a window of
// its own (Notes, Calendar, …), and one closed is kept hidden for ten minutes to come back.
// Appearance and First Day of Week chosen in the library window reach a Calendar window that is
// already open, at once.
//
//   URL=http://localhost:4203 ENGINE=webkit node .uiverify/settings-windows.mjs
import { engine, engineName } from "./engine.mjs";
import { SERVICES, installStub, openScreen, openPane } from "./stub.mjs";
const URL = process.env.URL ?? "http://localhost:4173";
const b = await engine.launch();
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${engineName} ${m}`); if (!p) failures++; };

const ctx = await b.newContext({ viewport: { width: 1280, height: 820 }, colorScheme: "light", locale: "en-US" });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, calendarFixture: true });
const library = await ctx.newPage();
await library.goto(URL, { waitUntil: "networkidle" });
const calendar = await ctx.newPage();
calendar.on("pageerror", (e) => ok(false, `page error: ${e.message}`));
await calendar.goto(`${URL}/?app=calendar`, { waitUntil: "networkidle" });
await calendar.waitForTimeout(300);
if ((await calendar.getByRole("button", { name: "Month", exact: true }).count()) === 0) await openScreen(calendar, "Calendar");
await calendar.getByRole("button", { name: "Month", exact: true }).click();
await calendar.waitForTimeout(200);
const head = () => calendar.evaluate(() => document.querySelector(".cx-weekdays span")?.textContent);
ok((await head()) === "Sun", `the other window's month starts on the locale's Sunday  [${await head()}]`);

await library.keyboard.press("Control+,");
await library.locator(".prefs-side").waitFor();
await openPane(library, "General");
await library.getByRole("button", { name: "Dark", exact: true }).click();
await calendar.waitForTimeout(300);
ok((await calendar.evaluate(() => document.documentElement.dataset.theme)) === "dark", "Dark chosen in Settings turns the other window dark too");

await library.getByRole("button", { name: /First Day of Week/ }).click();
await library.getByRole("option", { name: "Monday" }).click();
await calendar.waitForTimeout(300);
ok((await head()) === "Mon", `Monday chosen in Settings starts the other window's month on Monday  [${await head()}]`);

await library.getByRole("button", { name: "Automatic", exact: true }).click();
await library.getByRole("button", { name: /First Day of Week/ }).click();
await library.getByRole("option", { name: /^Automatic/ }).click();
await calendar.waitForTimeout(300);
ok((await calendar.evaluate(() => document.documentElement.dataset.theme)) === undefined, "and Automatic follows the system there again");
ok((await head()) === "Sun", `as does the locale's first day  [${await head()}]`);

await b.close();
console.log(failures ? `\n${failures} FAILED` : "\nall passed");
process.exit(failures ? 1 : 0);
