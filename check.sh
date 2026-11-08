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
names=() pids=() later=()
timed() { case "$*" in *notes-long.mjs*|*photos-scale.mjs*|*photos-scroll.mjs*|*photos-paging.mjs*|*photos-levels-scale.mjs*) return 0 ;; esac; return 1; }
pooled() {
  local name=$1; shift
  if timed "$@"; then later+=("$name"$'\x1f'"$(printf '%q ' "$@")"); return; fi
  local i=${#names[@]}; names+=("$name")
  while :; do
    local live=0 p; for p in "${pids[@]}"; do kill -0 "$p" 2>/dev/null && live=$((live + 1)); done
    [ "$live" -lt "$JOBS" ] && break; wait -n "${pids[@]}" 2>/dev/null
  done
  ( "$@" >"$out/$i.log" 2>&1; echo $? >"$out/$i.rc" ) &
  pids+=($!)
}

# The app must serve its own bundle. Pointing it at a dev server made it depend on `npm run
# dev` being up and healthy, which produced both a connection error and a blank window.
step "app is standalone" bash -c '! grep -q devUrl src-tauri/tauri.conf.json'

# Only what is meant to be public is tracked (.github/published-paths says what that is).
step "nothing stray is published" bash -c '! git ls-files | grep -vxE -f <(grep -vE "^(#|$)" .github/published-paths)'


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
# starts and never changes under it.
step "frontend build"   npm run build

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
  step "rust tests"       cargo test --workspace --quiet
  return "$fail"
}
rust_lane >"$out/rust.log" 2>&1 &
rust_pid=$!

step "typescript"       npx tsc --noEmit
step "calendar layout"  node --test .uiverify/calendar-model.test.mjs
step "notes layout"     node --test .uiverify/notes-model.test.mjs
step "notes editor model" node --test .uiverify/notes-rich.test.mjs
# Every model test, the way CI's "Model tests" runs them, so a test file this list doesn't name
# can't pass here and fail there.
step "model tests (all)" bash -c 'node --test .uiverify/*.test.mjs'

# The browser checks need the built bundle served.
npx vite preview --port 4173 --strictPort >/tmp/quince-preview.log 2>&1 &
preview=$!
trap 'kill $preview 2>/dev/null' EXIT
for _ in $(seq 20); do curl -sf -o /dev/null http://localhost:4173/ && break; sleep 0.5; done

export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=1
step "webkit: set up"   .uiverify/webkit-libs.sh

pooled "content policy"   node .uiverify/csp.mjs
pooled "sign out"         node .uiverify/signout.mjs
pooled "calendar edits"   node .uiverify/calendar-edits.mjs
pooled "updates"          node .uiverify/updates.mjs
pooled "ui renders"       node .uiverify/shots.mjs
pooled "nav behaviour"    node .uiverify/assert.mjs
pooled "accessibility"    node .uiverify/a11y.mjs
pooled "keyboard only"    node .uiverify/keyboard.mjs
pooled "somewhere else"   node .uiverify/elsewhere.mjs
pooled "writing a note"   node .uiverify/notes-compose.mjs
pooled "deleting a note"  node .uiverify/notes-trash.mjs
pooled "pinning a note"   node .uiverify/notes-pin.mjs
pooled "a long note"      node .uiverify/notes-long.mjs
pooled "uploading"        node .uiverify/drive-upload.mjs
pooled "calendar grids"   node .uiverify/calendar.mjs
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
pooled "photos levels"     env ENGINE=webkit node .uiverify/photos-levels-scale.mjs
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
pooled "settings"          node .uiverify/settings.mjs
pooled "calendar day steps" node .uiverify/calendar-daystep.mjs
pooled "pending writes"    node .uiverify/pending-writes.mjs
pooled "row hover"         node .uiverify/row-hover.mjs
pooled "reminders switch"  node .uiverify/reminders-switch.mjs
pooled "upload cancel"     node .uiverify/drive-upload-cancel.mjs
pooled "folder download"   node .uiverify/drive-download.mjs
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
pooled "calendar alerts"   node .uiverify/calendar-alerts.mjs
pooled "calendar repeat"   node .uiverify/calendar-repeat.mjs
pooled "reminder add/done"  node .uiverify/reminders-add-done.mjs
pooled "notes formatted"   node .uiverify/notes-formatted.mjs
pooled "notes merge"       node .uiverify/notes-merge.mjs

# The same screens in WebKit, the engine Quince draws in. Chromium above catches most things;
# this catches what only WebKit does — pointer capture, blur on removal, its own layout.
pooled "webkit: nav"      env ENGINE=webkit node .uiverify/assert.mjs
pooled "webkit: keyboard" env ENGINE=webkit node .uiverify/keyboard.mjs
pooled "webkit: notes"    bash -c 'ENGINE=webkit node .uiverify/notes-compose.mjs && ENGINE=webkit node .uiverify/notes-trash.mjs && ENGINE=webkit node .uiverify/notes-pin.mjs && ENGINE=webkit node .uiverify/notes-long.mjs'
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


wait "${pids[@]}" 2>/dev/null
for i in "${!names[@]}"; do
  printf '\n\033[1m── %s ──\033[0m\n' "${names[$i]}"
  if [ "$(cat "$out/$i.rc" 2>/dev/null)" = 0 ]; then printf '\033[32mok\033[0m\n'
  else cat "$out/$i.log"; printf '\033[31mFAILED\033[0m\n'; fail=1; fi
done
# The checks that measure time, alone on a quiet machine (the Rust lane may still be running, so
# it is waited for first).
wait "$rust_pid"; rust_rc=$?
cat "$out/rust.log"; [ "$rust_rc" -eq 0 ] || fail=1
for t in "${later[@]}"; do eval "step \"\${t%%$'\x1f'*}\" ${t#*$'\x1f'}"; done
rm -rf "$out"

printf '\n'
if [ "$fail" -eq 0 ]; then printf '\033[32mAll checks passed.\033[0m\n'; else printf '\033[31mSome checks failed.\033[0m\n'; fi
exit "$fail"
