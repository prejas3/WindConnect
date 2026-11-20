# Orbita

CCS/1
DEC:the current mark is variant C from WN-131, chosen by Szymon on 2026-11-13: the open orbit with arrows alone, without a monitor and phone inside. It replaces the devices in the original direction C from WN-93 (2026-11-11). The arcs, arrows, the WN-20 palette and the WN-115 margins are kept; the mark was not enlarged. Own paths, the project's MIT license; no borrowed icon library.
SOURCE:orbita.svg (64 units) for the apps; orbita-small.svg (16 units) with simpler geometry and fitted lines for the tray, small ICOs and the notification. We never enlarge a 16 px bitmap. Every PNG/RGBA is rendered separately from the SVG at its target size.
CHECK:WN-131: the centre of the connected mark is empty. check-icons checks the middle 25% of each dimension in the PNG/ICO/mipmaps and the connected tray of both palettes (alpha<=16 for transparent ones, no white ink for coloured ones). A native Android test checks foreground/monochrome and ic_connection. The strike-through of the unavailable state is an intended exception; colour is not the only sign of the state. The archived WN-115 raster is only in androidTest/assets for before/after comparisons, not in the app.
IO:scripts/generate-icons.cjs -> ICO 16/20/24/32/40/48/64/96/128/256, Tauri PNGs, the tray 3 states × 2 palettes × 4 sizes, Android VectorDrawable/adaptive/monochrome and mipmaps mdpi–xxxhdpi. generated.json records the hash of every output and the Chromium version.
RULE:the tray uses the SM_CXSMICON size for the DPI of Shell_TrayWnd, not the DPI of a window on another monitor; it picks the 16/20/24/32 variant without decoding PNG at runtime. SystemUsesLightTheme decides the contrast on the taskbar. It updates on a state change and on native WM_SETTINGCHANGE/WM_THEMECHANGED/WM_DISPLAYCHANGE/WM_DPICHANGED on the existing window (no polling, no new worker); the hook is released on WM_NCDESTROY. Tauri's ThemeChanged/ScaleFactorChanged are a fallback too. Connected: arrows/teal; searching: dashed orbit/amber; none: strike-through/graphite. Colour is not the only information about the state.
RULE:Android layers 108 dp, the mark 48 dp centred in the 66 dp safe zone (WN-115); foreground and monochrome are separate white vectors with a transparent background. The launcher mask shows about 72 dp of the layer: the previous 64 dp mark filled it too much while still meeting the safe zone. The current silhouette takes about 60–64% of the visible mask. The adaptive background is solid teal. The notification/tile 24 dp has a nominal 20 dp mark and a 2 dp margin. The fallback mipmaps have a matching margin (scale 2/3 inside 64); launchers on API 26+ use adaptive.
RULE:WN-115: the Windows app PNG/ICO also had a tight mark on a coloured background; a .75 scale inside the unchanged background gives a margin. The tray and title bar stay unchanged: their separate, transparent 16–32 px glyph has no mask problem and needs to be legible at 16 px. The source Orbita paths/colours are unchanged; rasters are still generated directly at the target resolution.
LIMIT:100–200% DPI have the 16/20/24/32 variants; above 200% the 32 px one is used. S25 Ultra/One UI needs a check of the physical launcher and themed icons; an emulator doesn't prove One UI's behaviour.

Generating (Playwright is an external tool, not an app dependency):

```powershell
$env:PLAYWRIGHT_MODULE = 'full/path/to/playwright'
node scripts/generate-icons.cjs
node scripts/generate-icons.cjs --check
node scripts/check-icons.cjs
powershell -File scripts/check-icons-native.ps1
cargo run --locked -p windconnect-windows --example icon_probe -- "$env:TEMP/wn93-native-tray.json"
$env:ANDROID_SERIAL = 'emulator-5558'
node scripts/check-android-icons.cjs
```

## WN-131 inventory

| Place | Source of the active mark |
| --- | --- |
| Android launcher adaptive, round, themed icons | `mipmap-anydpi-v26` -> foreground/monochrome from the generator; solid `icon_background` kept |
| Fallback launcher mdpi–xxxhdpi | 10 generated mipmap PNGs |
| Notifications for the connection, sending/receiving files, media and the ringer | the shared generated `ic_connection`; the system fallback stays only in case the resource is missing |
| QS tile | the `ClipboardTileService` manifest and `ClipboardTileControls` -> `ic_connection` |
| WN-72 shortcuts | static `shortcuts.xml` and dynamic `LauncherShortcuts` -> `mipmap/ic_launcher`; pinned references use the same resource |
| WN-33 widget | `WindWidget` shows text/status/buttons, has no mark of its own and no old monitor/phone; unchanged |
| Tauri bundle, EXE/MSI icon | `icons/icon.ico`, all 10 sizes generated from the chosen mark |
| Windows frontend | `.brand-mark` masks the generated `frontend/assets/orbita-small.svg`; no separate favicon in the HTML |
| Windows title bar and tray | `brand_icon` loads the generated RGBA 3 states × 2 palettes × 4 sizes; the battery badge is drawn at runtime without an old mark of its own |

CCS/1
LIMIT:the 76 generated files are the manifest's 75 outputs plus the manifest itself. The solid background and the adaptive references may stay byte-identical; they have no old centre. Generic phone/PC icons in UI controls stand for devices, are not the app's mark and stay. WN-131 removes the ink from the centre, so the sum of white pixels goes down; the scale and the outer outline are kept as in the accepted comparison. Whether the mark feels too light belongs to the S25 test and doesn't justify enlarging it unasked.

WN-131 incremental audit:

| Component | Version/license and source | Shipped in | Assessment |
| --- | --- | --- | --- |
| Orbita without the centre and all derivatives | WN-131, own paths, the project's MIT `LICENSE` | APK, EXE/ICO, frontend, tray | no third-party material; existing notices kept |
| Archived WN-115 raster for comparison | own sources under the project's MIT, rendered by Chromium | only the androidTest test APK | not an active app resource |

CCS/1
DEP:no new libraries/fonts/runtimes/installers; the existing APK/Windows notices need no regeneration. Playwright/Chromium remain a developer tool and don't go into the app. The MSI resources come from the same ICO; the installer itself is unchanged.

CCS/1
DEP:no new runtime package, font or third-party asset. The Windows SVG/CSS and Android XML use our own mark. The existing DM Sans/ZXing/JNA/Windows/Tauri licenses stay unchanged. The generator doesn't ship Playwright/Chromium in the APK/MSI.
CHECK:check-android-icons needs only an assigned, disposable emulator; it verifies ro.kernel.qemu. The notification test temporarily writes the offline RFC 8032 test public key into the pairing store (restoring the previous file), so that the production service doesn't end for lack of devices. It doesn't set up an authenticated session. The notification icon is checked through NotificationManager and after the title shows in SystemUI. Don't run this fixture on the user's phone.
EXT:https://learn.microsoft.com/en-us/windows/win32/menurc/about-icons ; https://developer.android.com/develop/ui/compose/system/icon_design_adaptive

WN-115 incremental audit:

| Component | Version | License and source | Shipped in | Assessment |
| --- | --- | --- | --- | --- |
| Orbita full/small, padding and regenerated resources | WN-115 | the project's MIT, `LICENSE`; own paths `orbita.svg`, `orbita-small.svg` | APK and the Windows app icons/ICO | No new third-party material; existing notices kept. |

CCS/1
DEP:the audit covers the changed icons; no new packages/runtimes/fonts. Playwright/Chromium remain only a build tool, not shipped to the app's users. No new license notices to generate.
