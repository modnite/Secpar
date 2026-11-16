# Don't obfuscate code (readable crash reports)
-dontobfuscate

# ML Kit discovers these from manifest metadata using reflective constructors.
# Its consumer rule preserves classes but does not preserve these constructors.
-keepclassmembers class * implements com.google.firebase.components.ComponentRegistrar {
    public <init>();
}

# Okio / OkHttp
-dontwarn org.codehaus.mojo.animal_sniffer.IgnoreJRERequirement
-dontwarn okio.**
# The managed engine (Moonlight, app/engine) is started from the manifest and from
# native code, so its entry points survive shrinking.
-keep class com.limelight.Game { *; }
-keepclasseswithmembernames class * { native <methods>; }
-keep class com.limelight.nvstream.jni.MoonBridge { *; }
-keep class com.limelight.binding.input.driver.UsbDriverService { *; }
-keep class com.pcremote.engine.NativeTunnel { *; }
# Moonlight creates this device's client certificate through BouncyCastle, which looks its
# algorithms up by class name. These are Moonlight's own keep rules for it.
-keep class org.bouncycastle.jcajce.provider.asymmetric.* {*;}
-keep class org.bouncycastle.jcajce.provider.asymmetric.util.* {*;}
-keep class org.bouncycastle.jcajce.provider.asymmetric.rsa.* {*;}
-keep class org.bouncycastle.jcajce.provider.digest.** {*;}
-keep class org.bouncycastle.jcajce.provider.symmetric.** {*;}
-keep class org.bouncycastle.jcajce.spec.* {*;}
-keep class org.bouncycastle.jce.** {*;}
-keep class sun.misc.Unsafe {*;}
