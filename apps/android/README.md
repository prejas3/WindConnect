# Android

## Accessibility: TalkBack and large font (WN-64)

CCS/1
RULE:the titles of screens and settings cards are headings (TalkBack heading navigation), icons and actions have descriptions, touch targets are at least 48 dp, the contrast of text and images meets the Accessibility Test Framework. Screens scroll, so the largest font size cuts nothing off.
CHECK:`AccessibilityTest` (instrumented): Google's Accessibility Test Framework through `tryPerformAccessibilityChecks` on the home screen (without and with a computer), the list of computers, settings, the "Done" screen and receiving files, piece by piece while scrolling, in the light and dark theme and at font scale 2; a negative control checks that the test detects an undescribed 10 dp button.

## Languages: Polish and English (WN-63)

CCS/1
RULE:every UI text is in the resources: `values/strings.xml` (English, the fallback language) and `values-pl/strings.xml` (Polish), separately in `app` and `wind-core`. A Polish phone sees Polish, any other language English; Android 13+ lets you choose the app's language in the settings (`res/xml/locales_config.xml`). `localeFilters` keeps only en and pl in the APK. Code outside Compose takes text through `str()`/`plural()` (`Strings.kt`), Compose through `stringResource`. Plurals: `<plurals>` (Polish one/few/many/other). Dates and numbers are formatted by the phone's locale.
CHECK:`TranslationParityTest` (JVM): the same names and placeholders in both languages of both modules. JVM tests read the Polish texts from `values-pl` (`StringsForTests`). Instrumented tests are run by `PolishTestRunner` (the app in Polish regardless of the emulator's language); `LocalizationTest` checks on the device that every text exists in English and in Polish.

## Receiving into the public Downloads (WN-120)

CCS/1
FLOW:a paired, admitted session -> admission with the FGS owner -> FileEngine chunk checksums -> a private verified file -> MediaStore.Downloads IS_PENDING -> fsync + journal -> public Download/WindConnect/<subfolders> -> only then the wire Complete. The engine's default hooks are None; Windows keeps its own writing. Android needs neither READ/WRITE_EXTERNAL_STORAGE nor MANAGE_EXTERNAL_STORAGE. The attempt/standby router has files/1 off; only an adopted session and a ready sink offer receiving.
LIFECYCLE:WN-127 chooses variant (a): the receiving switch is ON by default; a visible Activity holds a lease and advertises files/1 without a service or an ongoing receiving notification. Only a real Offer starts ReceiveFilesService from the visible Activity and waits for startForeground and the lease; before that there is no Partial::open, no write and no Accept. An exception/refusal/timeout of the start gives BUSY without changing partial files. Each attempt has its own token; late readiness doesn't resume an old attempt or release a newer one. The permit covers the whole transfer and publishing. The temporary FGS ends after the last active/pending receive, also with the Activity still open. An active accepted receive may outlive its closing. Without an FGS owner an offer in the background is rejected; closing the last Activity withdraws only the UI's readiness. ConnectionService after startForeground still keeps receiving with the background connection deliberately on. Turning receiving off withdraws the capability and cancels accepted transfers. We don't use IMPORTANCE_MIN to hide the FGS (Android may raise its importance).
STORAGE:limit 2 GiB/file, 4 GiB of active transfers (the reservation covers publishing), preflight 2×size+64 MiB for the private and the public copy. MediaStore picks a free name without overwriting; the journal and the UI read the real DISPLAY_NAME/RELATIVE_PATH. NFC names without control/bidi characters, up to 16 segments, 240 bytes/segment and 4096 bytes of input; MIME from the last extension, fallback octet-stream. This is a classification, not a security assessment of the file.
RECOVERY:AtomicFile, the full peer-fingerprint:id, path/size/SHA256 and the phases pending/ready/published. A restart removes only its own pending rows/private copies; an already visible ready one becomes published, and a retry of that token creates no duplicate. Journal up to 512 receipts/4 MiB; no space in the history refuses receiving. The explicit "Clear history" keeps the public files, but forgets the idempotency of old tokens. We don't delete files that are visible after a late cancel/error; after publishing the user manages them in Downloads.
UI:the home screen and the notification lead to receiving/history; Cancel identifies incoming transfers by peer+id, sending by outbound id. The notification at most 1/s during a transfer + an immediate end; names hidden on the lock screen. Open needs a separate tap, a content URI with a temporary read grant and the system's app chooser. "Show in Downloads" starts the system's downloads list. No auto-open/running of APKs.
CHECK:IncomingFilesTest on the real MediaStore (extensions json/tar.gz/none/apk, collisions, checksum, idempotency, ENOSPC/cancel, six interruption points). scripts/check-android-receive-recovery.cjs additionally force-stops the real process in each phase and checks there are no pending rows/duplicates after a retry. scripts/check-android-receive-files.cjs: real QR/TLS Windows -> Android, 7 files with a folder and 1 GiB, independent SHA256 and the sender's Complete. Needs built debug APK/androidTest, cargo build --locked -p windconnect-windows --example files_peer and an explicit ANDROID_SERIAL of a disposable emulator, JAVA_HOME/ANDROID_HOME. Data/logs in TEMP; don't run on the user's phone. A short S25 test: files+folder in My Files/Download/WindConnect, opening by hand, cancelling without a partial file. Long Wi-Fi/battery runs stay for the end of the lane.

## File progress and retry (WN-41)

CCS/1
RULE:our own screens use the engine's events (progress every 100 ms, no polling/idle timer); the system notification has a progress bar and bytes/speed at most once a second, and an immediate final state/Cancel. We don't promise SystemUI refreshing every 100 ms. Speed: monotonic deltas, a 1 s window + EMA; "—" before a full window and while waiting. While the sources are being imported the total size isn't known yet: an indeterminate bar, no made-up percentage. After all sources are prepared: the real bytes confirmed by the receiver / the size.
FLOW:after an error or cancelling, "Retry unsent" -> a light openFileDescriptor/close of each remaining URI -> confirming again the same, still paired PC -> a new import/transfer. Files confirmed as received are not retried. A lost grant requires choosing the sources again. The WN-40 confirmation and limits stay; no automatic sending from a notification action, no persisting of grants and no keeping of failed GiB copies.
LIMIT:the selection to retry lives only in the process; after it ends, choose the files again. Long measurements of physical Wi-Fi/battery are postponed. "Open folder" concerns the PC; Android's public receiving is described in WN-120 above. TalkBack gets semantic progress, not a live message every 100 ms.
CHECK:`scripts/check-android-transfer-progress.cjs`: real sources from a separate UID and a receiver in TEMP; 5 files, 2 confirmed, a controlled read error, Retry sends only 3; a missing grant blocks the confirmation; indeterminate in the Activity/notification. Needs debug APK/androidTest, files_peer and an explicit ANDROID_SERIAL of a disposable emulator, JAVA_HOME/ANDROID_HOME. Don't run on the user's phone.

## Quick notification actions (WN-97)

CCS/1
FLOW:the persistent connection notification: when connected, "Send clipboard", "Send file" (only when the PC accepts files/1), "Stop". Touching the content opens the device. Without a connection: "Open device", "Reconnect", "Stop". A change of the receiver's capabilities updates the actions; RTT measurements alone don't rebuild the notification.
SEC:both actions start the Activity directly through an explicit, immutable PendingIntent. API 31+ requires authentication; the public version of the notification has no sending actions and no device name. The Activity checks the Keyguard and asks to unlock before reading the clipboard/the automatic picker, also on API 29–30. The clipboard is read only on focus; the file is chosen in DocumentsUI, then our own WN-40 confirmation. Cancelling the choice sends nothing. Repeated clicks return to one screen; they keep the selection or the status/Cancel of an active send and don't open further pickers.
LIMIT:POST_NOTIFICATIONS must be granted and the persistent connection turned on for the actions to be visible. The look of the system notification/picker is decided by Android/One UI; the app's screens use WindApp/WindButton. An S25 test of both actions is still required.
CHECK:`cw-progress --title "WN-97 notification actions" -- node scripts/check-android-connection-notification.cjs`, with an explicit ANDROID_SERIAL of a disposable emulator and JAVA_HOME/ANDROID_HOME, after the debug APK/androidTest and building the example files_peer. The fixture uses an ephemeral pairing and a TEMP folder; it checks the text, the file's hash, the categories, withdrawing files/1 and no actions after disconnecting. Never run on the user's phone.

## Sending files to the PC (WN-40)

CCS/1
FLOW:Gallery/files -> Share -> "Send to computer · WindConnect", or Home -> "Send a file" -> the system's multi-document picker. Our own screen shows the names, the count and the paired computer; only the explicit "Send" starts reading the data and the transfer. Home requires an active session and accepted files/1; withdrawing the capability restores the unavailable state. The launcher shortcut "Send a file" opens the same file picker (WN-116), the connection notification's action is WN-97.
SEC:only content:// with a real grant from the provider; no broad storage permission and no persisting of grants. External extras don't choose the computer/key/target path. Name/MIME/size are untrusted metadata: names are reduced to portable basenames, collisions are told apart, the bytes actually read are limited. Text without a URI is not a file. Limit 100 files, 2 GiB/file, 4 GiB/batch and a 64 MiB free-space reserve. An unknown size is allowed; a declared size different from what was read stops that import.
FLOW:a temporary non-exported SendFilesService of type connectedDevice, started only by an explicit action from a visible Activity -> its own ConnectionOwner lease -> a private outbox -> the existing FileEngine. The pipeline sends a ready file while importing the next one; at most 3 unconfirmed copies, deleted after the receiver's ACK. Each file checks the space. The FFI accepts only canonical regular files from the fixed outbox and the session of an adopted, still pinned PC. A change of computer doesn't redirect the rest of the batch.
STATE:the service doesn't turn on the persistent-connection preference. The UI may be closed; the screen, when reopened, shows the service's current state and Cancel. When the session is lost, the service retries only the chosen paired target with backoff, and the engine resumes the files; no progress/connection for 120 s ends the wait. Finishing/cancelling cleans up the copies and releases the lease; no sticky replay. The transfers' state is in the process's memory: after the process is killed we don't claim automatic resumption; the next send removes abandoned folders of our own batches, the user chooses the files again.
PERF:observing transfers/capabilities through await-next-change, without polling. The FileEngine publisher waits for dirty progress, at most once/100 ms during changes; no constant timer when idle. The system notification at most once/s, the screen has the current progress. This is not proof of the WN-30 battery budget.
LIMIT:without POST_NOTIFICATIONS the FGS notification is hidden by Android; progress and Cancel are available in the sending screen once it is opened. We don't change the permission or the battery-saving settings. Receiving into the public Downloads on Android is WN-120; this client doesn't announce receiving files/1.
IO:the PC accepts files into Downloads/WindConnect, in the fixed subfolders Images/Video/Audio/Documents/Other by the last extension (the list only in wind-core files::incoming_folder). The type is organization, not a security assessment; no automatic opening. WN-41 extends the progress/speed/retry/open-folder UI.
CHECK:cargo tests files + the FFI bridge; FileImportTest, Home capability/disabled semantics and FileImportValidationTest. A real test with a separate source UID: `cw-progress --title "WN-40 transfers" -- node scripts/check-android-files.cjs`, with JAVA_HOME/ANDROID_HOME and an explicit ANDROID_SERIAL=emulator-…. Needs the debug APK + androidTest and `cargo build --locked -p windconnect-windows --example files_peer`. It clears only WindConnect on a verified disposable emulator. A separate Java-only provider/Activity exists only in the test APK; it generates 20 different JPEGs, a mixed set, pipes/an unknown size and bad grants. The PC receiver has an ephemeral pairing and TEMP as its root. The script checks hashes, categories, MOTW, a broken session, cancelling and the real DocumentsUI picker; it restores the emulator's font/settings and ends its own helpers. Artifacts: TEMP/windconnect-wn40-*; don't run on the user's phone. S25 test: 20 photos from the Gallery into the right folder and comparing the count/files, separately cancelling and closing the screen during sending.

## PC status widget (WN-33, WN-160)

CCS/1
FLOW:Settings -> "Add widget" -> the launcher's confirmation; alternatively the widget list -> "WindConnect — PC status". 4×2 by default (a base height of 110 dp for the launcher's grid computation), resizable down to 180×100 dp. WN-160: Glance responsive sizes chosen by height: small (the clipboard button; tapping the header opens the app), medium (two buttons, the battery on its own line), large (2×2: clipboard, file, touchpad, the app); no empty area. A status dot, a battery icon, buttons with icons, "Open the app"; with a large system font the medium size uses one-word labels without icons. Own Glance action surfaces, Material You/the WN-22 palette and the app's light/dark choice through the same readable scheme as the app; at least 48 dp for actions. "Send clipboard" opens the existing focused sender. The launcher accepting the request is not presented as a confirmed addition.
STATE:the battery percentage is the phone's (WN-20), the charging icon marks charging; the full description is available to TalkBack. "Connected · time" always gives the snapshot's time. The state uses the app's words ("Computer out of range", "No paired computer"). With the background connection off the widget says "Background connection off", even if an open Activity holds the connection for a moment. The clock follows the phone's 12/24 h setting.
RULE:updatePeriodMillis=0. No app timers, periodic WorkRequest, alarm or polling every minute. The observer runs only with an installed widget; it doesn't open WindCore/the connection. A change of the normalized state, percentage/charging or theme -> a snapshot in memory -> Glance.updateAll. RTT, raw errors, temperature/voltage don't change the presentation. Glance uses its own one-time WorkManager sessions. The receiver doesn't repeat sessions for already requested IDs on repeated APPWIDGET_UPDATE (protection against PACKAGE_CHANGED after WorkManager component changes); new IDs and resizing still work. Removing the last widget removes the observer and the battery receiver.
SEC:the widget/receiver/worker doesn't read or keep the clipboard. The PendingIntent leads to the existing private SendClipboardActivity, which requires focus/unlocking and keeps sensitive/limits. The provider is exported=false. From the open app's existing pairing list we keep only the name of the last non-removed PC in widget_presentation; a change/removal of a pairing updates the label also offline. We don't open the core just for the name. We don't persist Connected on disk; a new process reads the current ConnectionOwner, without restoring an old connection.
LIMIT:Android may keep the last RemoteViews after a force-stop or an unexpected kill of the process until an allowed event comes. That is why the snapshot's time is visible; we don't promise updates after a force-stop. With the connection service running, changes arrive without opening the Activity. Glance text is rendered by RemoteViews/the system font family; geometry/colours/sizes are our own, we don't use the system's default buttons.
CHECK:WidgetModelTest (off, privacy, RTT, unknown battery, the app's status words); scripts/check-android-widget.cjs and WidgetHostTest on an explicitly chosen disposable emulator (a real AppWidgetHost, percentage/temperature, the minimum resize 250×190 at 150% font, two themes, both actions, 125 s without updates). The test sets only the emulator's battery and its own preferences/host ID, and then restores them. Don't run widgetFixture on the user's phone. UiWidgetShotsTest takes screenshots of the sizes on demand (`-e uiShots true`). A full Android run separately; an S25 Ultra/One UI test is required before reporting.

### Incremental audit of the widget's libraries

CCS/1
SOURCE:the real AARs/JARs and POMs in the Gradle cache, Glance's META-INF licenses and the headers of the published sources.jar of the other modules. Comparing debugRuntimeClasspath before/after: 20 new artifacts, without raising the version of previously present libraries. The full list/hashes/source headers: the card's run record.
DEP:Glance 1.2.0 stable, compileSdk 35; the runtime WorkManager/Room/DataStore are Glance's dependencies. The versions below are those actually resolved by Gradle, not recommendations of new versions. No new native binary code. WorkManager adds the normal WAKE_LOCK and the standard scheduler components; ACCESS_NETWORK_STATE was already required by wind-core. No new runtime permission was added.

| Component | Version | License — file/source | What goes into the APK | Assessment |
| --- | --- | --- | --- | --- |
| glance-appwidget, glance, glance-material3, glance-appwidget-proto | 1.2.0 | Apache-2.0, own META-INF/LICENSE.txt; `assets/AndroidX-APACHE-2.0.txt` | Glance DEX/resources | Permissive, full text included. |
| core-remoteviews | 1.1.0 | Apache-2.0, sources.jar AOSP headers + the shared text above | DEX | Permissive. |
| lifecycle-livedata, lifecycle-service, lifecycle-livedata-core-ktx | 2.8.7 | Apache-2.0, sources.jar/POM AOSP (core-ktx an empty compatibility artifact) | DEX/metadata | Permissive. |
| datastore, datastore-core, datastore-preferences, datastore-preferences-core | 1.0.0 | Apache-2.0, sources.jar AOSP headers; protobuf below separately | DEX | Permissive; vendored code checked separately. |
| concurrent-futures-ktx | 1.1.0 | Apache-2.0, sources.jar AOSP | DEX | Permissive. |
| work-runtime, work-runtime-ktx | 2.7.1 | Apache-2.0, sources.jar AOSP | DEX/scheduler manifest | Permissive. |
| room-runtime, room-common | 2.2.5 | Apache-2.0, sources.jar AOSP | DEX | Permissive. |
| sqlite, sqlite-framework | 2.1.0 | Apache-2.0, sources.jar AOSP | DEX, a wrapper of the system SQLite | Permissive; no new .so. |
| glance-appwidget-external-protobuf | artifact 1.2.0 | BSD-3-Clause in the real `META-INF/.../LICENSE.txt`, `assets/Glance-Protobuf-BSD.txt` | A relocated protobuf runtime in DEX | Permissive; we don't equate it with the other modules' Apache. |
| protobuf-javalite inside datastore-preferences-core | 3.10.0, the version from its pom.properties | Google Inc./BSD-3-Clause, `assets/Protobuf-BSD.txt`, upstream LICENSE | A second relocated copy in DEX | Full copyright/conditions included. |

CCS/1
IO:NOTICE.txt was completed with all 20 modules and both protobuf copies. The Apache/BSD licenses are full texts in assets and are subject to a check of the final APK's contents. Our own preview layout/shape: the project's MIT. Existing fonts/ZXing/Rust unchanged.
EXT:https://developer.android.com/jetpack/androidx/releases/glance ; https://android.googlesource.com/platform/frameworks/support ; https://github.com/protocolbuffers/protobuf/blob/v3.10.0/LICENSE

## Launcher shortcuts (WN-72)

CCS/1
DEC:variant A approved by the user. WN-116: all three shortcuts work. "Send a file" opens ShareFilesActivity (PICK, the system file picker), "Touchpad" opens TouchpadActivity; both directly, without Home. The fixed IDs (send_file, touchpad) stay, and publish() at process start also calls updateShortcuts, so copies pinned earlier get the new labels and targets.
FLOW:holding the icon -> the static send_clipboard (available from installation) and the dynamic send_file/touchpad (published at process start). Settings -> one button per shortcut -> requestPinShortcut; an entry can also be dragged from the launcher's menu onto the home screen. The pin dialog is rendered by the launcher; the app's content uses our own theme components.
RULE:stable IDs; the clipboard is immutable, pinned by the ID alone. Files/touchpad have "unavailable" in the label and a separate explanation screen instead of pretending to work. A request to the launcher is not a confirmation of pinning: the UI says only that the request was handed over. The limit of dynamic entries takes the static clipboard into account.
SEC:the exported LauncherShortcutActivity accepts only three fixed actions; it ignores URIs/extras. Clipboard -> the existing SendClipboardActivity, without Home, reading only on focus/unlocking; offline/sensitive/limits as in WN-35. File/touchpad: the target screen itself says there is no connection or no accepted capability; the shortcut sends nothing by itself.
TEST:LauncherActionTest, LauncherShortcutsTest (registration, no duplicates, direct targets/offline, rejecting a foreign action). A physical S25 Ultra/One UI: holding the icon, three entries, dragging and pinning from Settings; a working clipboard to the PC and returning to the previous app, offline, locking/cancelling unlocking; file -> the file picker and the sending screen, touchpad -> touchpad (offline: the screen's message), without Home; copies pinned before WN-116 after an update. Light/dark, 150% text and TalkBack. An emulator doesn't replace Samsung's launcher.

## Quick Settings tile (WN-32)

CCS/1
FLOW: Android 13+: Settings -> "Add to Quick Settings" -> the system dialog requestAddTileService. Added/already added -> confirmation; refusal -> no retry, the action stays in Settings. Older Android: instructions for editing the panel by hand instead of a button. Tapping the tile -> unlocking -> the existing clipboard sending screen -> back after delivery. Holding -> the app.
STATE: active = a connected computer; inactive = connecting or no connection. The name/states are described for TalkBack. SystemUI renders the tile and the system theme's colours; the app provides its own icon and text.
RULE: the panel observes the state of the existing ConnectionOwner, without a new client, connection, timer or clipboard read. Observing lasts only while the TileService is listening. The last client release resets the state; reopening the panel reads the current state at once. The connection service updates the same state without opening the Activity.
SEC: the TileService is exported only with the system's BIND_QUICK_SETTINGS_TILE. Reading the clipboard happens after unlocking and SendClipboardActivity getting focus; the existing limits/sensitive apply. No payload and no raw errors in the tile.

Tests: `:app:testDebugUnitTest` (states, losing the connection, privacy),
`ClipboardTileRegistrationTest` in `:app:connectedDebugAndroidTest` (the service's
declaration, permission, discovery, the hold target).
On an S25 Ultra / One UI: add the tile, close the app with the service running,
disconnect/restore the PC and check the state change in the open panel and after
reopening it. Copy text, tap the tile, check the PC's clipboard and the return;
repeat with the screen locked and unlocking cancelled (no sending), a sensitive
clipboard and without a connection. Holding opens WindConnect. Check
the system's light/dark theme, 150% text and TalkBack reading. An emulator doesn't
confirm the behaviour of Samsung's panel.

The WN-93 icon "Orbita" has adaptive and monochrome layers, its own white
notification vector and mdpi–xxxhdpi mipmaps. Sources and regenerating:
`assets/brand/README.md`. On an S25 Ultra check the One UI launcher with themed
icons on and off, and the persistent notification; an emulator confirms
Android's resources/APIs, but not the behaviour of Samsung's launcher.

A Kotlin / Jetpack Compose app with a QR scanner and confirmation of pairing with the PC.
The `:wind-core` module contains UniFFI 0.32.2 bindings, a coroutine wrapper with
`StateFlow` and the Rust libraries for arm64-v8a and x86_64. Requires Android 10+
(minSdk 29). QR pairing uses the WN-19 protocol through UniFFI;
debug keeps an additional screen for a manual connection test with the PC.

## Building

Requirements: JDK 17, Android SDK Platform 35, Build Tools 35.0.0,
NDK 27.0.12077973 and Rust/Cargo in PATH (the version is pinned in `rust-toolchain.toml`,
together with both Android targets; rustup installs them on first use).
Set `JAVA_HOME` to the JDK folder and `ANDROID_HOME` to the SDK folder
(or write the `sdk.dir` path into a local, ignored `local.properties`).
The Gradle Wrapper downloads the pinned Gradle 8.11.1 and checks the distribution's SHA-256.

From the `apps/android` folder on Windows:

```powershell
.\gradlew.bat --no-daemon :app:lintDebug
.\gradlew.bat --no-daemon :app:assembleDebug
```

On Linux/macOS use `./gradlew` instead of `.\gradlew.bat`.
Output: `app/build/outputs/apk/debug/app-debug.apk`.

ABI (WN-95): a call with only debug tasks (`assembleDebug`,
`connectedDebugAndroidTest`, `testDebugUnitTest`, `lintDebug`…) outside CI
compiles and packages one ABI, `x86_64` by default (the emulator). For a phone
(S25 Ultra) add `-PwindAbi=arm64-v8a`; both at once: `-PwindAbi=all`.
Release, `build` and everything in CI (the `CI` variable) build both ABIs. The Rust
libraries go into the repository's shared `target/` (without a second copy in
`wind-core/build`); cleanup: `scripts/clean-build.ps1` (it removes MSIs only of
versions older than `[workspace.package]` from `Cargo.toml`, and without a version
read, none; test: `pwsh scripts/test-clean-build.ps1`).
The debug key is generated locally by Android's tools; the APK is for testing.

## CI and installation

The **Android debug APK** workflow runs lint and builds the APK for a PR,
a push to `master` and a manual run. In Actions download the artifact
`WindConnect-android-debug-apk` and unpack the ZIP.

On a phone with Android 10+ turn on USB debugging, approve the computer and run:

```sh
adb install -r app-debug.apk
adb shell am start -W -n io.github.prejas3.windconnect/.MainActivity
```

The "Connect to the computer" screen should appear. Debug: the button "PC connection test"
opens diagnostics with Android's fingerprint and public key.
The working icon is the project's own resource under the MIT license. The app
asks for the CAMERA permission only after choosing the scanner and pressing "Allow the camera";
receiving a link from the system camera doesn't need this permission. It also uses INTERNET. Release
test packages are published on GitHub Releases (WN-162).

The Gradle Wrapper is a build tool under the Apache-2.0 license;
the license text is in `gradle/wrapper/LICENSE`.


## QR pairing (WN-26)

CCS/1
FLOW: "Scan the QR code" -> CAMERA permission -> Camera2/YUV -> ZXing -> the strict Rust parser §12.4 -> the name and fingerprint screen -> an explicit "Pair devices" -> the authenticated WN-19 protocol -> storing the pinned key.
IO: The link `windconnect:pair?v=1&k=…&s=…&a=…[&n=…]` handled through ACTION_VIEW/DEFAULT/BROWSABLE (the scheme from PROTOCOL.md). An external camera opens the confirmation screen; a warm start is handled by onNewIntent. A link never starts pairing by itself.
SEC: The scan/intent use the same Rust parser: the 1024-byte limit, the version, the Ed25519 key, the one-time secret, addresses and repeated fields. A bad link grants no trust. The name from the link is untrusted; the UI also gives the fingerprint. We don't display or log the secret; the intent is cleared, the offer doesn't go into saved state/preferences. After the process is recreated, the code has to be scanned again.
STATE: The offer lives in a Rust object that erases the secret when it is dropped. Cancelling drops the offer; parallel intents don't replace an ongoing pairing. An attempt lasts at most 30 s; after an error a new code is required. The secret is consumed by the FFI once. Confirmed keys: noBackupFilesDir/paired.txt, loaded when WindCore opens; the private key stays in the existing KeystoreIdentityStore.
RULE: Damaged lines of paired.txt are skipped as in WN-19. load_paired returns the valid devices and a separate skipped_records; valid keys stay pinned. The number of skipped entries goes through WindCore to a non-blocking message on the home screen. A file read error is still a store error; a bad line doesn't block the start or cause the file to be cleared.
RULE: The camera only on the scanner screen and in the resumed state; the session/images are closed on leaving or onPause. No saving of photos, sending of frames or background camera service. A refused permission has its own message and a way to the settings; the system link works without the CAMERA permission. No camera doesn't block installation.
LIMIT: The devices screen and the service belong to further cards of the lane. WN-25 adds the QR code on the PC and connecting automatically after pairing: the address from discovery, the key only from the stored trust, a 7 s limit. No connection doesn't undo the pairing; the UI status follows the session. The physical behaviour of Samsung Camera on an S25 Ultra/One UI needs checking: the camera must offer to open the custom URI `windconnect:pair`. VIEW handling confirmed by an Android test doesn't replace this camera test.

Tests:

```powershell
cargo test --locked -p wind-ffi --test pairing
.\gradlew.bat --no-daemon :app:connectedDebugAndroidTest
```

CCS/1
STATE: PairingTest: a VIEW intent, explicit confirmation, cancelling without storing trust, recreating the Activity and a further intent, bad/foreign/too long links, the QR code in four rotations, empty frames and entering/leaving the scanner. The Rust bridge: real QUIC, no connection before confirmation, the key persisting after reopening, refusing a replay and no secret in errors.
IO: An optional local Android peer: `cargo run --locked -p wind-ffi --example qr_pairing_peer -- <TEMP folder>`. It writes a test offer.txt (a one-time secret, 120 s), listens on loopback and gives the emulator's 10.0.2.2 address. The test `PairingTest#explicitConfirmationPairsWithRealPc` takes the instrumentation argument pairingLink from this file. Don't use it as a production server or for real identities; the file stays only in TEMP, outside the test log.

A new dependency shipped in the APK:

| Component | Version | License and source files | Where it goes | Audit result |
| --- | --- | --- | --- | --- |
| ZXing Core | 3.5.4 | Apache-2.0, upstream LICENSE and NOTICE; core class headers and POM | DEX; assets/ZXing-LICENSE.txt and ZXing-NOTICE.txt | Permissive; an unchanged library, no transitive runtime dependencies. Full texts included and verified in the APK. |

CCS/1
EXT: https://github.com/zxing/zxing/tree/zxing-3.5.4/core ; license sources https://github.com/zxing/zxing/blob/zxing-3.5.4/LICENSE and NOTICE. The WN-26 audit covers the increment; the existing DM Sans/AndroidX/JNA/Rust stay without new resources/licenses. Camera2, our own preview and buttons add no UI library.

## Identity key (Android Keystore)

`identity/KeystoreIdentityStore.kt` keeps the device's identity key
(PKCS#8 PEM from `wind-core`) in `noBackupFilesDir/identity.key`, encrypted with
AES-256-GCM with a key from the Android Keystore (StrongBox when the phone has it).
The AES key can't be exported. The Ed25519 key itself can't live in
the Keystore (StrongBox has no Ed25519, the Keystore has it only from API 33,
and TLS in `wind-core` signs in-process), so on disk it is protected by hardware,
and while in use it is in the app's memory. Where the AES key actually is
is told by `securityLevel()` (StrongBox / TEE / software), read from `KeyInfo`.
The UniFFI adapter uses this store in `WindCore.open`; a read or decryption error
stops initialization, instead of generating a new identity.

Instrumented tests (an emulator or a phone with USB debugging), from
the `apps/android` folder:

```powershell
.\gradlew.bat --no-daemon :app:assembleDebug :app:assembleDebugAndroidTest
adb install -r -t app/build/outputs/apk/debug/app-debug.apk
adb install -r -t app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk
$runner = "io.github.prejas3.windconnect.test/androidx.test.runner.AndroidJUnitRunner"
$test = "io.github.prejas3.windconnect.identity.KeystoreIdentityStoreTest"
adb shell am instrument -w -r $runner
```

On a real phone add `-e requireHardware true`: the test then requires
the AES key to be in the TEE or StrongBox (an emulator has a software Keystore, so
this variant won't pass there). The status `wrappingKeySecurityLevel` in the result
says where the key is.

A device restart (the key survives a reboot):

```powershell
adb shell am instrument -w -r -e phase write -e class "$test#restartPhaseWrite" $runner
adb reboot; adb wait-for-device
# wait until `adb shell getprop sys.boot_completed` returns 1
adb shell am instrument -w -r -e phase verify -e class "$test#restartPhaseVerify" $runner
```

`bootId` must differ between the two phases, and `restartVerified` must equal `restartWritten`.

## The Gradle module and the coroutine interface (WN-17)

`crates/wind-ffi` is a separate UniFFI adapter over the unchanged `wind-core` API.
Gradle builds a host library for generating Kotlin, and then two Android NDK
libraries, with ELF pages aligned to 16 KB. The generated output goes
only into `wind-core/build/`; the repository holds the sources and the pinned
generator, not hand-edited bindings or binaries.

```powershell
.\gradlew.bat --no-daemon :wind-core:assembleRelease
.\gradlew.bat --no-daemon :wind-core:publishReleasePublicationToBuildRepository
```

AAR: `wind-core/build/outputs/aar/wind-core-release.aar`.
A local Maven repository with the AAR, sources, POM and Gradle metadata:
`wind-core/build/repo/`. Coordinates: `io.github.prejas3.windconnect:wind-core:0.41.0`.
Publishing writes the files locally; it doesn't send them to an external repository.
A consumer must use the POM/metadata (JNA and coroutines are transitive
dependencies); a copy of the AAR file alone won't provide JNA.

In this repository the app uses `implementation(project(":wind-core"))`.
A consumer outside the repository adds the local repository to
`dependencyResolutionManagement.repositories` in its `settings.gradle.kts`:

```kotlin
maven { url = uri("/path/to/wind-core/build/repo") }
```

and `implementation("io.github.prejas3.windconnect:wind-core:0.41.0")`.
In the consumer's app limit the ABIs to `arm64-v8a` and `x86_64`.

```kotlin
val core = WindCore.open(context, "My phone")
// Collect core.state: StateFlow<ConnectionState> in the owner's lifecycle.
// peerKey is the 32 bytes of the Ed25519 key, verified over another channel.
try {
    core.connect("192.168.1.10:47820", peerKey)
} finally {
    core.shutdown()
}
```

Network calls are suspending, and Keystore initialization runs on
`Dispatchers.IO`. `Connected` means completed mTLS, the Hello/HelloAck exchange
and a successful Ping/Pong. Closing the owner stops observing the connection,
closes the endpoint and releases the UniFFI handles; `shutdown` is cancellation-safe.
The session state is not a persistent record of pairings. In this card the module offers
outgoing connections and session diagnostics; the feature modules are not yet wired
to the app. Network changes wake discovery and an attempt to the last chosen peer
as described in WN-16 below.

## A real Android → PC test

1. Start the APK. Copy the displayed public key of Android (64 hex).
2. In the repository root run:
   `cargo run --locked -p wind-ffi --example android_peer -- <Android_key> 0.0.0.0:47820`.
   The program trusts only this key, prints `PC_PUBLIC_KEY` and ends after
   disconnecting or after 90 s. The PC identity in this tool is temporary.
3. On the phone's screen type the PC's IP and port 47820 and the checked
   `PC_PUBLIC_KEY`. Press "Connect with the given key". The screen should
   show "Connected: WindConnect PC test; ping … ms", and the PC `CONNECTED`.
   The PC's firewall must allow the test UDP port.
4. Press "Disconnect". The PC program ends.

On an Android emulator the host's address is `10.0.2.2:47820`; the PC process may then
bind only `127.0.0.1:47820`. The screen's instrumented test performs the
same actions:

```powershell
adb shell am instrument -w -r -e class io.github.prejas3.windconnect.CoreConnectionTest#identityLoadsThroughUniFFIAndSurvivesActivityRecreation $runner
# Read androidPublicKey from the result and run android_peer with this key.
adb shell am instrument -w -r -e class io.github.prejas3.windconnect.CoreConnectionTest#screenConnectsToPc -e pcAddress 10.0.2.2:47820 -e pcKey <PC_PUBLIC_KEY> $runner
adb shell am instrument -w -r -e class io.github.prejas3.windconnect.CoreApiTest $runner
```

The first test checks the native library, the Keystore and recreating the activity;
the second a real session between Android and a PC; the third callback errors and
cancelling the connection coroutine. Without the PC parameters the network test is skipped.
Checking an S25 Ultra / StrongBox and a physical network needs a phone; an emulator
confirms x86_64 and the software Keystore. arm64-v8a is compiled into the AAR/APK.

## Network changes (WN-16)

The `WindCore` owner registers a `ConnectivityManager.NetworkCallback` for
networks that need no restricted permissions, also Wi-Fi without Internet.
`onLost` and `LinkProperties` changes go into a `CONFLATED`
channel; the callback performs no network operations. Handling on IO calls
the suspending `CoreClient.networkChanged()` in a Tokio context: it changes the QUIC
socket, signals a new generation and wakes the current connection attempt.

`core.network` is a `StateFlow<NetworkSnapshot>` with the generation, the current
fingerprints/addresses and a possible error. Each generation removes the old view of
addresses and creates a new mDNS daemon with the current interfaces and the endpoint's port,
without waiting for a periodic timer. No network or an mDNS error stays visible
in the state; the next network change retries discovery.

A change is a difference in the interface or the network's addresses compared with what
discovery uses (WN-103, `NetworkChangeFilter`). Right after the callback is
registered Android repeats the networks it already has (`onAvailable`, `onLinkPropertiesChanged`);
earlier this restarted mDNS about 1 s after start, during probing, and
a browser started right away didn't see the phone within 5 s in 8 of 10
runs. The observer reads the network state before discovery starts, so a change
between the read and the repeat still counts. `onAvailable` doesn't decide (from API
26 `onLinkPropertiesChanged` always follows it). Checking:
`NetworkChangeFilterTest` (`./gradlew :wind-core:testDebugUnitTest`) and on
an emulator or a phone with connected Wi-Fi (NsdManager doesn't do mDNS on
cellular):

```powershell
adb shell am instrument -w -r -e class io.github.prejas3.windconnect.MdnsStartupTest $runner
```

`a_browserSeesPhoneRightAfterStart` opens `WindCore` 10 times and right away
browses `_windconnect._udp` with the system NsdManager (`-e runs N`); it requires
finding it within 5 s and generation 0 after 3 s. `b_networkChangeStillRestartsDiscovery`
turns Wi-Fi off and on, requires a new generation after both changes and waits for
Wi-Fi to come back. LIMIT: after turning Wi-Fi on the network recovers in steps
(restarts about every 0.6 s), so the phone comes back in the browser at varying times (5.5 s – over
20 s), the same as before WN-103; the test reports it (`seenAfterWifiOnMs`). An emulator
is sometimes left without a connection after a few Wi-Fi toggles; the test reports this as
a precondition, not a regression.

If the previous attempt to a manually chosen peer failed or the session
has already closed, a network event starts a new attempt. An ongoing attempt is
woken without waiting for its 10 s limit. An active session uses QUIC
migration. Disconnecting by hand, cancelling an attempt and a trust refusal remove the intent
to connect automatically. Trust is never created from mDNS.
The last peer's address stays the manually chosen address; automatically choosing
a new IP from the device list belongs to the device management integration.

The module declares the normal permissions `ACCESS_NETWORK_STATE` and
`CHANGE_WIFI_MULTICAST_STATE`. The `MulticastLock` is taken only when
discovery is needed (WN-83, `MulticastPolicy`):

- without an active session for 30 s from start, from a network change, from losing the session
  or from `WindCore.refreshDiscovery()` (e.g. refreshing the device list);
- the whole time a screen that needs discovery is open (the SAS
  pairing window, the device list): `WindCore.discoveryScreen(true)` …
  `discoveryScreen(false)`, calls counted in pairs.

An active session doesn't hold the lock, also after a network change: it comes back through QUIC
migration (WN-16, WN-84). `shutdown()` releases the lock together with the callback, the jobs
and the daemon. The rules are checked by `MulticastPolicyTest`
(`./gradlew :wind-core:testDebugUnitTest`), and on a device by
`MulticastLockTest`, which reads the list "Multicast Locks held" from
`dumpsys wifi` (needs `android_peer` as in the Android → PC test; `-e
toggleWifi true` adds a real network change):

```powershell
adb shell am instrument -w -r -e class io.github.prejas3.windconnect.MulticastLockTest -e pcAddress 10.0.2.2:47820 -e pcKey <PC_PUBLIC_KEY> -e toggleWifi true $runner
```

The phone finds other computers during a session (several PCs, WN-90) with the broadcast
fallback discovery (WN-15), without the lock. The basis is the phone's query
sent every 2 s (sending doesn't need the lock) and each PC's unicast answer.
Other PCs' broadcast queries reaching the phone are an extra, not
the basis: without the lock and with the screen off the Wi-Fi chip may filter them.
After a network change the second PC returns to the `WindCore.network` list within 5 s.
During a session there is no mDNS window: with the second PC off it would hold the lock
30 s after every network change. On the host this is checked by
`query_is_answered_by_unicast_so_the_asker_needs_no_broadcast`
(`cargo test --locked -p wind-core --test broadcast`), on an emulator by
`MulticastLockTest#otherPcFoundDuringSessionWithoutLock` through the runner
(the debug and androidTest APKs built,
`cargo build --locked -p wind-ffi --example android_peer`):

```powershell
node apps/android/scripts/check-multi-pc.cjs
```

The runner redirects the emulator's UDP (`emu redir`, 47823 → 47821 by default;
`REDIR_PORT`, `PC_PORT` change the host's ports), starts `android_peer` as the
first PC and sends the second PC's discovery packet every 2 s. LIMIT: through the emulator's
NAT the packet arrives without a query, so the test checks the session, the lock and
the discovery memory after a network change. The query and the unicast answer are checked by the host
test, and broadcast filtering by a real Wi-Fi chip without the lock needs
a phone.

The app version and Android's `versionCode` follow the repository's single version
(CONTRIBUTING.md, WN-114); release numbers only grow, so that updating the APK and the MSI never
goes back.

## First start (WC-34)

CCS/1
FLOW:no paired computer -> the card "Welcome to WindConnect" at the top of the home screen (what is needed, the button "Scan the code from the computer" as in step 2 on the PC, a sentence about the camera before the system's question) -> after the tap, the camera question at once -> the scanner -> "Done".
RULE:"Done" asks one question: "Keep the connection in the background" (ConnectionService + POST_NOTIFICATIONS on API 33+) or "Not now". Pairing by itself no longer turns on the service. Notifications refused: the service works, but without a visible notification there is no computer state and no "Send clipboard" action; the screen says so in one sentence and leads to the notification settings. Settings -> "Background connection" changes the decision later.
A11Y:the headings of the "Welcome" and "Done" cards are marked for TalkBack.

Test: `OnboardingTest` (the "Welcome" card, the "Done" decision, the sentences about effects);
`PairingTest` with `-e background true` chooses "Keep the connection in the background".

## Paired computers (WC-28)

CCS/1
FLOW:Home screen / Settings -> "Paired computers": a local name, "Connect", "Remove" (a question in the card, without a dialog).
RULE:removing with the computer connected sends Goodbye UNPAIRED at once; without a connection the key is revoked immediately, and the notice goes when discovery shows that computer (once per network generation, a full handshake without 0-RTT). A computer that removed the phone closes the session with the code UNPAIRED: the phone removes the peer's key from TLS and doesn't come back.
RULE:BUSY (the computer has another phone) and a TLS refusal ("The computer refuses the connection — pair again") hold back the background service's automatic retries until a network change or "Connect".
LIMIT:after BUSY the phone doesn't come back by itself when the computer becomes free. The removal notice is owed for 30 days.

Tests: `DevicesScreenTest` (the list, renaming, the removal question,
the end-of-session messages), `cargo test --locked -p wind-ffi --test bridge`
(`ffi_paired_list_rename_unpair_both_ways_and_busy`).

## Home screen (WN-31)

CCS/1
FLOW: the shared WindCore -> PairingViewModel -> the real name of the paired PC, the session state, the last RTT and the LAN/relay route. Disconnecting removes the RTT; the name stays. No additional polling and no CPU locks.
DEC: the four tiles from the mockup are inactive until their channels are wired; the transfer history shows an explicit empty state, without demo files. Settings under the gear keep pairing and the background service's options.
PERF: release uses R8 and the resource shrinker. The consumer rules keep the UniFFI/JNA reflection; only four AWT classes that Android doesn't have are silenced. `ReportDrawnWhen` waits for the local identity and the store, not for the remote PC to be reachable.

Measurement: install a signed **release** APK, set `ANDROID_HOME`, `ANDROID_SERIAL`,
run `cw-progress --title "Android: release startup measurement" -- node apps/windows/scripts/check-startup-android.cjs` from the repository root.
The script keeps the app's data, ends its process after measuring and records 10 cold
TTID/TTFD and the `WaitTime` of 10 returns from Home into `%TEMP%/windconnect-startup-*`.
The return `WaitTime` is the system command's time, not a TTID measurement.
The default `STARTUP_COMPILATION=verify` turns AOT off; a separate `speed` forces AOT
and must be reported separately. The <400 ms threshold has to be checked on the target
phone; an emulator's result is no proof of S25 Ultra performance.

The real session test and screenshots of both themes: after building debug/androidTest and
`cargo build --locked -p windconnect-windows --example background_peer`, set
`HOME_SCREEN_PROOF=1` and run `apps/windows/scripts/check-background-android.cjs`.
This test needs a disposable emulator and clears only its WindConnect data;
it checks real QR/TLS, recreating the Activity, settings and the tiles' state.

## Background connection (WN-29)

CCS/1
FLOW: confirmed pairing -> turning on the `connectedDevice` service -> a persistent notification with the PC's name, the real state and the actions "Open device", "Reconnect" (offline), "Stop". Clipboard/files are separate modules; the notification doesn't pretend to send them before they are implemented.
RULE: The Activity and the service lease one WindCore in the Application. Closing/rotating the screen releases only the screen's lease; the last lease closes the native handles, discovery and the session. Turning the background off disconnects the session and stores the off state before stopping the service.
FLOW: START_STICKY recreates the service after the system recovers the process. BOOT_COMPLETED after unlocking and MY_PACKAGE_REPLACED resume only a service that was turned on before; nothing starts before the identity is unlocked. A system force-stop requires opening the app again. Missing permissions for a background start give a message at the next opening.
SEC: reconnecting loads only persistent paired keys; discovery provides the address, not trust. Retrying with a 3–60 s backoff, without a default relay, alarms or new network channels. Currently the last stored computer is chosen; choosing among several devices is developed in WN-31.
PERF: the app holds no wake lock and no constant CPU keep-awake (the WAKE_LOCK permission in the manifest comes from WorkManager, which Glance uses; the Doze test checks that in Doze and after "Stop" the app's UID holds no wake lock beyond WorkManager, WN-141). A QUIC keepalive every 15 s on the PC and Android side already exists in the transport. An FGS doesn't lift Doze's network restrictions; the battery exemption is the user's decision. Offline without a CPU lock. The daily energy use is measured by a separate card, WN-30; an emulator doesn't prove the <1%/day budget.

### Phone and the overnight test

1. Install `app/build/outputs/apk/debug/app-debug.apk` and pair the PC. Allow notifications to see the device and the actions; after a refusal the section "Background connection" leads to the notification settings.
2. In the section "Background connection" open battery optimization and set WindConnect as not optimized. The FGS alone doesn't lift Doze's network limits. The app doesn't change this permission by itself.
3. Samsung: Settings → Battery → Background usage limits → never sleeping apps; add WindConnect and remove it from the deep sleeping list. The names may differ between One UI releases. Check the setting again after an update.
4. Note the model, Android/One UI, the APK version, the battery state and the battery-saving settings. The PC should stay running on the same network. Close the app's screen, turn off the phone's screen and unplug the charger for 8 h. In the morning check "Connected", a working session and the battery change. Repeat with a phone restart: after unlocking the service should come back without opening the app. "Stop" should turn it off also after a further restart.

CCS/1 LIMIT: an API 35 emulator with forced Doze and an explicit battery exemption checks the life cycle; it doesn't replace a night on an S25 Ultra/the newest One UI. Don't claim the overnight criterion is met without this measurement.

Sources of the platform rules: [the connectedDevice type](https://developer.android.com/develop/background-work/services/fgs/service-types), [Doze and battery exemptions](https://developer.android.com/training/monitoring-device-state/doze-standby), [starting an FGS from the background](https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start), [Samsung's app management](https://developer.samsung.com/mobile/app-management.html).

### Repeatable checks

```powershell
cargo build --locked -p windconnect-windows --example background_peer
# ANDROID_HOME and JAVA_HOME as in the WN-25 QR tests; the QR matrix encodes the same Rust qrcode/ECC M as the PC app
node apps/windows/scripts/check-background-android.cjs
```

CCS/1 STATE: the fixture requires a verified emulator and clears only its test app. The PC peer has an ephemeral identity and a 2100 s limit; the installed PC app isn't stopped. The QR image is decoded on Android, full pairing and TLS without bypassing trust. The public discovery packet is passed through the emulator's NAT. The test closes the Activity, forces 60 s of Doze (SOAK_SECONDS=1800: 30 min; BATTERY_EXEMPT=0: without the battery exemption), checks there are no CPU locks, tries the Stop action, restarts the service and restarts the emulator; it requires a new real session after BOOT_COMPLETED. It restores the power settings and removes the raw one-time link. TEMP/windconnect-wn29-*/ contains the result and logs. ConnectionOwnerTest separately checks sharing, the last release and the persistent identity.

Test on an emulator/phone: `CoreApiTest#platformNetworkCallbackWakesDiscoveryWithoutPolling`
confirms the real system callback and the generation from native discovery.
The opt-in `CoreApiTest#wifiReturnWakesNativeDiscovery` with the instrumentation argument
`-e toggleWifi true` turns Wi-Fi off and back on on the chosen test device,
and then confirms a new discovery generation; by default this test is skipped.
A manual test: enter the PC's verified key and address, try to connect out of
the PC's range, and then come into that PC's Wi-Fi. The attempt should start after the network
event. Checking a switch between physical networks on an S25 Ultra remains
a hardware test.

## Relay outside the home (WN-60)

`WindCore.setRelay(url)` turns on the E2E relay (PROTOCOL.md §13) with the address
`wss://…`, and `setRelay(null)` turns it off. `relayUrl: StateFlow<String?>`
shows the state. The setting is stored in SharedPreferences
(`windconnect.core`) and comes back after a restart. By default the relay is
off (D3). The switch "Relay outside the home" in the settings
(`RelayControls.kt`, WN-92) stores the address, shows an error for a bad address at the
field, and turning it off at once ends the session through the relay (`relay-off`) and
tries the LAN. The home screen shows "Connected through the relay". With the relay
on, `connect` tries the PC's address for 2 s, then connects through the
relay. `ConnectionState.Connected.route` is `"lan"` or `"relay"`. After
a network change during a session through the relay the core tries the LAN address, and
if it answers, switches to the LAN.

Test on an emulator (Wi-Fi turned off by the test, only the cellular network): with
`npm run dev` in `relay/` and `windctl serve --relay ws://127.0.0.1:8787 --trust
<Android key> --bind 127.0.0.1:47821`:

```powershell
adb shell am instrument -w -r -e class io.github.prejas3.windconnect.RelayRouteTest -e pcKey <PC_PUBLIC_KEY> -e relayUrl ws://10.0.2.2:8787 -e awayAddress 10.0.2.2:47999 $runner
```

`awayAddress` is an address that doesn't answer (home out of range). The result from
2026-11-10, emulator API 35: `network=cellular only`, `route=relay`,
connected after 2057 ms (including the 2 s LAN attempt), ping 6 ms. A real cellular
network needs a deployed relay (relay/README.md) and a phone.

Turning it off during a session (WN-92): `RelaySettingsTest#turningTheRelayOffEndsARelaySessionAtOnce`
with the same arguments (`awayAddress 192.0.2.1:47820`). The result from 2026-11-16,
emulator API 35, a local `wrangler dev`: a `route=relay` session, the end after 86 ms,
then a LAN attempt without returning to the relay.

Turning it off during connecting (WN-139): wind-ffi reads the relay once, at the
start of `connect`, so a connection in progress may still end up on the
relay. `adopt` checks this after accepting the session and releases it at once
(like `leaveRelay`). Test: `RelaySettingsTest#turningTheRelayOffDuringAConnectLeavesNoRelaySession`
with the same arguments; 2026-11-16: 4/4 without a session through the relay, and without
the fix 2/2 stay `route=relay`.

## Battery (WC-30)

The plan's budget: < 1% of battery per day. Component measurements:

| Date | Device | Measurement | Result |
| --- | --- | --- | --- |
| 2026-11-10 | emulator API 35 (x86_64), `MulticastLockTest` with `android_peer` | `MulticastLock` (WN-83) | taken from start; released 243 ms after the session was set up; with an active session a network event and a refresh don't take it; after losing the session taken after 237 ms and released by itself after 30 253 ms; after turning Wi-Fi off without a session taken after 958 ms; released after `shutdown()` |
| 2026-11-11 | emulator API 35 (x86_64), `check-multi-pc.cjs` (WN-90), 3 runs | `MulticastLock` with a session with PC1 and a second PC on the network | the lock not taken during the whole session (checked every 200 ms); the second PC on the list after 912–920 ms; after turning Wi-Fi off after 1 818–1 870 ms, after turning it on after 1 850–1 872 ms (a packet every 2 s); the session with PC1 kept |

Before WN-83 the lock was held for the whole life of `WindCore`. Now, with a constant
connection to the PC, it is held only while the session is being set up, and without
a connection for 30 s windows. Looking for other PCs during a session (WN-90) adds
no time with the lock: it goes through broadcast. Usage in mAh needs a measurement on a phone
(S25 Ultra, Battery Historian) and belongs to WC-30.

## Theme and components (WN-22, WN-160)

`ui/Theme.kt` defines DM Sans and the palette from the accepted WN-20. By default
Android 12+ takes Material You from the wallpaper (`dynamicLightColorScheme` /
`dynamicDarkColorScheme`); Android 10/11 keep the teal/graphite palette.
WN-160: `readable()` gives the app and the widget one readable scheme: under Material You cards take
`surfaceContainerHigh`, and secondary text on a card is pushed to at least 7:1 contrast,
accents and errors to 4.5:1, lines to 1.8:1 (`ThemeContrastTest`).
The System theme follows the OS configuration, and the manual light/dark choice and the dynamic
colour option are remembered in the app's private preferences.
Changing the settings needs no restart. The content respects safe drawing/IME insets,
font scaling and accessibility semantics; the system bars' icons match
the theme. The icons and the geometry of cards/actions come from the WN-20 mockup.

`ui/Components.kt`: our own buttons, cards, action tiles, fields based on
BasicTextField, a Canvas switch and vector icons. No default Button,
Switch or OutlinedTextField. Roles, field descriptions, errors and progress are in Compose semantics;
interactive elements are at least 48 dp. The manual connection screen still
uses the real core and a pinned key; its test tags were kept.

In debug press **Preview** in the header. The screen demonstrates both themes, the system/brand
palette, the SZYMON-PC card, four actions, a transfer, a switch and field validation.
This is sample data: a click doesn't connect, doesn't lock the computer and doesn't send
files. **Connection** restores the real screen. Release doesn't show the preview
(`BuildConfig.DEBUG`); tests and the sample device's state don't reach the release's
UX. No notifications/QS/widget were added here — those are separate features from the mockups.

DM Sans is the same unchanged font as in WN-21; a copy of the OFL and NOTICE
are in `app/src/main/assets`, the font in `res/font`. No dependencies or fonts
from the network were added. Icons and our own code: MIT.

Tests: `./gradlew :app:connectedDebugAndroidTest` on a connected device.
`ComponentsTest` checks the themes, dynamic colour, an action, the switch, validation,
recreating the Activity and returning to the real screen; it saves screenshots of both palettes
in the debug app's cache (`wn22-Light.png`, `wn22-Dark.png`). CI compiles the
instrumentation APK; running on a device is local. In the end check on an S25
Ultra / the newest One UI, also a large font and gesture navigation. An API 35 emulator
doesn't replace checking the layout and the wallpaper colour on a Samsung.

## Pairing without the camera (WN-27)

The QR screen has "Pair without the camera": a list of computers with an open code
window (mDNS TXT `pair`, PROTOCOL.md §12.4; multicast turned on for the screen's duration,
`discoveryScreen`). A choice connects to the pairing port (`CoreClient.startSas`),
both screens show the same code (`SasPairing.code`), and a pair is made only
after "The codes are the same" on the phone and on the computer (`SasPairing.decide`).
The list confirms nothing; the address is a hint, the code comparison
authenticates (§12.6). FFI test: `cargo test -p wind-ffi --test pairing`
(`sas_pairs_only_when_both_accept_the_same_code`).

## Shared clipboard (WN-35)

CCS/1
FLOW:the system ClipboardManager -> UniFFI -> clipboard/1 on the paired session -> the PC's clipboard; Applied comes back only after a successful OS write. Receiving PC -> phone also works with the Activity closed, if the connection service is running.
RULE:automatic mode on by default. The switch "Automatic sync" on Home/in Settings remembers the decision; it turns off automatic reading and receiving, but doesn't stop the connection. The manual "Send clipboard" stays as an extra. Merely reopening/focus doesn't send content already sent/received.
LIMIT:Android 10+ reads the clipboard only when the app has focus. Sending from the phone works automatically with WindConnect in focus and after returning to the app, if the content changed. In other apps use "Send clipboard" in the persistent notification: it briefly opens a separate screen, waits for focus/unlocking and after delivery returns to the previous app. No READ_LOGS, Accessibility or keyboard change. The time to open the screen and the person's actions are not the <150 ms transport budget.
SEC:only plain text/links up to 256 KiB without NUL; no opening of URLs or reading of providers' URIs. EXTRA_IS_SENSITIVE also blocks a manual send. Received text always has sensitive=true (the Android 13+ preview hidden and no echo). The status doesn't contain the content; no logging of the payload; the history only in memory, pinned items encrypted (WN-117).
DEC:we don't compare the devices' clock chronology. A Lamport counter+16 B id decides simultaneous changes, a hash in memory prevents an echo, a reconnect doesn't restore old content. Wire and limits in PROTOCOL.md §9.3.
RULE:the automatic mode sends only a clipboard copied after the current session was accepted (ClipDescription.timestamp against the local System.currentTimeMillis). Old text after a process restart or a reconnect won't replace the PC's clipboard on focus; a manual send may send it. A timestamp of 0/equal to the boundary is skipped. After the clock goes back use the manual action. No persisting of the payload.

Phone test: pair Windows and an S25 Ultra, turn on the service/notifications. Copy
ordinary Unicode text or a link on the PC and paste it on the phone in another app
while WindConnect is in the background. Copy new text on the phone, tap the action
of the notification and paste on the PC. With WindConnect open check the automatic mode.
Text marked by a password manager as sensitive should be skipped.
Check turning sync off/on again, losing the network and no old
queue after a reconnect. Confirm on the current One UI; an emulator is not an S25.

An automatic OS test: from the root `cargo build --locked -p windconnect-windows
--example clipboard_peer`; build Android debug/androidTest, set
`JAVA_HOME`, `ANDROID_HOME`, `CLIPBOARD_PROOF=1` and run through `cw-progress`
`node apps/windows/scripts/check-background-android.cjs`.
The test needs a disposable emulator (it clears only its WindConnect data).
Before starting the emulator turn off its "Enable clipboard sharing"
(Extended Controls → Settings). Host sharing is a second clipboard
channel, it may overwrite samples and lose sensitive. Afterwards restore
your setting. The test checks every copy with a control read on focus;
it doesn't retry or hide a bad write of a sample.
Windows uses a separate desktop in a non-interactive station and refuses the test
if its clipboard isn't empty. It doesn't use the user's interactive clipboard.
100 samples/direction measure from copying in the OS to Applied after the other
OS's write (a conservative upper bound of copy→paste time), and separately check Android's
receiving without an Activity, sensitive, the notification action and no echo after focus.
Raw samples and Light/Dark screenshots: `%TEMP%/windconnect-wn35-*`.

## Passwords in the clipboard (WN-37)

CCS/1
SEC:a clip with `ClipDescription.EXTRA_IS_SENSITIVE` (`android.content.extra.IS_SENSITIVE`) isn't sent to the PC, also manually (test: `ClipboardProof`, "WN35-PRIVATE-ANDROID"). The phone reads the clipboard only with focus or manually, never in the background. In the other direction, passwords from managers on the PC don't arrive (the OS test with 4 Windows markers, `apps/windows/README.md`, WN-37).
LIMIT:managers set IS_SENSITIVE on Android 13+; older or badly written ones don't set it, and a password copied from them looks like ordinary text. We don't recognize passwords by their content.

## Clipboard history (WN-117, WN-160)

CCS/1
FLOW:an item goes into the history when the PC confirmed a sent clipboard (Applied) or when a clipboard from the PC was written on the phone; a received image only after the whole PNG arrived. The card "Clipboard history" on Home and in Settings: newest at the top, a tap copies it back, pin/unpin and remove as icons on the row (WN-160), "Clear" (the unpinned ones).
RULE:rules as on the PC (WN-37, `apps/windows/README.md`): the last 20 + up to 20 pinned (at the top, outside the twenty); the same content (SHA-256) moves to the top. Unpinned images up to 64 MB in total (the oldest drops out), pinned ones also up to 64 MB. Sync turned off: nothing new (manual sending neither), turning it off clears the unpinned ones.
RULE:copying keeps the flags: text from the PC comes back with IS_SENSITIVE as when received (so it isn't sent back), an image through our own `ClipImageProvider` (no echo); text from the phone as an ordinary clip, which the automatic mode sends to the PC like any new copy.
SEC:a clip with IS_SENSITIVE never goes into the history (it isn't read or sent). The history only in memory; the UI gets a preview (one line, 120 characters, control/bidi/zero-width → space, leading blanks skipped and runs of blanks collapsed (WN-160), plain Text) or a thumbnail. Pinned items in `noBackupFilesDir/clipboard-pins.bin`: AES-256-GCM, the key in the Android Keystore, header `WCPN` 1. A damaged/newer file or no key: start without pinned items, the file stays; no pinned items deletes the file.
LIMIT:after a process restart only pinned items remain; no search.
CHECK:`ClipboardHistoryTest` (JVM: order, limits, the image budget, the pinned file, the preview); `ClipboardHistoryInstrumentedTest` (`:wind-core:connectedDebugAndroidTest`: the real clipboard and Keystore, IS_SENSITIVE outside the history, flags when copying, pinned after a restart, sync off); `ClipboardHistoryUiTest` (`:app`, Light/Dark, screenshots `cache/wn117-history-*.png`).

## Shared clipboard: images (WN-36)

CCS/1
FLOW:PC -> phone: ImageSet (thumbnail, size) -> setPrimaryClip with `content://<package>.clipimage/<id>` (image/png) -> Applied; the full PNG arrives in the background. `ClipImageProvider` (the wind-core module, not exported, grantUriPermissions) gives previews (EXTRA_SIZE, `loadThumbnail`) the thumbnail at once, and a paste a stream of the full PNG as it arrives.
FLOW:phone -> PC: on focus a clip with another app's image/* URI (the clip's permission) -> the PNG unchanged or re-encoded -> thumbnail -> clipboard/2. The provider's own URI never goes back to the PC.
RULE:the limit "Max image size" 5 / 16 / 50 MB in Settings (16 MB by default), for sending and receiving.
SEC:a received image has no EXTRA_IS_SENSITIVE (unlike text), so that the thumbnail preview works; it comes from one's own PC, which doesn't send private content. An image with IS_SENSITIVE isn't sent. Memory only, only the last image; after a process restart the old URI gives FileNotFoundException (the paste fails, without a crash).
LIMIT:a paste before the transfer ends waits up to 10 s without new bytes; a newer clipboard or a break end it with an error, never with a partial image.

Tests: `ClipImageProviderTest` (the thumbnail for the preview, the full PNG as it
arrives, an error with a newer image, an unknown image = FileNotFoundException).
The measurement "a screenshot from the PC pasteable on the phone < 500 ms" is part of the WN-35
OS test above: the PC copies a synthetic screenshot as CF_DIB alone (PNG encoding on
the PC's side) 10× 1080p and 3× 4K; the report gives the time to Applied (pasteable) and to
ImageDone (the whole PNG on the phone), and the phone reads the full PNG through the URI and
the preview. The time depends on the profile: run `clipboard_peer` built with
`CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true cargo build --release
--example clipboard_peer` and `PEER_PROFILE=release` (the debug PNG encoder is
~10× slower than the shipped app).

## The phone's battery on the PC (WN-49)

CCS/1
FLOW:a session accepted by `WindCore` -> `BatteryReporter` registers an `ACTION_BATTERY_CHANGED` receiver (the sticky intent gives the state at once) -> `CoreConnection.sendBattery` -> the PC (`battery/1`, PROTOCOL.md §9.4). The end of the session unregisters the receiver.
RULE:one normalization `BatteryReading.from(intent)`: the percentage as in the status bar (`level * 100 / scale`, rounded down), charging = plugged in, CHARGING or FULL; an unknown level = no reading (nothing is sent). The widget (WN-33) uses it too.
RULE:no polling and no waking: the receiver only during a session; equal readings (the intent also comes on voltage and temperature changes) are skipped, and the core sends only changes of the pair (level, charging).
LIMIT:the PC sees changes only with a live session (background connection on). In Doze or without a background connection the state on the PC isn't updated, so the 15% alert may come late or not at all. It doesn't touch ConnectionService or the home screen.

## Find my phone (WN-50)

CCS/1
FLOW:the computer asks (`find/1`, PROTOCOL.md §9.5) -> `PhoneRinger`: the alarm volume to max (the previous one stored), a generated tone (880/1320 Hz) through AudioTrack with `USAGE_ALARM`, vibration, a notification -> state 0 to the PC only once the playback head has moved.
RULE:the sound bypasses silent mode and vibration (the alarm stream); the tone is generated (no file, no preparation delay, independent of the user's ringtone). No WAKE_LOCK: with the screen off the audioserver keeps playback going.
RULE:stopping: a notification on its own channel "Find my phone" (IMPORTANCE_HIGH, without its own sound, persistent while ringing, it disappears after the final state, the action "Stop" and a tap), the bar "Stop ringing" on every app screen (works without notification permission) and a volume key with the app open. The core enforces the 60 s limit; the end of the session doesn't stop ringing.
RULE:the alarm volume comes back after every ring; when the process dies in the middle, the next start restores it (unless the user changed it). The setting "Let the computer ring" (on by default) withdraws `find/1` also in an ongoing session.
LIMIT:DND that blocks alarms silences the sound (bypassing it needs access to the DND policy, which the app doesn't ask for). Without a background connection or in Doze the request arrives late or not at all. A volume key with the screen off or outside the app doesn't stop ringing (that would need a media session). It doesn't touch ConnectionService or the home screen; the bar is an overlay in MainActivity.

## The phone's player on the PC (WN-46)

CCS/1
FLOW:an accepted session + access + the setting -> `PhoneMedia`: `getActiveSessions` and `addOnActiveSessionsChangedListener` through `MediaListenerService`, `MediaController.Callback` on the chosen session -> `CoreConnection.sendMedia` (changes only, the cover once) -> the PC. A command from the PC (`media-control/1`, checked in the core against the last `actions`) -> `transportControls`.
RULE:choosing the session: the first one in the PLAYING state, otherwise the first in the system's order; a change on `onActiveSessionsChanged` and on a state change. The cover ≤ 256 px JPEG ≤ 96 KiB, computed once per bitmap, on the media thread. Texts trimmed to 256 B (the app to 64 B) at a character boundary.
RULE:consent: Android calls it "notification access". `MediaListenerService` reads no notifications; the card in Settings says so plainly. The notification-reading feature (WN-42) MUST NOT silently use this consent: it gets its own switch (off by default) and its own explanation (PROTOCOL.md §9.6, rule 6).
RULE:`media-control/1` is announced only with the access granted and "Show media on the computer" on, also during a session (`Router::start_disabled` / `set_accepted`). Turned off: the phone sends `Gone` and stops following players, and the listener service unbinds by itself (`requestUnbind`; `requestRebind` after turning it on).
PERF:without a session nothing runs in `PhoneMedia` (no callbacks and no cover scaling). A bound listener service does keep the process alive also without a session: a new residency factor for the battery measurement (WN-30).
LIMIT:Spotify isn't installed on the emulator; the test uses an ordinary `MediaSession` (Spotify exposes the same kind). Notification access is granted only in the system settings.

## The computer's player on the phone (WN-47)

CCS/1
FLOW:the PC (the phone announces `media/1`) -> `PcPlayer`: a StateFlow for the "On the computer" card on the home screen and our own `MediaSession` "WindConnect PC" with a MediaStyle notification on the channel "Computer media" (IMPORTANCE_LOW, without sound) -> the system media controls (notification, QS, lock screen) -> `sendMediaCommand` -> the PC.
RULE:the session plays no sound and has no media button receiver (`setMediaButtonReceiver(null)`), so headphones and Bluetooth control the phone's players. LIMIT:if no player of the phone was playing, Android may route a headphone button to the active WindConnect session; then it pauses the computer's player.
RULE:echo: `PhoneMedia` (WN-46) skips its own mirror session of the computer's player (by its token), so the computer's player never goes back to the PC as "On the phone".
RULE:cleanup: `Gone`, the end of the session, turning off "Show the computer's player" and closing the core release the session and remove the notification; after the process dies the session goes with it, and the next start removes a leftover notification. Texts from the computer: one line, without control and bidirectional characters; the cover only JPEG/PNG by its magic bytes, decoded to ≤ 512 px.
RULE:privacy: the computer player's titles are visible on the lock screen, as with other players; the switch's description says so.

## Notifications on the computer (WN-42)

`NotifyListenerService` reads new notifications and through `WindCore` sends them
on the `notify` channel (PROTOCOL.md §9.7) to a computer that announced `notify/1`
(Windows from WN-43). A separate consent (§9.6, rule 6): its own entry
"WindConnect: notifications on the computer" in the system's notification
access and a switch in Settings, off by default; turned off,
the service unbinds by itself and reads nothing. The access for media (WN-46) isn't
enough.

Skipped: persistent ones (ongoing, foreground services, non-dismissible),
group summaries, players, `VISIBILITY_SECRET`, our own and empty ones. Title:
the conversation title, the big title or the title; content: the conversation's last message
("sender: text" when the sender isn't the title), the big text or the text. Without
formatting, control and bidirectional characters, trimmed to the protocol's
bytes. The app's icon: a 64 px PNG, once per session. Identifier: the SHA-256 of
the installation's random salt and the notification key (16 B). The core enforces the pace:
at most one `Posted` per second per notification, a progress bar only at the
start and at the end. Notifications from before the connection aren't sent.

Android 15+ hides sensitive content from listening apps (one-time
codes and the like): the computer then sees what the system gave, e.g. "Content
hidden" instead of the text, or nothing. WindConnect doesn't get around that. On an emulator
with the notification assistant even ordinary texts with numbers are sometimes hidden.

Latency: with a notification assistant (Google, Samsung) Android holds a new
notification for about 200 ms before any listener gets it, also
the system's notification shade; from the listener call to the write into the QUIC
stream it takes p95 about 10 ms. A measurement on an emulator (first
`gradlew :app:assembleDebug :app:assembleDebugAndroidTest` and
`cargo build --locked -p wind-ffi --example notify_peer`, then from the repository
root):

```powershell
$env:ANDROID_SERIAL = "emulator-5590"; node apps/android/scripts/check-notify-android.cjs
```

`notify_peer` pretends to be a computer with `notify/1`. The first run measures
the listener call → write, the second (the assistant turned off for the test)
`postTime` → write; both p95 < 50 ms.

### Replying and dismissing from the computer (WN-44)

With the feature on and the access granted, the phone also announces
`notify-action/1` (PROTOCOL.md §9.8; live on a change). `Posted` gets
`reply` (a label) when the notification has an action with a `RemoteInput` accepting
any text: first `SEMANTIC_ACTION_REPLY`, then the first such one;
contextual ones (the system's smart replies) are skipped (`NotificationRules.replyAction`).
`Reply` from the computer goes through `RemoteInput.addResultsToIntent` +
`SOURCE_FREE_FORM_INPUT` and the action's `PendingIntent`, as if typed on the phone,
then `Replied`. `Dismiss` is the listener's `cancelNotification`; then the usual
`Removed`. Only for ids sent in this session (`WindCore.sentIds`; the listener
remembers the key and the reply action of the 256 newest, without content). The reply
text is not logged or kept.

### App filter and "Do not disturb" (WN-45)

In the card "Notifications on the computer": the mode "All" (all except the ones turned off,
the default) or "Chosen" (new apps off) and a switch per
app, with the app's real name and icon (WN-159, WN-160). The list is the apps seen by the listener (package names, up to
300, without content) and those of current notifications, without `QUERY_ALL_PACKAGES` and
without manifest changes. A turned-off app is rejected in the listener before
anything reaches the core (`AppFilter`), and once more at the point of sending
(`NotifyForwarder`: a notification from the queue from before the block is skipped, with
its icon). Blocking an app sends `Removed` for its notifications of this
session and forgets them for replying and dismissing (WN-44). A notification that
doesn't get through the phone's "Do not disturb"
(`Ranking.matchesInterruptionFilter()`) goes with `quiet`: on the PC without
popping up and sound. Tests: `NotifyAppsTest` (unit and Compose) and
`apps/windows/scripts/check-notify-filter-android.cjs`.

## Calls and volume for the computer (WN-48)

`CallWatcher` gives the core the call state (PROTOCOL.md §9.10) without
new permissions and without manifest changes: the audio mode
(`addOnModeChangedListener`, API 31+), the telephony state without the number
(`LISTEN_CALL_STATE`, API 29-30) and a notification of the call category from
the notification listener (vibrate mode, only with the consent "Notifications on the
computer"). Ringing and a call go at once, idle after 1.5 s.
`PhoneVolume` reports `STREAM_MUSIC` (a `Settings.System` observer, without
polling) and applies a `Set` from the computer only in what it gives (a level alone
while muted doesn't unmute, `VolumeScale.plan`). The setting
"Volume from the computer" (on by default) announces `volume/1`; the card
"Computer volume" on the home screen (a slider without "bouncing", mute).
Tests: `CallVolumeTest` (unit), `PcVolumeTest` (Compose) and
`apps/windows/scripts/check-call-volume-android.cjs`.

## SMS on the computer (WN-55)

`PhoneSms` shares the SMS/MMS threads with the computer for reading (PROTOCOL.md
§9.11): the thread list (`mms-sms/conversations?simple=true`, addresses from
`canonical-addresses`, unread from `sms`/`mms`), a thread's messages
(SMS and MMS text, newest first, `before`/`since` inclusive, without drafts),
contacts' names and photos (`PhoneLookup`, a 96 px PNG up to 64 KiB). Provider changes
(a `ContentObserver` on `content://mms-sms/`) wake the computer's
watch; the core gathers them over 250 ms.
The setting "SMS on the computer" (off by default) asks for `READ_SMS` and
`READ_CONTACTS` only when it is turned on, after an explanation; contacts are
optional (without them, numbers only). A refusal leaves the switch off,
the rest of the app works. The permission is checked on every request,
so revoking it in Android's settings takes effect at once. Content and numbers
don't go into logs.
Decision D2: only this version (GitHub/F-Droid) declares the SMS permissions;
the Google Play variant must leave them out.
Test: `apps/windows/scripts/check-sms-android.cjs` (with `SmsProof`).

### Sending from the computer (WN-56)

A separate setting "Sending from the computer" (visible with reading on,
off by default) asks for `SEND_SMS` and optionally `READ_PHONE_STATE`
(only the SIM card list) only when it is turned on. `SmsSender` sends text to
one number as an SMS (`divideMessage` + `sendMultipartTextMessage` with
sent and delivery reports, with the card chosen on the PC), and an image or several
recipients as an MMS: `MmsPdu` (a minimal M-Send.req with SMIL) -> a file in
`cache/mms-out` -> `FileProvider` (`${applicationId}.mmsout`) ->
`sendMultimediaMessage`. As a non-default SMS app WindConnect doesn't write
to the SMS provider: Android stores sent messages itself, so the computer sees
them through the watch. At most 10 sends per minute (the core). Images from
received MMS go to the computer on request (`Part`), as JPEG up to 1600 px.
Tests: `MmsPduTest`, `SmsRulesTest`, `apps/windows/scripts/check-sms-send-android.cjs`.

## Photos on the computer (WN-58)

`PhoneGallery` shares the newest photos and screenshots with the computer's
panel (PROTOCOL.md §9.13, `gallery/1`): a list from MediaStore (all
external volumes, by `DATE_ADDED`; MediaStore itself hides pending and trashed
ones), thumbnails from `loadThumbnail` (JPEG up to 64 KiB) and the original as a file
descriptor (`openFileDescriptor` + `detachFd`, the core reads and closes it). A screenshot
is a photo from the `Screenshots` folder (`GalleryRules`). Changes
(a `ContentObserver` on `content://media`, only `images` paths) wake
the computer's watch; the core gathers them over 250 ms.
The setting "Photos on the computer" (off by default) asks for access to
photos only when it is turned on (Android 13+: `READ_MEDIA_IMAGES`, 10–12:
`READ_EXTERNAL_STORAGE`). A refusal leaves the switch off; access is
checked on every request. Names don't go into logs.
LIMIT:Android 14's partial access ("selected photos") isn't handled
separately: without full access the feature is off.
Tests: `GalleryRulesTest`, `apps/windows/scripts/check-gallery-android.cjs`
(with `GalleryProof`).

## Calls on the computer (WN-57)

The setting "Calls on the computer" (off by default) asks, when it is
turned on, for `READ_PHONE_STATE`, `READ_CALL_LOG` (the caller's number and
missed calls), `ANSWER_PHONE_CALLS` (rejecting from the computer) and optionally
`READ_CONTACTS` (the name). `PhoneCalls` listens to `PHONE_STATE` (the number comes
only with `READ_CALL_LOG`), gives each ring a new `call` and adds
the caller to the state from `CallWatcher` (PROTOCOL.md §9.10); it reads missed calls from
the call log (7 days, at most 20). From the computer (§9.12): "Reject" is
`TelecomManager.endCall`, "Silence" mutes the ringer stream (an ordinary app
can't use `silenceRinger`) and restores it after the call; both work
only on the ringing call they name. Without the setting the computer gets
the state alone, as in WN-48. Numbers don't go into logs.
CCS/1 SEC:Silence needs the ordinary `MODIFY_AUDIO_SETTINGS`; without the permission
or with a fixed device volume this action is removed. An Android refusal (e.g. the
Do not disturb policy) doesn't call a privileged API and doesn't store its own
muting. A refusal to restore keeps the record for a later recovery attempt.
DEP:[TelecomManager.silenceRinger](https://developer.android.com/reference/android/telecom/TelecomManager#silenceRinger()),
[AudioManager.adjustStreamVolume](https://developer.android.com/reference/android/media/AudioManager#adjustStreamVolume(int,int,int)).
Test: `apps/windows/scripts/check-calls-android.cjs`.

## Remote actions (WN-100)

Phone -> PC: "Lock PC" and "Open on PC" on the home screen, and the computer's
other actions from `list_actions` ("Sleep PC" when the PC allows it, custom "x-…"
ones, and any a newer PC adds) in the card "Actions on the computer";
each status has a message (done, refused, bad address, busy, failed,
no connection). "Open on PC" takes the link from the clipboard, and without
one asks for an address; shared text with a link goes there through "Open on
PC" in the share sheet (`OpenOnPcActivity`, an existing
connection only). PC -> phone: the core hosts `actions/1` with `ActionHost`
(only `open-url`, checked by `check_url`), `LinkOpener` opens the link
at once when the app is on screen, and in the background shows the notification
"Open the link from the computer". The setting "Open links from the computer"
(on by default); turned off -> the computer gets "refused".
RULE (WN-165): built-ins are named by their id in the phone's language
(`PcActionText.label`); the PC's label is shown only for a custom action and for an
id this version doesn't know. Likewise a computer without a name is "Computer" in the
phone's language: the core returns an empty name and the app fills it in.
Test: `apps/windows/scripts/check-actions-android.cjs`, `PcActionTextTest`,
`PcActionNamesTest`.

## Pairing after an update (WN-126)

A report from the WN-120 test on the S25 (2026-11-13): "a connectivity problem, after
pairing again the test passed". What was found:

- The identities didn't change. `paired.txt` on the PC (an append-only journal)
  has one phone key from 11-10 to 11-13: removal at 12:52:37,
  added again at 12:52:56 with the same key; the PC's `identity.key` from 11-10.
  All test APKs 0.30–0.61 have one certificate (the user's
  debug.keystore) and growing `versionCode`s, so the update needed no
  uninstalling (that would have erased `noBackupFilesDir`). The key stores never
  create a new key in place of an unreadable one. Pairing again
  therefore worked as a manual reconnect, not a repair of keys.
- A reproduced case (master, since WN-51): after a session ended (e.g. a restart
  or an update of the PC) the touchpad capability loop got `channel closed`
  without handling and the app crashed. Android restarts a service after a crash with a growing
  delay (30 min after the second crash), so the phone stayed
  disconnected until the app was opened. The fix is in `WindCore.adopt`; before it
  `check-update-android.cjs` failed at the 2nd of 12 PC restarts, after it
  12/12 and 0 crashes.
- The cause in version 0.54.0 can no longer be pinned down (no logs; this loop
  came later). When the problem comes back on the phone, before pairing
  again note: the APK and EXE versions and the order of installation, the text of
  the WindConnect notification and the state on the PC, and from the PC with adb:
  `adb logcat -b crash -d` and `adb shell dumpsys activity services io.github.prejas3.windconnect`
  (look for "Restarting" and "crashed"). From `paired.txt` the first
  8 characters of the keys are enough; don't copy private key files.

CHECK: `apps/windows/scripts/check-update-android.cjs` (a disposable emulator,
`ANDROID_SERIAL`, `JAVA_HOME`, `ANDROID_HOME`; debug APK + androidTest and
`cargo build -p windconnect-windows --example background_peer`). It pairs once
with a real QR code, then: (A) `adb install -r` with the PC running, (B) a PC restart
with the identity and the list kept (`PC_RESTARTS=n` repeats), (C) an APK
update with the PC off (`PC_DOWN_MS`). Each step requires a new session without
pairing, an unchanged paired list and no app crash.

## The GitHub/F-Droid version and the Google Play version (WN-124, decision D2)

One code base, two builds chosen by the Gradle property `windStore`:

- `github` (the default, GitHub Releases and F-Droid): all features, including
  "SMS on the computer" (READ_SMS, SEND_SMS) and "Calls on the computer"
  (READ_CALL_LOG, ANSWER_PHONE_CALLS) and contacts (READ_CONTACTS).
  APK: `app/build/outputs/apk/<type>/app-<type>.apk`, as before.
- `play` (`./gradlew :app:assembleRelease -PwindStore=play`): Google Play
  restricts the SMS and call log permission groups, so
  `app/src/play/AndroidManifest.xml` removes (`tools:node="remove"`, also from
  libraries) READ_SMS, SEND_SMS, RECEIVE_SMS/MMS/WAP_PUSH, READ_CALL_LOG,
  WRITE_CALL_LOG, READ_CONTACTS, ANSWER_PHONE_CALLS and READ_PHONE_STATE (only
  SMS sending and calls ask for them; the call state of other features comes from the
  audio mode without permissions, WN-146). Built in
  `app/build-play`, the APK `app-play-<type>.apk`, so it doesn't overwrite the full one.

RULE:features follow from what the APK declares (`declaresPermission`, wind-core),
not from a flag: without READ_SMS WindCore doesn't set the SMS source, so `sms/1` is
neither routed nor announced, and Settings have no "SMS on the computer"; without
READ_CALL_LOG there is no "Calls on the computer". The "Messages" window on the PC then says
that the phone doesn't share SMS (with a hint about the Google Play version).
CHECK:`node scripts/check-android-store-permissions.cjs <apk> github|play`
(aapt; CI checks both builds), `StoreVariantTest` (`-e windStore github|play`)
on each build, `apps/windows/scripts/check-sms-play-android.cjs`
(a real PC + an emulator with the play APK: no `sms/1`, `sms_status`
`{connected:true, available:false}`, the window's text).

## A quick return to the computer after it restarts (WN-142)

RULE:the background service retries the connection with an interval of 3 s → 60 s, but the wait ends
at once when the paired computer comes back in discovery: it appears after disappearing or
at an address other than the one of the failed attempt (e.g. the PC app after an update on
a new port). The same addresses don't wake the loop, so BUSY and refusals aren't retried
over and over (WC-28); discovery only wakes the loop, the connection still requires the pinned
key (`PcReturn.kt`, `PcReturnTest`).
CHECK:`check-update-android.cjs` with `PC_DOWN_MS=150000`: the phone comes back 0.5 s after
the PC returns (earlier 14–22 s, at worst ~60 s); 8 PC restarts: 0.5–2.7 s.
RULE (WN-149): the reconnect loop runs on the main thread; while connected (or connecting) it waits for the end of the session or the interval, never for discovery, and each turn takes at least 500 ms (`reconnectWait`, `ReconnectWaitTest`). `check-update-android.cjs` also checks that the main thread is free after every return.
