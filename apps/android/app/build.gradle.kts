plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "io.github.prejas3.windconnect"
    compileSdk = 35

    defaultConfig {
        applicationId = "io.github.prejas3.windconnect"
        minSdk = 29
        targetSdk = 35
        versionCode = 83
        versionName = "0.72.0"
        @Suppress("UNCHECKED_CAST")
        ndk { abiFilters += rootProject.extra["windAbis"] as List<String> }

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    // WN-124: the Play build's manifest removes the restricted permissions; the features
    // follow from what the APK declares (PermissionsDeclared.kt, WindCore), not from a flag.
    if (rootProject.extra["windStore"] == "play") {
        sourceSets.getByName("debug").manifest.srcFile("src/play/AndroidManifest.xml")
        sourceSets.getByName("release").manifest.srcFile("src/play/AndroidManifest.xml")
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"))
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    implementation(project(":wind-core"))
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")
    implementation(platform("androidx.compose:compose-bom:2025.04.01"))
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.compose.material3:material3")
    implementation("com.google.zxing:core:3.5.4")
    implementation("androidx.glance:glance-appwidget:1.2.0")
    implementation("androidx.glance:glance-material3:1.2.0")
    testImplementation("junit:junit:4.13.2")

    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("junit:junit:4.13.2")
    androidTestImplementation(platform("androidx.compose:compose-bom:2025.04.01"))
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}

// WN-124: a Play APK is named app-play-*.apk and built in app/build-play, so it is never mistaken
// for the full one and the two builds never overwrite each other's outputs.
if (rootProject.extra["windStore"] == "play") {
    base.archivesName.set("app-play")
    layout.buildDirectory.set(layout.projectDirectory.dir("build-play"))
}
