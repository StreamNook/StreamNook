import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

// Release signing. keystore.properties and the .jks it points at are gitignored
// and live only on the release machine; when they are absent the release build
// still produces an unsigned APK rather than failing, so CI and clean clones can
// keep building.
val keystoreProperties = Properties().apply {
    val propFile = rootProject.file("keystore.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}
val hasReleaseKeystore = keystoreProperties.getProperty("storeFile") != null

android {
    compileSdk = 36
    // Must match local.properties `ndk.dir`. Without it AGP assumes its own
    // bundled default (27.0.12077973 for AGP 8.11), fails to find llvm-strip at
    // that path, and packages the .so UNSTRIPPED with only a warning: a debug
    // APK goes from about 110 MB to 494 MB, which is the difference between an
    // install that works over wifi adb and one that dies mid-transfer. The
    // mismatch also undermines the r28 pin, which exists so the .so gets 16 KB
    // aligned segments. Keep this in step whenever the NDK moves.
    ndkVersion = "28.2.13676358"
    // `app.streamnook`, NOT the desktop's `com.streamnook.dev`, and the split is
    // deliberate. Both derive from the Tauri identifier, but Android's is
    // overridden in `src-tauri/tauri.android.conf.json` (platform config files
    // merge over the base per JSON Merge Patch, RFC 7396). The desktop
    // identifier has to stay put: it keys the Windows installer/upgrade
    // identity, the updater, and the WebView2 profile dir that
    // `account_store.rs` reads cookies out of, for a shipped v8.3.9 app.
    //
    // Renamed while Android was still unreleased, because after release the
    // applicationId IS the app's identity: changing it offers no update path and
    // forces every user to uninstall and lose their login.
    namespace = "app.streamnook"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "app.streamnook"
        minSdk = 26
        targetSdk = 36
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
    }
    signingConfigs {
        if (hasReleaseKeystore) {
            create("release") {
                storeFile = rootProject.file(keystoreProperties.getProperty("storeFile"))
                storePassword = keystoreProperties.getProperty("storePassword")
                keyAlias = keystoreProperties.getProperty("keyAlias")
                keyPassword = keystoreProperties.getProperty("keyPassword")
            }
        }
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            // Tauri generates keepDebugSymbols for every ABI here. Removing it does
            // NOT shrink anything on its own: Tauri symlinks the prebuilt .so into
            // jniLibs and the AGP strip task skips symlinked libraries. The actual
            // fix is -C strip=debuginfo, applied in src-tauri/.cargo/config.toml.
            // Keeping this block empty simply avoids implying AGP handles it.
        }
        getByName("release") {
            if (hasReleaseKeystore) {
                signingConfig = signingConfigs.getByName("release")
            }
            isMinifyEnabled = true
            proguardFiles(
                *fileTree(".") { include("**/*.pro") }
                    .plus(getDefaultProguardFile("proguard-android-optimize.txt"))
                    .toList().toTypedArray()
            )
        }
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
    buildFeatures {
        buildConfig = true
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    // Foldable posture. The CSS Viewport Segments API would avoid this entirely
    // but it is still experimental / origin-trial only, so the hinge has to come
    // from WindowManager and be pushed into the web shell like the insets are.
    implementation("androidx.window:window:1.3.0")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.7")
    implementation("com.google.android.material:material:1.12.0")
    // Notification delivery while the app is closed. Two version traps here:
    // work-runtime-ktx has been an empty shim since WorkManager 2.9.0 (all the
    // Kotlin API, CoroutineWorker included, moved into work-runtime itself), and
    // 2.11.x switched its kotlin-stdlib dependency to 2.1.20, whose metadata the
    // Kotlin 1.9.25 plugin this project builds with cannot read. 2.10.5 is the
    // newest release still on stdlib 1.8.22, and it has everything needed
    // (ExistingPeriodicWorkPolicy.UPDATE landed in 2.8.0). Revisit if the
    // project's Kotlin plugin is ever bumped past 2.0.
    implementation("androidx.work:work-runtime:2.10.5")
    // Push receiver (FCM). Same Kotlin-metadata trap as WorkManager above:
    // 24.x is built against a stdlib the 1.9.25 plugin can read; check before
    // bumping to 25.x. The dependency is unconditional but inert without the
    // google-services plugin below: FirebaseApp never initializes, every call
    // site catches, and delivery falls back to the WorkManager poll.
    implementation("com.google.firebase:firebase-messaging:24.1.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = "tauri.build.gradle.kts")

// Same shape as the release keystore above: the config file is machine-local
// (google-services.json is committed, but a stripped clone or a fork without a
// Firebase project must still build), so the plugin only applies when it can
// possibly work. Without it push stays dormant and the poll lane delivers.
if (file("google-services.json").exists()) {
    apply(plugin = "com.google.gms.google-services")
}