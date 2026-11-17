// Photos: the library grouped by month, newest first; views from the sidebar; a photo opens on
// a dark stage, the arrows go through, Escape closes; the original downloads.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";
const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 800 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
const page = await ctx.newPage();
page.on("pageerror", (e) => { console.log(`FAIL page error: ${e.message}`); process.exitCode = 1; });
await page.goto(process.env.URL ?? process.env.BASE ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Photos");
await page.waitForTimeout(400);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

const months = await page.locator(".px-month-title").allInnerTexts();
ok(months.length === 2 && /2026/.test(months[0]), `the library is grouped by month, newest first  ${JSON.stringify(months)}`);
ok((await page.locator(".px-tile").count()) === 5, "every photo in the library is a tile; a deleted one is not");
ok((await page.locator(".px-duration").innerText()).includes("0:12"), "a video shows its length");
ok((await page.locator(".px-tile .px-heart").count()) === 1, "a favorite shows its heart");

await page.locator(".px-source", { hasText: "Favorites" }).click();
await page.waitForTimeout(200);
ok((await page.locator(".px-tile").count()) === 1, "Favorites holds the favorite");
await page.locator(".px-source", { hasText: "Recently Deleted" }).click();
await page.waitForTimeout(200);
ok((await page.locator(".px-tile").count()) === 1, "Recently Deleted holds the deleted one");
// An album: in the sidebar with its count, its photos in one grid with no month headings.
await page.locator(".px-source", { hasText: "Trips" }).click();
await page.waitForTimeout(200);
ok((await page.locator(".px-tile").count()) === 2 && (await page.locator(".px-month-title").count()) === 0 &&
  (await page.locator(".px-bar h1").innerText()) === "Trips", "an album opens to its photos, one grid, titled with its name");
await page.locator(".px-source", { hasText: "Library" }).click();
await page.waitForTimeout(200);

await page.locator(".px-tile").first().dblclick();
await page.waitForTimeout(150);
ok((await page.locator(".px-viewer").count()) === 1, "a photo opens in the viewer");
const first = await page.locator(".px-stage .px-shown").getAttribute("data-picture");
await page.keyboard.press("ArrowRight");
await page.waitForTimeout(100);
const second = await page.locator(".px-stage .px-shown").getAttribute("data-picture");
ok(first && second && first !== second && /fit\d+/.test(second), "the right arrow goes to the next photo, made the size the screen shows it");
await page.getByRole("button", { name: "Download" }).click();
await page.waitForTimeout(200);
ok((await page.evaluate(() => window.__downloads?.at(-1)?.id)) === "p2", "Download saves that photo's original");
// Favorite, from the heart and from ".", at once; then Delete, after asking, moves on and the
// photo is in Recently Deleted, from where Recover brings it back.
await page.getByRole("button", { name: "Favorite", exact: true }).click();
ok((await page.getByRole("button", { name: "Unfavorite" }).count()) === 1, "the heart fills at once");
await page.keyboard.press(".");
await page.waitForTimeout(200);
ok((await page.getByRole("button", { name: "Favorite", exact: true }).count()) === 1, '"." takes it back');
const flags = await page.evaluate(() => window.__flags.map((f) => `${f.id}:${f.flag}:${f.on}`));
ok(JSON.stringify(flags) === JSON.stringify(["p2:favorite:true", "p2:favorite:false"]), `and each goes to iCloud  ${JSON.stringify(flags)}`);
await page.getByRole("button", { name: "Delete", exact: true }).click();
await page.locator(".confirm .confirm-answer").click();
await page.waitForTimeout(300);
const next = await page.locator(".px-stage .px-shown").first().evaluate((i) => i.getAttribute("data-picture") ?? i.getAttribute("src"));
ok(/p3|p4/.test(next ?? "") && (await page.locator(".px-tile").count()) === 4, "Delete, once agreed, takes it out and shows the next");
// A video shows its still; Play opens it in the computer's own player.
if (await page.getByRole("button", { name: "Play in your video player" }).count()) {
  await page.getByRole("button", { name: "Play in your video player" }).click();
  await page.waitForTimeout(150);
  ok((await page.evaluate(() => window.__plays?.at(-1)?.id)) === "p3", "a video plays in the computer's own player");
}
await page.keyboard.press("Escape");
await page.locator(".px-source", { hasText: "Recently Deleted" }).click();
await page.waitForTimeout(200);
ok((await page.locator(".px-tile").count()) === 2, "the deleted photo is in Recently Deleted");
await page.locator(".px-tile").first().dblclick();
await page.locator(".px-viewer").getByRole("button", { name: "Recover" }).click();
await page.waitForTimeout(300);
ok((await page.evaluate(() => window.__flags.at(-1))).on === false, "Recover brings it back");
await page.keyboard.press("Escape");
await page.locator(".px-source", { hasText: "Library" }).click();
await page.waitForTimeout(200);
await page.evaluate(() => { window.__flagFail = true; });
await page.locator(".px-tile").nth(1).dblclick();
await page.getByRole("button", { name: "Favorite", exact: true }).click();
await page.waitForTimeout(300);
ok((await page.getByRole("button", { name: "Favorite", exact: true }).count()) === 1 && (await page.locator(".toast.is-error").count()) === 1,
  "a change iCloud refuses goes back, and says why");
ok(await page.locator(".toast.is-error").evaluate((n) => {
  const r = n.getBoundingClientRect();
  return n.contains(document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2));
}), "and says it in front of the open photo, not hidden behind it");
await page.evaluate(() => { window.__flagFail = false; });
await page.locator(".toast.is-error").getByRole("button", { name: "Dismiss" }).click();
await page.keyboard.press("Escape");
// It goes back into its tile first, then is gone.
await page.waitForTimeout(650);
ok((await page.locator(".px-viewer").count()) === 0, "Escape closes the viewer");

// Opening is at once — no growing out of the tile, which flickered on his screen: the photo is
// its whole size in the very frame it first appears (caught in the page, not after a round trip).
await page.evaluate(() => {
  window.__firstWidth = null;
  const watch = new MutationObserver(() => {
    const i = document.querySelector(".px-stage .px-shown");
    if (!i) return;
    watch.disconnect();
    requestAnimationFrame(() => { window.__firstWidth = i.getBoundingClientRect().width; });
  });
  watch.observe(document.body, { childList: true, subtree: true });
});
await page.locator(".px-tile").first().dblclick();
await page.waitForFunction(() => window.__firstWidth !== null);
const mid = await page.evaluate(() => window.__firstWidth);
await page.waitForTimeout(600);
const end = await page.locator(".px-shown").evaluate((i) => i.getBoundingClientRect().width);
ok(Math.abs(mid - end) <= 1 && end > 200, `a photo opens at once, its whole size from its first frame  ${Math.round(mid)} → ${Math.round(end)}px`);
await page.keyboard.press("Escape");
await page.waitForTimeout(650);
// Another photo opened while the last is still closing stays open, and is seen.
await page.locator(".px-tile").nth(1).dblclick();
await page.waitForTimeout(500);
await page.keyboard.press("Escape");
await page.waitForTimeout(60);
await page.locator(".px-tile").nth(2).dblclick();
await page.waitForTimeout(800);
ok((await page.locator(".px-viewer").count()) === 1 &&
  (await page.locator(".px-backdrop").evaluate((b) => getComputedStyle(b).opacity)) === "1",
  "a photo opened while another closes stays open, fully shown");
await page.keyboard.press("Escape");
await page.waitForTimeout(650);

// Selecting, as on a Mac: a click selects, Ctrl-click adds, Shift-click takes the run
// between, Ctrl+A everything; the title says how many; Escape lets go; arrows move.
await page.locator(".px-tile").nth(0).click();
ok((await page.locator('.px-tile[aria-selected="true"]').count()) === 1 && (await page.locator(".px-viewer").count()) === 0,
  "a click selects a photo and does not open it");
await page.locator(".px-tile").nth(2).click({ modifiers: ["Control"] });
ok((await page.locator('.px-tile[aria-selected="true"]').count()) === 2, "Ctrl-click adds another");
ok(/2 Items Selected/.test(await page.locator(".px-span").innerText()), `the title says what is selected  ${await page.locator(".px-span").innerText()}`);
await page.locator(".px-tile").nth(3).click({ modifiers: ["Shift"] });
ok((await page.locator('.px-tile[aria-selected="true"]').count()) === 2, "Shift-click takes the run from the last chosen");
await page.keyboard.press("Control+a");
ok((await page.locator('.px-tile[aria-selected="true"]').count()) === (await page.locator(".px-tile").count()), "Ctrl+A selects everything");
await page.keyboard.press("Escape");
ok((await page.locator('.px-tile[aria-selected="true"]').count()) === 0, "Escape lets the selection go");
await page.locator(".px-tile").nth(0).click();
await page.keyboard.press("ArrowRight");
ok((await page.locator(".px-tile").nth(1).getAttribute("aria-selected")) === "true", "the right arrow moves the selection on");
await page.keyboard.press("ArrowDown");
const downAt = await page.locator(".px-tile").evaluateAll((t) => t.findIndex((x) => x.getAttribute("aria-selected") === "true"));
ok(downAt === 4, `the down arrow goes to the same column of the next month  → tile ${downAt}`);
await page.keyboard.press("ArrowUp");
const upAt = await page.locator(".px-tile").evaluateAll((t) => t.findIndex((x) => x.getAttribute("aria-selected") === "true"));
ok(upAt === 1, `and the up arrow comes back to it  → tile ${upAt}`);
await page.keyboard.press("ArrowDown");
await page.keyboard.press("Enter");
await page.waitForTimeout(500);
ok((await page.locator(".px-viewer").count()) === 1, "Return opens the selected photo");
await page.keyboard.press("Escape");
await page.waitForTimeout(650);
// Two favorited together with ".", each sent to iCloud.
await page.locator(".px-tile").nth(1).click();
await page.locator(".px-tile").nth(3).click({ modifiers: ["Control"] });
const sentBefore = await page.evaluate(() => window.__flags.length);
await page.keyboard.press(".");
await page.waitForTimeout(300);
ok((await page.evaluate(() => window.__flags.length)) - sentBefore === 2 && (await page.locator(".px-tile .px-heart").count()) >= 2,
  '"." favorites every selected photo at once');
await page.locator(".px-scroll").click({ position: { x: 600, y: 600 } });
ok((await page.locator('.px-tile[aria-selected="true"]').count()) === 0, "a click on the empty space lets the selection go");

// Hide, as Photos hides: asked, out of the library, into Hidden (under Utilities), and back.
await page.locator(".px-tile").nth(1).click({ button: "right" });
await page.locator(".row-menu").getByText("Hide Photo").click();
await page.locator(".confirm .confirm-answer").click();
await page.waitForTimeout(300);
ok((await page.locator(".px-tile").count()) === 4, "Hide Photo takes it out of the library at once");
await page.locator(".px-source", { hasText: "Hidden" }).click();
// Locked, as icloud.com keeps Hidden (photos-hidden-lock.mjs): unlocked with the password first.
await page.getByRole("button", { name: "View Hidden Photos" }).click();
await page.getByRole("dialog", { name: "View Hidden Photos" }).getByLabel("Password").fill("right");
await page.keyboard.press("Enter");
await page.waitForTimeout(300);
ok((await page.locator(".px-tile").count()) === 1, "and it is in Hidden");
await page.locator(".px-tile").first().click();
await page.keyboard.press("Control+l");
await page.waitForTimeout(300);
ok((await page.locator(".px-tile").count()) === 0 && (await page.evaluate(() => window.__flags.at(-1))).flag === "hidden",
  "Ctrl+L in Hidden unhides it");
await page.locator(".px-source", { hasText: "Library" }).click();
await page.waitForTimeout(250);
ok((await page.locator(".px-tile").count()) === 5, "and it is back in the library");

// Zooming into a photo: double-click zooms to where was clicked, with the full picture
// laid over; Escape zooms back out before it closes; Ctrl+wheel zooms; the grid's own zoom
// keys are not taken while a photo is open; the arrows step at once.
await page.locator(".px-tile").nth(1).dblclick();
await page.waitForTimeout(600);
const scaleOf = () => page.locator(".px-pan").evaluate((el) => new DOMMatrix(getComputedStyle(el).transform).a);
await page.locator(".px-pan").dblclick({ position: { x: 40, y: 40 } });
await page.waitForTimeout(400); // the zoom glides
ok((await scaleOf()) > 2, `double-click zooms in  ×${(await scaleOf()).toFixed(2)}`);
ok(await page.locator(".px-pan").evaluate((p) => {
  const cs = getComputedStyle(p);
  return cs.willChange === "auto" && cs.transitionDuration.split(",").every((d) => parseFloat(d) === 0) && !/matrix3d|translate3d/.test(p.style.transform) && p.getAnimations().length === 0;
}), "zooming is drawn by the page: no transition, no will-change, no 3D layer, no compositor animation (each flickered)");
ok(/full/.test((await page.locator(".px-sharp").getAttribute("src")) ?? ""), "and the full picture is laid over");
await page.keyboard.press("Escape");
// The zoom glides back: waited for until it is home (on a loaded machine the glide takes
// longer than its usual 400 ms, and a fixed wait looked mid-glide).
await page.waitForFunction(() => new DOMMatrix(getComputedStyle(document.querySelector(".px-pan")).transform).a === 1, null, { timeout: 3000 }).catch(() => {});
ok((await scaleOf()) === 1 && (await page.locator(".px-viewer").count()) === 1, "Escape zooms back out first, and keeps the photo open");
const box = await page.locator(".px-pan").boundingBox();
await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
await page.keyboard.down("Control");
await page.mouse.wheel(0, -120);
await page.keyboard.up("Control");
await page.waitForTimeout(150);
ok((await scaleOf()) > 1.5, `Ctrl + scroll zooms about the pointer  ×${(await scaleOf()).toFixed(2)}`);
const gridAcross = await page.evaluate(() => getComputedStyle(document.querySelector(".px-scroll")).getPropertyValue("--across"));
await page.keyboard.press("Control+0");
await page.keyboard.press("Control+=");
await page.waitForTimeout(400); // the zoom glides
ok((await scaleOf()) > 1.2 && (await page.evaluate(() => getComputedStyle(document.querySelector(".px-scroll")).getPropertyValue("--across"))) === gridAcross,
  "Ctrl = zooms the photo, not the grid behind it");
await page.keyboard.press("Control+0");
await page.keyboard.press("ArrowLeft");
await page.waitForTimeout(60);
ok(/p1/.test((await page.locator(".px-shown").getAttribute("data-picture")) ?? ""), "the left arrow goes to the photo before, at once");
await page.keyboard.press("ArrowRight");
await page.keyboard.press("ArrowRight");
await page.waitForTimeout(60);
ok((await page.locator(".px-video").count()) === 1, "and on to a video");
await page.waitForTimeout(400);
await page.keyboard.press("Escape");
await page.waitForTimeout(650);

// Recently Deleted, as Photos keeps it: each photo with the time it has left (red in its last
// days); Recover and Delete for what is selected, Delete All otherwise; Delete is for good
// and asked first, in Photos' words.
await page.locator(".px-source", { hasText: "Recently Deleted" }).click();
await page.waitForTimeout(250);
const left = page.locator(".px-tile .px-left");
ok((await left.count()) === 1 && (await left.innerText()) === "2 days" && /is-soon/.test(await left.getAttribute("class")),
  `a deleted photo shows its days left, red near the end  "${await left.innerText()}"`);
ok((await page.getByRole("button", { name: "Delete All" }).count()) === 1, "with nothing selected, the toolbar offers Delete All");
await page.locator(".px-tile").first().click();
ok((await page.locator(".px-word").allInnerTexts()).join() === "Recover,Delete", "with one selected, Recover and Delete");
await page.locator(".px-word", { hasText: /^Delete$/ }).click();
const asked = await page.locator(".confirm").innerText();
ok((await page.locator(".confirm .confirm-symbol").count()) === 1 && (await page.locator(".confirm button").first().evaluate((b) => b === document.activeElement)),
  "asked in a sheet, as icloud.com asks: the trash symbol, Cancel holding the focus");
await page.keyboard.press("Escape");
ok((await page.locator(".confirm").count()) === 0 && (await page.locator(".px-tile").count()) === 1, "Escape cancels: nothing deleted");
await page.locator(".px-word", { hasText: /^Delete$/ }).click();
await page.locator(".confirm .confirm-answer").click();
await page.waitForTimeout(300);
ok(/permanently/.test(asked) && /cannot be undone/.test(asked), `Delete asks first, in Photos' words  "${asked.split("\n")[0]}"`);
ok((await page.locator(".px-tile").count()) === 0 && (await page.evaluate(() => window.__flags.at(-1).flag)) === "expunged",
  "and deletes it for good");
await page.locator(".px-source", { hasText: "Library" }).click();
await page.waitForTimeout(250);

const across = async () => page.locator(".px-grid").first().evaluate((g) => getComputedStyle(g).gridTemplateColumns.split(" ").length);
const before = await across();
await page.getByRole("button", { name: "Zoom In" }).click();
await page.getByRole("button", { name: "Zoom In" }).click();
await page.waitForTimeout(100);
ok((await across()) < before, `Zoom In shows fewer across  ${before} → ${await across()}`);
ok(await page.getByRole("button", { name: "Zoom In" }).isDisabled(), "and stops at the closest");
await page.keyboard.press("Control+-");
await page.waitForTimeout(100);
ok((await page.getByRole("button", { name: "Zoom In" }).isEnabled()), "Ctrl − zooms back out, as in Photos");
ok(/Photos?, 1 Video/.test(await page.locator(".px-tally").innerText()), `the foot of the library says what it holds  ${await page.locator(".px-tally").innerText()}`);

// Adding, as icloud.com shows it: a panel at the right — "Uploading 3 Items to iCloud", each
// file with how much has gone, Pending with a way to stop it; a file Photos can't take says why
// at once; Stop Upload stops the rest; at the end what did not go, and why, until Done.
const say = (e) => page.evaluate((x) => window.__emit("quince://photo-upload", { batch: "B1", within: "", folder: false, error: null, sent: 0, total: 1000, ...x }), e);
await page.evaluate(() => { window.__pick = ["/home/test/Pictures/beach.jpg", "/home/test/Pictures/notes.txt", "/home/test/Pictures/dog.heic"]; });
await page.getByRole("button", { name: "Add Photos" }).click();
await page.waitForTimeout(400);
ok(JSON.stringify(await page.evaluate(() => window.__photoUploads?.at(-1))) === JSON.stringify(["/home/test/Pictures/beach.jpg", "/home/test/Pictures/notes.txt", "/home/test/Pictures/dog.heic"]),
  "Add Photos sends the chosen files to the library");
const panel = page.locator(".px-up");
ok((await panel.count()) === 1 && (await page.locator(".px-up-title").innerText()) === "Uploading 3 Items to iCloud",
  `the panel slides in: ${await page.locator(".px-up-title").innerText()}`);
await say({ id: "B1:0", name: "beach.jpg", state: "sending", sent: 500 });
await page.waitForTimeout(100);
ok(/500 B of 1000 B/.test(await panel.locator(".px-up-row", { hasText: "beach.jpg" }).innerText()), "a file on its way shows how much has gone");
ok(/Pending/.test(await panel.locator(".px-up-row", { hasText: "dog.heic" }).innerText()), "one waiting reads Pending");
await panel.getByRole("button", { name: "Stop dog.heic" }).click();
await page.waitForTimeout(100);
ok(JSON.stringify(await page.evaluate(() => window.__photoCancels?.at(-1))) === JSON.stringify({ ids: ["B1:2"], batch: null }), "its own ✕ stops just that one");
await say({ id: "B1:0", name: "beach.jpg", state: "verifying", sent: 1000 });
await page.waitForTimeout(100);
ok((await page.locator(".px-up-title").innerText()) === "Finalizing Uploads", "while Apple makes the photo: Finalizing Uploads");
await say({ id: "B1:0", name: "beach.jpg", state: "done", sent: 1000 });
await page.waitForTimeout(150);
ok((await page.locator(".px-up-title").innerText()) === "2 of 3 Items Not Uploaded" || (await page.locator(".px-up-title").innerText()) === "1 of 3 Items Not Uploaded",
  `at the end, what did not go: ${await page.locator(".px-up-title").innerText()}`);
ok(/can’t add this kind/.test(await panel.innerText()), "and why");
await panel.getByRole("button", { name: "Done" }).click();
await page.waitForTimeout(60);
ok((await page.locator(".px-up").count()) === 0, "Done puts it away at once");
// Stop Upload, for a batch still going.
await page.evaluate(() => { window.__pick = ["/home/test/Pictures/ferry.jpg"]; });
await page.getByRole("button", { name: "Add Photos" }).click();
await page.waitForTimeout(300);
await page.getByRole("button", { name: "Stop Upload" }).click();
await page.waitForTimeout(150);
ok((await page.evaluate(() => window.__photoCancels?.at(-1)))?.batch === "B2", "Stop Upload stops the whole batch");
await b.close();
process.exitCode = failures ? 1 : (process.exitCode ?? 0);
