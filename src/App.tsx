import { useCallback, useEffect, useState } from "react";
import { SignIn, sentence } from "./screens/SignIn";
import { CodeEntry } from "./screens/CodeEntry";
import { Library } from "./screens/Library";
import { ImportSheet } from "./components/ImportSheet";
import { StatusLine } from "./components/StatusLine";
import { dismissToast, toast } from "./components/Toast";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { api, messageOf, NEEDS_CODE, NEEDS_SIGNIN } from "./lib/ipc";

type Phase = "checking" | "signin" | "code" | "library";

export default function App() {
  const [phase, setPhase] = useState<Phase>("checking");
  /** How the code screen asks for its code (`CodeEntry`'s `request`). */
  const [codeRequest, setCodeRequest] = useState<"now" | "ask" | undefined>(undefined);
  /**
   * Skykeep signed itself back in and Apple wants a code first. Nothing has been sent: the
   * library stays, with one line saying so, and the code is asked for once the person opens
   * the code screen from it.
   */
  const [codeLapse, setCodeLapse] = useState(false);
  // Why the stored session could not be taken up, said once on the sign-in screen.
  const [why, setWhy] = useState<string | null>(null);
  // The window is shown once the first real screen (the library, or sign-in) is in the page:
  // never the session check before it (see window_ready, main.rs). Said as soon as React has
  // put it there, not after an animation frame: the window is hidden until then, and a hidden
  // WebKitGTK window draws no frames, so waiting on one left every launch to the three-second
  // fallback. The page is laid out by now and the window paints it as it maps.
  //
  // The library says so itself (`onShown`), once its first screen is drawn with its code in:
  // said as the phase changed, the window mapped onto an empty sidebar and a blank pane.
  const [shown, setShown] = useState(false);
  const show = useCallback(
    (needsYou: boolean) => {
      if (shown) return;
      setShown(true);
      // Started at login, the window stays in the tray unless the person is needed here.
      invoke("window_ready", { needsYou }).catch(() => {});
    },
    [shown],
  );
  useEffect(() => {
    if (phase === "checking" || phase === "library") return;
    show(true);
  }, [phase, show]);
  const libraryShown = useCallback(() => show(false), [show]);

  // On launch, try the stored session before showing anything. This is the whole promise:
  // a returning user should never see the sign-in screen.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const status = await api.status();
        // Always resume rather than trusting stored metadata. Apple's service endpoints
        // authenticate by cookie, and holding a token is not the same as being able to call
        // them — resuming is what establishes the cookies that make the session usable.
        if (status.hasStoredSession && status.libraryReady) {
          // The library is this session's account's already: it opens at once on what it
          // holds, and the session is taken up behind it — a round trip to Apple (seconds on
          // a slow network) is never what stands between a launch and the window. The first
          // refresh waits on it in the engine. Apple wanting a code shows the code screen;
          // a session it will not take back leaves the library readable with the sign-in
          // line over it, as a lapse does.
          const resumed = api.resume();
          setPhase("library");
          // A resume that failed says why (a locked account, say); only Apple declining the
          // session is "signed you out".
          const next = await resumed.catch((err) => {
            if (!cancelled) setWhy(messageOf(err));
            return "signin" as const;
          });
          if (cancelled) return;
          if (next === "code") setCodeLapse(true);
          else if (next === "signin") setLapsed(true);
          return;
        }
        if (status.hasStoredSession) {
          // A resume that could not be made says why on the sign-in screen — a keyring that
          // is locked, most often — rather than a bare form that reads as signed out.
          const next = await api.resume().catch((err) => {
            if (!cancelled) setWhy(messageOf(err));
            return "signin" as const;
          });
          if (next === "code") setCodeRequest("ask");
          if (next !== "signin") return !cancelled && setPhase(next);
        }
      } catch {
        /* fall through to sign-in */
      }
      if (!cancelled) setPhase("signin");
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // A session that lapsed while the window was open, and that Apple will take back only
  // with a code: the code screen, not the sign-in screen — the password is already kept.
  useEffect(() => {
    const stop = listen(NEEDS_CODE, () => setCodeLapse(true));
    return () => {
      void stop.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);

  // Apple signed the session out: the library stays — still readable — with one line over it
  // saying so, and the sign-in a click away rather than forced in its place.
  const [lapsed, setLapsed] = useState(false);
  useEffect(() => {
    const stop = listen(NEEDS_SIGNIN, () => setLapsed(true));
    return () => {
      void stop.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);
  // Said where everything passing is said, over the foot of the content: drawn above the
  // library, it pushed the whole window down and cut off the sidebar's foot. It stays until
  // put away; Sign In is on it.
  useEffect(() => {
    if (!lapsed || phase !== "library") return undefined;
    const id = toast({
      message: `${why ? sentence(why) : "Apple signed you out."} Your copy on this computer is still here.`,
      tone: "error",
      duration: null,
      action: { label: "Sign In", run: () => setPhase("signin") },
    });
    return () => dismissToast(id);
  }, [lapsed, phase, why]);

  useEffect(() => {
    if (!codeLapse || phase !== "library") return undefined;
    const id = toast({
      message: "Sign in again to keep syncing. Your copy on this computer is still here.",
      duration: null,
      action: {
        label: "Sign In",
        run: () => {
          setCodeRequest("now");
          setPhase("code");
        },
      },
    });
    return () => dismissToast(id);
  }, [codeLapse, phase]);

  // Signed out, the window starts over: nothing of the account is left in its memory either —
  // no picture, no list, no cached answer — for whoever signs in next.
  async function signOut(keep: boolean) {
    await api.signOut(keep);
    window.location.reload();
  }

  // Filling the library is the library's own business now: one refresh covers every
  // service, runs the moment the window opens, and repeats on its own thereafter.


  if (phase === "library")
    return (
      <>
        <div className="app-ground" aria-hidden="true" />
        <Library onSignOut={signOut} onShown={libraryShown} />
        <ImportSheet />
      </>
    );

  return (
    <div className="shell">
      <div className="app-ground" aria-hidden="true" />
      {phase === "checking" && <StatusLine tone="pending">Checking your session…</StatusLine>}
      {phase === "signin" && (
        <SignIn
          onCancel={lapsed ? () => setPhase("library") : undefined}
          notice={why}
          onResult={(r) => {
            setLapsed(false);
            setCodeLapse(false);
            setCodeRequest(undefined);
            setPhase(r === "complete" ? "library" : "code");
          }}
        />
      )}
      {phase === "code" && (
        <CodeEntry
          request={codeRequest}
          onVerified={() => {
            setCodeLapse(false);
            setPhase("library");
          }}
          onCancel={() => setPhase("signin")}
        />
      )}
    </div>
  );
}
