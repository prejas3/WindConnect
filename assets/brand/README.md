# Orbita

CCS/1
DEC:Szymon wybrał kierunek C na WN-93, 2026-11-11. Dwa urządzenia w otwartej orbicie ze strzałkami synchronizacji; paleta WN-20. Własne ścieżki, licencja MIT projektu; bez zapożyczonej biblioteki ikon.
SOURCE:orbita.svg (64 jednostki) dla aplikacji; orbita-small.svg (16 jednostek) z prostszą geometrią i dopasowanymi liniami dla zasobnika, małych ICO i powiadomienia. Nie powiększamy bitmapy16px. Każda PNG/RGBA jest renderowana osobno z SVG w rozmiarze docelowym.
IO:scripts/generate-icons.cjs -> ICO16/20/24/32/40/48/64/96/128/256, PNG Tauri, zasobnik3stany×2palety×4rozmiary, Android VectorDrawable/adaptive/monochrome i mipmapy mdpi–xxxhdpi. generated.json zapisuje hash każdego wyniku i wersję Chromium.
RULE:zasobnik używa rozmiaru SM_CXSMICON dla DPI Shell_TrayWnd, nie DPI okna na innym monitorze; dobiera wariant16/20/24/32 bez dekodowania PNG w runtime. SystemUsesLightTheme określa kontrast na pasku. Aktualizacja przy zmianie stanu oraz natywnych WM_SETTINGCHANGE/WM_THEMECHANGED/WM_DISPLAYCHANGE/WM_DPICHANGED na istniejącym oknie (bez pollingu/nowego workera); hook zwalnia się na WM_NCDESTROY. ThemeChanged/ScaleFactorChanged Tauri są też fallbackiem. Połączono: strzałki/teal; szukanie: przerywana orbita/amber; brak: przekreślenie/grafit. Kolor nie jest jedyną informacją o stanie.
RULE:Android warstwy108dp, znak64dp wyśrodkowany w66dp bezpiecznej strefie; foreground i monochrome są osobnymi białymi wektorami z przezroczystym tłem. Tło adaptive jednolite teal. Powiadomienie używa uproszczonego białego wektora24dp. Mipmapy to fallback, launcher API26+ używa adaptive.
LIMIT:100–200%DPI mają warianty16/20/24/32; powyżej200% używany jest32px. S25 Ultra/One UI wymaga sprawdzenia fizycznego launchera i themed icons; emulator nie dowodzi zachowania One UI.

Generowanie (Playwright jest narzędziem zewnętrznym, nie zależnością aplikacji):

```powershell
$env:PLAYWRIGHT_MODULE = 'pełna/ścieżka/do/playwright'
node scripts/generate-icons.cjs
node scripts/generate-icons.cjs --check
node scripts/check-icons.cjs
powershell -File scripts/check-icons-native.ps1
cargo run --locked -p windconnect-windows --example icon_probe -- "$env:TEMP/wn93-native-tray.json"
$env:ANDROID_SERIAL = 'emulator-5558'
node scripts/check-android-icons.cjs
```

CCS/1
DEP:brak nowej paczki runtime, fontu ani assetu osoby trzeciej. Windows SVG/CSS i Android XML wykorzystują własny znak. Istniejące licencje DM Sans/ZXing/JNA/Windows/Tauri pozostają bez zmian. Generator nie dostarcza Playwright/Chromium w APK/MSI.
CHECK:check-android-icons wymaga wyłącznie przypisanego, jednorazowego emulatora; weryfikuje ro.kernel.qemu. Test powiadomienia tymczasowo zapisuje publiczny klucz testowy RFC8032 offline do magazynu parowań (przywracając wcześniejszy plik), aby produkcyjna usługa nie zakończyła się z powodu braku urządzeń. Nie nawiązuje uwierzytelnionej sesji. Ikona powiadomienia jest sprawdzana przez NotificationManager i po pokazaniu tytułu w SystemUI. Nie uruchamiać tego fixture na telefonie użytkownika.
EXT:https://learn.microsoft.com/en-us/windows/win32/menurc/about-icons ; https://developer.android.com/develop/ui/compose/system/icon_design_adaptive
