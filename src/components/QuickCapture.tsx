import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { api, intakeApi, CAPTURE } from "../lib/ipc";
import { Picker, type PickerOption } from "./Picker";
import "../styles/intake.css";
import { composing } from "../lib/keys";

type Kind = "reminder" | "note";

/** Where a reminder made in Today or All goes in the Reminders app: the list last added to. */
const DEFAULT_LIST = "quince.reminders.defaultList";
/** The Notes folder a note made without one goes to, at Apple. */
const DEFAULT_FOLDER = "DefaultFolder-CloudKit";
const TRASH_FOLDER = "TrashFolder-CloudKit";

const stored = (key: string) => {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
};

/**
 * The quick-capture window (`skykeep --new reminder`, `--new note`): one field, Enter, and it
 * is in iCloud — a reminder in the list last added to, a note in Notes — and the window goes.
 * Escape leaves without making anything.
 */
export function QuickCapture({ initial }: { initial: Kind }) {
  const [kind, setKind] = useState<Kind>(initial);
  const [text, setText] = useState("");
  const [where, setWhere] = useState<PickerOption[] | null>(null);
  const [target, setTarget] = useState("");
  const [state, setState] = useState<"idle" | "saving" | "signin">("idle");
  const [error, setError] = useState<string | null>(null);
  const field = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    document.documentElement.dataset.app = kind === "note" ? "notes" : "reminders";
    document.title = kind === "note" ? "New Note" : "New Reminder";
  }, [kind]);

  // Asked for again from the launcher, perhaps for the other kind.
  useEffect(() => {
    const stop = listen<Kind>(CAPTURE, (e) => {
      if (e.payload === "note" || e.payload === "reminder") setKind(e.payload);
      // A fresh field each time, whatever the last one was left at.
      setText("");
      setState("idle");
      requestAnimationFrame(() => field.current?.focus());
    });
    return () => {
      void stop.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);

  useEffect(() => {
    let live = true;
    setWhere(null);
    setError(null);
    (async () => {
      try {
        if (kind === "reminder") {
          const lists = (await api.reminderLists()).filter((l) => l.enabled);
          if (!live) return;
          const kept = stored(DEFAULT_LIST);
          setWhere(lists.map((l) => ({ value: l.guid, label: l.title })));
          setTarget(lists.some((l) => l.guid === kept) ? (kept as string) : (lists[0]?.guid ?? ""));
        } else {
          const folders = (await api.noteFolders()).filter((f) => f.guid !== TRASH_FOLDER && f.guid !== DEFAULT_FOLDER);
          if (!live) return;
          setWhere([{ value: DEFAULT_FOLDER, label: "Notes" }, ...folders.map((f) => ({ value: f.guid, label: f.title }))]);
          setTarget(DEFAULT_FOLDER);
        }
      } catch {
        if (live) setWhere([]);
      }
    })();
    return () => {
      live = false;
    };
  }, [kind]);

  // Shown once in the page (window_ready, main.rs) — not after an animation frame, which a
  // hidden window never draws, so the capture waited out the three-second fallback.
  useEffect(() => {
    invoke("window_ready", { needsYou: true }).catch(() => {});
    field.current?.focus();
  }, []);

  const close = () => void intakeApi.captureClose().catch(() => window.close());

  async function save() {
    const title = text.trim();
    if (!title || state === "saving") return;
    if (kind === "reminder" && !target) return;
    setState("saving");
    setError(null);
    try {
      // Started from the launcher, Skykeep may still be signing back in: done here first, so
      // the write does not race it.
      const status = await api.status();
      if (!status.signedIn) {
        const next = status.hasStoredSession ? await api.resume() : "signin";
        if (next !== "library") {
          setState("signin");
          return;
        }
      }
      if (kind === "reminder") {
        await api.reminderCreate(target, title);
        try {
          window.localStorage.setItem(DEFAULT_LIST, target);
        } catch {
          /* forgets the choice; nothing else */
        }
      } else {
        await api.noteCreate(target === DEFAULT_FOLDER ? null : target, title);
      }
      close();
    } catch (err) {
      setState("idle");
      setError(String(err));
    }
  }

  const noun = kind === "note" ? "Note" : "Reminder";
  return (
    <main
      className="qc"
      onKeyDown={(e) => {
        if (composing(e)) return;
        if (e.key === "Escape" && !document.querySelector(".picker-menu")) {
          e.preventDefault();
          close();
        }
      }}
    >
      <form
        className="qc-row"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <span className="qc-mark" aria-hidden="true" data-kind={kind} />
        <input
          ref={field}
          className="qc-field"
          aria-label={`New ${noun}`}
          placeholder={`New ${noun}`}
          value={text}
          disabled={state === "saving"}
          onChange={(e) => setText(e.target.value)}
          autoFocus
          spellCheck
        />
      </form>
      <div className="qc-foot">
        {where && where.length > 0 ? (
          <Picker
            value={target}
            options={where}
            onChange={setTarget}
            label={kind === "note" ? "Folder" : "List"}
            disabled={state === "saving"}
          />
        ) : where ? (
          <span className="qc-note">
            {kind === "reminder" ? "No lists yet. Make one in Reminders first." : "Notes"}
          </span>
        ) : (
          <span />
        )}
        <span className="qc-note" role="status">
          {state === "saving" ? (
            "Saving…"
          ) : state === "signin" ? (
            <>
              Sign in to Skykeep first.{" "}
              <button type="button" className="link qc-link" onClick={() => void intakeApi.openMain()}>
                Open Skykeep
              </button>
            </>
          ) : error ? (
            <span className="qc-error">{error}</span>
          ) : (
            "Enter to add"
          )}
        </span>
      </div>
    </main>
  );
}
