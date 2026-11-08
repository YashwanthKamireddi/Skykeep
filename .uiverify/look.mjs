import { openScreen } from "./stub.mjs";
// Screenshots of Reminders drawn from real account data, light and dark, for looking at.
//
//   cargo run -p quince-probe --example ui_fixture -- <end-of-today> > fixture.json
//   node .uiverify/look.mjs fixture.json <out-dir>
//
// The fixture holds someone's reminders, so it is written outside the repository and never
// committed; this only reads it.
import { chromium } from "playwright";
import fs from "node:fs";

const [fixturePath, out] = process.argv.slice(2);
const fx = JSON.parse(fs.readFileSync(fixturePath, "utf8"));
const b = await chromium.launch();
for (const scheme of ["dark", "light"]) {
  const ctx = await b.newContext({ viewport: { width: 1280, height: 820 }, colorScheme: scheme, deviceScaleFactor: 2 });
  await ctx.addInitScript((fx) => {
    window.__TAURI_INTERNALS__ = {
      transformCallback: (cb) => cb,
      invoke: async (cmd, args) => {
        if (cmd === "auth_status") return { hasStoredSession: true, signedIn: true };
        if (cmd === "auth_resume") return "library";
        if (cmd === "account_services") return ["reminders", "notes", "calendar", "contacts", "drivews"];
        if (cmd === "reminder_lists") return fx.lists;
        if (cmd === "reminder_groups") return fx.groups;
        if (cmd === "reminder_counts") return fx.counts;
        if (cmd === "reminders_smart") return fx.smart[args.kind] ?? [];
        if (cmd === "reminders_in") return fx.byList[args.listGuid] ?? [];
        if (cmd === "reminder_sections") return fx.sections[args.listGuid] ?? [];
        if (cmd === "sync_all" || cmd === "sync_status") return { services: [], asOf: 1, tookMs: 0 };
        if (cmd === "plugin:event|listen") return 0;
        return [];
      },
    };
  }, fx);
  const p = await ctx.newPage();
  await p.goto("http://localhost:4173", { waitUntil: "networkidle" });
  await openScreen(p, "Reminders");
  await p.waitForTimeout(500);
  await p.screenshot({ path: `${out}/reminders-${scheme}.png` });
  await ctx.close();
}
await b.close();
