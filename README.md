# MikroCockpit

A native macOS menu-bar controller for MikroTik RouterOS routers: tray status, an
AppKit dashboard window, and an optional local web panel.

MikroCockpit is a *remote control*, not a WinBox replacement. It covers the
frequent operations — see router state, switch VPN exits, flip whole-LAN or
per-device direct access, adjust volume-like controls for a local audio device —
and leaves configuration editing to WinBox/WebFig.

> **Status: v0.1, first release. This is a rough first cut.**
> It was built around one real setup and is honest about its limits rather than
> pretending to be finished. Read [Known limitations](#known-limitations) before
> relying on it. Expect rough edges, macOS-only support, and features that exist
> in the menu but are deliberately narrow. Bug reports and patches are welcome.

[Русская версия README](README.ru.md)

## What works

- **SSH with your existing key.** Strict `known_hosts` checking, no passwords
  stored, no unknown-key auto-accept. First contact with an unseen router must be
  verified manually with plain `ssh` first.
- **Router state at a glance**: system info, WireGuard tunnels, DHCP leases,
  routes of the selected table, interfaces.
- **VPN exit selection** through the mihomo/Clash REST API; `DIRECT`, circular and
  unknown groups are rejected.
- **Route priority** for existing static default routes in a VPN table; the `main`
  table is never modified.
- **WireGuard toggles** for explicitly listed tunnels; tunnels listed as
  *protected* are read-only.
- **Direct-access exceptions** (IPv4 destination list) and **per-device / whole-LAN
  direct mode**: 15 minutes, 1 hour, or permanent. The last two require the
  MikroCockpit policy to be installed and verified on the router first.
- **Local web panel** served from the menu — same actions and state as the tray.
- **Customization**: dark/light/system theme, hideable panel sections, tray
  section order and composition, tray caption (exit, CPU, profile name, or icon
  only). Preferences persist.

Mihomo is optional. An empty `mihomo_url` disables that integration entirely.

## What it is not

- Not a replacement for WinBox/WebFig. No arbitrary config editing, no CLI passthrough.
- Not a VPN provider manager: it never creates WG peers, keys, or providers.
- Not multi-vendor: RouterOS 7 only, and only with `:serialize to=json` support
  (verified on 7.24.x). RouterOS 6 is not supported.

## Requirements

- macOS 12+ (currently Apple Silicon; the build script builds for the host arch)
- Rust stable toolchain and Xcode Command Line Tools
- A RouterOS 7 router you can reach over SSH with a key

## Build and run

```sh
git clone https://github.com/Recluse/MikroCockpit.git
cd MikroCockpit
cp settings.example.json settings.json   # then edit your profile
./build.sh
open dist/MikroCockpit.app
```

Notes:

- `dist/settings.json` lives **next to** the `.app`, not inside the signed bundle.
  Rebuilding does not overwrite an existing profile.
- State, panel preferences, lock file and audit log live in `dist/state/`. Do not
  separate the `.app` from its profile directory.
- Copying to `Applications`, login autostart, Developer ID signing and notarization
  are **not** set up yet; local builds are ad-hoc signed. Gatekeeper may complain
  about unsigned downloads — right-click → Open, or rebuild locally.
- Another profile can be selected with
  `MIKROCOCKPIT_CONFIG=/absolute/path/profile.json`.
- The mihomo bearer token is **not** stored in JSON: set `mihomo_secret_env` to the
  name of an environment variable that holds it.

## Profile reference

`settings.example.json` is a sanitized example. The fields:

| Field | Meaning |
|---|---|
| `host`, `user`, `identity` | Router SSH target and key path |
| `lan_cidrs` | One or more IPv4 LAN networks |
| `lan_interface_list` | RouterOS interface list containing the LAN side |
| `wan_interfaces` | WAN interfaces (members of the app's WAN list) |
| `route_table` | VPN routing table the app manages |
| `bypass_list` | Address list of destinations that must go direct |
| `managed_wireguard` | Tunnels the app may toggle |
| `protected_wireguard` | Tunnels shown but never toggled |
| `extra_direct_rules` | Explicit extra IPv4 exceptions: address, protocol, ports |
| `mihomo_url`, `mihomo_group` | Optional mihomo API (empty = disabled) |
| `poll_seconds` | Poll interval, 5…300 |
| `tray_*`, `status_*` | Tray and status-line appearance |

Router-side preconditions: the chosen VPN table and a LAN mark-routing rule with
`dst-address-list=!<bypass_list>` must already exist. The app does not create
them.

## Installing the direct-access policy

The app **never installs rules by itself**. Generate a plan, review it, approve it,
and only then import it:

```sh
./dist/MikroCockpit.app/Contents/MacOS/MikroCockpit --policy-plan > policy.rsc
```

Do **not** import `router-setup.example.rsc` blindly — it is an example generated
from a public profile. The generated plan checks the existing mark-routing and
absence of active fasttrack, makes a backup/export, then adds the app's own rules.
Re-importing stops deliberately instead of creating duplicates.
`router-rollback.rsc` removes only MikroCockpit's rules and restores the previous
firewall; destination exceptions you created yourself are kept.

What the policy does: a WAN guard chain placed before `established/related` that
allows only direct devices, bypass-list destinations, and explicit
`extra_direct_rules`, then drops; a mangle exception for direct devices before VPN
marking; forwarded IPv6 on the configured WANs blocked. This is an IPv4
direct-access mode, **not** a firewall replacement — DHCP/DNS/router management
and VPN transport are unaffected.

Timers use RouterOS `address-list timeout`, so they survive without the app and
expire on their own. Permanent exceptions survive reboots.

## Security model

- Network operations run on a single worker thread; the macOS event loop is never
  blocked. Before any change a fresh snapshot is read, and afterwards the result is
  verified. A timeout means *unknown result* — there is no automatic retry.
- RouterOS commands are built through a single escaping choke point plus
  config-string validation; arbitrary scripts are never accepted.
- The web server listens on `127.0.0.1` only, on an ephemeral port. The API
  requires a random token, mutations additionally check `Origin`/`Host`; no CORS,
  no CLI passthrough. The token travels in the URL fragment and is stripped from
  the address bar. Router-provided strings (DHCP hostnames) are rendered as text,
  never HTML.
- An audit log records intent first, then outcome.
- Recommended: a dedicated RouterOS user with only the `ssh/read/write/test`
  rights your profile needs. Menu restrictions are **not** server-side ACLs.

## Verification

```sh
cargo test
cargo clippy --all-targets -- -D warnings
python3 check_web.py       # real local API in demo mode
python3 check_memory.py    # bounded RSS over repeated menu rebuilds
./dist/MikroCockpit.app/Contents/MacOS/MikroCockpit --check    # read-only
./dist/MikroCockpit.app/Contents/MacOS/MikroCockpit --smoke    # 3s demo tray
```

`--check` only reads the router. `--demo` performs no network actions. Unit tests
cover CLI escaping, CIDR handling, the guard placement, WAN mapping, the nested
DIRECT ban, HTTP auth/Origin/command allowlist, port-list validation and rate
counting. Mutations against a real router still need manual verification after
deploy.

CI runs tests, clippy, build and the two check scripts on every push.

## Known limitations

Honest list for v0.1 — this is the "first release is rough" part:

- **macOS only.** The router/policy/web logic is portable Rust, but
  `build.rs`/`native/dashboard.m` require AppKit, dialogs use `osascript`, and
  helper paths (`/usr/bin/ssh`, `uuidgen`, `open`) are hardcoded. Windows/Linux
  support is planned, not present.
- **No notarization, no autostart installer, single-arch builds.** Local ad-hoc
  signing only.
- **No integration tests against a real router** in CI; mutations are manually
  verified.
- **Native preferences window** cannot drag-reorder tray sections (the web panel
  can).
- **Some checks are structural, not behavioural**: `check_memory.py` and
  `--native-smoke` validate rendering and memory, not router outcomes.
- **Internet reachability indicators are not a leak test.** Purple means reachable
  plus an active non-WAN default in the configured VPN table. The VPN public IP
  probe runs *on the router* against `api.ipify.org`.
- **IPv4-focused.** Forwarded IPv6 on the configured WANs is blocked by the policy,
  but IPv6 VPN, VRFs, dynamic routing and ECMP are not supported.
- **Error handling degrades loudly, not gracefully** in a few native-menu paths
  (`expect` on menu construction): a failure there panics rather than showing a
  degraded tray.

## Interface language

The tray, native dialogs and dashboard are English-only for now. `src/locale.rs`
detects the macOS preferred language at startup and falls back to English;
browser language cannot override it. Translations are deferred until the feature
set stabilizes.

## License

MIT — see [LICENSE](LICENSE).

Third-party SVG artwork (Lucide) is ISC/MIT; see `assets/lucide/LICENSE`.
