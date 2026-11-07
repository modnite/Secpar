# Preserve upstream's JNI callback names and native signatures in the final APK.
# Matches the Moonlight Android rule, without disabling obfuscation app-wide.
-keep class com.limelight.nvstream.jni.** { *; }
