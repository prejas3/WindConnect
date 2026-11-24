plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
    id("maven-publish")
}

group = "io.github.prejas3.windconnect"
version = "0.88.1"

val repositoryRoot = rootProject.projectDir.resolve("../..").canonicalFile
// WN-95: Rust builds share the repository's Cargo target folder, so the
// host library for bindings reuses `cargo build` output instead of a second
// copy under build/rust; scripts/clean-build.ps1 cleans it.
val rustTarget = repositoryRoot.resolve("target")
@Suppress("UNCHECKED_CAST")
val windAbis = rootProject.extra["windAbis"] as List<String>
val generatedKotlin = layout.buildDirectory.dir("generated/uniffi")
val generatedJni = layout.buildDirectory.dir("generated/jniLibs")
val isWindows = System.getProperty("os.name").startsWith("Windows")
val isMac = System.getProperty("os.name").startsWith("Mac")
val hostTag = if (isWindows) "windows-x86_64" else if (isMac) "darwin-x86_64" else "linux-x86_64"
val hostLibrary = if (isWindows) "wind_ffi.dll" else if (isMac) "libwind_ffi.dylib" else "libwind_ffi.so"

android {
    namespace = "io.github.prejas3.windconnect.core"
    compileSdk = 35
    ndkVersion = "27.0.12077973"
    defaultConfig {
        minSdk = 29
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk { abiFilters += windAbis }
        consumerProguardFiles("consumer-rules.pro")
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    sourceSets["main"].java.srcDir(generatedKotlin)
    sourceSets["main"].jniLibs.srcDir(generatedJni)
    publishing { singleVariant("release") { withSourcesJar() } }
}

fun Exec.rustInputs() {
    workingDir(repositoryRoot)
    environment("CARGO_TARGET_DIR", rustTarget.absolutePath)
    inputs.files(fileTree(repositoryRoot.resolve("crates")) {
        include("**/src/**/*.rs", "**/Cargo.toml", "**/uniffi.toml")
        exclude("**/target/**")
    })
    inputs.file(repositoryRoot.resolve("Cargo.toml"))
    inputs.file(repositoryRoot.resolve("Cargo.lock"))
}

val buildHost by tasks.registering(Exec::class) {
    rustInputs()
    commandLine("cargo", "build", "--locked", "-p", "wind-ffi", "--lib")
    outputs.file(rustTarget.resolve("debug/$hostLibrary"))
}

val generateBindings by tasks.registering(Exec::class) {
    dependsOn(buildHost)
    rustInputs()
    commandLine("cargo", "run", "--locked", "-p", "wind-ffi", "--features", "bindgen",
        "--bin", "wind-bindgen", "--", "generate", "--library",
        rustTarget.resolve("debug/$hostLibrary").absolutePath,
        "--language", "kotlin", "--no-format",
        "--out-dir", generatedKotlin.get().asFile.absolutePath)
    outputs.dir(generatedKotlin)
}

val nativeTasks = listOf("arm64-v8a" to "aarch64-linux-android", "x86_64" to "x86_64-linux-android")
    .filter { (abi, _) -> abi in windAbis }.map { (abi, triple) ->
    val buildNative = tasks.register<Exec>("buildRust${abi.replace("-", "").replace("_", "")}") {
        rustInputs()
        val toolchain = android.sdkDirectory.resolve("ndk/${android.ndkVersion}/toolchains/llvm/prebuilt/$hostTag/bin")
        val extension = if (isWindows) ".exe" else ""
        environment("CC_${triple.replace('-', '_')}", toolchain.resolve("clang$extension").absolutePath)
        environment("CFLAGS_${triple.replace('-', '_')}", "--target=${triple}29")
        environment("AR_${triple.replace('-', '_')}", toolchain.resolve("llvm-ar$extension").absolutePath)
        val prefix = "CARGO_TARGET_${triple.uppercase().replace('-', '_')}"
        inputs.property("ndkVersion", android.ndkVersion ?: "")
        inputs.property("androidTarget", "${triple}29")
        inputs.property("elfPageSize", 16384)
        environment("${prefix}_LINKER", toolchain.resolve("clang$extension").absolutePath)
        // WN-162: the published .so must not carry the builder's home folder (the cargo registry's source paths):
        // remap it to "~". Encoded flags (\u001f between them): a home path may contain spaces. With --target these
        // apply to the target only, as the target-specific variable did.
        val home = System.getProperty("user.home")
        environment("CARGO_ENCODED_RUSTFLAGS", listOf("-C", "link-arg=--target=${triple}29", "-C", "link-arg=-Wl,-z,max-page-size=16384",
            "-C", "link-arg=-Wl,-z,common-page-size=16384", "--remap-path-prefix=$home=~").joinToString("\u001f"))
        commandLine("cargo", "build", "--locked", "--release", "-p", "wind-ffi", "--lib", "--target", triple)
        outputs.file(rustTarget.resolve("$triple/release/libwind_ffi.so"))
    }
    tasks.register<Copy>("stageRust${abi.replace("-", "").replace("_", "")}") {
        dependsOn(buildNative)
        from(rustTarget.resolve("$triple/release/libwind_ffi.so"))
        into(generatedJni.map { it.dir(abi) })
    }
}

tasks.named("preBuild") { dependsOn(generateBindings, nativeTasks) }
tasks.matching { it.name == "sourceReleaseJar" || it.name == "releaseSourcesJar" }
    .configureEach { dependsOn(generateBindings) }

dependencies {
    implementation("androidx.annotation:annotation:1.9.1")
    // WN-56: FileProvider and receiver flags for MMS sending; the version the app already ships (Apache-2.0).
    implementation("androidx.core:core:1.13.1")
    api("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    testImplementation("junit:junit:4.13.2")
    // WN-117: SystemClipboard is internal, so its instrumentation test lives here.
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("junit:junit:4.13.2")
}

afterEvaluate {
    publishing {
        repositories {
            maven {
                name = "build"
                url = layout.buildDirectory.dir("repo").get().asFile.toURI()
            }
        }
        publications {
            create<MavenPublication>("release") {
                from(components["release"])
                artifactId = "wind-core"
            }
        }
    }
}
