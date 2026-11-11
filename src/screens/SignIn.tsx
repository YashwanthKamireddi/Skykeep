import { Fragment, useEffect, useRef, useState } from "react";
import { Mark } from "../components/Mark";
import { Spinner } from "../components/Spinner";
import { StatusLine } from "../components/StatusLine";
import { api, messageOf, type SignInResult } from "../lib/ipc";
import { onWindows } from "../lib/platform";

// Where the copy is kept, as this system names it.
/** A path that may wrap only after a separator or a dot, as macOS wraps one, never mid-name. */
const breakable = (path: string) =>
  (path.match(/[^\\/.]*[\\/.]?/g) ?? []).filter(Boolean).map((part, i) => (
    <Fragment key={i}>
      {part}
      <wbr />
    </Fragment>
  ));

const STORE = onWindows ? "%APPDATA%\\io.github.yashwanthkamireddi.Skykeep" : "~/.local/share/io.github.yashwanthkamireddi.Skykeep";

/**
 * An error as a sentence: some of the engine's messages start in lower case and end without
 * a stop ("couldn't reach Apple: check your connection"), which reads as a log line.
 */
export function sentence(message: string) {
  const s = message.trim();
  if (!s) return "Something went wrong. Try again.";
  const capped = s[0].toUpperCase() + s.slice(1);
  return /[.!?]$/.test(capped) ? capped : `${capped}.`;
}

export function SignIn({
  onResult,
  onCancel,
  notice,
}: {
  onResult: (r: SignInResult) => void;
  /** Back to the library: offered when signing in again after Apple signed the session out. */
  onCancel?: () => void;
  /** Why the saved session could not be taken up, said in the error's place until a try. */
  notice?: string | null;
}) {
  const [appleId, setAppleId] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(notice ? sentence(notice) : null);
  const passwordRef = useRef<HTMLInputElement>(null);
  // A wait Apple's limits ask for, counted down where it is said, and the button held until
  // it is over — not a number frozen at the moment it was said.
  const [wait, setWait] = useState(0);
  useEffect(() => {
    if (wait <= 0) return;
    const t = setTimeout(() => setWait((w) => w - 1), 1000);
    return () => clearTimeout(t);
  }, [wait]);

  const ready = appleId.trim().length > 0 && password.length > 0 && !busy && wait <= 0;

  // Back to the password once a try has come back, so the next one is a type and an Enter.
  // After the fields are enabled again: a disabled field cannot take focus.
  useEffect(() => {
    if (!busy && error && appleId) passwordRef.current?.focus();
  }, [busy, error, appleId]);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!ready) return;
    setBusy(true);
    setError(null);
    try {
      onResult(await api.signIn(appleId.trim(), password));
    } catch (err) {
      const message = messageOf(err);
      const seconds = /Try again in (\d+) seconds/.exec(message);
      if (seconds) {
        setWait(Number(seconds[1]));
        setError(null);
      } else {
        // Apple's refusal names no field, so neither can this; it can say what to do.
        setError(
          /Apple (ID|Account) or password/.test(message)
            ? "Apple Account or password is incorrect. Check both and try again."
            : sentence(message),
        );
      }
    } finally {
      setPassword("");
      setBusy(false);
    }
  }

  function edit(set: (v: string) => void) {
    return (e: React.ChangeEvent<HTMLInputElement>) => {
      set(e.target.value);
      if (error) setError(null);
    };
  }

  return (
    <form className="sheet enter-sheet" onSubmit={submit} aria-busy={busy} noValidate>
      <Mark className="mark" />
      <h1>Connect your iCloud</h1>
      <p className="lede">Your iCloud, kept on this computer.</p>

      <div className={`field-group${error ? " is-invalid" : ""}`}>
        <input
          value={appleId}
          onChange={edit(setAppleId)}
          // Enter on the Apple ID with no password yet goes on to the password; with both
          // filled it signs in.
          onKeyDown={(e) => {
            if (e.key === "Enter" && !password) {
              e.preventDefault();
              if (appleId.trim()) passwordRef.current?.focus();
            }
          }}
          type="email"
          autoComplete="username"
          autoCapitalize="off"
          spellCheck={false}
          placeholder="Apple Account email"
          aria-label="Apple Account"
          aria-invalid={!!error}
          autoFocus
          disabled={busy}
        />
        <input
          ref={passwordRef}
          value={password}
          onChange={edit(setPassword)}
          type="password"
          autoComplete="current-password"
          placeholder="Password"
          aria-label="Password"
          aria-invalid={!!error}
          aria-describedby={error ? "signin-error" : undefined}
          disabled={busy}
        />
      </div>

      <button className="primary" type="submit" disabled={!ready} aria-disabled={!ready}>
        {busy && <Spinner small />}
        <span>{busy ? "Signing in" : wait > 0 ? `Try again in ${wait}s` : "Sign in"}</span>
      </button>
      {onCancel && (
        <button type="button" className="quiet" onClick={onCancel}>
          Not Now
        </button>
      )}

      {/* One slot under the button, its height kept: an error takes the assurance's place
          rather than pushing the screen about. */}
      <div className="sheet-status">
        {error ? (
          <StatusLine tone="error" id="signin-error">{error}</StatusLine>
        ) : (
          <p className="assurance">Your password never leaves this computer.</p>
        )}
        {/* Said where the password is typed, not only in the fold below: whose software
            this is. */}
        <p className="independence">Skykeep is independent and not made by Apple.</p>
      </div>

      {/*
        * What somebody is owed before they type an Apple ID into software a stranger wrote.
        *
        * Folded away rather than laid across the screen: a wall of warnings is not read, and
        * a page nobody reads discloses nothing. One line that says the important part, and
        * the rest a click away for anyone who wants it.
        */}
      <details className="disclosure">
        <summary>How Skykeep keeps you private</summary>
        <div className="disclosure-body">
          <p>
            <strong>Private by design.</strong> Your password never leaves this computer.
            Skykeep proves you know it without sending it, and keeps your session in your
            system keyring. No Skykeep account and no analytics: your iCloud travels only between this computer and Apple.
          </p>
          <p>
            <strong>Your full Apple Account password.</strong> An app-specific password opens
            only mail, contacts and calendars to other apps. Notes, Reminders, Drive and Photos
            open to a full sign-in with your two-factor code, the same one icloud.com uses.
          </p>
          <p>
            <strong>Personal Apple Accounts.</strong> Managed Apple Accounts from a school or
            workplace aren&rsquo;t supported yet.
          </p>
          <p>
            <strong>Your iCloud, right here.</strong> Your calendar, reminders, notes, contacts
            and Drive live on this computer in <code>{breakable(STORE)}</code>, so everything opens
            instantly and works offline.
          </p>
          <p>
            <strong>Sealed by Apple.</strong> Health, Messages, Keychain and your other
            end-to-end encrypted data stay protected on your Apple devices, just as Apple
            designed.
          </p>
          <p>
            <strong>Independent.</strong> Made by an independent developer, not affiliated with
            Apple. It uses the web services icloud.com uses, which Apple can change at any time.
          </p>
        </div>
      </details>
    </form>
  );
}
