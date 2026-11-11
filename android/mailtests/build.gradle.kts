// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// T16: the tests of the app's mail layer against the real Rust core and a real (in-process) IMAP server, as plain JVM tests.
// They have no Android or Compose types, so they run without Robolectric (`./gradlew :mailtests:test`; also a dependency of
// `:app:testDebugUnitTest`, so one command and CI run both), like :swipe.
//
// There is no main code here. The tests compile the app's own data layer in place (`app/src/main/kotlin/.../mail`, minus the one
// file that needs an Android Context) and UniFFI bindings generated from the host library, which is built with croft-ffi's
// `test-server` feature so that they hold `TestImapServer`. The Android library never has that feature (see app/build.gradle.kts).
plugins {
    alias(libs.plugins.kotlin.jvm)
}

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

kotlin {
    compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) }
    sourceSets {
        getByName("test") {
            kotlin.srcDir(rootProject.layout.projectDirectory.dir("app/src/main/kotlin/com/example/croft/mail"))
            kotlin.srcDir(rootProject.layout.projectDirectory.dir("app/build/generated/uniffi-host/kotlin"))
            // The only file of that folder that needs Android (a Context).
            kotlin.exclude("**/MailSession.kt")
        }
    }
}

val hostLibDir = rootProject.layout.projectDirectory.dir("../core/target/debug")

dependencies {
    testImplementation(libs.junit)
    testImplementation(libs.jna)
    testImplementation(libs.kotlinx.coroutines.core)
    testImplementation(libs.kotlinx.serialization.json)
}

tasks.named("compileTestKotlin") { dependsOn(":app:generateUniffiHostBindings") }

tasks.withType<Test>().configureEach {
    dependsOn(":app:buildRustHost")
    // JNA finds the host build of the library (with TestImapServer) here.
    systemProperty("jna.library.path", hostLibDir.asFile.path)
    // A few tests wait for real timers (the undo window, the back-off); each test is bounded by its own timeout.
    maxHeapSize = "512m"
    testLogging {
        exceptionFormat = org.gradle.api.tasks.testing.logging.TestExceptionFormat.FULL
        showCauses = true
        showStackTraces = true
        events("failed")
    }
}
