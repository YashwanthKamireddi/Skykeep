#!/usr/bin/env bash
# Everything that has to be true before a change is worth keeping.
# One command, so "is it broken?" is answerable rather than a matter of trust.
set -uo pipefail
cd "$(dirname "$0")"
export PATH="$HOME/.cargo/bin:$PATH"

fail=0
step() {
  printf '\n\033[1m── %s ──\033[0m\n' "$1"; shift
  if "$@"; then printf '\033[32mok\033[0m\n'; else printf '\033[31mFAILED\033[0m\n'; fail=1; fi
}

# The machine has many cores and these checks are independent of one another, so they run side by
# side: the Rust lane beside the web lane, and the browser checks in a pool. Run one by one they
# took about forty minutes at a load of 1.5 on twelve cores. Each browser is several processes, so
# the pool is half the cores; CHECK_JOBS changes it. Checks that measure time run alone at the
# end, where a busy machine can't make them fail (run six at a time, the scroll and paging
# checks measured frames of 450–1000 ms that alone are 17–575 ms). Results print in this file's order.
JOBS=${CHECK_JOBS:-$(( $(nproc) / 2 ))}; [ "$JOBS" -ge 1 ] || JOBS=1
out=$(mktemp -d)
names=() cmds=() later=()
# Also alone: photos-strip opens a 2,000-photo library (under six browsers its window took past
# 30 s to appear), and contacts-lists times "at once" in both modes (9 and 10 Nov, each failed
# only in the pool and passed 3 of 3 alone).
timed() { case "$*" in *notes-long.mjs*|*photos-scale.mjs*|*photos-scroll.mjs*|*photos-paging.mjs*|*photos-dense-months.mjs*|*photos-levels-scale.mjs*|*photos-strip.mjs*|*contacts-lists.mjs*|*glass-frames.mjs*) return 0 ;; esac; return 1; }
# A browser check, noted here and started by run_pool once every one is known.
pooled() {
  local name=$1; shift
  if timed "$@"; then later+=("$name"$'\x1f'"$(printf '%q ' "$@")"); return; fi
  names+=("$name"); cmds+=("$(printf '%q ' "$@")")
}
# How long each browser check took in the last run (target/ is this checkout's own and never
# published). The longest start first: started in this file's order, a long suite that happened
# to come late kept the pool open with one browser running after every other was done. A check
# with no time yet (a new one) starts first.
times=target/check-times.tsv
run_pool() {
  local -A took=()
  local ms name i p live pids=()
  if [ -f "$times" ]; then while IFS=$'\t' read -r ms name; do took[$name]=$ms; done <"$times"; fi
  for i in $(for i in "${!names[@]}"; do echo "${took[${names[$i]}]:-999999} $i"; done | sort -rn | cut -d' ' -f2); do
    while :; do
      live=0; for p in "${pids[@]}"; do kill -0 "$p" 2>/dev/null && live=$((live + 1)); done
      [ "$live" -lt "$JOBS" ] && break; wait -n "${pids[@]}" 2>/dev/null
    done
    ( s=$(date +%s%N); eval "${cmds[$i]}" >"$out/$i.log" 2>&1; echo $? >"$out/$i.rc"
      echo $(( ($(date +%s%N) - s) / 1000000 )) >"$out/$i.ms" ) &
    pids+=($!)
  done
  wait "${pids[@]}" 2>/dev/null
  mkdir -p target
  for i in "${!names[@]}"; do printf '%s\t%s\n' "$(cat "$out/$i.ms" 2>/dev/null)" "${names[$i]}"; done >"$times"
}

# The app must serve its own bundle. Pointing it at a dev server made it depend on `npm run
# dev` being up and healthy, which produced both a connection error and a blank window.
step "app is standalone" bash -c '! grep -q devUrl src-tauri/tauri.conf.json'

# Only what is meant to be public is tracked (.github/published-paths says what that is).
step "nothing stray is published" bash -c '! git ls-files | grep -vxE -f <(grep -vE "^(#|$)" .github/published-paths)'
# Every workflow's expressions and inputs are what GitHub will accept (actionlint).
step "workflows lint" actionlint -shellcheck=
# The Flatpak builds offline: every package the lockfiles name must be in its sources.
step "flatpak sources current" python3 packaging/flatpak/sources-current.py
# The open-source licences the app shows (src/licences.json) are the ones the lockfiles give now.
# Writing them needs cargo-about; without it this says so and passes, and the release checks.
licences_current() {
  if ! cargo about --version >/dev/null 2>&1; then
    echo "cargo-about is not installed, so the licence list was not checked (cargo install --locked cargo-about@0.9.3)"
    return 0
  fi
  LICENCES_OUT="$out/licences.json" npm run --silent licences \
    && git show :src/licences.json | diff -q - "$out/licences.json" >/dev/null \
    || { echo "src/licences.json is out of date: run npm run licences and commit"; return 1; }
}
# cargo-about takes from half a minute to several on a busy machine, and nothing else waits for
# it: the list is written beside the committed one, not over it, so it runs behind everything and
# its result is shown at the end.
licences_current >"$out/licences.log" 2>&1 &
licences_pid=$!


# Every suite CI runs is one this gate runs too, so a suite renamed or removed here can't
# leave CI asking for it, and nothing CI checks goes unchecked here.
ci_suites() {
  local t missing=0
  for t in $(sed -n '/suites="/,/"/p' .github/workflows/ci.yml | tr -d '"' | sed 's/.*suites=//'); do
    grep -q "\.uiverify/$t\.mjs" check.sh || { echo "CI runs $t, which this gate does not"; missing=1; }
  done
  return $missing
}
step "CI's suites are the gate's" ci_suites

# The app embeds the built bundle when it compiles, so the bundle is built before the Rust lane
# starts and never changes under it. Vite alone: `npm run build` type-checks first, and the
# type check runs in the pool below, where nothing waits on it.
step "frontend build"   npx vite build

# The Rust lane runs behind everything else; its output is kept and shown at the end.
rust_lane() {
  step "rust format"      cargo fmt --all -- --check
  step "rust lint"        cargo clippy --workspace --all-targets -- -D warnings
  # The keyring code against a real gnome-keyring: a throwaway one, on a private session bus with
  # its own folders, never the person's own (session.rs, secret_service_tests).
  real_keyring() {
    local t; t=$(mktemp -d) && mkdir -p "$t/data" "$t/run" && chmod 700 "$t/run"
    XDG_DATA_HOME="$t/data" XDG_RUNTIME_DIR="$t/run" dbus-run-session -- bash -c '
      echo -n throwaway | gnome-keyring-daemon --unlock --components=secrets >/dev/null 2>&1
      export QUINCE_ISOLATED_KEYRING=$DBUS_SESSION_BUS_ADDRESS
      cargo test -q -p quince-auth --lib secret_service -- --ignored'
    local rc=$?; rm -rf "$t"; return $rc
  }
  if command -v gnome-keyring-daemon >/dev/null && command -v dbus-run-session >/dev/null; then
    step "real keyring"   real_keyring
  fi
  # nextest, as CI runs it (.config/nextest.toml): every core, hung tests stopped, slow ones named.
  step "rust tests"       cargo nextest run --workspace --lib --bins --tests --profile ci --hide-progress-bar
  # Windows-only code compiles here too, so a change to shared code cannot break the Windows
  # build unseen (cfg(windows) files are never compiled for Linux).
  step "windows compiles" cargo check --workspace --tests --target x86_64-pc-windows-gnu
  return "$fail"
}
rust_lane >"$out/rust.log" 2>&1 &
rust_pid=$!

pooled "typescript"       npx tsc --noEmit
# Every model test, the way CI's "Model tests" runs them, so a test file this list doesn't name
# can't pass here and fail there. It includes the calendar and notes layout and the notes editor
# model, which were also run on their own before it.
pooled "model tests (all)" bash -c 'node --test .uiverify/*.test.mjs'

# The browser checks need the built bundle served.
# CHECK_PORT moves it, so a gate can run while another worktree's preview holds 4173.
PORT=${CHECK_PORT:-4173}
export URL="http://localhost:$PORT"
npx vite preview --port "$PORT" --strictPort >"$out/preview.log" 2>&1 &
preview=$!
trap 'kill $preview 2>/dev/null' EXIT
for _ in $(seq 20); do curl -sf -o /dev/null "$URL/" && break; sleep 0.5; done

export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=1
step "webkit: set up"   .uiverify/webkit-libs.sh

pooled "content policy"   node .uiverify/csp.mjs
pooled "sign out"         node .uiverify/signout.mjs
pooled "calendar edits"   node .uiverify/calendar-edits.mjs
pooled "sidebar button"   node .uiverify/side-pane.mjs
pooled "popover on screen" node .uiverify/calendar-popover-fit.mjs
pooled "shortcuts fit"    node .uiverify/shortcuts-fit.mjs
pooled "webkit: shortcuts fit" env ENGINE=webkit node .uiverify/shortcuts-fit.mjs
pooled "notes add link"     node .uiverify/notes-add-link.mjs
pooled "launcher status"    node .uiverify/launcher-status.mjs
pooled "webkit: launcher status" env ENGINE=webkit node .uiverify/launcher-status.mjs
pooled "german"             node .uiverify/german.mjs
pooled "webkit: german"     env ENGINE=webkit node .uiverify/german.mjs
pooled "webkit: notes add link" env ENGINE=webkit node .uiverify/notes-add-link.mjs
pooled "updates"          node .uiverify/updates.mjs
pooled "licences"         node .uiverify/licences.mjs
pooled "webkit: licences" env ENGINE=webkit node .uiverify/licences.mjs
pooled "ui renders"       node .uiverify/shots.mjs
pooled "nav behaviour"    node .uiverify/assert.mjs
pooled "accessibility"    node .uiverify/a11y.mjs
pooled "keyboard only"    node .uiverify/keyboard.mjs
pooled "somewhere else"   node .uiverify/elsewhere.mjs
pooled "writing a note"   node .uiverify/notes-compose.mjs
pooled "notes saved on quit" node .uiverify/notes-flush.mjs
pooled "webkit: notes saved on quit" env ENGINE=webkit node .uiverify/notes-flush.mjs
pooled "photos screen quiet" node .uiverify/photos-screen-quiet.mjs
pooled "webkit: photos screen quiet" env ENGINE=webkit node .uiverify/photos-screen-quiet.mjs
pooled "deleting a note"  node .uiverify/notes-trash.mjs
pooled "pinning a note"   node .uiverify/notes-pin.mjs
pooled "a long note"      node .uiverify/notes-long.mjs
pooled "uploading"        node .uiverify/drive-upload.mjs
pooled "calendar grids"   node .uiverify/calendar.mjs
pooled "calendar accounts" node .uiverify/calendar-accounts.mjs
pooled "every size"       node .uiverify/sizes.mjs
pooled "narrow windows"   node .uiverify/compact.mjs
pooled "a screen failing"  node .uiverify/boundary.mjs
pooled "coming back"       node .uiverify/resume.mjs
pooled "birthdays, holidays" node .uiverify/occasions.mjs
pooled "drive folders"    node .uiverify/drive-nav.mjs
pooled "birthdays in contacts" node .uiverify/contact-birthday.mjs
pooled "contact labels"    node .uiverify/contacts-labels.mjs
pooled "calendar go to date" node .uiverify/calendar-goto.mjs
pooled "calendar calendars" node .uiverify/calendar-collections.mjs
pooled "drive names"       node .uiverify/drive-names.mjs
pooled "notes autoformat"  node .uiverify/notes-autoformat.mjs
pooled "reminder ticks"    node .uiverify/reminders-tick.mjs
pooled "waiting to sync"   node .uiverify/outbox-pending.mjs
pooled "photos"            node .uiverify/photos.mjs
pooled "photos preparing"  node .uiverify/photos-preparing.mjs
pooled "photo albums"      node .uiverify/photos-albums.mjs
pooled "photos at scale"   node .uiverify/photos-scale.mjs
pooled "photos memory"     node .uiverify/photos-memory.mjs
pooled "webkit: photos memory" env ENGINE=webkit node .uiverify/photos-memory.mjs
pooled "photos scrolling"  node .uiverify/photos-scroll.mjs
pooled "webkit: photos scrolling" env ENGINE=webkit node .uiverify/photos-scroll.mjs
pooled "photos paging"     node .uiverify/photos-paging.mjs
pooled "photos dense months" node .uiverify/photos-dense-months.mjs
pooled "webkit: dense months" env ENGINE=webkit node .uiverify/photos-dense-months.mjs
pooled "photos levels"     env ENGINE=webkit node .uiverify/photos-levels-scale.mjs
pooled "photos years, months" node .uiverify/photos-levels.mjs
pooled "webkit: years, months" env ENGINE=webkit node .uiverify/photos-levels.mjs
pooled "photos drag"       node .uiverify/photos-drag.mjs
pooled "drag pictures"     node .uiverify/drag-picture.mjs
pooled "photos keys"       node .uiverify/photos-keys.mjs
pooled "photos sort"       node .uiverify/photos-sort.mjs
pooled "slideshow"         node .uiverify/photos-show.mjs
pooled "photo captions"    node .uiverify/photos-caption.mjs
pooled "photos real faults" node .uiverify/photos-real.mjs
pooled "whole photos"      node .uiverify/photos-fit.mjs
pooled "star ratings"      node .uiverify/photos-rating.mjs
pooled "viewer, real sizes" node .uiverify/photos-viewer-real.mjs
pooled "filmstrip"         node .uiverify/photos-strip.mjs
pooled "place & keywords"  node .uiverify/photos-info.mjs
pooled "download options"  node .uiverify/photos-download.mjs
pooled "adjust date"       node .uiverify/photos-date.mjs
pooled "photo search"      node .uiverify/photos-search.mjs
pooled "full quality"      node .uiverify/photos-quality.mjs
pooled "library switcher"  node .uiverify/photos-library-switcher.mjs
pooled "notes delete"      node .uiverify/notes-delete.mjs
pooled "notes no text"     node .uiverify/notes-no-text.mjs
pooled "empty today"     node .uiverify/reminders-empty.mjs
pooled "calendar keys"   node .uiverify/calendar-keys.mjs
pooled "reminders instant" node .uiverify/reminders-instant.mjs
pooled "app window quiet" node .uiverify/app-window-quiet.mjs
pooled "webkit: app window quiet" env ENGINE=webkit node .uiverify/app-window-quiet.mjs
pooled "reminders current" node .uiverify/reminders-current.mjs
pooled "webkit: reminders current" env ENGINE=webkit node .uiverify/reminders-current.mjs
pooled "calendar instant"  node .uiverify/calendar-instant.mjs
pooled "notes instant"     node .uiverify/notes-instant.mjs
pooled "drive sort"        node .uiverify/drive-sort.mjs
pooled "drive trash"       node .uiverify/drive-trash.mjs
pooled "drive info"        node .uiverify/drive-info.mjs
pooled "drive busy quiet"  node .uiverify/drive-busy-quiet.mjs
pooled "drive delete"      node .uiverify/drive-delete.mjs
pooled "drive delete changed" node .uiverify/drive-delete-changed.mjs
pooled "notes folders"     node .uiverify/notes-folders.mjs
pooled "list info"         node .uiverify/reminders-list-info.mjs
pooled "reminder date"     node .uiverify/reminders-date.mjs
pooled "reminder keys"     node .uiverify/reminders-keys.mjs
pooled "contacts order"    node .uiverify/contacts-order.mjs
pooled "contacts lists"    node .uiverify/contacts-lists.mjs
pooled "drive keys"        node .uiverify/drive-keys.mjs
pooled "drive drag"        node .uiverify/drive-drag.mjs
pooled "drive recents"     node .uiverify/drive-recents.mjs
pooled "drive gone folder" node .uiverify/drive-gone.mjs
pooled "drive move back"   node .uiverify/drive-move-back.mjs
pooled "settings"          node .uiverify/settings.mjs
pooled "calendar day steps" node .uiverify/calendar-daystep.mjs
pooled "pending writes"    node .uiverify/pending-writes.mjs
pooled "row hover"         node .uiverify/row-hover.mjs
pooled "reminders switch"  node .uiverify/reminders-switch.mjs
pooled "upload cancel"     node .uiverify/drive-upload-cancel.mjs
pooled "folder download"   node .uiverify/drive-download.mjs
pooled "note row pictures" node .uiverify/notes-list-pictures.mjs
pooled "webkit: note row pictures" env ENGINE=webkit node .uiverify/notes-list-pictures.mjs
pooled "contacts by card"  node .uiverify/contacts-items.mjs
pooled "drive widths"      node .uiverify/drive-widths.mjs
pooled "glass rules"       node .uiverify/glass-motion.mjs
pooled "webkit: glass rules" env ENGINE=webkit node .uiverify/glass-motion.mjs
pooled "glass frames"      node .uiverify/glass-frames.mjs
pooled "webkit: glass frames" env ENGINE=webkit node .uiverify/glass-frames.mjs
pooled "open at login"     node .uiverify/login-item.mjs
pooled "update this app"   node .uiverify/footer-refresh.mjs
pooled "webkit: update this app" env ENGINE=webkit node .uiverify/footer-refresh.mjs
pooled "notes tables"      node .uiverify/notes-table.mjs
pooled "reminder menu"     node .uiverify/reminders-context.mjs
pooled "notes media"       node .uiverify/notes-media.mjs
pooled "webkit: tables"    env ENGINE=webkit node .uiverify/notes-table.mjs
pooled "webkit: labels"    env ENGINE=webkit node .uiverify/contacts-labels.mjs
pooled "webkit: go to date" env ENGINE=webkit node .uiverify/calendar-goto.mjs
pooled "webkit: calendars"  env ENGINE=webkit node .uiverify/calendar-collections.mjs
pooled "webkit: drive names" env ENGINE=webkit node .uiverify/drive-names.mjs
pooled "webkit: autoformat" env ENGINE=webkit node .uiverify/notes-autoformat.mjs
pooled "webkit: ticks"     env ENGINE=webkit node .uiverify/reminders-tick.mjs
pooled "webkit: unsent"    env ENGINE=webkit node .uiverify/outbox-pending.mjs
pooled "webkit: photos"    env ENGINE=webkit node .uiverify/photos.mjs
pooled "app windows"       node .uiverify/app-window.mjs
pooled "webkit: app windows" env ENGINE=webkit node .uiverify/app-window.mjs
pooled "new event"         node .uiverify/calendar-new-event.mjs
pooled "webkit: new event" env ENGINE=webkit node .uiverify/calendar-new-event.mjs
pooled "notes attach"      node .uiverify/notes-attach-feedback.mjs
pooled "webkit: notes attach" env ENGINE=webkit node .uiverify/notes-attach-feedback.mjs
pooled "webkit: albums"    env ENGINE=webkit node .uiverify/photos-albums.mjs
pooled "webkit: at scale"  env ENGINE=webkit node .uiverify/photos-scale.mjs
pooled "webkit: drag"      env ENGINE=webkit node .uiverify/photos-drag.mjs
pooled "webkit: drag pictures" env ENGINE=webkit node .uiverify/drag-picture.mjs
# As Skykeep's window is on Windows, where no drag event of the engine's reaches the page.
pooled "windows: photos drag"    env WEBVIEW2=1 node .uiverify/photos-drag.mjs
pooled "windows: reminder drag"  env WEBVIEW2=1 node .uiverify/reminders-drag-list.mjs
pooled "windows: contacts drag"  env WEBVIEW2=1 node .uiverify/contacts-lists.mjs
pooled "webkit: keys"      env ENGINE=webkit node .uiverify/photos-keys.mjs
pooled "webkit: sort"      env ENGINE=webkit node .uiverify/photos-sort.mjs
pooled "webkit: slideshow" env ENGINE=webkit node .uiverify/photos-show.mjs
pooled "webkit: captions"  env ENGINE=webkit node .uiverify/photos-caption.mjs
pooled "webkit: real faults" env ENGINE=webkit node .uiverify/photos-real.mjs
pooled "webkit: whole photos" env ENGINE=webkit node .uiverify/photos-fit.mjs
pooled "webkit: ratings"   env ENGINE=webkit node .uiverify/photos-rating.mjs
pooled "webkit: viewer sizes" env ENGINE=webkit node .uiverify/photos-viewer-real.mjs
pooled "webkit: filmstrip" env ENGINE=webkit node .uiverify/photos-strip.mjs
pooled "webkit: place & keywords" env ENGINE=webkit node .uiverify/photos-info.mjs
pooled "webkit: downloads" env ENGINE=webkit node .uiverify/photos-download.mjs
pooled "webkit: adjust date" env ENGINE=webkit node .uiverify/photos-date.mjs
pooled "webkit: search"    env ENGINE=webkit node .uiverify/photos-search.mjs
pooled "webkit: full quality" env ENGINE=webkit node .uiverify/photos-quality.mjs
pooled "webkit: delete"    env ENGINE=webkit node .uiverify/notes-delete.mjs
pooled "webkit: row hover" env ENGINE=webkit node .uiverify/row-hover.mjs
pooled "reminder colours"  node .uiverify/calendar-reminder-colour.mjs
pooled "calendar dblclick"  node .uiverify/calendar-dblclick.mjs
pooled "menus"             node .uiverify/menus.mjs
pooled "webkit: menus"     env ENGINE=webkit node .uiverify/menus.mjs
pooled "contacts vcard"    node .uiverify/contacts-vcard.mjs
pooled "contacts select"   node .uiverify/contacts-select.mjs
pooled "contacts search"   node .uiverify/contacts-search.mjs
pooled "webkit: contacts search" env ENGINE=webkit node .uiverify/contacts-search.mjs
pooled "contacts instant"  node .uiverify/contacts-instant.mjs
pooled "calendar alerts"   node .uiverify/calendar-alerts.mjs
pooled "calendar repeat"   node .uiverify/calendar-repeat.mjs
pooled "reminder add/done"  node .uiverify/reminders-add-done.mjs
pooled "notes formatted"   node .uiverify/notes-formatted.mjs
pooled "notes merge"       node .uiverify/notes-merge.mjs

# The same screens in WebKit, the engine Quince draws in. Chromium above catches most things;
# this catches what only WebKit does — pointer capture, blur on removal, its own layout.
pooled "webkit: nav"      env ENGINE=webkit node .uiverify/assert.mjs
pooled "webkit: keyboard" env ENGINE=webkit node .uiverify/keyboard.mjs
# One check each: run as one chain, the chain held a timed suite (notes-long), so all four waited
# for the quiet end and ran one after another there.
pooled "webkit: notes"    env ENGINE=webkit node .uiverify/notes-compose.mjs
pooled "webkit: deleting a note" env ENGINE=webkit node .uiverify/notes-trash.mjs
pooled "webkit: pinning a note" env ENGINE=webkit node .uiverify/notes-pin.mjs
pooled "webkit: a long note" env ENGINE=webkit node .uiverify/notes-long.mjs
pooled "webkit: drive"    env ENGINE=webkit node .uiverify/drive-upload.mjs
pooled "webkit: calendar" env ENGINE=webkit node .uiverify/calendar.mjs
pooled "webkit: sizes"    env ENGINE=webkit node .uiverify/sizes.mjs
pooled "webkit: narrow"   env ENGINE=webkit node .uiverify/compact.mjs
pooled "webkit: failing"  env ENGINE=webkit node .uiverify/boundary.mjs
pooled "webkit: back"     env ENGINE=webkit node .uiverify/resume.mjs
pooled "webkit: occasions" env ENGINE=webkit node .uiverify/occasions.mjs
pooled "webkit: drive nav" env ENGINE=webkit node .uiverify/drive-nav.mjs
pooled "webkit: download" env ENGINE=webkit node .uiverify/drive-download.mjs
pooled "sign-in keyring"  node .uiverify/signin-keyring.mjs
pooled "lapse reason"     node .uiverify/lapse-reason.mjs
pooled "device approval"  node .uiverify/device-approval.mjs
pooled "code only when asked" node .uiverify/code-lapse.mjs
pooled "new iCloud terms"  node .uiverify/icloud-terms.mjs
pooled "security keys"     node .uiverify/security-key.mjs
pooled "webkit: approval" env ENGINE=webkit node .uiverify/device-approval.mjs
pooled "calendar undo"    node .uiverify/calendar-undo.mjs
pooled "backup"           node .uiverify/settings-backup.mjs
pooled "webkit: backup"   env ENGINE=webkit node .uiverify/settings-backup.mjs

# One browser of each engine serves the whole pool (browsers.mjs): each suite connects to it
# rather than starting its own, so the pool no longer starts and stops two hundred browsers.
# Stopped before the timed checks, which start their own browsers as before.
node .uiverify/browsers.mjs "$out/browsers.env" >"$out/browsers.log" 2>&1 &
browsers=$!
trap 'kill $preview $browsers 2>/dev/null' EXIT
for _ in $(seq 600); do [ -f "$out/browsers.env" ] && break; kill -0 $browsers 2>/dev/null || break; sleep 0.1; done
if [ -f "$out/browsers.env" ]; then set -a; . "$out/browsers.env"; set +a
else echo "The shared browsers did not start, so each check starts its own:"; cat "$out/browsers.log"; fi
run_pool
kill $browsers 2>/dev/null; wait $browsers 2>/dev/null
unset PW_CHROMIUM_WS PW_WEBKIT_WS
for i in "${!names[@]}"; do
  printf '\n\033[1m── %s ──\033[0m\n' "${names[$i]}"
  if [ "$(cat "$out/$i.rc" 2>/dev/null)" = 0 ]; then printf '\033[32mok\033[0m\n'
  else cat "$out/$i.log"; printf '\033[31mFAILED\033[0m\n'; fail=1; fi
done
# The checks that measure time, alone on a quiet machine (the Rust lane may still be running, so
# it is waited for first).
wait "$licences_pid"; licences_rc=$?
step "licence list current" bash -c "cat '$out/licences.log'; exit $licences_rc"
wait "$rust_pid"; rust_rc=$?
cat "$out/rust.log"; [ "$rust_rc" -eq 0 ] || fail=1
# Each engine warmed first: the first launch after the build measured a cold disk, not the app.
step "warm the browsers" bash -c 'node .uiverify/warm.mjs && ENGINE=webkit node .uiverify/warm.mjs'
for t in "${later[@]}"; do eval "step \"\${t%%$'\x1f'*}\" ${t#*$'\x1f'}"; done
rm -rf "$out"

printf '\n'
if [ "$fail" -eq 0 ]; then printf '\033[32mAll checks passed.\033[0m\n'; else printf '\033[31mSome checks failed.\033[0m\n'; fi
exit "$fail"
