// Which browser engine a suite drives. Quince draws in WebKitGTK, so WebKit is the one that
// matters; Chromium stays the default only because it needs nothing extra to run.
//
//   ENGINE=webkit node .uiverify/calendar.mjs   WebKit's WPE port, off-screen
//   ENGINE=gtk    node .uiverify/calendar.mjs   WebKit's GTK port — Quince's own — in a real
//                                              window on this display
//   ENGINE=msedge node .uiverify/calendar.mjs   Microsoft Edge — the engine WebView2 is, with
//                                              Windows' own fonts — on a Windows machine
//   WEBVIEW2=1 node .uiverify/photos-drag.mjs   Chromium as Skykeep's window is on Windows:
//                                              WebView2's user agent, and no drag event of the
//                                              engine's own reaching the page — Tauri holds the
//                                              window's drop target there, for files dropped
//                                              from Explorer (src/lib/drag.ts)
//
// Playwright's WebKit build wants a few libraries this machine's distribution names
// differently; .uiverify/webkit-libs.sh fetches them into a private folder, no root needed.
import { chromium, webkit } from "playwright";

export const engineName = ["webkit", "gtk", "msedge"].includes(process.env.ENGINE) ? process.env.ENGINE : "chromium";
// Edge is Chromium (the engine WebView2 is); only the two WebKit ports take Playwright's WebKit.
const base = engineName === "webkit" || engineName === "gtk" ? webkit : chromium;
/**
 * In the GTK port the window is a real one on this desktop, and a tiling window manager sizes
 * it — Playwright's attempt to resize it is refused ("Failed to resize window"). So a GTK run
 * takes whatever size it is given: contexts get no fixed viewport and pages ignore requests to
 * change it. The suites that matter here measure from the elements themselves.
 */
export const engine = {
  launch: async (options = {}) => {
    const browser = await base.launch({
      ...options,
      ...(engineName === "gtk" ? { headless: false } : {}),
      ...(engineName === "msedge" ? { channel: "msedge" } : {}),
    });
    englishUnlessAsked(browser);
    if (webview2) return asWebView2(browser);
    if (engineName !== "gtk") return browser;
    const newContext = browser.newContext.bind(browser);
    browser.newContext = async (opts = {}) => {
      const ctx = await newContext({ ...opts, viewport: null });
      const newPage = ctx.newPage.bind(ctx);
      ctx.newPage = async () => {
        const page = await newPage();
        page.setViewportSize = async () => {};
        return page;
      };
      return ctx;
    };
    browser.newPage = async (opts = {}) => (await browser.newContext(opts)).newPage();
    return browser;
  },
};

/**
 * Every context in American English unless a suite asks for another locale, as on CI. A browser
 * otherwise takes this machine's language, and on a German desktop the interface (which follows
 * the system language) would answer in German to a suite that reads its English words.
 */
function englishUnlessAsked(browser) {
  const newContext = browser.newContext.bind(browser);
  browser.newContext = async (opts = {}) => newContext({ locale: "en-US", ...opts });
  browser.newPage = async (opts = {}) => (await browser.newContext(opts)).newPage();
}

/** Run as Skykeep's window on Windows (WEBVIEW2=1, Chromium only). */
const webview2 = Boolean(process.env.WEBVIEW2) && base === chromium;
const WEBVIEW2_AGENT =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0";

/** Every context WebView2's user agent, and the engine's own drag events kept from the page. */
function asWebView2(browser) {
  const newContext = browser.newContext.bind(browser);
  browser.newContext = async (opts = {}) => {
    const ctx = await newContext({ ...opts, userAgent: WEBVIEW2_AGENT });
    await ctx.addInitScript(() => {
      for (const type of ["dragenter", "dragover", "dragleave", "drop"]) {
        window.addEventListener(type, (e) => { if (e.isTrusted) e.stopImmediatePropagation(); }, true);
      }
    });
    return ctx;
  };
  browser.newPage = async (opts = {}) => (await browser.newContext(opts)).newPage();
  return browser;
}
