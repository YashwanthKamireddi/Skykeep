// `npm run licences`: writes src/licences.json, every open-source licence the desktop app ships,
// for Settings › Open-Source Licences. The Rust crates built into the app come from cargo-about
// (with about.toml's rules), the npm packages bundled into the window from package-lock.json,
// each with its own licence file, and the C libraries a build links in that are neither
// (packaging/licences/extra.json: libheif and what it brings in the Windows build). Run when dependencies change and the result committed: the
// build only reads the file, so Flatpak, COPR, the AUR and every CI build need no cargo-about,
// and the release (tag.yml) and the local gate (check.sh) run this again and refuse a list that
// differs. It fails, writing nothing, on a licence about.toml does not accept, a package with no
// licence file, or a package in the window's bundle that the list does not name.
import { execFile } from "node:child_process";
import { mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const run = promisify(execFile);
const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(root, "src", "licences.json");

// Every target a desktop build is made for. One list for all of them, so the committed file is
// the same whichever machine writes it; a crate only one of them uses is listed for all.
const TARGETS = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "x86_64-pc-windows-msvc"];

/** @typedef {{ id: string, name: string, text: string, users: string[] }} Licence */

/** Plain code-point order, the same on every machine (localeCompare follows the locale). */
const order = (a, b) => (a < b ? -1 : a > b ? 1 : 0);

/** Texts as written, with one kind of line ending and no space around them. */
const same = (text) => text.replace(/\r\n?/g, "\n").trim();

/** about.toml's accepted licences, in its order: the order an either-or licence is chosen in. */
function accepted() {
  const toml = readFileSync(join(root, "about.toml"), "utf8");
  const list = toml.match(/^accepted\s*=\s*\[([\s\S]*?)\]/m);
  if (!list) throw new Error("about.toml has no accepted list");
  return [...list[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
}

/** The app's crates and their licences, from cargo-about, Skykeep's own crates left out. */
async function crates() {
  try {
    await run("cargo", ["about", "--version"], { cwd: root });
  } catch {
    throw new Error("cargo-about is needed: cargo install --locked cargo-about@0.9.3");
  }
  const dir = mkdtempSync(join(tmpdir(), "skykeep-licences-"));
  const out = join(dir, "about.json");
  try {
    await run(
      "cargo",
      ["about", "generate", "--format", "json", "--fail", "--locked", "-c", "about.toml",
        "-m", "src-tauri/Cargo.toml", ...TARGETS.flatMap((t) => ["--target", t]), "-o", out],
      { cwd: root, maxBuffer: 64 * 1024 * 1024 },
    );
    const about = JSON.parse(readFileSync(out, "utf8"));
    if (!Array.isArray(about.licenses)) throw new Error("cargo about wrote no licences");
    /** @type {Licence[]} */
    const list = [];
    for (const l of about.licenses) {
      // Skykeep's own crates (path dependencies, with no source) are the app itself, under its
      // own GPL-3.0, not third-party work.
      const users = l.used_by.filter((u) => u.crate.source != null).map((u) => `${u.crate.name} ${u.crate.version}`);
      if (users.length > 0) list.push({ id: l.id, name: l.name, text: l.text, users });
    }
    return list;
  } catch (e) {
    throw new Error(`cargo about could not list the app's crate licences:\n${e.stderr || e.message}`);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

/** The licence files at the top of a package, as npm and most authors name them. */
function licenceFiles(dir) {
  return readdirSync(dir).filter(
    (f) => /^(licen[cs]e|copying)/i.test(f) && !/\.spdx$/i.test(f) && statSync(join(dir, f)).isFile(),
  );
}

/**
 * The npm packages the window is built from: every package in package-lock.json that is not a
 * development one, with its licence (an either-or licence taken as the first of about.toml's
 * accepted list it offers) and that licence's text from the package's own files.
 */
function packages(allowed) {
  const lock = JSON.parse(readFileSync(join(root, "package-lock.json"), "utf8"));
  const list = [];
  for (const [path, entry] of Object.entries(lock.packages ?? {})) {
    if (!path.startsWith("node_modules/") || entry.dev) continue;
    const dir = join(root, path);
    let pkg;
    try {
      pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
    } catch {
      // An optional package for another system is not installed here and not in this build.
      if (entry.optional) continue;
      throw new Error(`${path} is not installed: run npm ci first`);
    }
    const name = pkg.name ?? path.slice(path.lastIndexOf("node_modules/") + 13);
    const user = `${name} ${pkg.version}`;
    const expr = typeof pkg.license === "string" ? pkg.license : pkg.license?.type;
    if (!expr) throw new Error(`${user} states no licence in its package.json`);
    if (/\bAND\b/.test(expr)) throw new Error(`${user} is under ${expr}: a licence with AND needs handling here`);
    const options = expr.replace(/[()]/g, "").split(/\s+OR\s+/).map((s) => s.trim());
    const id = allowed.find((a) => options.includes(a));
    if (!id) throw new Error(`${user} is under ${expr}, which about.toml does not accept`);
    const files = licenceFiles(dir);
    // With a file for each licence (LICENSE-MIT, LICENSE-APACHE-2.0), the one for the licence taken.
    const word = id.split("-")[0].toLowerCase();
    const file = files.length === 1 ? files[0] : files.find((f) => f.toLowerCase().includes(word));
    if (!file) throw new Error(`${user} has no ${id} licence file to ship`);
    list.push({ id, text: readFileSync(join(dir, file), "utf8"), user, name });
  }
  return list;
}

/**
 * The C libraries listed by hand in packaging/licences/extra.json, each text its notice then its
 * licence from the file named (a path from the repository's root).
 */
function extras() {
  const file = join(root, "packaging", "licences", "extra.json");
  const { licences } = JSON.parse(readFileSync(file, "utf8"));
  if (!Array.isArray(licences)) throw new Error("packaging/licences/extra.json has no licences list");
  return licences.map((l) => {
    for (const key of ["id", "name", "notice", "file"]) {
      if (typeof l[key] !== "string" || l[key] === "") throw new Error(`packaging/licences/extra.json: an entry has no ${key}`);
    }
    if (!Array.isArray(l.users) || l.users.length === 0) throw new Error(`packaging/licences/extra.json: ${l.id} names no users`);
    const text = `${same(l.notice)}\n\n\n${same(readFileSync(join(root, l.file), "utf8"))}`;
    return { id: l.id, name: l.name, text, users: l.users };
  });
}

/** Crates, packages and the listed C libraries, one entry for each licence text, in a fixed order. */
function combine(rust, web, extra) {
  const names = new Map(rust.map((l) => [l.id, l.name]));
  // The texts of one licence used by exactly the same crates are one entry, each text kept
  // once: ring carries its ISC notice in 21 source files, and listed apart they were 21 rows
  // all reading "ring 0.17.14".
  const squash = (text) => text.replace(/\s+/g, " ").trim();
  const together = new Map();
  for (const l of rust) {
    const users = [...l.users].sort(order);
    const key = `${l.id}\n${users.join("\n")}`;
    const found = together.get(key);
    if (!found) together.set(key, { ...l, users, texts: [l.text] });
    else if (!found.texts.some((t) => squash(t) === squash(l.text))) found.texts.push(l.text);
  }
  // One entry per licence text: crates and packages with the very same text (the same
  // copyright holder) share it, as cargo-about shares it between crates.
  /** @type {Map<string, Licence>} */
  const byText = new Map();
  const add = (id, name, text, users) => {
    const key = `${id}\n${text}`;
    const found = byText.get(key);
    if (found) found.users.push(...users);
    else byText.set(key, { id, name, text, users: [...users] });
  };
  for (const { id, name, texts, users } of together.values()) {
    add(id, name, texts.map(same).sort(order).join("\n\n\n"), users);
  }
  for (const p of web) add(p.id, names.get(p.id) ?? p.id, same(p.text), [p.user]);
  for (const l of extra) add(l.id, l.name, l.text, l.users);
  const licences = [...byText.values()];
  for (const l of licences) l.users = [...new Set(l.users)].sort(order);
  licences.sort((a, b) => order(a.name, b.name) || order(a.users[0], b.users[0]) || order(a.text, b.text));
  // Keys in one order too, so the file's text depends only on the licences.
  return licences.map(({ id, name, users, text }) => ({ id, name, users, text }));
}

/** The package a bundled module comes from, by its path under node_modules. */
function packageOf(id) {
  const at = id.lastIndexOf("/node_modules/");
  if (at < 0) return null;
  const parts = id.slice(at + 14).split("/");
  return parts[0].startsWith("@") ? `${parts[0]}/${parts[1]}` : parts[0];
}

/**
 * The window built in memory (vite.config.ts, nothing written), and every package in it named
 * by the list: one that reaches the bundle some way package-lock.json does not show fails here.
 */
async function checkBundle(npm) {
  const { build } = await import("vite");
  const result = await build({ root, logLevel: "error", build: { write: false } });
  const outputs = Array.isArray(result) ? result : [result];
  const missing = new Set();
  for (const o of outputs) {
    for (const chunk of o.output) {
      if (chunk.type !== "chunk") continue;
      for (const id of Object.keys(chunk.modules)) {
        const name = packageOf(id);
        if (name && !npm.has(name)) missing.add(name);
      }
    }
  }
  if (missing.size > 0) throw new Error(`bundled but not in the licence list: ${[...missing].sort(order).join(", ")}`);
}

try {
  const allowed = accepted();
  const web = packages(allowed);
  const licences = combine(await crates(), web, extras());
  const text = JSON.stringify(licences, null, 2) + "\n";
  // The bundle is checked against the new list, then the old file is put back if it fails.
  let before = null;
  try {
    before = readFileSync(OUT, "utf8");
  } catch {
    /* the first run */
  }
  writeFileSync(OUT, text);
  try {
    await checkBundle(new Set(web.map((p) => p.name)));
  } catch (e) {
    if (before === null) rmSync(OUT, { force: true });
    else writeFileSync(OUT, before);
    throw e;
  }
  const packagesShipped = new Set(licences.flatMap((l) => l.users)).size;
  console.log(`src/licences.json: ${licences.length} licence texts, ${packagesShipped} crates and packages`);
} catch (e) {
  console.error(`npm run licences: ${e.message}`);
  process.exitCode = 1;
}
