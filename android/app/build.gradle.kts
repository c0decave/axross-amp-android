import java.io.File

plugins {
    id("com.android.application")
}

// The repository's root (Cargo.toml, src/): the Gradle project is android/.
val crateDir: File = rootProject.projectDir.parentFile

// One version: Cargo.toml's. 0.1.0 -> versionCode 100 (major * 10000 +
// minor * 100 + patch).
val crateVersion: String = Regex("""(?m)^version\s*=\s*"([^"]+)"""")
    .find(File(crateDir, "Cargo.toml").readText())!!.groupValues[1]
val crateVersionCode: Int = crateVersion.substringBefore('-').split('.')
    .map { it.toInt() }
    .let { (major, minor, patch) -> major * 10000 + minor * 100 + patch }

// Which ABIs go into the APK: -Pabis=arm64-v8a (the release) or the default,
// arm64 for phones and x86_64 for emulators.
val abis: List<String> = ((findProperty("abis") as String?) ?: "arm64-v8a,x86_64")
    .split(',').map { it.trim() }.filter { it.isNotEmpty() }

// The release key, from the environment only (the release workflow sets it
// from the repository's secrets; see README, "Signing"). Without it a release
// build is left unsigned rather than signed with some other key.
val keystorePath: String? = System.getenv("AMP_KEYSTORE_FILE")?.takeIf { it.isNotEmpty() }

android {
    namespace = "io.github.c0decave.axrossamp"
    compileSdk = 36
    // (strips the native library's debug symbols out of the APK)
    ndkVersion = providers.gradleProperty("amp.ndkVersion").get()

    defaultConfig {
        applicationId = "io.github.c0decave.axrossamp"
        // 26: AAudio (cpal's Android backend), adaptive icons,
        // AudioFocusRequest.
        minSdk = 26
        targetSdk = 36
        versionCode = crateVersionCode
        versionName = crateVersion
        ndk {
            abiFilters += abis
        }
    }

    signingConfigs {
        keystorePath?.let { path ->
            create("release") {
                storeFile = File(path)
                storePassword = System.getenv("AMP_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("AMP_KEY_ALIAS")
                keyPassword = System.getenv("AMP_KEY_PASSWORD")
                enableV1Signing = false
                enableV2Signing = true
                enableV3Signing = true
            }
        }
    }

    buildTypes {
        debug {
            applicationIdSuffix = ".debug"
            versionNameSuffix = "-debug"
        }
        release {
            // (two small Java classes: nothing to shrink, and no R8 rules to
            // keep the JNI names by)
            isMinifyEnabled = false
            // (the JNI names, kept should minify ever be turned on)
            proguardFiles("proguard-rules.pro")
            signingConfig = signingConfigs.findByName("release")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    // The native library, where cargo-ndk puts it (one folder a build type),
    // and the licence texts (as assets: licenses/ in the APK).
    sourceSets {
        getByName("main").assets.srcDir(layout.buildDirectory.dir("licenses").get().asFile)
        getByName("debug").jniLibs.srcDir(layout.buildDirectory.dir("rust/debug").get().asFile)
        getByName("release").jniLibs.srcDir(layout.buildDirectory.dir("rust/release").get().asFile)
    }

    packaging {
        jniLibs {
            // (stored uncompressed and page-aligned: loaded straight from the APK)
            useLegacyPackaging = false
        }
    }

    lint {
        // (reported, not fatal: the first builds happen in CI only)
        abortOnError = false
        checkReleaseBuilds = false
    }
}

// -- the Rust side -----------------------------------------------------------------------------
//
// `cargo ndk` builds libaxrossamp.so for every ABI into build/rust/<type>/<abi>/
// before the APK's native libraries are merged. It needs cargo, cargo-ndk,
// the Rust targets, CMake (libopus) and ANDROID_NDK_HOME; -PskipCargo leaves
// it out (the libraries are already there).
fun cargoNdk(name: String, release: Boolean) = tasks.register<Exec>(name) {
    group = "build"
    description = "Builds libaxrossamp.so with cargo-ndk (${if (release) "release" else "debug"})"
    workingDir = crateDir
    val out = layout.buildDirectory.dir("rust/${if (release) "release" else "debug"}")
    val command = mutableListOf("cargo", "ndk")
    abis.forEach { command += listOf("--target", it) }
    command += listOf("--platform", "26", "--output-dir", out.get().asFile.absolutePath)
    command += listOf("build", "--locked", "--lib")
    // (the debug APK: Cargo.toml's preview profile - a release's checks, not
    // the dev profile's overflow checks and debug assertions)
    command += if (release) listOf("--release") else listOf("--profile", "preview")
    commandLine(command)
    // (cargo and cargo-ndk where rustup puts them, also when that is not on
    // the PATH Gradle was started with - F-Droid's build server installs
    // Rust in a step of its own)
    val cargoBin = File(System.getProperty("user.home"), ".cargo/bin")
    if (cargoBin.isDirectory) {
        environment("PATH", cargoBin.path + File.pathSeparator + (System.getenv("PATH") ?: ""))
    }
    // (a reproducible library: the checkout's and Cargo's home's absolute
    // paths - in panic messages, file!() - become fixed ones, so a build in
    // another folder, on F-Droid's build server, is the same byte for byte;
    // cargo-ndk sets no Rust flags of its own, nor does this project)
    val cargoHome = System.getenv("CARGO_HOME")?.takeIf { it.isNotEmpty() }
        ?: File(System.getProperty("user.home"), ".cargo").path
    environment(
        "CARGO_ENCODED_RUSTFLAGS",
        // (each as given and as resolved: a path through a symlink stays one)
        (
            setOf(File(cargoHome).absolutePath, File(cargoHome).canonicalPath)
                .map { "--remap-path-prefix=$it=/cargo" } +
                setOf(crateDir.absolutePath, crateDir.canonicalPath)
                    .map { "--remap-path-prefix=$it=/axross-amp-android" }
        ).joinToString("\u001f"),
    )
    doFirst {
        check(!System.getenv("ANDROID_NDK_HOME").isNullOrEmpty()) {
            "ANDROID_NDK_HOME is not set: cargo-ndk and libopus's build need the NDK"
        }
    }
    // (cargo knows itself what is up to date)
    outputs.upToDateWhen { false }
}

val cargoDebug = cargoNdk("cargoNdkDebug", false)
val cargoRelease = cargoNdk("cargoNdkRelease", true)

if (!project.hasProperty("skipCargo")) {
    tasks.matching { it.name == "mergeDebugJniLibFolders" }.configureEach { dependsOn(cargoDebug) }
    tasks.matching { it.name == "mergeReleaseJniLibFolders" }.configureEach { dependsOn(cargoRelease) }
}

// -- the licences ------------------------------------------------------------------------------
//
// What the APK is built from comes with it: this project's licence, its notes
// on third parties and the licence texts of every crate linked in, as
// assets/licenses/ in the APK.
val licenseAssets = tasks.register<Copy>("licenseAssets") {
    from(File(crateDir, "LICENSE"), File(crateDir, "THIRD_PARTY.md"), File(crateDir, "THIRD_PARTY_LICENSES.md"))
    into(layout.buildDirectory.dir("licenses/licenses"))
}

// (every task that reads the assets' folders depends on it: Gradle refuses a
// task that reads another's output without saying so)
tasks.matching {
    it.name == "preBuild" ||
        Regex("""merge\w*Assets|\w*[Ll]int\w*""").matches(it.name)
}.configureEach { dependsOn(licenseAssets) }
