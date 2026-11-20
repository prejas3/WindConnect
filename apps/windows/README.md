# Windows

## Review with mockups (WN-65)

CCS/1
CHECK:`node apps/windows/scripts/capture-screens-ui.cjs` (Playwright) takes screenshots in both themes: the panel with every section, the panel while pairing, the main window, SMS and the tray menu (TEMP/windconnect-wn65-screens). Panel: the header with the phone first, cards every 20 px, the touchpad and the clipboard as cards, the touchpad card only with a connected phone.

## Accessibility: Narrator, keyboard, contrast, large text (WN-64)

CCS/1
RULE:the whole panel and the other windows can be gone through with Tab/Shift+Tab alone (order as on screen, no traps, a visible focus ring), every control has a name for Narrator, text has WCAG AA contrast in both themes, Windows contrast mode (forced-colors) keeps borders and focus. Large text (Accessibility → Text size, up to 200%) moves the tiles and the battery to further rows instead of cutting them off.
CHECK:`node apps/windows/scripts/check-a11y-ui.cjs` (Playwright, `PLAYWRIGHT_MODULE`): the panel with every section, index, SMS and the tray menu; the Chromium accessibility tree (the same one UIA gives Narrator), text contrast, 200% without horizontal scrolling, screenshots in TEMP.

## Languages: Polish and English (WN-63)

CCS/1
RULE:the UI language is the Windows display language: Polish for Polish, English for any other. The frontend reads it from WebView2's `navigator.language`, native code (tray, toasts, Explorer menu, statuses and errors shown in the UI) from `GetUserDefaultUILanguage`; `WINDCONNECT_LANG=pl|en` forces the language of native texts. Texts: `frontend/locales/{pl,en}.js` (keys, `{name}` as a placeholder, plurals through `plural()`), `src-tauri/locales/{pl,en}.json`. Pages mark texts with the attributes `data-i18n` / `data-i18n-<attribute>`, scripts use `t()`; dates and numbers are formatted by `Intl` in the UI language.
CHECK:`node apps/windows/scripts/check-i18n.cjs`: the same keys and placeholders in both languages, every key used exists; with `PLAYWRIGHT_MODULE` it renders all pages in English and Polish (no Polish characters and raw keys in the English version, screenshots in TEMP). The Playwright UI tests open pages with `locale: 'pl-PL'`. `cargo test` checks the parity of `src-tauri/locales`.

## Touchpad from the phone (WN-51)

The touchpad is allowed by default for a paired phone. Open "Touchpad" on Android's home screen: one finger moves the cursor, a tap clicks, holding drags, two fingers scroll (with inertia), a two-finger tap gives a right click. The buttons under the area also let you hold the left/right button. Sensitivity and natural scrolling are set on the phone.

Three fingers: up is Task View, down is the desktop, left the previous app, right the next one. Requires a PC with `input/2`; basic motion also works with `input/1`. The keyboard is a separate feature, WN-52.

### Presentation mode (WN-53)

On the touchpad screen choose "Presentation". Volume up is the next slide, volume down the previous one (the →/← arrows, so it works in PowerPoint, Google Slides and other slideshows in the browser without configuration). Hold "laser pointer" and aim the phone: the PC shows a red dot over the cursor, transparent to clicks and not taking focus from the slideshow. The talk timer starts at the first slide. The laser needs a PC with `input/4` (PROTOCOL.md §8.11); an older PC keeps slide switching.

The test of the real overlay (the live cursor, a topmost window) is skipped in `cargo test` because it depends on the desktop (WN-139). Run it separately, without moving the mouse: `cargo test -p windconnect-windows laser::tests::real_overlay -- --ignored`.

The PC panel shows "Phone controls the cursor · name". "Turn off the touchpad" withdraws input acceptance at once and releases the buttons. The setting stays after a restart. Closing the phone's screen, losing focus, locking and a connection change stop sending; nothing is replayed after reconnecting. A detected desktop other than Default (lock/UAC) or an input insertion error turns the touchpad off with a message and requires turning it on again locally, without raising privileges.

CCS/1 SEC:a partial shortcut releases only the still-pressed keys of the accepted prefix, in reverse order; before Win-up it sends a neutral VK 0xFF down/up so as not to open Start. Recovery is a single attempt, without retrying the gesture. LIMIT:on the ordinary desktop UIPI may block controlling an administrator's app. We don't check the integrity level of the active window; the result of SendInput/GetLastError doesn't identify UIPI as the cause. We don't guarantee detecting this block, nor turning off automatically/showing a message in every case. DEP:[Microsoft SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput).

CCS/1 PERF:QUIC loopback + the PC's shared interpolator, 360 samples at 120 Hz: p95 16.285 ms idle and 11.929 ms with a 268 500 992 B transfer; the raw CSV is in the card's report. This is a software measurement, without SendInput and the screen. Physical touch → cursor and the smoothness of a real 120 Hz monitor: WN-134 (final tests, linked with WN-77). To reproduce: `WIND_TOUCHPAD_SAMPLES=<folder>` and `cargo test -p wind-core --test input touchpad_latency_with_interpolation_idle_and_bulk -- --ignored --nocapture`.

## Transfer progress (WN-41)

CCS/1
FLOW:the panel shows up to 50 of the newest transfers in both directions, the state, the confirmed bytes/size, the speed and the available actions. Stable DOM rows keep focus during updates. Speed: monotonic deltas, a 1 s window + EMA; "—" before the first window and while waiting. An unknown size has an indeterminate bar (the current engine knows the size after importing the source). States/bytes are coalesced before serialization/IPC into the newest snapshot in a 100 ms window; one pending deadline, no idle timer. Android's system notification is limited separately to 1/s.
SEC:Cancel/Retry/Open folder are resolved natively by ID. Ambiguous IDs (different devices/directions) are rejected. Retry only a failed/cancelled outbound transfer and a still-paired original peer; no redirecting to the current phone. Received files need a retry on the sender's side. Open folder checks, at the moment of the click, the local source file or the verified incoming saved_as, canonical/containment/reparse; SHOpenFolderAndSelectItems selects the file in Explorer and doesn't run it. A moved/deleted file gives a readable message.
CHECK:`check-send-ui.cjs` (Playwright): both palettes at 150%, keyboard/focus after 100 updates, actions, indeterminate and untrusted names. `check-transfer-performance.cjs` + the release example `transfers_bench`: real QUIC, 1 GiB and 256x1 MiB, the median of 3 runs off/on with the production model and the Chromium renderer; target >=95% of the baseline. Raw timings and event cadence in TEMP/performance.json. The initial unbounded emission: small batch 86.2% (FAIL); after coalescing, the first measurement 104.2%/117.1%; a repeat of 256 MiB had 91.4% (FAIL) with a large time spread. The final fixture uses a longer 1 GiB run and a balanced off/on order: large 99.35%, small batch 101.28% (PASS); raw timings in the run record. Values >100% mean host fluctuations, not a claim of speed-up. This is not a measurement of real Wi-Fi nor of a physical screen's display every 100 ms.

## Receiving files from Android (WN-40)

CCS/1
FLOW:an admitted paired session -> files/1 -> the existing FileEngine -> Downloads/WindConnect/Images, Video, Audio, Documents or Other. The category is chosen locally at the verified finalization by the last extension, case-insensitively; e.g. photo.jpg.exe and no extension -> Other. The full table of extensions: `wind-core::files::incoming_folder`; the engine's default policy is OFF, the Windows runtime ON.
SEC:the signalled type is not an assessment of the content. The existing path checks, no overwriting, checksum/resume and Mark-of-the-Web also apply in the category subfolder; partial files stay outside the inbox. We never open/run a received file automatically. The router passes the incoming queue on only after the session is admitted (BUSY/unpair still before receiving).
IO:the generated fixture `files_peer` writes only into the given TEMP, with the same opt-in engine policy. The test of 20 images and a mixed set is described in the Android README; it doesn't touch the logged-in user's Downloads or the installed app. Sending from Windows is WN-39; the Android public Downloads receiver stays WN-120, the extended progress UI WN-41.

The WN-93 icon "Orbita": the SVG sources and how to regenerate them are in `assets/brand/README.md`.
The ICO contains the sizes 16–256 px; the tray has separate variants 16/20/24/32 px
for three states and both palettes. A small bitmap is never scaled up for a higher DPI.

A Tauri 2 client with a prerendered phone panel and a static frontend
without Node.js. Pairing and the authenticated session run in `wind-core`.

## Requirements

Windows 11 x64, Rust (MSVC; pinned in `rust-toolchain.toml`), Microsoft C++ Build Tools with the Windows SDK,
Microsoft Edge WebView2. The MSI needs the Windows feature
**VBSCRIPT** turned on. Tauri downloads WiX at the first packaging.

```powershell
cargo install tauri-cli --version 2.11.4 --locked
cd apps/windows
cargo tauri dev
```

## MSI

```powershell
cd apps/windows
cargo tauri build --bundles msi -- --locked
```

Output: `target/release/bundle/msi/*.msi` in the repository root.
The installer downloads the WebView2 bootstrapper if the runtime isn't installed;
the first installation then needs an Internet connection. The MSI is not signed.

Build the files for a public release (WN-162) so that they don't carry paths from
the building computer (Rust embeds the source paths from the Cargo registry by default,
the linker the PDB path):

```powershell
$env:RUSTFLAGS = "--remap-path-prefix=$env:USERPROFILE=~ -C link-arg=/PDBALTPATH:%_PDB%"
cargo tauri build --bundles msi -- --locked
```

A profile path with a space needs `CARGO_ENCODED_RUSTFLAGS` (flags separated
by the character `\x1f`). The APK does this by itself (`apps/android/wind-core/build.gradle.kts`).

The **Windows MSI** workflow builds the package on `windows-latest` for a PR, a push to
`master` or a manual run. In the Actions tab download the artifact
`WindConnect-windows-x64-msi`, unpack the ZIP and run the MSI.
After installing, start WindConnect: the panel shows a QR code to pair the phone.
After pairing and setting up the session it shows "Connected" and the phone's name.
Close the panel and check uninstalling in Settings.

In `tauri.conf.json` the values `identifier` and
`bundle.windows.wix.upgradeCode` must stay the same between versions,
so that MSI updates recognize the installed app correctly.
The frontend uses no JavaScript libraries or resources from the network. The local DM Sans
comes from the accepted WN-20 mockups (OFL 1.1). The licenses of the font and of the directly
called Mica library are included in `frontend/assets/`; the inventory and the font's
hash: `frontend/assets/NOTICE.txt`. The project's icon and SVGs: MIT.

The CI runs on GitHub (WN-163); a manual installation and uninstallation of the MSI
is part of the device tests before a release.

## Windows controls (WN-21)

`frontend/tokens.css` defines the light/dark palette, DM Sans, spacing,
radii and motion from WN-20. `style.css` draws buttons, tiles, switches,
the checkbox, fields, the list, the dialog, progress and the scrollbar. Semantic HTML elements
keep their keyboard handling; their look doesn't use the default controls.
Focus is visible, Escape closes the dialog and restores focus, the list handles
arrows/Home/End. `prefers-reduced-motion` removes animations; forced-colors
keeps borders and the selection. The System theme follows the OS preference change,
and the user's choice is remembered locally.

The window has its own title bar (dragging, double click, minimize, maximize,
close). Mica is applied natively through `window-vibrancy`; only
a successful DWM call turns on the semi-transparent tint. An error or no support
leaves an opaque background. Windows may turn the effect off by itself according to
the accessibility/power settings. We don't emulate Mica with an image or a CSS blur.

Gallery: `cargo tauri dev` and the **Controls gallery** button in the footer.
Optionally `?gallery` opens it at once. The native `ui_dev_mode` returns false
in release, regardless of the URL. The gallery's data and actions are for demonstration,
without access to the transport, trust or devices. A release offers the real
WN-25 pairing panel, and the gallery stays in debug only.

UI test: install Playwright 1.61.1 outside the repository, and its Chromium,
set `PLAYWRIGHT_MODULE` to the folder of the `playwright` module, then:

```powershell
node apps/windows/scripts/check-ui.cjs
```

The runner starts and closes a local server and the browser. It checks both themes,
400/700/1024 px, font loading, the keyboard, validation, the dialog, reduced-motion
and the release gate with injected IPC. PNGs go to `%TEMP%/windconnect-wn21-ui`
(or `UI_ARTIFACTS`). Windows CI runs the same test. In addition, on Windows 11
check natively dragging/resizing, the window buttons, Alt+F4, Mica on the desktop background
and the mode with transparency off; headless Chromium doesn't confirm the DWM effect.

## Tray and autostart (WN-23)

A left click on the icon opens the WN-24 panel, a right click opens our own menu (DM Sans, the app's
theme, arrows/Home/End/Tab, Enter/Space, Escape). The menu is also available
through **Tray options** in the window. Losing focus hides the menu. X/Alt+F4 hides
the window to the tray; **Quit** ends the process and removes the icon. A further start
restores the existing window instead of adding a second icon (a logon-session mutex).

Three own 32 px icons have a distinct shape and colour: a check mark/teal for connected,
three dots/amber for searching and a crossed circle/gray for not connected.
The tooltip and the menu header contain a verbal description. `indicator.rs` publishes events
through a watch, without timers. The native receiver swaps the icon and tooltip on the UI thread;
`tray_snapshot` gives the time from publishing to the end of the native calls.
This measures the API, not the moment Explorer repaints.

Integrating the Windows session coordinator (the following pairing cards): take
`app.state::<TrayRuntime>().indicator`, call `searching()` before an attempt,
`attempt_failed(generation)` after an error, or run `track_session(generation,
&session)` for an authenticated session. Its close switches the icon to not
connected. The generation guards against a late event of the previous session.
The WN-25 coordinator publishes these events for real sessions. Without a session the icon
shows not connected, also after only storing a pairing. Keys from discovery
are not pinned; trust comes from the pairing protocol and the stored store.

**Start after signing in** is off by default. The user's explicit choice
writes a REG_SZ in HKCU `Software\Microsoft\Windows\CurrentVersion\Run`, the value
`WindConnect`, with the quoted path of the current EXE and `--autostart`. No shell
and no administrator rights. A start with this argument creates only the icon.
The MSI doesn't turn autostart on; it tries to remove the entry on uninstall in the current
user's context. The cleanup doesn't block: a missing or locked EXE or a registry error
doesn't stop the uninstall; the Run entry may then stay. Entries of other profiles are not removed. After the EXE's location changes,
turn the option on again. Task Manager/Windows policy may disable the start regardless
of the Run entry. There is no service and no Task Scheduler task.

Tests: `cargo test -p windconnect-windows` (an old generation, quoting, reading/
writing/removing in a private HKCU key of the test, never in Run). `check-ui.cjs`
also checks our own menu and the autostart checkbox with a fake IPC. The native test:

```powershell
node apps/windows/scripts/check-tray-native.cjs target/debug/windconnect-windows.exe
node apps/windows/scripts/check-tray-native.cjs target/release/windconnect-windows.exe --release
```

Set `PLAYWRIGHT_MODULE` as for the UI test. The runner starts and closes its own
app/WebView2, checks the silent start, closing to the tray, a second instance
and the menu's keyboard. In debug `dev_tray_probe` creates two local authenticated
QUIC/Session nodes, measures 15 icon transitions and requires each <100 ms; release
refuses this operation. Manual shell tests remain (right/left click on the real
icon, restarting Explorer, signing in and uninstalling on a VM, different DPIs/monitors).

## Panel by the tray (WN-24)

CCS/1
M: Windows / panel 380×540 DIP
FLOW: start -> hidden WebView2 -> font and layout ready -> left click on the tray or Win+Shift+W -> show the existing window.
RULE: Escape, X, Alt+F4 and losing focus hide the panel; the button "Open the app" opens the main window. A right click keeps the WN-23 menu.
RULE: The shortcut is registered through RegisterHotKey without repeats; ending the process releases the registration. A shortcut conflict doesn't stop the start: the panel shows an explanation, the tray still works.
IO: The icon's physical rectangle -> monitor, DPI and work area -> a position at the matching edge of the taskbar. The size is limited to the work area; smaller screens get scrolling.
RULE: Local DM Sans, WN-20 tokens, four own tiles, the app's theme, a 160 ms / 8 px entrance; reduced-motion turns the animation off. Without a paired device the actions are inactive; no made-up photos, notifications or battery.
PERF: panel_snapshot.painted_us measures the time from the start of the native handling of the open to the confirmation of two requestAnimationFrame calls in WebView2. That confirms a frame was submitted, not a measurement of the physical screen; it doesn't wait for the 160 ms animation to end.
STATE: Geometry test: the four taskbar edges, negative coordinates, 200% DPI, a small monitor and auto-hide. Registration test: a conflict, dispatch and releasing the shortcut with Ctrl+Alt+Shift+F24 (it doesn't take over the user's Win+Shift+W).

```powershell
node apps/windows/scripts/check-panel-native.cjs target/debug/windconnect-windows.exe
node apps/windows/scripts/check-panel-native.cjs target/release/windconnect-windows.exe
```

CCS/1
DEP: PLAYWRIGHT_MODULE as for check-ui.cjs.
IO: 20 measurements <80 ms; preloading without a flash, Escape, losing focus, opening the app, themes, DM Sans, reduced-motion. Without a phone (debug: an empty store in the report folder) the quick actions are hidden, and only the unbuilt Ring and Touchpad are disabled; Send and Clipboard work after connecting (WN-109). Report and PNGs: %TEMP%/windconnect-wn24 (or PANEL_ARTIFACTS). The runner closes its own process and browser.
LIMIT: With Win+Shift+W taken, the report explicitly records shortcut=false and the OS message; the other checks work. A full test of the physical shortcut needs a free shortcut. Changing the taskbar edge/monitor/DPI and a real click on the icon remain for manual shell validation; the runner doesn't change the desktop configuration.

## QR and connecting the phone (WN-25)

CCS/1
FLOW: normal start -> hidden panel ready -> QR at first start -> scan/confirmation on Android -> WN-19 pairing -> storing the key -> authenticated Session -> "Connected" + the phone's real name and the tray icon.
SEC: DPAPI and paired.txt in the user's profile as in the core. Damaged records are skipped and counted; a read/identity error doesn't generate substitute keys. Trusted keys are loaded before the main session starts listening. Discovery alone or a pairing success is not the "Connected" state.
RULE: The QR code has a one-time secret and is valid for 5 min. A new code, cancelling or a network change invalidates the previous window; after expiry/3 failed attempts the image disappears. No LAN address gives a description and a retry. After pairing we wait for the Session; after 10 s without it we show the kept pairing and not connected. Disconnecting removes the "Connected" state, the key stays.
IO: QR addresses: the IPv4 addresses of the LAN interfaces, the default route's interface first (max. 8); a separate pairing port, the main endpoint for sessions. Discovery in parallel: mDNS + the WN-15 broadcast. Wi-Fi/Ethernet on the same network; the relay is not turned on automatically here.
RULE: The code is rendered in Rust into a module matrix, onto a canvas: black/white in every theme, an integer scale, a quiet zone of 4 modules. The raw link never goes into the DOM, localStorage or logs. The phone's four actions stay inactive until their cards; no made-up battery/photos.
SEC (WN-94): The QR modules encode a one-time secret, so the app clears each of its copies. The secret and the link are `Zeroizing` in the core; the module buffer from `qrcode` is overwritten after copying; `QrBits` zeroes itself when dropped (the loop's state, the `watch` value, the clones in `publish` and `pairing_snapshot`). Closing, a new code, expiry, pairing and the end of the loop set `qr = None`. The panel zeroes the module array after drawing and clears the canvas when the QR code disappears.
LIMIT (WN-94): Outside the app's reach are the `qrcode` encoder's working buffers (freed in it without zeroing), the JSON serialized by Tauri for IPC (the `pairing-state` event and the `pairing_snapshot` answer), and the copies in WebView (the IPC string, the JS heap, the bitmap in the GPU/compositor), which the GC and the browser free. The one-time protection doesn't depend on zeroing: a closed window invalidates the secret in the core, and an old code is rejected.
FLOW: After pairing, Android looks up the address only for the stored key, then connects with this pinned key; a 7 s limit. An unreachable computer doesn't remove a successful pairing. The phone's screen follows the session's real state.

Native checks (external Playwright as above; JDK 17 in JAVA_HOME,
ZXING_CORE_JAR points at the `core-3.5.4.jar` downloaded by Gradle):

```powershell
cargo build --locked -p windconnect-windows
cargo build --locked -p wind-ffi --example desktop_pairing_peer
node apps/windows/scripts/check-pairing-native.cjs
node apps/windows/scripts/check-pairing-android.cjs
```

CCS/1
STATE: The native test decodes the real canvas with ZXing, uses the real pairing protocol and Session, checks the name, the icon, the QR code disappearing, a restart, cancelling and refusing an old code. The peers have ephemeral identities; DPAPI/trust in a unique TEMP. The command dev_pairing_address and the variables WINDCONNECT_TEST_DATA_DIR/WINDCONNECT_TEST_PAIR_IP work only in debug; release doesn't let these variables redirect the identity.
STATE: The Android test needs emulator-5556 (or ANDROID_SERIAL), the APK and the test APK. It decodes the canvas image on Android, confirms in the UI and requires "Connected" in <10 s. The emulator's NAT needs a UDP redirect 47823 -> 47821; the test sends only the public discovery packet with the fingerprint and port, it doesn't pin keys. Full pairing/TLS/Session is production code. The test ends the port/timer/processes; the test's raw link is deleted. PNGs and run.json: TEMP/windconnect-wn25-*.
LIMIT: The budget is measured from recognizing the QR code, with the test's automatic confirmation, to the real session. Human time and physical focusing are not limited. A physical S25 Ultra and real Wi-Fi/Private firewall have to be confirmed on the phone. The MSI/APK for the first test are built locally; the test doesn't install the MSI.

Audit of the new dependency shipped in the EXE/MSI:

| Component | Version | License from the package | Where it goes | Result |
| --- | --- | --- | --- | --- |
| qrcode | 0.14.1 | MIT OR Apache-2.0; MIT chosen, LICENSE-MIT.txt | The native EXE; full text frontend/assets/qrcode-MIT.txt | Permissive; without default features and the image/SVG/PIC dependencies. |
| if-addrs | 0.15.0 | An existing wind-core dependency | The same EXE, no new runtime library | A new direct call of an existing component. |

CCS/1
EXT: qrcode upstream https://github.com/kennytm/qrcode-rust (package 0.14.1, unchanged); NOTICE in frontend/assets. No external QR generator, photos or new fonts.

## Discovery and the firewall rule (WN-15)

The core runs mDNS and IPv4 broadcast in parallel. Broadcast uses
UDP port 47821, sends the key's fingerprint and the actual QUIC port, and the result stays
an untrusted hint for a session with a pinned key. The fallback can be checked
with `windctl discover --broadcast-only` and a node
`windctl serve --advertise --broadcast-only` with a separate key.

The fragment `src-tauri/wix/firewall.wxs` adds at installation the rule
**WindConnect - LAN UDP (Private)**. The rule applies only to the full path of the
installed `windconnect-windows.exe`, the UDP protocol, the Private profile and
LocalSubnet addresses. It covers mDNS 5353, broadcast 47821 and the QUIC port chosen
by the app, also after a socket migration. It doesn't open TCP, the Public profile
or other apps (including the separate `windctl.exe`).

WiX FirewallExtension manages adding, rolling back the installation and removing
the rule together with the MSI component. An error registering the rule stops the installation
and runs a rollback, instead of silently leaving an incomplete configuration.
An administrator can further restrict traffic with an organization's policy.

Building the MSI compiles and links the fragment with the standard WiX tools;
it doesn't install it and doesn't change the building computer's firewall. Test installation
and uninstallation on a Windows VM meant for it: after installing,
check `Get-NetFirewallRule -DisplayName 'WindConnect - LAN UDP (Private)'`
and the related application/address/protocol filters, and after uninstalling that the
rule is gone. The two-node fallback test with mDNS off is independent
of the installer's local policy.

## Remote actions (WN-54)

A paired phone can ask the computer for an action from its allowed list
(PROTOCOL.md §9.2): locking the screen (`LockWorkStation`), sleeping
(`SetSuspendState`, after the answer is sent), opening an `http`/`https` link in the
default browser (`ShellExecuteW`) or a custom action. The phone sends
only the identifier (and the address for a link), never commands.

The list is `%LOCALAPPDATA%\WindConnect\actions.json`, next to the identity and
`paired.txt`. The main window shows it in the section "Remote actions": the switches
write the file, and "Edit custom actions" opens it in Notepad. The file is
read on every request, so turning something off takes effect at once. No file:
built-in actions on, no custom ones. A file that can't be read:
nothing runs until you fix or delete it.

```json
{
  "lock": true,
  "sleep": true,
  "openUrl": true,
  "custom": [
    { "id": "x-backup", "label": "Backup", "enabled": true,
      "program": "C:\\Program Files\\Backup\\backup.exe", "args": ["--now"] }
  ]
}
```

A custom action: `id` starts with `x-` (lower-case letters, digits, `-`), `program`
is the full path to an `.exe` file (no `.bat`/`.cmd` and no shell), `args`
are fixed. An invalid entry is skipped and described in the window. The last request
run or refused is visible under the list.

Test of the section (Playwright as for `check-ui.cjs`, IPC stubbed):
`node apps/windows/scripts/check-actions-ui.cjs`.

Checking without a phone: pin the `windctl` key like a phone (pairing)
and use `windctl action` (crates/windctl/README.md).

## Sending files (WN-39)

Files and folders go to the phone with the WN-38 engine (PROTOCOL.md §9.1), to
the connected phone, and when none is connected, to the most recently
paired one: transfers wait in a queue and start after connecting. A folder is
a separate transfer for each file, with a relative path.

From where:

- **The Explorer menu**: "Send to phone" for files and folders
  (`HKCU\Software\Classes\*\shell` and `Directory\shell`, the key
  `WindConnect.SendToPhone`, without administrator rights). The app
  writes the entry at every start (the exe's path may change), and
  uninstalling removes it (`--remove-shell-menu`). In Windows 11 the entry is in
  the classic menu ("Show more options", Shift+F10); the top menu of Windows 11
  requires a package with an identity (MSIX), which the MSI doesn't have.
  Explorer runs `WindConnect.exe --send "<item>"` for each
  selected item; such a process passes the path to the running
  app through the pipe `\\.\pipe\io.github.prejas3.windconnect.send-<session>`
  (this logon session only, no remote clients) and ends. When
  the app isn't running, the first such process starts it.
- **Dragging** onto the panel or the main window. The panel doesn't hide
  while a dragged item is over it; it can be opened during the
  drag with the shortcut Win + Shift + W.
- **The "Send" tile** in the panel: the system's multi-file picker.

The tray icon doesn't accept dropped files: the Windows notification area
doesn't pass dragging on to apps (there is no OLE target for icons),
so the panel is the target.

The panel shows sending: the total progress, up to four items (cancel,
retry) and "Clear" for finished ones. A phone that doesn't accept
files yet (`files/1`, WN-40) gives "The phone doesn't accept files now".
The computer doesn't announce `files/1` yet (receiving is WN-40); the folder
`Downloads\WindConnect` is prepared for receiving.

Test of the panel (Playwright as for `check-ui.cjs`, IPC stubbed):
`node apps/windows/scripts/check-send-ui.cjs`.

## Pairing without the camera (WN-27)

The panel in QR code mode has a button "Pair without the camera": it opens a SAS window
(PROTOCOL.md §12.6) and announces the pairing port in mDNS (TXT `pair`, §12.4) for
the window's duration. The phone in "Pair without the camera" shows computers with this field;
after choosing, both screens show the same six-digit code with the other
side's name. The other side's key is stored only when "The codes are the same"
was chosen on **both** screens. "The codes differ" or no decision
within 60 s is a failed attempt; after 3 attempts the window closes (§12.2). The list on
the phone is untrusted hints from the network: the code comparison decides
trust.

Test of the panel (IPC stubbed): `node apps/windows/scripts/check-sas-ui.cjs`.

## First start (WC-34)

CCS/1
FLOW:start without `--autostart` and without a paired phone -> the main window with three steps and a QR code without clicking (the same pairing window as the panel) -> pairing -> the "Done" step (the panel's shortcut, "Start with Windows") -> the normal window. With a paired phone the start opens the panel as before.
RULE:the QR code as in the panel (WN-94): only a canvas with `aria-label="Pairing code"`, modules zeroed after drawing, no link in the DOM. After expiry a new code only on "Show a new code". Focus on the step's main button. Settings that need a phone wait until onboarding ends.
RULE:"a paired phone" has one definition (WN-113): `pairing::has_paired_in`, a phone in `devices`; a removed phone that is still owed an unpairing notice (`owed`) doesn't count. Choosing the first window (`has_paired`) and the onboarding view (the `paired` field in the pairing state) read the same rule.
LIMIT:the verdict "a new user pairs without instructions in < 60 s" comes from a test with a person; the scripts measure only the part the system adds.

Tests: `node apps/windows/scripts/check-onboarding-ui.cjs` (stubbed IPC:
steps, QR rules, SAS, expiry, "Done", focus, Light/Dark, 150% text,
a start with only an `owed` phone)
and `node apps/windows/scripts/check-onboarding-native.cjs` (the real app
with an empty store, `JAVA_HOME`, `ZXING_CORE_JAR`, the installed
WindConnect closed): the report `launchToQrMs`, `qrToDoneMs`, `systemAddsMs`.

## Phones in the panel (WC-28)

CCS/1
FLOW:the "Phones" panel -> device_action(rename|unpair|disconnect) -> the pairing actor; "Add a phone" opens the QR code also during a session.
RULE:one session at a time. The same phone replaces its old connection; another gets BUSY (PROTOCOL.md §12.10). A phone paired in the panel a moment ago (2 min) takes over the session. "Disconnect" answers that phone BUSY for 10 min or until another phone's session.
SEC:unpairing writes `-<key>`, unpins and revokes the key at once; the phone gets Goodbye UNPAIRED in the session or at its next connection (notice mode: no router and no channels). A phone that removed the pairing itself removes only the peer's key from TLS.
LIMIT:the name is local (it doesn't go to the phone). After BUSY the phone doesn't come back by itself when the computer becomes free.
UI (WN-161): one row per phone: a phone icon, the friendly name (a model code such as "SM-S938B" stored at pairing gives way to the name the connected phone sends in its session Hello; a name you set wins), the state with a dot and the battery. One main button ("Disconnect" for the connected phone); "Rename", "Details" (fingerprint, pairing date, model) and "Remove pairing" (confirmed in place) are in an own "…" menu.

UI test (stubbed IPC): `node apps/windows/scripts/check-devices-ui.cjs`
with `PLAYWRIGHT_MODULE`. The logic of both sides is checked by
`cargo test --locked -p wind-core --test unpair` (unpairing in a session and
outside it in both directions, no 0-RTT from an old ticket, the streams of a
revoked key without a receiver, two phones at once without pushing each other out) and
`--test relay` (notice mode through the relay, needs `wrangler dev`).

## Shared clipboard (WN-35)

CCS/1
FLOW:AddClipboardFormatListener/WM_CLIPBOARDUPDATE -> reading CF_UNICODETEXT -> clipboard/1 -> Applied after the phone's OS write. No polling. Our own operations are serialized, and the window procedure defers the event when the adapter is busy, instead of blocking the clipboard owner's messages.
RETRY:a clipboard held by another process longer than 30 ms -> one WM_TIMER on the window's thread, backoff 50/100/200/400/800/1600 ms (≈3.2 s), then giving up (manual sending remains). It retries only the sequence that caused the event; a new change, turning monitoring off and closing the window cancel the timer (WN-105).
RULE:automatic by default; the setting in clipboard.json next to the identity contains only a bool. A damaged file means off. The switch turns off automatic sending and receiving. The "Clipboard" tile and Ctrl+Alt+Shift+C send manually; a shortcut conflict is visible and doesn't disable the button. Win+Shift+W stays the panel's shortcut.
SEC:we don't send the format ExcludeClipboardContentFromMonitorProcessing nor CanIncludeInClipboardHistory/CanUploadToCloudClipboard=0; this also applies to the manual action. Receiving sets both opt-out formats=0 before writing the text. No payload in IPC/the frontend/logs/a file history. Limit 256 KiB, no NUL (the Windows terminator is cut off); a link is text, not a command to run. Details in PROTOCOL.md §9.3.
LIMIT:PC -> phone automatically also without an open Activity. Phone -> PC from another app needs a notification action and a brief WindConnect focus; without additional system permissions. Marked passwords are not synced. No clipboard replay after a reconnect.

A native test without changing the interactive clipboard:
`cargo run --locked -p windconnect-windows --example clipboard_probe`.
It creates a separate desktop in a non-interactive window station, requires that station's
clipboard to be empty, checks events, Unicode and three privacy flags, removes its own
synthetic data and ends its threads. A second process (the same example,
`--hold-child`) in the same station holds the clipboard with its window for longer than 30 ms:
it checks delivery once, no sensitive text, the retry limit, cancellation
when turned off and no timer after closing. The session's non-interactive station
outlives the runs: an interrupted test may leave synthetic text, and
the next one refuses to start until someone removes it. The Android/PC test and the measurement of 100 samples in both
directions are described in `apps/android/README.md`, section WN-35.

### Images (WN-36)

CCS/1
FLOW:WM_CLIPBOARDUPDATE without text -> the "PNG" format (unchanged) or CF_DIBV5/CF_DIB (encoded to PNG off the window thread, `png` 0.18, fast compression) + a thumbnail ≤ 512 px -> clipboard/2 (PROTOCOL.md §9.3.1). Status: "Sending the image…" after the phone's Applied (the image is already pasteable), "Image sent to the phone" after ImageDone.
RULE:a received image goes into the clipboard at once as "PNG" and CF_DIB with delayed rendering; WM_RENDERFORMAT waits for the bytes (at most 10 s without progress), then renders. A newer clipboard, a break or a stall: the paste fails, without a partial image. WM_RENDERALLFORMATS on close renders only a complete image, otherwise it clears the clipboard. We don't read our own image (owner = our window): no echo and no waiting on ourselves.
RULE:the limit "Max image size" 5 / 16 / 50 MB (16 by default) in Settings and the panel, stored in clipboard-images.json next to clipboard.json; it applies to sending and receiving.
SEC:the same privacy flags as for text: we don't copy an image marked as private from the OS; a received one has the history and cloud opt-out. Text takes precedence when both are in the clipboard.
LIMIT:the window thread waits for bytes only when pasting before the transfer ends; the chunk-receiving path doesn't need it (no deadlock). Screenshots larger than 16384 px are not sent. Pasting a received image as CF_DIB decodes the PNG with a limit of 64 Mi pixels (protection against a "decompression bomb"): a larger image (e.g. 8K) pastes only as "PNG".

OS test (the same station as above): `clipboard_probe` checks reading "PNG" and
DIB alone, no copy of a private image, delayed rendering (the paste
waits and gets the whole PNG, CF_DIB from PNG), a paste error with a newer
clipboard at once and with a stall after ~10 s, a responsive window after a stall and
not reading our own image. The PC -> phone screenshot measurement: `apps/android/README.md`, WN-36.

### History and passwords (WN-37)

CCS/1
FLOW:an item goes into the history when the phone confirmed a sent clipboard (Applied) or when a clipboard from the phone was written in Windows. The panel and Settings show "Clipboard history": newest at the top, a click (Enter) copies it back into the clipboard with the flags it had (from the phone: outside the Win+V history and the cloud, as when receiving) and, with sync on, sends it to the phone; a pin and "Remove from history" beside it, "Clear" removes the unpinned ones.
RULE:sync turned off: nothing new goes into the history (manual sending neither), and turning it off clears the unpinned ones; pinned ones stay.
RULE:the last 20 + up to 20 pinned (pinned at the top, outside the twenty); a repeat of the same content (SHA-256) moves to the top instead of duplicating. Unpinned images up to 64 MB in total in memory (the oldest drops out), pinned ones also up to 64 MB; an image from the phone can be copied and pinned after the whole PNG has arrived.
SEC:the history is only in memory; only pinned items go to disk, in `clipboard-pins.bin` next to clipboard.json, encrypted with the current user's DPAPI with its own purpose (header `WCPN` 1). Another user's file, a damaged or a newer one: start without pinned items, the file stays.
SEC:only a text preview (one line, 120 characters) or a thumbnail goes to the panel (IPC); the full text and PNG stay in the process, copying goes by id. The preview is inserted as text, never HTML; control, bidi (U+202A–202E, U+2066–2069) and zero-width (U+200B–200F, U+FEFF) characters are replaced with a space. Thumbnails as `data:image/png`: the CSP allows `data:` only in `img-src`.
SEC:passwords: a clipboard with ExcludeClipboardContentFromMonitorProcessing, "Clipboard Viewer Ignore" (KeePass, Password Safe and others), CanIncludeInClipboardHistory = 0 or CanUploadToCloudClipboard = 0 isn't even copied from the OS, so it doesn't leave the PC and doesn't go into the history. We don't recognize passwords by their content: only the markers a manager sets count.
LIMIT:Android has its own history with the same rules (WN-117, `apps/android/README.md`); after a restart only pinned items remain; no search.
DATA:the clipboard files in the app's data folder (next to the identity): clipboard.json (sync on), clipboard-images.json (the image limit), clipboard-pins.bin (pinned, DPAPI). Uninstalling the MSI doesn't remove the data folder (like the identity); removing all pinned items deletes clipboard-pins.bin.

The test "a password doesn't leave the device": `clipboard_probe` (each of the 4 markers
separately, atomically, like a password manager: nothing is copied; an item from the history
keeps its flags; an image from the history pasteable as "PNG" and CF_DIB, without an echo) and the
OS test with a phone (`check-background-android.cjs`): the PC copies 4 passwords with
single markers, the phone gets none, and the next ordinary text
arrives. The history logic: the `clipboard_history` unit tests (order,
limits, memory, pinning, the DPAPI file).
UI test: `node apps/windows/scripts/check-clipboard-ui.cjs` with an external
Playwright (`PLAYWRIGHT_MODULE`). It checks the keyboard, the switch, manual
sending without a connection, the sensitive/shortcut-conflict status, the history (preview as text, copy/pin/remove/clear by keyboard, own scrolling in the panel) and the Light/Dark layout.
The stubbed IPC checks the UI; OS delivery is confirmed by a separate native test.

## The phone's battery (WN-49)

CCS/1
FLOW:the phone (ACTION_BATTERY_CHANGED, only during a session) -> `Battery` on the `battery` channel (`07`, PROTOCOL.md §9.4) -> `Snapshot.battery` in `pairing-state` (pushed, no polling) -> the panel header, the tray icon's tooltip, a low-battery badge, a toast.
RULE:panel: the level and a lightning bolt when charging next to the phone's name, `role="img"` with the label "Phone battery 82%, charging"; no aria-live (a change of percent isn't announced). Without a session or without a reading the element disappears.
RULE:tray: the tooltip "WindConnect · Connected · battery 82%, charging". At ≤ 15% without charging an amber dot in the bottom right corner, laid over the WN-23 state (not a separate state); it disappears after plugging in the charger or when the session ends.
RULE:alerts (wind-core `battery::Alerts`): the toast "Phone battery: N%" on dropping to ≤ 15% without charging, once per discharge (again after charging or > 20%); "Phone charged" at 100% on the charger, once (again after unplugging or < 95%). The session's first reading gives no toast. A toast carries only the level, the phone's name and fixed text; a click opens the panel.
DEC:the release version registers the sender `HKCU\Software\Classes\AppUserModelId\io.github.prejas3.windconnect` (DisplayName "WindConnect", `IconUri` = `%LOCALAPPDATA%\WindConnect\toast-icon.png`) and the scheme `windconnect-panel:` (`HKCU\Software\Classes\windconnect-panel`, command: this exe with `"%1"`). The battery toast has `launch="windconnect-panel:" activationType="protocol"`: a click starts the exe, which through a pipe (`WCP1`, send_pipe) tells the running app to show the panel, and when the app isn't running, starts with the panel itself. So it also works after the app restarts and from the notification center (WN-118). Debug still sends as PowerShell through `tauri-winrt-notification` with an in-process click. The MSI's Start menu shortcut also carries this AUMID; the HKCU entry stays, because the test release EXEs are portable (no shortcut). Only writing DisplayName is required; the icon and the scheme are "best effort" (an error goes to the log and doesn't turn off battery toasts or phone notifications, WN-146). An exe takes over the scheme only when nobody has it, the registered exe doesn't exist, it is the same exe, or it is the installed app (Program Files): a portable test EXE doesn't take the MSI's working scheme, and the installed app takes it back at its next toast. Uninstalling the MSI removes the scheme regardless of who wrote it; a portable EXE writes it again with its toast.
LIMIT:changes arrive only with a live session (background connection on the phone). In Doze or without a background connection the computer doesn't see changes, so the 15% alert may come late or not at all. Uninstalling the MSI removes the sender, the scheme and the icon (`--remove-toasts`, like `--remove-autostart`).
CHECK (WN-118, installed MSI 0.69/0.70, an emulator paired by QR with the release app): both toasts shown as "WindConnect", a click opens the panel (~265 ms through the scheme); with the app closed a click starts it with the panel. On Windows 11 26300 the toast header still shows a generic icon despite `IconUri` and the shortcut's icon (probably the shell's cache; check after signing in again).

Tests: `cargo test -p wind-core` (`battery`: the codec, invalid messages, alerts with hysteresis; `tests/battery.rs`: a real session, changes only and in order, a PC without `battery/1` gets nothing), the app's unit tests (badge, tooltip, texts), `node apps/windows/scripts/check-battery-ui.cjs` (stubbed IPC: level, charging, low, hidden without a session, updates without IPC calls, Light/Dark, 150%) and `node apps/windows/scripts/check-battery-android.cjs` (the real app and an emulator: `dumpsys battery` changes the level, the panel and the tooltip keep up, the 15% and full alerts).

Audit of the new dependencies in the EXE/MSI:

| Component | Version | License from the package | Where it goes | Result |
| --- | --- | --- | --- | --- |
| tauri-winrt-notification | 0.7.3 | MIT OR Apache-2.0 (LICENSE_MIT, LICENSE_APACHE-2.0, LICENSE.spdx); MIT chosen | The native EXE; frontend/assets/tauri-winrt-notification-MIT.txt | Permissive |
| windows (+ windows-core, -collections, -future, -link, -numerics, -result, -strings, -threading) | 0.61.3 (parts 0.1–0.61) | MIT OR Apache-2.0 (license-mit, license-apache-2.0, identical in all); MIT chosen | The native EXE; frontend/assets/windows-rs-MIT.txt | Permissive; no NOTICE files |

## Find my phone (WN-50)

CCS/1
FLOW:the "Ring" tile in the panel -> `Ring` on the `find` channel (`08`, PROTOCOL.md §9.5) -> the phone plays an alarm tone and only then sends back state 0 -> the "Stop" tile (aria-pressed) -> `Stop` from the PC or stopping on the phone -> the final state -> the status line ("Stopped", "Stopped on the phone", "The phone rang for 60 s", "The phone can't ring now").
RULE:the tile works only with a session and when the phone announces `find/1`; turned off on the phone ("Let the computer ring") gives an inactive tile with an explanation. The status line has `role="status"` (an answer to one's own action).
RULE:a session broken during ringing doesn't mean "stopped": the line says "Connection lost — the phone may still be ringing (up to 60 s)", because the phone keeps ringing until the limit or until stopped on it.
PERF:`ring_ms` in the pairing state is the time from the request to state 0, i.e. an upper bound of the sound starting on the phone.

Tests: `cargo test -p wind-core` (`find`: the codec; `tests/find.rs`: ringing and Stop, stopping on the phone, the limit, a second Ring, ringing after the session ends, no sound, withdrawing and enabling `find/1` during a session), `node apps/windows/scripts/check-find-ui.cjs` (stubbed IPC: tile states, keyboard, results, unavailable, Light/Dark, 150%) and `node apps/windows/scripts/check-find-android.cjs` (the real app and an emulator with an audio device, `QEMU_AUDIO_DRV=none`; `FIND_NOTIFICATIONS=denied` checks stopping without notifications).

## The phone's player in the panel (WN-46)

CCS/1
FLOW:the phone (the PC announces `media/1`, PROTOCOL.md §9.6) -> `Now`/`Art`/`Gone` -> `media::Player` (texts through the clipboard preview sanitizer, the cover only JPEG/PNG by its magic bytes) -> `Snapshot.media` in `pairing-state` -> the "On the phone" card (cover, title, artist, app, progress, ⏮ ⏯ ⏭, click/arrows on the bar = seek) -> `media_action` -> `Command` on `media-control` (the phone announces `media-control/1`).
RULE:texts from the phone's other apps go into the panel only through `textContent`; the cover only as a `data:image/jpeg|png;base64` that the panel checks too. The buttons depend on the player's `actions`; the phone skips a command outside `actions` anyway.
PERF:the panel computes progress from the last state (`position_ms + elapsed × speed`) with a local 1 s clock, only when the panel is visible and playing; nothing is polled.

Tests: `cargo test -p wind-core` (`media`: the codec, limits, extrapolation, the action gate; `tests/media.rs`: a real session, Now/Art/Gone only on change, Command back, nothing without the capability), the app's tests (sanitization, the cover by key), `node apps/windows/scripts/check-media-ui.cjs` (stubbed IPC: texts, cover, progress without IPC, commands, the action gate, hostile text, hiding, Light/Dark, 150%) and `node apps/windows/scripts/check-media-android.cjs` (the real app + an emulator with a test media session instead of Spotify; 10 samples of ⏯ and ⏭, seeking, times both ways, a command outside `actions` without a reaction).

## The computer's player on the phone (WN-47)

CCS/1
FLOW:Windows SMTC (`GlobalSystemMediaTransportControlsSessionManager`, session and manager events, an MTA thread) -> `pc_media::PcMedia` -> `Now`/`Art`/`Gone` on `media` (the phone announces `media/1`) -> the phone: a card in the app and the system media controls. A command from the phone (`media-control/1`, announced by the PC) -> checked against `actions` -> `TryPause/Play/TogglePlayPause/SkipNext/SkipPrevious/ChangePlaybackPosition`.
RULE:choosing the session: the first playing one, otherwise the system's current session if it may be followed, otherwise the first allowed one. Cover: the session's thumbnail decoded and encoded as JPEG ≤ 256 px (Windows.Graphics.Imaging), no cover above 96 KiB. After our own command the state is read once more at once (an answer to an action, not polling).
RULE:the setting in the main window "Share playback with the phone" (on by default, `pc-media.json`): turned off, it withdraws `media-control/1` also in an ongoing session, sends `Gone` and ends the SMTC subscriptions. When the phone accepts `media/1` again, the PC sends the state anew.
SEC:in the debug version `WINDCONNECT_TEST_MEDIA_APP=<AUMID>` limits following to one app, so that the test doesn't pause the user's players; release ignores this variable.

Tests: unit tests (`pc_media`: choosing the session, actions, time in ticks, names, limits), `node apps/windows/scripts/check-pc-media-android.cjs` (the real app + `examples/smtc_player` with a real SMTC session `WindConnect.TestPlayer` + an emulator: the card and the notification on the phone, 10× pause/play from Android's controls and from the app, a command outside `actions` without a reaction, the switch and `Gone` clean up, no echo).

## Notifications from the phone (WN-43)

CCS/1
FLOW:the phone's `Posted`/`Icon`/`Removed` on `notify` (the PC announces `notify/1`, PROTOCOL.md §9.7) -> the session loop -> `notify::Notifications` (whether it pops up) -> one worker thread (order kept) -> `ToastNotifier.Show` / `ToastNotificationHistory`.
RULE:a `ToastGeneric` toast: the title (without a title, the app's name), the content, "app · phone", the app's icon (`appLogoOverride`), a header per phone app (grouping in the notification center). Tag = the notification's id (hex), group `phone`: a change replaces the toast without popping up again, `Removed` takes it down. At most 5 pop-ups in 2 s; the rest go quietly to the notification center. A click opens the panel.
SEC:the texts come from the phone's other apps: a second sanitization on the PC (one line, without control, bidirectional and zero-width characters, trimmed) and XML escaping. The icon (PNG ≤ 16 KiB) is decoded and encoded anew as a 64 px PNG (Windows.Graphics.Imaging), so Windows doesn't read the sender's bytes; the file `notify-icons\<key hex>.png` in the app's data folder, at most 256 files (the oldest removed). Nothing is logged.
RULE:Windows keeps the toasts' content in its notification database (the notification center): each expires after 24 h, and turning off the setting "Notifications from the phone" (main window, on by default, `notify.json`; it withdraws `notify/1` also in an ongoing session) or removing the pairing (from the PC or from the phone) removes all toasts from the phone and the icon cache at once. An ordinary end of a session doesn't remove them.

### Replying and dismissing (WN-44)

FLOW:the field "Reply…" in the toast (when `Posted` has `reply`) and in the panel's "Notifications" section (the 20 newest of the session) -> `Reply` on `notify-action` (the phone announces `notify-action/1`, PROTOCOL.md §9.8) -> the phone sends the reply through the app's action (RemoteInput) and sends back `Replied` (the state "Sent" / "Failed").
RULE:**A change of behaviour:** closing a popped-up toast (✕, swiping) or ✕ in the panel dismisses the notification on the phone too (`Dismiss`); expiry and removal by the app don't. One dismissed on the phone disappears from Windows (`Removed`, as in WN-43). No separate setting.
SEC:the reply text isn't logged or kept after sending; in the panel the field is cleared only after `Replied` ok, after an error the text stays. Over 1024 B of UTF-8: sending is blocked with a message, never trimmed (from a toast the text goes into the panel's field). Actions only for notifications shown in this session (checked by the PC and the phone).
LIMIT:an unpackaged app (without a COM activator) gets a toast's events only as long as its object lives in the process: the worker thread keeps them until `Removed`/dismissal/expiry (at most 256). That is why in the release the content and header of a toast from the phone (like the battery toasts) open the panel through the `windconnect-panel:` scheme (protocol activation, WN-144), which also works after the app restarts; debug stays with the in-process click. The reply field is a background action in the process: after the app restarts, a reply from an old toast doesn't work (no COM activator). Clearing in the notification center gives no event, so it doesn't dismiss on the phone.

Tests: unit tests (`notify`: the reply field in the XML only with `reply`, toast activation by arguments and text (reply, empty, too long without trimming), only a close by a person dismisses on the phone, tag -> id, the panel list: limit 20, states, draft, removal), `wind-core` (the `Dismiss`/`Reply`/`Replied` codec, limits, the vectors from PROTOCOL.md §9.7–9.8, session: `notify-action/1` only with the phone's consent), Kotlin (`NotificationRules.replyAction`: `SEMANTIC_ACTION_REPLY`, any text, not contextual), `node apps/windows/scripts/check-notify-panel-ui.cjs` (the panel section with stubbed IPC: Enter, the field and focus survive a refresh, cleared only after `Replied` ok, the text stays after an error, > 1024 B blocked, a draft from a toast, ✕, Light/Dark) and `node apps/windows/scripts/check-notify-reply-android.cjs` (the real debug app + an emulator + Google Messages fed by `adb emu sms send`: a toast with a reply field, a reply from the panel and from the toast (`dev_toast_activate`, the same function as WinRT) sent as an SMS, ✕ in the panel disappears from the phone, snoozed on the phone disappears from the PC, closing the popup by a person dismisses, expiry doesn't). A real click on and close of the system toast can't be automated here: a short manual test on the phone checks them. Needs the same as the WN-43 test; the notification assistant is turned off for the test (Android 15 hides SMS content as sensitive).

### Filters and focus mode (WN-45)

RULE:choosing apps is on the phone (a turned-off app sends nothing). The PC only silences: in "Quiet hours" (main window, off by default, 22:00–07:00, 15 min steps, across midnight), when `SHQueryUserNotificationState` reports full screen, a Direct3D game, a presentation or the quiet time after installation, and when the phone marks a notification `quiet` (its "Do not disturb"), the toast goes without popping up and with `<audio silent="true"/>` to the notification center and the panel, without using up the pop-up limit. Windows' own "Do not disturb"/focus mode holds back toasts of apps not on the priority list by itself, so we don't read it (no undocumented WNF). Checked on every `Posted`, without timers. A silenced toast dismisses nothing on the phone (only UserCanceled, WN-44).
RULE:`notify.json` is `{enabled, quiet:{on,from,to}}` (minutes from local midnight); an old `true`/`false` file is read, a damaged one gives the default settings; written through a temporary file and a rename.

Tests: unit tests (`notify`: quiet hours across midnight, `from == to`, limits; migration and a damaged `notify.json`, writing; the `SHQueryUserNotificationState` states; quiet without popping up and sound, without using up the limit, closing doesn't dismiss on the phone), `node apps/windows/scripts/check-notify-quiet-ui.cjs` (the setting with stubbed IPC, keys, midnight, Light/Dark) and `node apps/windows/scripts/check-notify-filter-android.cjs` (the real app + an emulator, the shell and Google Messages as senders: a blocked app passes nothing to the core, "Only chosen" without a choice passes nothing, blocking removes its notifications from the PC, "Do not disturb" on the phone and quiet hours on the PC without popping up, and the notification stays on the phone).

## Calls and volume (WN-48)

FLOW:the phone's `State` on `call` (the PC announces `call/1` with the setting "Pause playback during calls", main window, on by default, PROTOCOL.md §9.10) -> the session loop -> one `call_pause` thread (it lives across sessions) -> `CallPause` decides -> SMTC `TryPauseAsync` on every playing session / `TryPlayAsync` on those it paused, if they are still paused (≤ 30 min). In debug `WINDCONNECT_TEST_MEDIA_APP` limits this to one player.
FLOW:volume (`volume/1`, §9.9, the setting "Volume from the phone", on by default): Core Audio (the default device, `IAudioEndpointVolume` + its change callback, `IMMNotificationClient` on a device change, no polling) -> `Level` to the phone; `Set` from the phone -> Core Audio. Panel: "Phone volume" (a slider: arrows 5 %, PageUp/PageDown 10 %, Home/End; mute), without "bouncing": for 500 ms after one's own change and while dragging, an incoming level doesn't move the slider.
LIMIT:a device with a fixed volume (some HDMI/S/PDIF/USB outputs) doesn't change the level; muting works. In vibrate mode some phones don't change the audio mode: then the pause comes from the call notification, which needs the "Notifications on the computer" access (the setting's description says so). "In a call" also covers calls in messaging apps (Discord, Teams, voice notes).

Tests: unit tests (`call_pause`: one pause per call, resuming what it paused, a broken session decided by the next state, the 30 min limit, settings; `pc_volume`: the scale; a manual `cargo test pc_volume -- --ignored` reads the default device), `wind-core` (the `State`/`Level`/`Set` codecs, limits, the vectors from §9.9–9.10, "newest wins"), `node apps/windows/scripts/check-call-volume-ui.cjs` (the slider, keys, muting, no bouncing, the switches, Light/Dark) and `node apps/windows/scripts/check-call-volume-android.cjs` (the real app + `examples/smtc_player` + an emulator: 5 calls with `adb emu gsm call`, ringing -> pause p95 < 1 s (measured about 0.2-0.3 s; the second ring about 6 s), hanging up -> resume, answered, stopped manually, the setting off, vibrate mode, volume both ways; the PC's volume and muting are restored at once).

RULE:the toast sender as for the battery (`toast.rs`): in debug PowerShell, in release the registered AUMID "WindConnect" (checked from the installer: WN-118).

Tests: unit tests (`notify`: XML, escaping and sanitization, no title, limits, the pop-up limit, quiet updates, names and the icon cache's limit), `node apps/windows/scripts/check-notify-settings-ui.cjs` (the switch with stubbed IPC, Light/Dark) and `node apps/windows/scripts/check-notify-toasts-android.cjs` (the real debug app + an emulator; QR pairing, then `NotifyProof` twice in one session: with the notification assistant and without). It checks: exactly 21 toasts per run (20 + a call; the player and an unchanged repeat give nothing), one snoozed on the phone disappears from Windows, the other 20 in the history, ≤ 5 pop-ups in 2 s, one 64 px icon, turning the setting off clears the history and the icons. Latency (clocks corrected for the emulator's offset): the listener call on the phone -> `Show` on the PC p95 < 250 ms (checked; measured about 20 ms), `postTime` -> `Show` reported (with the assistant about 230 ms because of a system hold of about 200 ms, without it about 30 ms). Needs `JAVA_HOME`, `ZXING_CORE_JAR`, `PLAYWRIGHT_MODULE`, `ANDROID_SERIAL`.

## SMS messages (WN-55)

The "Messages" button in the panel (when the phone is connected) opens a separate
window: threads on the left, the conversation on the right (PROTOCOL.md §9.11, `sms/1`).
The phone shares SMS only after "SMS on the computer" is turned on and access to
SMS is granted; without that the window says what to turn on. A thread opens from memory
(`sms.rs`) and fetches only newer messages (`since`); the first opening
fetches the 1000 newest, older pages when scrolling to the top. Changes on
the phone come through the watch (`Watch` → `sms-changed`). The content is
only in the app's memory (not written to disk), it disappears when another
phone connects or on unpairing, and goes onto the page only as text.
Measurement: `scripts/check-sms-android.cjs` (the real app + an emulator,
1000 SMS in a thread, opening from the click to drawing).

### Sending SMS and MMS (WN-56)

In the "Messages" window the field at the bottom sends a reply in the open thread, and "+"
starts a new message (numbers separated by commas). When the phone has several
SIM cards, a card choice is next to it. The image icon attaches a photo: the window scales
it down to 1280 px and a JPEG of about 300 KB, and the message goes as an MMS (also with
several recipients). The sending state shows at once (sending…, sent,
delivered, not sent); then the message replaces the record from the phone.
Pictures from received MMS load while scrolling. Sending has to be
turned on on the phone ("Sending from the computer"); until then the field says so.
Measurement: `scripts/check-sms-send-android.cjs`.

## Latest photos in the panel (WN-58)

When the phone shares photos ("Photos on the computer", PROTOCOL.md §9.13,
`gallery/1`), the panel shows the 10 newest photos and screenshots (two rows
of five). A new photo arrives through the watch (`Watch` → `Recent` →
thumbnails of the new ones only), without polling. Pressing a photo starts
fetching the original; dragging hands it to Windows as a file (OLE
`DoDragDrop`, a shell data object from `SHCreateItemFromParsingName`, copy
only), and a click or Enter opens it in the default viewer. When
the button was released before the original arrived, the panel asks to drag
again (the file is already waiting). Our own drag isn't taken for
files to send to the phone and doesn't hide the panel.
Thumbnails are only in memory; originals in `%TEMP%\WindConnect\gallery`
(at most 8), deleted when the session ends, when another phone connects and at
start. CCS/1 SEC:the PC allows only the final extensions jpg, jpeg,
png, webp, heic, heif, gif, avif, bmp (case-insensitively).
Any other name is replaced with `IMG_<id>.<extension from the MIME>`; no allowed
MIME means a refusal. Before opening/dragging it checks the image's signature
and writes `Zone.Identifier` (`ZoneId=3`); an error means a refusal
and deleting the partial file. The name stays one path part.
Measurement: `scripts/check-gallery-android.cjs` (the real app + an emulator:
a new photo from the command to the drawn thumbnail < 1 s, the original matching by
SHA-256, the phone's setting off/on). Dragging with the mouse itself needs
a manual test.

## Calls from the phone (WN-57)

The setting "Calls from the phone" (main window, on by default,
`call-volume.json`) shows an incoming call as a Windows
notification (`calls.rs`, the incomingCall scenario, without sound): the contact's name or
the number, "Silence" and "Reject" (only those the phone can do), it disappears when answered
or ended. Missed calls: a notification about new ones and a list in the panel ("Missed
calls", at most 5). The phone has to share this ("Calls on the
computer"). The computer accepts `call/1` when this setting
or pausing playback is on; the pause works only with the latter.
Measurement: `scripts/check-calls-android.cjs` (a call -> a notification on the PC).

## Open on the phone (WN-100)

The field "Open on the phone" in the panel (when the phone accepts `actions/1`)
sends an http(s) address to the phone (PROTOCOL.md §9.2 `open-url`, checked
by `check_url` already on the PC). The phone opens it at once or shows a
notification when it is in the background; turned off on the phone -> "The phone
refused…". The phone only opens the link, nothing else.
Measurement: `scripts/check-actions-android.cjs`.

## Relay outside the home (WN-92)

Settings in the main window: the switch "Relay outside the home" and the address
(`wss://windconnect-relay.<subdomain>.workers.dev`, relay/README.md). Off by
default (D3). The address is in the file `relay-url` next to the identity key and
comes back after a restart; an address rejected by `check_url` shows an error at the
field. On: `relay.rs` keeps one `RelayLink` for each paired
phone (like `windctl serve --relay`, retries 1–60 s; pairing and
removing a pairing change the list at once, WN-139). Off: all
connections to the relay are broken off at once, and a session that runs through them
is closed (GoingAway `relay-off`), so the phone returns to the LAN
instead of waiting for QUIC's idle limit (WN-139). The window asks for the setting
until the core has read the file (earlier, a stored "on" showed as
off right after start). What the operator sees: PROTOCOL.md §13.4.

Tests: `cargo test -p windconnect-windows relay::` and, with `npm run dev` in
`relay/`, `RELAY_URL=ws://127.0.0.1:8787 cargo test -p windconnect-windows
relay::tests::off_closes -- --ignored` (a session through two `RelayLink`s,
turning off closes it; 2026-11-16: the phone sees the end after 2.5 ms).
The real app: `node apps/windows/scripts/check-relay-native.cjs`
(a visible section, off by default, an error for a bad address, the file `relay-url`
written and deleted, the state after a restart; screenshots in `RELAY_ARTIFACTS`).

## The panel opens at the top (WN-145)

RULE:the panel remembered the last scroll position, so it could open in the middle of its content
(under a taskbar at the top it looked cut off; the window's position itself is correct,
`place` takes the taskbar on any side into account). Every opening starts at the top, and while
pairing is going on (a visible QR code or code), the pairing card at the bottom of the panel is
scrolled into view, also when the QR code appears with the panel open ("Add a phone").
CHECK:`node apps/windows/scripts/check-panel-scroll-ui.cjs` (a fake IPC, without the app's
window; without the fix it fails on "opens at the top").
