import { useEffect, useRef, useState } from "react";
import { LockKeyhole } from "lucide-react";
import { useI18n } from "../../lib/i18n";
import { messageOf, notesLockApi, type UnlockedNote } from "../../lib/ipc";
import { Spinner } from "../../components/Spinner";

/**
 * A locked note, as Notes shows one: a lock, one line, the password field and Unlock, the hint
 * when asked for. The password lives in this field only while it is typed — it is sent once,
 * then cleared, and never kept by the window (the backend holds it in memory for the session,
 * so the same note opens again without asking until Lock Now or quitting).
 */
export function LockScreen({
  guid,
  title,
  autoFocus,
  onOpen,
}: {
  guid: string;
  title: string;
  /** Into the password field at once — not while someone is moving through the list. */
  autoFocus: boolean;
  onOpen: (opened: UnlockedNote) => void;
}) {
  const { t } = useI18n();
  const field = useRef<HTMLInputElement | null>(null);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  /** undefined: not asked; null: there is none; a string: the hint. */
  const [hint, setHint] = useState<string | null | undefined>(undefined);
  const [hintBusy, setHintBusy] = useState(false);

  // Another note is another lock: nothing typed or said for one carries to the next.
  useEffect(() => {
    setPassword("");
    setProblem(null);
    setHint(undefined);
    setBusy(false);
    if (autoFocus) field.current?.focus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [guid]);

  const unlock = async () => {
    if (!password || busy) return;
    const typed = password;
    // Out of the field the moment it is sent: nothing lingers in the page to be read later.
    setPassword("");
    setBusy(true);
    setProblem(null);
    try {
      const opened = await notesLockApi.noteUnlock(guid, typed);
      onOpen(opened);
    } catch (e) {
      const said = messageOf(e);
      setProblem(/password/i.test(said) ? t("notes.wrongPassword") : said);
      setBusy(false);
      window.requestAnimationFrame(() => field.current?.focus());
    }
  };

  const showHint = async () => {
    setHintBusy(true);
    try {
      setHint((await notesLockApi.noteLockHint(guid)) ?? null);
    } catch (e) {
      setProblem(messageOf(e));
    } finally {
      setHintBusy(false);
    }
  };

  return (
    <div className="note-lock" role="group" aria-label={t("notes.lockedAria", { name: title || t("notes.note") })}>
      <span className="note-lock-glyph" aria-hidden="true">
        <LockKeyhole size={26} strokeWidth={1.75} />
      </span>
      <h2 className="note-lock-title">{t("notes.lockedTitle")}</h2>
      <p className="note-lock-lede">{t("notes.lockedLede")}</p>
      <form
        className="note-lock-form"
        onSubmit={(e) => {
          e.preventDefault();
          void unlock();
        }}
      >
        <input
          ref={field}
          className="field note-lock-field"
          type="password"
          autoComplete="off"
          spellCheck={false}
          placeholder={t("notes.password")}
          aria-label={t("notes.notePassword")}
          aria-invalid={problem ? true : undefined}
          aria-describedby={problem ? "note-lock-problem" : undefined}
          value={password}
          disabled={busy}
          onChange={(e) => {
            setPassword(e.currentTarget.value);
            if (problem) setProblem(null);
          }}
        />
        <button type="submit" className="primary note-lock-open" disabled={!password || busy}>
          {busy ? <Spinner small /> : null}
          {busy ? t("notes.unlocking") : t("notes.unlock")}
        </button>
      </form>
      <p id="note-lock-problem" className="note-lock-problem" role="alert">
        {problem ?? ""}
      </p>
      {hint === undefined ? (
        <button type="button" className="link note-lock-hint-ask" onClick={() => void showHint()} disabled={hintBusy}>
          {t("notes.showHint")}
        </button>
      ) : (
        <p className="note-lock-hint">{hint ? t("notes.hint", { hint }) : t("notes.noHint")}</p>
      )}
    </div>
  );
}
