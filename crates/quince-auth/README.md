# quince-auth

Sign-in to an Apple Account, and the session that survives a restart. Every HTTP client
Skykeep makes is built here.

- **`Client`** is one account's session: `new(dir)`, `sign_in`, the two-factor calls,
  `resume`, `reopen` (when the app opens, decides between the library, a code and sign-in,
  offline included), and `http()`, `session()` and `service_params()` for the service crates.
- **Before every sync pass** (`keep.rs`): `ensure_session` runs the heartbeat and, when Apple
  says the session lapsed, the resume, and says where it stands (`Kept`). Apple is asked at
  most once a minute, and not again once it has ended the session, until a new sign-in.
- **SRP and two-factor** (`srp.rs`, `lib.rs`) are Apple's sign-in done natively, with no
  webview. `state.rs` holds the password only until Apple accepts it.
- **Session storage** (`session.rs`, `sealed.rs`): the trust token and keys go to the system
  keyring (Secret Service, Windows Credential Manager). The cookie file is sealed with
  AES-256-GCM under a key kept there. Android passes its Keystore in through
  `PlatformSecrets` / `use_platform_secrets`.
- **Advanced Data Protection** (`pcs.rs`) asks Apple to release a service's keys to this session
  once the person approves on a trusted device.
- **Network locks:** `apple_only()` (its resolver refuses any host outside Apple's domains),
  `listed_only()`, `account_only()` and `releases::releases_only()`. Nothing else in the
  workspace should build a client.
- **Diagnostics** (`diag.rs`) is the redaction every `SKYKEEP_LOG` line passes through.

Depends on no other workspace crate. Used by the desktop app, `quince-mobile` and the probe.

## Tests

```sh
cargo nextest run -p quince-auth
```

`tests/` runs the sign-in flows against `wiremock`. The `loopback` feature, which treats this
computer as Apple's, is turned on only for this crate's own tests. `tests/apple_only.rs` guards
the network lock. The real-keyring test is `#[ignore]` and runs inside the throwaway
gnome-keyring that `check.sh` and CI set up.
