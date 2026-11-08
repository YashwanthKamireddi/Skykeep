// Not a check: measures what the design language should hold the same across apps —
// pane titles, column tops, source-list headings, row heights, control heights.
// node .uiverify/design-measure.mjs [scheme]   (BASE= the preview server)
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen, openSettings } from "./stub.mjs";
const [scheme = "light"] = process.argv.slice(2);
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1400, height: 850 }, colorScheme: scheme });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null,
  calendarFixture: true, notesFixture: true, contactsFixture: true, driveFixture: true, remindersFixture: true });
const p = await ctx.newPage();
await p.goto(process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
const out = {};
for (const s of ["Reminders", "Notes", "Drive", "Calendar", "Contacts", "Photos", "Settings"]) {
  if (s === "Settings") await openSettings(p);
  else await openScreen(p, s);
  await p.waitForTimeout(400);
  out[s] = await p.evaluate(() => {
    const main = document.querySelector("main.content:not([hidden])") ?? document.querySelector("main.content");
    const slot = [...main.querySelectorAll(".screen-slot")].find((e) => !e.hidden) ?? main;
    const r = (el) => { if (!el) return null; const b = el.getBoundingClientRect(); const cs = getComputedStyle(el);
      return { cls: el.className?.baseVal ?? el.className, x: Math.round(b.left), y: Math.round(b.top), w: Math.round(b.width), h: Math.round(b.height), fs: cs.fontSize, fw: cs.fontWeight, br: cs.borderRadius }; };
    const vis = (el) => el.getClientRects().length && getComputedStyle(el).visibility !== "hidden";
    const all = (sel) => [...slot.querySelectorAll(sel)].filter(vis);
    const h1 = all("h1")[0];
    const seps = [...slot.querySelectorAll("*")].filter((e) => vis(e) && parseFloat(getComputedStyle(e).borderRightWidth) > 0 && e.getBoundingClientRect().height > 300).map(r);
    const heads = all(".side-heading").slice(0, 1).map(r);
    const rowSel = [".source-row", ".note-row", ".contact-row", ".drive-row", ".rx-row", ".settings-row", ".px-source"];
    const rows = Object.fromEntries(rowSel.map((s) => [s, all(s).slice(0, 2).map((e) => Math.round(e.getBoundingClientRect().height))]).filter(([, v]) => v.length));
    const ctl = {};
    for (const s of [".icon-button", ".tool", ".toolbar-group", ".quiet", ".primary", ".search-field", ".picker-button", ".segment"]) {
      const hs = [...new Set(all(s).map((e) => Math.round(e.getBoundingClientRect().height) + "/" + getComputedStyle(e).borderRadius))];
      if (hs.length) ctl[s] = hs;
    }
    const firstCol = slot.firstElementChild?.firstElementChild;
    return { h1: r(h1), heads, seps, rows, ctl, firstCol: r(firstCol) };
  });
}
console.log(JSON.stringify(out, null, 1));
await b.close();
