# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
#
# R8 keep rules for the release build. Each rule has a reason; app/build.gradle.kts
# (checkReleaseKeepRules) fails the release build when the first two are missing.

# The generated UniFFI bindings: JNA maps their Structure/Callback/Library interfaces to native symbols by
# reflection on field and method names, so nothing here may be renamed or removed.
-keep class uniffi.croft_ffi.** { *; }
-keep interface uniffi.croft_ffi.** { *; }

# JNA: native code (libjnidispatch.so) calls back into these classes by name and reads their fields (JNI).
-keep class com.sun.jna.** { *; }
-keep interface com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.** { public *; }

# JNA's optional AWT and desktop helpers are not on Android; they are only referenced from code paths that never run.
-dontwarn java.awt.**
-dontwarn javax.swing.**

# kotlinx.serialization looks up generated serializers by name for @Serializable classes.
-keepattributes *Annotation*, InnerClasses
-keepclassmembers class kotlinx.serialization.json.** { *** Companion; }
-keepclasseswithmembers class **.*$Companion { kotlinx.serialization.KSerializer serializer(...); }
-if @kotlinx.serialization.Serializable class **
-keepclassmembers class <1> { static <1>$Companion Companion; }
