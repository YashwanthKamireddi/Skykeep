// The open-source licences the desktop app ships, written into the web build so Settings can
// show them: the Rust crates built into the app (cargo-about, with about.toml's rules) and the
// npm packages bundled into the window. Read from the lockfiles at every build, so the list is
// never typed by hand and cannot fall behind a dependency change. A crate or package under a
// licence about.toml does not accept, or with no licence text, fails the build rather than
// ship unlisted.
import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const run = promisify(execFile);
const VIRTUAL = "virtual:licences";
const RESOLVED = "\0" + VIRTUAL;

/** @typedef {{ id: string, name: string, text: string, users: string[] }} Licence */

/** about.toml's accepted licences, in its order: the order an either-or licence is chosen in. */
function accepted(root) {
  const toml = readFileSync(join(root, "about.toml"), "utf8");
  const list = toml.match(/^accepted\s*=\s*\[([\s\S]*?)\]/m);
  if (!list) throw new Error("about.toml has no accepted list");
  return [...list[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
}

/** The Rust target the app is built for: the one `tauri build` names, else this machine's. */
async function target(root) {
  if (process.env.TAURI_ENV_TARGET_TRIPLE) return process.env.TAURI_ENV_TARGET_TRIPLE;
  const { stdout } = await run("rustc", ["-vV"], { cwd: root });
  const host = stdout.match(/^host: (\S+)/m);
  if (!host) throw new Error("rustc -vV named no host target");
  return host[1];
}

/**
 * The app's crates and their licences, from cargo-about. Kept in node_modules/.cache under a
 * key of everything that decides the answer (the lockfile, every manifest, about.toml, the
 * target, cargo-about's version and this file): a fresh run reads every crate's licence files
 * and takes over half a minute, and run.sh builds the window on every start.
 */
async function crates(root) {
  let version;
  try {
    version = (await run("cargo", ["about", "--version"], { cwd: root })).stdout.trim();
  } catch {
    throw new Error("cargo-about is needed to write the app's licence notices: cargo install --locked cargo-about@0.9.3");
  }
  const triple = await target(root);
  const manifests = [
    join(root, "Cargo.toml"),
    join(root, "src-tauri", "Cargo.toml"),
    ...readdirSync(join(root, "crates"))
      .map((c) => join(root, "crates", c, "Cargo.toml"))
      .filter((f) => existsSync(f)),
  ];
  const hash = createHash("sha256").update(`${version}\n${triple}\n`);
  for (const f of [fileURLToPath(import.meta.url), join(root, "Cargo.lock"), join(root, "about.toml"), ...manifests]) {
    hash.update(readFileSync(f));
  }
  const dir = join(root, "node_modules", ".cache", "skykeep-licences");
  const cached = join(dir, `${hash.digest("hex").slice(0, 32)}.json`);
  if (existsSync(cached)) return JSON.parse(readFileSync(cached, "utf8"));

  const out = join(tmpdir(), `skykeep-about-${process.pid}-${Date.now()}.json`);
  try {
    await run(
      "cargo",
      ["about", "generate", "--format", "json", "--fail", "--locked", "-c", "about.toml",
        "-m", "src-tauri/Cargo.toml", "--target", triple, "-o", out],
      { cwd: root, maxBuffer: 64 * 1024 * 1024 },
    );
  } catch (e) {
    rmSync(out, { force: true });
    throw new Error(`cargo about could not list the app's crate licences:\n${e.stderr || e.message}`);
  }
  const about = JSON.parse(readFileSync(out, "utf8"));
  rmSync(out, { force: true });
  if (!Array.isArray(about.licenses)) throw new Error("cargo about wrote no licences");
  /** @type {Licence[]} */
  const list = [];
  for (const l of about.licenses) {
    // Skykeep's own crates (path dependencies, with no source) are the app itself, under its
    // own GPL-3.0, not third-party work.
    const users = l.used_by.filter((u) => u.crate.source != null).map((u) => `${u.crate.name} ${u.crate.version}`);
    if (users.length > 0) list.push({ id: l.id, name: l.name, text: l.text, users });
  }
  mkdirSync(dir, { recursive: true });
  // Earlier answers are dropped: only the current one is ever read again.
  for (const old of readdirSync(dir)) rmSync(join(dir, old), { force: true });
  writeFileSync(cached, JSON.stringify(list));
  return list;
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
function packages(root, allowed) {
  const lock = JSON.parse(readFileSync(join(root, "package-lock.json"), "utf8"));
  const list = [];
  for (const [path, entry] of Object.entries(lock.packages ?? {})) {
    if (!path.startsWith("node_modules/") || entry.dev) continue;
    const dir = join(root, path);
    // An optional package for another system is not installed here and not in this build.
    if (entry.optional && !existsSync(dir)) continue;
    const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
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

/** Texts compared as written, apart from line endings and the space around them. */
const same = (text) => text.replace(/\r\n/g, "\n").trim();

async function collect(root) {
  const allowed = accepted(root);
  const rust = await crates(root);
  const web = packages(root, allowed);
  const names = new Map(rust.map((l) => [l.id, l.name]));
  // The texts of one licence used by exactly the same crates are one entry, each text kept
  // once: ring carries its ISC notice in 21 source files, and listed apart they were 21 rows
  // all reading "ring 0.17.14".
  const squash = (text) => text.replace(/\s+/g, " ").trim();
  const together = new Map();
  for (const l of rust) {
    const users = [...l.users].sort((a, b) => a.localeCompare(b));
    const key = `${l.id}\n${users.join("\n")}`;
    const found = together.get(key);
    if (!found) together.set(key, { ...l, users, texts: [l.text] });
    else if (!found.texts.some((t) => squash(t) === squash(l.text))) found.texts.push(l.text);
  }
  // One entry per licence text: crates and packages with the very same text (the same
  // copyright holder) share it, as cargo-about shares it between crates.
  const byText = new Map();
  for (const { texts, ...l } of together.values()) {
    const text = texts.map((t) => same(t)).join("\n\n\n");
    const key = `${l.id}\n${text}`;
    const found = byText.get(key);
    if (found) found.users.push(...l.users);
    else byText.set(key, { ...l, text });
  }
  for (const p of web) {
    const key = `${p.id}\n${same(p.text)}`;
    const found = byText.get(key);
    if (found) found.users.push(p.user);
    else byText.set(key, { id: p.id, name: names.get(p.id) ?? p.id, text: p.text, users: [p.user] });
  }
  const licences = [...byText.values()];
  for (const l of licences) l.users = [...new Set(l.users)].sort((a, b) => a.localeCompare(b));
  licences.sort((a, b) => a.name.localeCompare(b.name) || a.users[0].localeCompare(b.users[0]));
  return { licences, npm: new Set(web.map((p) => p.name)) };
}

/** The package a bundled module comes from, by its path under node_modules. */
function packageOf(id) {
  const at = id.lastIndexOf("/node_modules/");
  if (at < 0) return null;
  const parts = id.slice(at + 14).split("/");
  return parts[0].startsWith("@") ? `${parts[0]}/${parts[1]}` : parts[0];
}

/**
 * The vite plugin: `import("virtual:licences")` is every licence the app ships. A production
 * build also checks that every package in the bundle is in the list, so one that reaches the
 * bundle some way package-lock.json does not show still fails the build.
 */
export function licences() {
  let root = process.cwd();
  let building = false;
  /** @type {Promise<{ licences: Licence[], npm: Set<string> }> | null} */
  let pending = null;
  const result = () => (pending ??= collect(root));
  return {
    name: "skykeep-licences",
    configResolved(config) {
      root = config.root;
      building = config.command === "build";
    },
    // Started with the build, beside the bundling. The dev server reads them only when the
    // licences are first opened, so `npm run dev` starts as fast as before.
    buildStart() {
      if (building) result().catch(() => {});
    },
    resolveId(id) {
      return id === VIRTUAL ? RESOLVED : null;
    },
    async load(id) {
      if (id !== RESOLVED) return null;
      const { licences } = await result();
      return `export default JSON.parse(${JSON.stringify(JSON.stringify(licences))});`;
    },
    async generateBundle(_options, bundle) {
      const { npm } = await result();
      for (const chunk of Object.values(bundle)) {
        if (chunk.type !== "chunk") continue;
        for (const id of Object.keys(chunk.modules)) {
          const name = packageOf(id);
          if (name && !npm.has(name)) this.error(`${name} is bundled but not in the licence notices`);
        }
      }
    },
  };
}
