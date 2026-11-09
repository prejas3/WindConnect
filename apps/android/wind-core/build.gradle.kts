plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
    id("maven-publish")
}

group = "io.github.prejas3.windconnect"
version = "0.21.0"

val repositoryRoot = rootProject.projectDir.resolve("../..").canonicalFile
val rustTarget = layout.buildDirectory.dir("rust")
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
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
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
    environment("CARGO_TARGET_DIR", rustTarget.get().asFile.absolutePath)
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
    outputs.file(rustTarget.map { it.file("debug/$hostLibrary") })
}

val generateBindings by tasks.registering(Exec::class) {
    dependsOn(buildHost)
    rustInputs()
    commandLine("cargo", "run", "--locked", "-p", "wind-ffi", "--features", "bindgen",
        "--bin", "wind-bindgen", "--", "generate", "--library",
        rustTarget.get().file("debug/$hostLibrary").asFile.absolutePath,
        "--language", "kotlin", "--no-format",
        "--out-dir", generatedKotlin.get().asFile.absolutePath)
    outputs.dir(generatedKotlin)
}

val nativeTasks = listOf("arm64-v8a" to "aarch64-linux-android", "x86_64" to "x86_64-linux-android").map { (abi, triple) ->
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
        environment("${prefix}_RUSTFLAGS", "-C link-arg=--target=${triple}29 -C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-z,common-page-size=16384")
        commandLine("cargo", "build", "--locked", "--release", "-p", "wind-ffi", "--lib", "--target", triple)
        outputs.file(rustTarget.map { it.file("$triple/release/libwind_ffi.so") })
    }
    tasks.register<Copy>("stageRust${abi.replace("-", "").replace("_", "")}") {
        dependsOn(buildNative)
        from(rustTarget.map { it.file("$triple/release/libwind_ffi.so") })
        into(generatedJni.map { it.dir(abi) })
    }
}

tasks.named("preBuild") { dependsOn(generateBindings, nativeTasks) }
tasks.matching { it.name == "sourceReleaseJar" || it.name == "releaseSourcesJar" }
    .configureEach { dependsOn(generateBindings) }

dependencies {
    implementation("androidx.annotation:annotation:1.9.1")
    api("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    implementation("net.java.dev.jna:jna:5.17.0@aar")
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
