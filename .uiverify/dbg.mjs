import { chromium } from "playwright";
const b = await chromium.launch();
const ctx = await b.newContext({ viewport:{width:1040,height:720} });
await ctx.addInitScript(() => {
  window.__TAURI_INTERNALS__ = {
    transformCallback: (cb) => cb,
    invoke: async (cmd) => {
      (window.__calls ||= []).push(cmd);
      switch (cmd) {
        case "auth_status": return { hasStoredSession:false, signedIn:false };
        case "auth_sign_in": return "needs_two_factor";
        case "account_services": return ["calendar"];
        // One refresh covers every service, so there is one command to stub rather than
        // a pair per screen.
        case "sync_all":
        case "sync_status":
          return {
            services: [
              { service: "calendar", label: "Calendar", outcome: "ok", detail: null, lastSyncedAt: Math.floor(Date.now() / 1000) - 120 },
              { service: "reminders", label: "Reminders", outcome: "ok", detail: null, lastSyncedAt: Math.floor(Date.now() / 1000) - 60 },
              { service: "notes", label: "Notes", outcome: "ok", detail: null, lastSyncedAt: Math.floor(Date.now() / 1000) - 90 },
              { service: "contacts", label: "Contacts", outcome: "ok", detail: null, lastSyncedAt: Math.floor(Date.now() / 1000) - 300 },
            ],
            asOf: Math.floor(Date.now() / 1000) - 300,
            tookMs: 40,
          };
        // The event plugin is how a finished refresh reaches the screens. Nothing emits in
        // a plain browser, so listening is a no-op that hands back an unsubscribe id.
        case "plugin:event|listen":
          return 0;
        case "plugin:event|unlisten":
          return null;
        default: return null;
      }
    },
  };
});
const p = await ctx.newPage();
p.on("console", m => console.log("[console]", m.type(), m.text()));
p.on("pageerror", e => console.log("[pageerror]", String(e)));
await p.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil:"networkidle" });
await p.waitForTimeout(400);
console.log("stub reached?", await p.evaluate(() => window.__calls ?? "STUB NEVER CALLED"));
console.log("visible root:", (await p.innerText("#root")).slice(0,120).replace(/\n/g," "));
const btn = await p.$('button[type="submit"]');
console.log("submit button present:", !!btn, "disabled:", btn && await btn.isDisabled());
await p.fill('input[aria-label="Apple Account"]', "you@icloud.com");
await p.fill('input[aria-label="Password"]', "secret123");
console.log("after fill, disabled:", await (await p.$('button[type="submit"]')).isDisabled());
await p.click('button[type="submit"]');
await p.waitForTimeout(800);
console.log("calls:", await p.evaluate(() => window.__calls));
console.log("root now:", (await p.innerText("#root")).slice(0,140).replace(/\n/g," "));
await b.close();
