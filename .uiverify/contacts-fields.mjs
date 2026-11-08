// Every field Apple's Contacts keeps, shown and written as the iPhone stores it: the whole
// name (prefix, middle, suffix, nickname, phonetic), work, dates, related people, social
// profiles, instant messages, labels in Apple's own words — and the photo, chosen and cropped.
import { engine } from "./engine.mjs";
import { SERVICES, installStub, openScreen } from "./stub.mjs";

const b = await engine.launch();
const ctx = await b.newContext({ viewport: { width: 1280, height: 860 } });
await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null, contactsFixture: true });
await ctx.addInitScript(() => {
  const internals = window.__TAURI_INTERNALS__;
  const invoke = internals.invoke;
  internals.invoke = async (cmd, args) => {
    const answer = await invoke(cmd, args);
    if (cmd === "contacts_list" && Array.isArray(answer))
      return answer.map((c) => c.guid !== "c1" ? c : {
        ...c, photoUrl: "https://p.example/c1", photoSignature: "s1", photoCrop: "0,0,100,100",
        prefix: "Dr.", suffix: "PhD", nickname: "Zo", phoneticFirstName: "ZOY-ah", jobTitle: "Architect", department: "Design",
        methods: [
          { kind: "phone", label: "_$!<Mobile>!$_", value: "+91 90000 00001" },
          { kind: "email", label: "HOME", value: "zoya@example.com" },
          { kind: "date", label: "ANNIVERSARY", value: "2015-02-14" },
          { kind: "related", label: "MOTHER", value: "Amma" },
          { kind: "profile", label: "TWITTER", value: "zoya" },
          { kind: "im", label: "SKYPE", value: "zoya.a" },
        ],
      });
    return answer;
  };
});
const page = await ctx.newPage();
await page.goto(process.env.URL ?? "http://localhost:4173", { waitUntil: "networkidle" });
await openScreen(page, "Contacts");
await page.waitForTimeout(400);
await page.locator(".contact-row", { hasText: "Zoya" }).click();
await page.waitForTimeout(300);
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };

// Each label carries a hidden "copied", for when it is clicked; read without it.
const card = (await page.locator(".contact-detail").innerText()).replace(/\s+/g, " ").replace(/ copied/g, "");
ok(card.includes("Dr. Zoya Ahuja, PhD"), `the whole name, prefix and suffix  ${card.slice(0, 80)}`);
ok(card.includes("“Zo”") && card.includes("ZOY-ah"), "the nickname and the phonetic name");
ok(card.includes("Architect, Design"), "job title and department");
ok(/mobile \+91 90000 00001/.test(card), "Apple's internal _$!<Mobile>!$_ reads as \"mobile\"");
ok(/anniversary (14 February 2015|February 14, 2015)/.test(card), `a date, labelled in Apple's words`);
ok(/mother Amma/.test(card) && /Skype zoya\.a/.test(card) && /Twitter @zoya/.test(card), "a related name, an instant message, a social profile");
ok((await page.locator('.contact-detail a[href="https://twitter.com/zoya"]').count()) === 1, "the profile opens its page");

await page.getByRole("button", { name: "Edit", exact: true }).click();
await page.waitForTimeout(200);
ok(/mobile/.test(await page.getByLabel("Label for detail 1").innerText()) && (await page.getByLabel("Custom label for detail 1").count()) === 0,
  "the internal spelling is Apple's own label in the editor, not a custom one");
ok(await page.getByLabel("Prefix").inputValue() === "Dr." && await page.getByLabel("Nickname").inputValue() === "Zo", "fields holding something are in the form");
ok((await page.getByLabel("Middle").count()) === 0, "fields holding nothing wait in Add Field");

await page.getByLabel("Nickname").fill("Zee");
await page.getByRole("button", { name: "Add Field" }).click();
const offered = await page.getByRole("menuitem").allInnerTexts();
ok(["Phone", "Email", "URL", "Address", "Date", "Related Name", "Social Profile", "Instant Message", "Middle name", "Phonetic last name"].every((x) => offered.includes(x)),
  `Add Field offers every kind  ${JSON.stringify(offered)}`);
await page.getByRole("menuitem", { name: "Middle name" }).click();
await page.getByLabel("Middle").fill("K");
await page.getByRole("button", { name: "Add Field" }).click();
await page.getByRole("menuitem", { name: "Related Name" }).click();
await page.getByLabel("Value for detail 7").fill("Ravi");
await page.getByLabel("Label for detail 7").click();
await page.getByRole("option", { name: "spouse", exact: true }).click();
await page.getByRole("button", { name: "Add Field" }).click();
await page.getByRole("menuitem", { name: "Date" }).click();
await page.getByLabel("Date 8, month").click();
await page.getByRole("option", { name: /^Jul/ }).click();
await page.getByLabel("Date 8, day").fill("4");
await page.getByRole("button", { name: "Add Field" }).click();
await page.getByRole("menuitem", { name: "Instant Message" }).click();
await page.getByLabel("Service for detail 9").click();
const services = await page.getByRole("option").allInnerTexts();
ok(services.some((s) => /Jabber/.test(s)) && !services.some((s) => /Custom/.test(s)), "a service is picked from Apple's list");
await page.getByRole("option", { name: "Jabber" }).click();
await page.getByLabel("Value for detail 9").fill("zoya@jabber.example");

// A photo: a new one chosen, moved in the circle, kept.
await page.getByRole("button", { name: "Edit photo" }).click();
const png = await page.evaluate(() => {
  const c = document.createElement("canvas");
  c.width = 900; c.height = 600;
  const x = c.getContext("2d");
  const g = x.createLinearGradient(0, 0, 900, 600);
  g.addColorStop(0, "#f4a261"); g.addColorStop(1, "#2a9d8f");
  x.fillStyle = g; x.fillRect(0, 0, 900, 600);
  x.fillStyle = "#264653"; x.beginPath(); x.arc(600, 260, 120, 0, 7); x.fill();
  return c.toDataURL("image/png").split(",")[1];
});
await page.getByLabel("Photo file").setInputFiles({ name: "face.png", mimeType: "image/png", buffer: Buffer.from(png, "base64") });
await page.locator(".photo-frame-image").waitFor();
const frame = await page.locator(".photo-frame").boundingBox();
await page.mouse.move(frame.x + 120, frame.y + 120);
await page.mouse.down();
await page.mouse.move(frame.x + 40, frame.y + 120, { steps: 4 });
await page.mouse.up();
await page.getByLabel("Zoom").fill("40");
await page.getByRole("button", { name: "Done" }).click();
await page.waitForTimeout(150);
ok((await page.locator(".contact-form-head .avatar img").getAttribute("src"))?.startsWith("data:image/jpeg"), "the chosen square shows on the card at once");

await page.getByRole("button", { name: "Save" }).click();
await page.waitForTimeout(300);
const sent = await page.evaluate(() => (window.__contactSaves ?? []).at(-1));
const c = sent?.contact ?? {};
ok(c.nickname === "Zee" && c.middleName === "K" && c.prefix === "Dr." && c.jobTitle === "Architect", `the name and work go together  ${JSON.stringify({ n: c.nickname, m: c.middleName })}`);
ok(c.methods?.[0]?.label === "_$!<Mobile>!$_", "a label left alone goes back exactly as it came");
const kinds = JSON.stringify(c.methods?.slice(6));
ok(kinds === JSON.stringify([
  { kind: "related", label: "SPOUSE", value: "Ravi" },
  { kind: "date", label: "ANNIVERSARY", value: "1604-07-04" },
  { kind: "im", label: "JABBER", value: "zoya@jabber.example" },
]), `new details in Apple's spelling, a date with no year as 1604  ${kinds}`);
const crop = (c.photoCrop ?? "").split(",").map(Number);
ok(c.photoUrl?.startsWith("data:image/jpeg;base64,") && c.photoUrl.length <= 229376, "the picture goes whole, as icloud.com sends it, under its size");
ok(crop.length === 4 && crop[2] === crop[3] && crop[2] > 0 && crop[0] > 0, `with the square chosen  ${c.photoCrop}`);
ok(sent?.base?.nickname === "Zo", "against the card as the form opened it");

// Taken away again.
await page.locator(".contact-row", { hasText: "Zoya" }).click();
await page.getByRole("button", { name: "Edit", exact: true }).click();
await page.getByRole("button", { name: /photo/ }).first().click();
const remove = page.getByRole("button", { name: "Delete Photo" });
ok((await remove.count()) === 1, "a card with a picture offers to delete it");
await remove.click();
await page.getByRole("button", { name: "Save" }).click();
await page.waitForTimeout(300);
const gone = (await page.evaluate(() => (window.__contactSaves ?? []).at(-1)))?.contact ?? {};
ok(gone.photoUrl === null && gone.photoCrop === null, "Delete Photo takes it away");

await b.close();
process.exitCode = failures === 0 ? 0 : 1;
