// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import org.gradle.api.tasks.Exec

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

// The one place the application id is defined: android/gradle.properties.
val croftApplicationId = providers.gradleProperty("croft.applicationId").get()

// Rust side. Paths are relative to android/ (this project's parent).
val coreDir = rootProject.layout.projectDirectory.dir("../core")
val rustSources = fileTree(coreDir) {
    // Every crate croft-ffi depends on (engine -> mail-core, protocols, storage), the vendored
    // async-imap that [patch.crates-io] points at, and the workspace manifest and lock file.
    // test-support is a dev-dependency only: the library builds do not compile it.
    include(
        "Cargo.toml", "Cargo.lock",
        "ffi/**", "engine/**", "mail-core/**", "protocols/**", "storage/**", "vendor/**",
    )
    exclude("**/target/**")
}
val cargoVersion = Regex("""(?m)^version\s*=\s*"([^"]+)"""")
    .find(coreDir.file("Cargo.toml").asFile.readText())?.groupValues?.get(1)
    ?: error("no workspace version in core/Cargo.toml")
val hostLibDir = coreDir.dir("target/debug")
val hostLib = hostLibDir.file(System.mapLibraryName("croft_ffi"))
val rustJniLibs = layout.buildDirectory.dir("generated/rust/jniLibs")
val uniffiKotlin = layout.buildDirectory.dir("generated/uniffi/kotlin")
val abis = listOf("arm64-v8a", "x86_64")

// The database is encrypted (SQLCipher) in every build the app runs: `Core.open` refuses the key a
// library without this feature is given, so the app cannot silently store mail unencrypted.
// "-vendored-openssl" compiles OpenSSL's crypto from source (openssl-src) for the target: the NDK has
// none to link against, and the host build must not depend on the computer's OpenSSL either. The
// OpenSSL build needs perl and make on the build computer. The bindings generator uses the same features
// so that it shares the host build instead of compiling everything twice.
val rustFeatures = "sqlcipher-vendored-openssl"

// cargo-ndk builds libcroft_ffi.so for each ABI. Release profile: a debug Rust build is tens of MB.
// NDK r28+ links with 16 KB page alignment by default (Play requirement); see android/README.md.
tasks.register<Exec>("buildRustAndroid") {
    group = "rust"
    description = "Builds the Rust FFI library for arm64-v8a and x86_64 with cargo-ndk."
    workingDir = coreDir.asFile
    inputs.files(rustSources)
    outputs.dir(rustJniLibs)
    val ndkDir = androidComponents.sdkComponents.ndkDirectory
    doFirst { environment("ANDROID_NDK_HOME", ndkDir.get().asFile.path) }
    commandLine(
        listOf("cargo", "ndk") + abis.flatMap { listOf("-t", it) } +
            listOf(
                "--platform", "26",
                "-o", rustJniLibs.get().asFile.path,
                "build", "--release", "--locked", "-p", "croft-ffi", "--features", rustFeatures,
            ),
    )
}

// The host library: loaded by the JVM unit tests, and read by the bindings generator.
tasks.register<Exec>("buildRustHost") {
    group = "rust"
    description = "Builds the Rust FFI library for this computer (JVM unit tests, bindings generation)."
    workingDir = coreDir.asFile
    inputs.files(rustSources)
    outputs.file(hostLib)
    commandLine("cargo", "build", "--locked", "-p", "croft-ffi", "--features", rustFeatures)
}

// Kotlin bindings, generated from the library's own metadata with the UniFFI version it was built with.
tasks.register<Exec>("generateUniffiBindings") {
    group = "rust"
    description = "Generates the Kotlin bindings for croft-ffi with uniffi-bindgen."
    dependsOn("buildRustHost")
    workingDir = coreDir.asFile
    inputs.files(rustSources)
    inputs.file(hostLib)
    outputs.dir(uniffiKotlin)
    doFirst { uniffiKotlin.get().asFile.deleteRecursively() }
    commandLine(
        "cargo", "run", "--locked", "-q", "-p", "croft-ffi", "--features", "cli,$rustFeatures", "--bin", "uniffi-bindgen", "--",
        "generate", "--library", hostLib.asFile.path, "--language", "kotlin", "--no-format",
        "--out-dir", uniffiKotlin.get().asFile.path,
    )
}

android {
    namespace = croftApplicationId
    // 37, not 36: Material 3 1.5 (needed for the public MotionScheme), Compose 1.12 and core-ktx 1.19
    // declare minCompileSdk 37. targetSdk stays 36 (the Play requirement); compileSdk only picks the API stubs.
    compileSdk = 37
    ndkVersion = "29.0.14206865"

    defaultConfig {
        applicationId = croftApplicationId
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = cargoVersion
        ndk { abiFilters += abis }
    }

    buildFeatures { compose = true }

    // F-Droid rejects the encrypted dependency block AGP adds by default, and it makes builds non-reproducible.
    dependenciesInfo {
        includeInApk = false
        includeInBundle = false
    }

    buildTypes {
        // Distribution build: R8 and resource shrinking, not debuggable. No signing config: CI and F-Droid build it
        // unsigned and the owner signs outside the repository (android/README.md, "Release build").
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            isDebuggable = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
        // For the macrobenchmark module (T14): the release build, signed with the debug key so it installs, and profileable
        // (app/src/benchmark/AndroidManifest.xml) so that the benchmark can measure it. Not for distribution.
        create("benchmark") {
            initWith(getByName("release"))
            signingConfig = signingConfigs.getByName("debug")
            matchingFallbacks += listOf("release")
            isDebuggable = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    testOptions {
        unitTests {
            isIncludeAndroidResources = true
        }
    }

    sourceSets {
        getByName("main") {
            // The generated design tokens are compiled in place, never copied.
            kotlin.directories.add("../../design/generated/kotlin")
            // Build outputs; preBuild (below) runs the tasks that fill them.
            kotlin.directories.add("build/generated/uniffi/kotlin")
            jniLibs.directories.add("build/generated/rust/jniLibs")
        }
    }
}

// The shrunk build loads the Rust library by name through the generated bindings and JNA: if R8 renames or removes
// those classes the app crashes at start, and nothing but a device run would notice. Two checks fail the release build
// instead: the keep rules must exist (before R8 runs), and R8's own mapping must show the bindings were not renamed (after).
val rulesFile = layout.projectDirectory.file("proguard-rules.pro")
val mappingFile = layout.buildDirectory.file("outputs/mapping/release/mapping.txt")
val checkReleaseKeepRules = tasks.register("checkReleaseKeepRules") {
    group = "verification"
    description = "Fails if proguard-rules.pro lacks the UniFFI/JNA keep rules."
    inputs.file(rulesFile)
    doLast {
        val text = rulesFile.asFile.readText()
        for (needle in listOf("uniffi.croft_ffi.**", "com.sun.jna.**")) {
            check(Regex("(?m)^-keep\\S*\\s+(class|interface)\\s+" + Regex.escape(needle)).containsMatchIn(text)) {
                "proguard-rules.pro has no -keep rule for $needle (the shrunk app would not load the Rust library)"
            }
        }
    }
}
val checkReleaseMapping = tasks.register("checkReleaseMapping") {
    group = "verification"
    description = "Fails if R8 renamed a UniFFI or JNA class, using R8's mapping.txt."
    inputs.file(mappingFile)
    doLast {
        // mapping.txt lists "original -> obfuscated:"; a kept class maps to its own name.
        val renamed = mappingFile.get().asFile.useLines { lines ->
            lines.filter { it.startsWith("uniffi.croft_ffi.") || it.startsWith("com.sun.jna.") }
                .filter { it.endsWith(":") }
                // D8 desugaring creates and renames these; nothing looks them up by name.
                .filter { !it.contains("ExternalSynthetic") }
                .filter { it.substringBefore(" -> ") != it.substringAfter(" -> ").removeSuffix(":") }
                .take(5).toList()
        }
        check(renamed.isEmpty()) { "R8 renamed classes that JNA/UniFFI look up by name: $renamed" }
    }
}
afterEvaluate {
    tasks.matching { it.name == "minifyReleaseWithR8" }.configureEach { dependsOn(checkReleaseKeepRules) }
    checkReleaseMapping.configure { mustRunAfter("minifyReleaseWithR8") }
    tasks.matching { it.name == "assembleRelease" || it.name == "bundleRelease" }.configureEach { finalizedBy(checkReleaseMapping) }
}

tasks.named("preBuild") { dependsOn("generateUniffiBindings", "buildRustAndroid") }

tasks.withType<Test>().configureEach {
    dependsOn("buildRustHost")
    // The pure swipe machine's tests (module :swipe) run with the app's, so one command (and CI) covers both.
    dependsOn(":swipe:test")
    // JNA finds the host build of the library here; the generated bindings load it by name.
    systemProperty("jna.library.path", hostLibDir.asFile.path)
    systemProperty("croft.cargoVersion", cargoVersion)
    // Robolectric downloads its Android jars from Maven Central at run time. Behind a proxy or
    // mirror, set ROBOLECTRIC_REPO_URL to the Maven repository to use instead.
    // Screenshot tests (Roborazzi): verify (default), record-goldens or off. See ScreenshotKit.
    systemProperty("croft.screenshots", providers.gradleProperty("croft.screenshots").getOrElse("verify"))
    // Screenshot tests draw many large pictures on native graphics.
    maxHeapSize = "3g"
    // A failing test prints its whole stack trace in the log: the HTML report is not kept by CI, and the short form
    // ("AssertionError at File.kt:12") says too little to fix a test that was never run on the machine it was written on.
    testLogging {
        exceptionFormat = org.gradle.api.tasks.testing.logging.TestExceptionFormat.FULL
        showCauses = true
        showStackTraces = true
    }
    System.getenv("ROBOLECTRIC_REPO_URL")?.let { systemProperty("robolectric.dependency.repo.url", it) }
}

dependencies {
    // T15: the swipe state machine, settings types and pill geometry (plain Kotlin, its own tests).
    implementation(project(":swipe"))
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.material.icons.core)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.core.ktx)
    // The generated bindings call the Rust library through JNA. Android needs the aar (it carries
    // libjnidispatch.so); the JVM unit tests need the plain jar.
    implementation("${libs.jna.get()}@aar")
    // T12 theme engine. Material Color Utilities (Apache-2.0): only the com.google.android.material.color.utilities
    // classes are used. They ship inside this artifact (there is no separate one); its resources need the transitive
    // AndroidX Views libraries for the build to link, and R8 removes what the app does not call.
    implementation(libs.material.components)
    implementation(libs.androidx.datastore)
    implementation(libs.kotlinx.serialization.json)
    testImplementation(libs.jna)

    testImplementation(libs.junit)
    testImplementation(libs.robolectric)
    testImplementation(libs.roborazzi)
    testImplementation(libs.roborazzi.compose)
    testImplementation(platform(libs.androidx.compose.bom))
    testImplementation(libs.androidx.compose.ui.test.junit4)
    debugImplementation(libs.androidx.compose.ui.test.manifest)
}
