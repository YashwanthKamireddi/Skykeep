// One browser of each engine for a whole run of the checks, which every suite connects to
// (engine.mjs) instead of starting its own. Starting a browser costs more than most suites spend
// checking, and the gate starts hundreds of them; each suite still gets contexts of its own, as
// isolated from the others' as a browser of its own would be.
//
//   node .uiverify/browsers.mjs <file>   writes PW_CHROMIUM_WS=… and PW_WEBKIT_WS=… to <file>,
//                                        then serves until it is stopped (SIGTERM or SIGINT)
//
// Nothing is checked here, and a suite run on its own (without those variables) still starts
// its own browser, as before.
import { writeFileSync, renameSync } from "node:fs";
import { chromium, webkit } from "playwright";

const [chrome, wk] = await Promise.all([chromium.launchServer(), webkit.launchServer()]);
const file = process.argv[2];
// Written whole and then renamed, so a reader waiting for the file never sees half of it.
writeFileSync(`${file}.part`, `PW_CHROMIUM_WS=${chrome.wsEndpoint()}\nPW_WEBKIT_WS=${wk.wsEndpoint()}\n`);
renameSync(`${file}.part`, file);
const stop = async () => { await Promise.all([chrome.close(), wk.close()]); process.exit(0); };
process.on("SIGTERM", stop);
process.on("SIGINT", stop);
