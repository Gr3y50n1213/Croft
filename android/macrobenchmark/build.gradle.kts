// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Macrobenchmarks of the app (T14: scrolling the 5,000-row Inbox). They run on a device or an emulator, never in the JVM unit
// tests: `./gradlew :macrobenchmark:connectedBenchmarkAndroidTest` with a device attached. Nothing here is part of the app.
plugins {
    alias(libs.plugins.android.test)
}

val croftApplicationId = providers.gradleProperty("croft.applicationId").get()

android {
    namespace = "$croftApplicationId.macrobenchmark"
    compileSdk = 37

    defaultConfig {
        minSdk = 28 // Macrobenchmark needs API 28 or later.
        targetSdk = 36
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        // The application id lives in one place (android/gradle.properties); the benchmark reads it from here.
        testInstrumentationRunnerArguments["croft.targetPackage"] = croftApplicationId
        // T34c: the app has store flavours; the benchmark measures the Play build.
        missingDimensionStrategy("store", "play")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    targetProjectPath = ":app"
    // The test APK instruments the target app from outside (a separate process), as macrobenchmarks need.
    experimentalProperties["android.experimental.self-instrumenting"] = true

    buildTypes {
        create("benchmark") {
            isDebuggable = true
            signingConfig = signingConfigs.getByName("debug")
            matchingFallbacks += listOf("release")
        }
    }
}

androidComponents {
    // Only the benchmark build type makes sense here: it benchmarks the app's `benchmark` build type.
    beforeVariants(selector().all()) { variant -> variant.enable = variant.buildType == "benchmark" }
}

dependencies {
    implementation(libs.androidx.test.ext.junit)
    implementation(libs.androidx.test.uiautomator)
    implementation(libs.androidx.benchmark.macro.junit4)
}
