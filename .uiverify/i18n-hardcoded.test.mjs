// No words written into the screens: every sentence, label and button comes from the locale
// files, so a language picked in Settings is the language shown (#40). A Chinese reader saw
// English mixed in because about 450 strings were typed straight into the markup; this keeps
// new ones from creeping back. Brand and key names are fine as they are; so is the Quick Event
// example, which shows the English its reader understands (calendar/quick.ts).
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const ROOT = new URL("../src/", import.meta.url).pathname;
const files = (dir) => readdirSync(dir).flatMap((n) => {
  const p = join(dir, n);
  return statSync(p).isDirectory() ? files(p) : p.endsWith(".tsx") ? [p] : [];
});
const PATTERNS = [
  />\s*([A-Z][A-Za-z’'][A-Za-z ,.’'!?…:-]{1,})\s*</g,
  /\b(?:aria-label|title|placeholder|alt|label)="([A-Z][^"]{1,})"/g,
  /\b(?:label|message|title|body|action|heading):\s*"([A-Z][a-z][^"]*)"/g,
];
const BARE = /^\s+([A-Z][a-z][A-Za-z ,.’'!?…:-]{2,})\s*$/;
const ALLOWED = /^(Promise|Record|Set|Map|Partial|Array|HTMLElement|HTMLDivElement|Element|Skykeep|iCloud|iPhone|iPad|Mac|Apple|GitHub|URL|LIVE|OK|Ctrl|Shift|Alt|Enter|Tab|Esc|Return|Space|Delete|F\d+|Notes|AR|HEIC|JPEG|PDF|CalDAV|Nextcloud|Lunch with Asha tomorrow at 1pm)$/;

test("no screen has words written into it instead of the locale files", () => {
  const found = [];
  for (const f of files(ROOT)) {
    let comment = false;
    readFileSync(f, "utf8").split("\n").forEach((line, i) => {
      const s = line.trim();
      if (s.startsWith("/*") || s.startsWith("{/*")) comment = !s.includes("*/");
      else if (comment) { if (s.includes("*/")) comment = false; return; }
      if (comment || s.startsWith("//") || s.startsWith("*") || line.includes("import ")) return;
      const hits = PATTERNS.flatMap((p) => [...line.matchAll(p)].map((m) => m[1].trim()));
      const bare = BARE.exec(line);
      if (bare && !/[,({;=]$/.test(s) && !line.includes("=>")) hits.push(bare[1].trim());
      for (const h of hits) if (!ALLOWED.test(h) && !h.includes("{")) found.push(`${f.slice(ROOT.length)}:${i + 1}  ${h}`);
    });
  }
  assert.deepEqual(found, [], `words written into the screens — put them in src/locales/*.json:\n${found.join("\n")}`);
});
