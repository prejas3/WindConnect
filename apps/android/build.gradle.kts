plugins {
    id("com.android.application") version "8.9.2" apply false
    id("com.android.library") version "8.9.2" apply false
    id("org.jetbrains.kotlin.android") version "2.1.20" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.1.20" apply false
}

// WN-95: which ABIs this build compiles and packages. A build that asks
// only for debug tasks (assembleDebug, connectedDebugAndroidTest,
// testDebugUnitTest, lintDebug, ...) outside CI takes one ABI: -PwindAbi=
// arm64-v8a for a phone (S25 Ultra), x86_64 (the default) for the emulator,
// or a comma list / "all". Release, `build`, and everything in CI (CI set)
// take every ABI.
val allAbis = listOf("arm64-v8a", "x86_64")
val askedTasks = gradle.startParameter.taskNames.filterNot { it.contains("clean", ignoreCase = true) }
val debugOnly = System.getenv("CI").isNullOrEmpty() &&
    askedTasks.isNotEmpty() && askedTasks.all { it.contains("Debug") }
val requestedAbi = providers.gradleProperty("windAbi").getOrElse("x86_64")
val windAbis: List<String> = if (!debugOnly || requestedAbi == "all") allAbis else
    requestedAbi.split(",").map { it.trim() }.filter { it.isNotEmpty() }.also { abis ->
        require(abis.isNotEmpty() && abis.all { it in allAbis }) {
            "windAbi must be one of $allAbis, a comma list of them, or all"
        }
    }
extra["windAbis"] = windAbis

// WN-124 (decision D2): which store a build is for. "github" (the default; GitHub Releases
// and F-Droid) has every feature; -PwindStore=play leaves out what Google Play restricts:
// the SMS and Call Log permission groups and contacts (app/src/play/AndroidManifest.xml),
// with "SMS na komputerze" and "Połączenia na komputerze" hidden. Its APK is app-play-*.apk.
val windStore = providers.gradleProperty("windStore").getOrElse("github")
require(windStore in listOf("github", "play")) { "windStore must be github or play" }
extra["windStore"] = windStore
