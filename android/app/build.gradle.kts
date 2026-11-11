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
    include("Cargo.toml", "Cargo.lock", "ffi/**", "storage/**", "protocols/**")
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
                "build", "--release", "--locked", "-p", "croft-ffi",
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
    commandLine("cargo", "build", "--locked", "-p", "croft-ffi")
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
        "cargo", "run", "--locked", "-q", "-p", "croft-ffi", "--features", "cli", "--bin", "uniffi-bindgen", "--",
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

tasks.named("preBuild") { dependsOn("generateUniffiBindings", "buildRustAndroid") }

tasks.withType<Test>().configureEach {
    dependsOn("buildRustHost")
    // JNA finds the host build of the library here; the generated bindings load it by name.
    systemProperty("jna.library.path", hostLibDir.asFile.path)
    systemProperty("croft.cargoVersion", cargoVersion)
    // Robolectric downloads its Android jars from Maven Central at run time. Behind a proxy or
    // mirror, set ROBOLECTRIC_REPO_URL to the Maven repository to use instead.
    System.getenv("ROBOLECTRIC_REPO_URL")?.let { systemProperty("robolectric.dependency.repo.url", it) }
}

dependencies {
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.core.ktx)
    // The generated bindings call the Rust library through JNA. Android needs the aar (it carries
    // libjnidispatch.so); the JVM unit tests need the plain jar.
    implementation("${libs.jna.get()}@aar")
    testImplementation(libs.jna)

    testImplementation(libs.junit)
    testImplementation(libs.robolectric)
    testImplementation(platform(libs.androidx.compose.bom))
    testImplementation(libs.androidx.compose.ui.test.junit4)
    debugImplementation(libs.androidx.compose.ui.test.manifest)
}
