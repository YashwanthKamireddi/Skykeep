// The arrival motion set (lib/fade.ts arrive / leave, kit.css .receiving-line), which live sync
// uses when something comes in from another device:
//   - arrive: a fade of 120ms or less, rising at most 4px, opacity and transform only — no
//     highlight, and nothing beside the row moves
//   - leave: out by opacity, then `done`; the row keeps its height until it is removed
//   - Reduce Motion: both instant
//   - the receiving line: one calm line in the secondary text, figures that do not jitter
// Runs against the built app as the other suites do; the helpers it drives (lib/fade.ts) are
// compiled from source here and served at their source path, so the page imports them as is:
//   BASE=http://localhost:4173 node .uiverify/design-arrival.mjs
import { readFile } from "node:fs/promises";
import { transform } from "esbuild";
import { engine } from "./engine.mjs";
import { SERVICES, installStub } from "./stub.mjs";

const BASE = process.env.URL ?? process.env.BASE ?? "http://localhost:4173";
const fade = (await transform(await readFile(new URL("../src/lib/fade.ts", import.meta.url), "utf8"), { loader: "ts", format: "esm" })).code;
let failures = 0;
const ok = (p, m) => { console.log(`${p ? "PASS" : "FAIL"} ${m}`); if (!p) failures++; };
const b = await engine.launch();

async function open(reducedMotion) {
  const ctx = await b.newContext({ viewport: { width: 1000, height: 700 }, reducedMotion });
  await ctx.addInitScript(installStub, { screen: "library", services: SERVICES, twoFactor: null });
  await ctx.route(`${BASE}/src/lib/fade.ts`, (route) => route.fulfill({ contentType: "text/javascript", body: fade }));
  const page = await ctx.newPage();
  await page.goto(BASE, { waitUntil: "networkidle" });
  // Three rows in a plain list, the middle one the one that arrives or leaves.
  await page.evaluate(() => {
    const ul = document.createElement("ul");
    ul.id = "arrival-list";
    ul.style.cssText = "position:fixed;top:100px;left:100px;width:300px;margin:0;padding:0;list-style:none;z-index:999";
    for (const name of ["one", "two", "three"]) {
      const li = document.createElement("li");
      li.id = `row-${name}`;
      li.textContent = name;
      li.style.cssText = "height:30px;padding:4px 8px";
      ul.appendChild(li);
    }
    document.body.appendChild(ul);
  });
  return page;
}

// ---- arrive ----------------------------------------------------------------------------------
{
  const page = await open("no-preference");
  const seen = await page.evaluate(async () => {
    const { arrive } = await import("/src/lib/fade.ts");
    const row = document.getElementById("row-two");
    const below = document.getElementById("row-three");
    const belowTop = below.getBoundingClientRect().top;
    const frames = [];
    const start = performance.now();
    arrive(row);
    const first = { opacity: row.style.opacity, transform: row.style.transform };
    await new Promise((done) => {
      const look = () => {
        frames.push({
          t: performance.now() - start,
          opacity: row.style.opacity,
          transform: row.style.transform,
          belowTop: below.getBoundingClientRect().top,
          bg: getComputedStyle(row).backgroundColor,
          shadow: getComputedStyle(row).boxShadow,
        });
        if (performance.now() - start < 400) requestAnimationFrame(look);
        else done();
      };
      requestAnimationFrame(look);
    });
    return { first, frames, belowTop };
  });
  const moving = seen.frames.filter((f) => f.opacity !== "");
  const lastMoving = moving.at(-1);
  const rises = seen.frames.map((f) => Math.abs(parseFloat(/translateY\((-?[\d.]+)px\)/.exec(f.transform)?.[1] ?? "0")));
  ok(seen.first.opacity === "0", `arrive starts the row unseen, before its first frame  [${JSON.stringify(seen.first)}]`);
  ok(moving.length > 0 && lastMoving.t <= 120 + 34, `it fades in within 120ms  [last changing frame at ${Math.round(lastMoving?.t ?? -1)}ms]`);
  ok(Math.max(...rises, parseFloat(/-?[\d.]+/.exec(seen.first.transform)?.[0] ?? "0")) <= 4, `it rises 4px at most  [${Math.max(...rises)}px]`);
  ok(seen.frames.at(-1).opacity === "" && seen.frames.at(-1).transform === "", "and leaves no style of its own behind");
  ok(seen.frames.every((f) => f.belowTop === seen.belowTop), "the row below never moves");
  ok(seen.frames.every((f) => f.bg === "rgba(0, 0, 0, 0)" && f.shadow === "none"), "nothing is highlighted: no wash, no tint");
  await page.context().close();
}

// ---- leave -----------------------------------------------------------------------------------
{
  const page = await open("no-preference");
  const seen = await page.evaluate(async () => {
    const { leave } = await import("/src/lib/fade.ts");
    const row = document.getElementById("row-two");
    const below = document.getElementById("row-three");
    const belowTop = below.getBoundingClientRect().top;
    const height = row.getBoundingClientRect().height;
    const start = performance.now();
    const tops = [];
    const heights = [];
    let doneAt = null;
    leave(row, () => { doneAt = performance.now() - start; row.remove(); });
    const sync = doneAt;
    await new Promise((done) => {
      const look = () => {
        if (row.isConnected) { tops.push(below.getBoundingClientRect().top); heights.push(Math.round(row.getBoundingClientRect().height * 100) / 100); }
        if (performance.now() - start < 500) requestAnimationFrame(look);
        else done();
      };
      requestAnimationFrame(look);
    });
    return { sync, doneAt, belowTop, tops, heights, height, opacityAtEnd: row.style.opacity };
  });
  ok(seen.sync === null, "leave lets the row go over a few frames, not at once");
  ok(seen.doneAt !== null && seen.doneAt <= 160 + 50, `then hands it over to be removed, within the small-motion limit  [${Math.round(seen.doneAt ?? -1)}ms]`);
  ok(seen.tops.every((t) => t === seen.belowTop) && seen.heights.every((h) => h === seen.height), `its height never changes while it fades — no layout is animated  [${seen.belowTop}: ${seen.tops.join(",")}; ${seen.height}: ${seen.heights.join(",")}]`);
  await page.context().close();
}

// ---- Reduce Motion: instant ------------------------------------------------------------------
{
  const page = await open("reduce");
  const seen = await page.evaluate(async () => {
    const { arrive, leave } = await import("/src/lib/fade.ts");
    const row = document.getElementById("row-two");
    arrive(row);
    const after = { opacity: row.style.opacity, transform: row.style.transform };
    let done = false;
    leave(document.getElementById("row-three"), () => { done = true; });
    return { after, done };
  });
  ok(seen.after.opacity === "" && seen.after.transform === "", `under Reduce Motion a new row is simply there  ${JSON.stringify(seen.after)}`);
  ok(seen.done, "and a deleted one goes at once");
  await page.context().close();
}

// ---- the receiving line ----------------------------------------------------------------------
{
  const page = await open("no-preference");
  // The markup ReceivingLine draws (components/ReceivingLine), with the kit's styles.
  const look = await page.evaluate(() => {
    const p = document.createElement("p");
    p.className = "receiving-line";
    p.setAttribute("role", "status");
    p.innerHTML = '<span class="receiving-line-text">Receiving 48 photos</span><span class="progress" role="progressbar"><span style="transform:scaleX(0.25)"></span></span>';
    document.body.appendChild(p);
    const cs = getComputedStyle(p);
    const probe = document.createElement("span");
    probe.style.color = "var(--text-2)";
    document.body.appendChild(probe);
    const text2 = getComputedStyle(probe).color;
    const bar = p.querySelector(".progress").getBoundingClientRect().height;
    const w1 = p.getBoundingClientRect().width;
    p.querySelector(".receiving-line-text").textContent = "Receiving 11 photos";
    const w2 = p.getBoundingClientRect().width;
    return { colour: cs.color, text2, numeric: cs.fontVariantNumeric, bar, w1, w2, size: cs.fontSize };
  });
  ok(look.colour === look.text2, `the receiving line is quiet, in the secondary text  [${look.colour}]`);
  ok(look.numeric.includes("tabular-nums") && look.w1 === look.w2, `its count changes without the line changing width  [${look.w1} → ${look.w2}]`);
  ok(look.bar === 2, `its bar is a thin 2px line  [${look.bar}]`);
  await page.context().close();
}

await b.close();
process.exitCode = failures ? 1 : 0;
