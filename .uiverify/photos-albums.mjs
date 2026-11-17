// Photos albums: a new one from the sidebar's +, named in place, first in the list; a photo
// added from its right-click menu and from the viewer; taken out again in the album, staying
// in the library; renamed and deleted from the album's own right-click menu. Each shows at
// once and reaches iCloud.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
// Every question agreed to, as soon as it is asked.
await page.addInitScript(() => addEventListener("DOMContentLoaded", () =>
  new MutationObserver(() => document.querySelector(".confirm .confirm-answer")?.click()).observe(document.body, { childList: true, subtree: true })));
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const albumNames = () => page.locator(".px-albums .px-source").evaluateAll((els) => els.map((e) => e.querySelector(".px-label")?.textContent ?? ""));

// The Albums page: the row opens it — every album a card with its cover, name and count —
// while the chevron only opens the list in the sidebar; a card opens its album.
await page.locator(".px-parent", { hasText: "Albums" }).click();
await page.waitForTimeout(200);
ok((await page.locator(".px-albums-head h1").innerText()) === "Albums" && (await page.locator(".px-albums-head .px-span").innerText()) === "1 Item",
  "Albums opens the Albums page");
ok((await page.locator(".px-card").count()) === 1 && /Trips[\s\S]*2 items/.test(await page.locator(".px-card").innerText()),
  `each album a card with its name and count  ${JSON.stringify(await page.locator(".px-card").innerText())}`);
// The grid stays built underneath (back is instant) but none of it shows.
ok((await page.locator(".px-tile").evaluateAll((els) => els.filter((e) => e.getClientRects().length > 0).length)) === 0, "and the photos give way to it");
await page.getByRole("button", { name: "Hide Albums" }).click();
ok((await page.locator(".px-albums .px-source").count()) === 0 && (await page.locator(".px-card").count()) === 1, "the chevron only folds the list away");
await page.getByRole("button", { name: "Show Albums" }).click();
await page.locator(".px-card").click();
await page.waitForTimeout(250);
ok((await page.locator(".px-bar h1").innerText()) === "Trips" && (await page.locator(".px-tile").count()) === 2, "a card opens its album");
await page.locator(".px-source", { hasText: "Library" }).first().click();
await page.waitForTimeout(250);

await page.locator(".px-side").getByRole("button", { name: "New Album" }).click();
await page.locator(".row-menu").getByText("New Album", { exact: true }).click();
await page.getByLabel("Album name").fill("Beach");
await page.keyboard.press("Enter");
await page.waitForTimeout(250);
ok(JSON.stringify(await albumNames()) === JSON.stringify(["Beach", "Trips"]), `a new album is named in place and sorts first  ${JSON.stringify(await albumNames())}`);
ok((await page.evaluate(() => window.__albumCalls.filter((c) => c.cmd === "photos_album_create").length)) === 1, "and is made once, not twice for Enter and the blur after it");

await page.locator(".px-tile").first().click({ button: "right" });
await page.locator(".row-menu").getByText("Add to Album").hover();
await page.waitForTimeout(200);
await page.locator(".row-menu").getByText("Beach", { exact: true }).click();
await page.waitForTimeout(250);
const add = await page.evaluate(() => window.__albumCalls.find((c) => c.cmd === "photos_album_add"));
ok(add && add.args.ids[0] === "p1", "right-click → Add to Album puts the photo in it");
ok(/1/.test(await page.locator(".px-source", { hasText: "Beach" }).innerText()), "and its count shows at once");

await page.locator(".px-tile").nth(1).dblclick();
await page.getByRole("button", { name: "Add to Album" }).click();
await page.locator(".row-menu").getByText("Beach", { exact: true }).click();
await page.waitForTimeout(250);
ok((await page.evaluate(() => window.__albumCalls.filter((c) => c.cmd === "photos_album_add").length)) === 2, "the viewer's Add to Album does the same");
await page.keyboard.press("Escape");

await page.locator(".px-source", { hasText: "Beach" }).click();
await page.waitForTimeout(250);
ok((await page.locator(".px-tile").count()) === 2, "the album holds both");
await page.locator(".px-tile").first().click({ button: "right" });
ok((await page.locator(".row-menu").getByText("Delete", { exact: true }).count()) === 0, "in an album the menu offers Remove, not Delete");
await page.locator(".row-menu").getByText("Remove from Album").click();
await page.waitForTimeout(250);
ok((await page.locator(".px-tile").count()) === 1, "Remove from Album takes it out at once");
ok((await page.evaluate(() => window.__photos.filter((p) => !p.deleted).length)) === 5, "and the photo stays in the library");

await page.locator(".px-source", { hasText: "Beach" }).click({ button: "right" });
await page.locator(".row-menu").getByText("Rename Album").click();
await page.getByLabel("Album name").fill("Sea");
await page.keyboard.press("Enter");
await page.waitForTimeout(250);
ok((await page.locator(".px-bar h1").innerText()) === "Sea" && (await albumNames()).includes("Sea"), "Rename Album renames it in place and in the title");

await page.locator(".px-source", { hasText: "Sea" }).click({ button: "right" });
await page.locator(".row-menu").getByText("Delete Album").click();
await page.waitForTimeout(250);
ok(!(await albumNames()).includes("Sea") && (await page.locator(".px-bar h1").innerText()) === "Library", "Delete Album, once agreed, removes it and goes back to the library");

// With an album open, what is added goes into it too, and a drop says so.
await page.locator(".px-albums .px-source", { hasText: "Trips" }).click();
await page.waitForTimeout(200);
await page.evaluate(() => { window.__pick = ["/home/test/Pictures/ferry.jpg"]; });
await page.getByRole("button", { name: "Add Photos" }).click();
await page.waitForTimeout(300);
ok((await page.evaluate(() => window.__photoUploadAlbums?.at(-1))) === "AL1", "Add Photos with an album open puts them in it too");
ok((await page.locator(".px").getAttribute("data-drop-label")) === "Add to Trips", "and a drop there reads Add to Trips");
await page.locator(".px-source", { hasText: "Library" }).click();
await page.waitForTimeout(200);
ok((await page.locator(".px").getAttribute("data-drop-label")) === "Add to Library", "in the library, Add to Library");
await page.evaluate(() => window.__emit?.("quince://photo-upload", { id: "B1:0", batch: "B1", name: "ferry.jpg", within: "", folder: false, error: null, state: "cancelled", sent: 0, total: 1 }));
await page.evaluate(() => { window.__albumFail = true; });
await page.locator(".px-side").getByRole("button", { name: "New Album" }).click();
await page.locator(".row-menu").getByText("New Album", { exact: true }).click();
await page.getByLabel("Album name").fill("Nope");
await page.keyboard.press("Enter");
await page.waitForTimeout(300);
ok(!(await albumNames()).includes("Nope") && (await page.locator(".toast.is-error").count()) === 1, "an album iCloud refuses goes away again, and says why");

// Folders, as icloud.com has them: New Folder, named in place; an album made in it sits in it;
// the folder's page shows what it holds; deleting it takes what it holds with it.
await page.evaluate(() => { window.__albumFail = false; });
await page.locator(".px-side").getByRole("button", { name: "New Album" }).click();
await page.locator(".row-menu").getByText("New Folder", { exact: true }).click();
await page.getByLabel("Album name").fill("Holidays");
await page.keyboard.press("Enter");
await page.waitForTimeout(300);
const made = await page.evaluate(() => window.__albumCalls.filter((c) => c.cmd === "photos_album_create").at(-1).args);
ok(made.name === "Holidays" && made.folder === true, `New Folder makes a folder  ${JSON.stringify(made)}`);
const folderRow = page.locator(".px-parent", { hasText: "Holidays" });
ok((await folderRow.count()) === 1, "and it stands in the sidebar as a folder");
await folderRow.click({ button: "right" });
await page.locator(".row-menu").getByText("New Album", { exact: true }).click();
await page.getByLabel("Album name").fill("Goa");
await page.keyboard.press("Enter");
await page.waitForTimeout(300);
const inner = await page.evaluate(() => window.__albumCalls.filter((c) => c.cmd === "photos_album_create").at(-1).args);
ok(inner.name === "Goa" && typeof inner.parent === "string" && inner.parent.startsWith("AL"), `an album made in it is made in it  ${JSON.stringify(inner)}`);
await folderRow.click();
await page.waitForTimeout(250);
ok((await page.locator(".px-albums-head h1").innerText()) === "Holidays" && /Goa/.test(await page.locator(".px-cards").innerText()),
  "the folder's page shows what it holds");
await folderRow.click({ button: "right" });
await page.locator(".row-menu").getByText("Delete Folder").click();
await page.waitForTimeout(300);
ok((await page.locator(".px-parent", { hasText: "Holidays" }).count()) === 0 && (await page.locator(".px-source", { hasText: "Goa" }).count()) === 0,
  "deleting the folder takes what it holds with it");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
