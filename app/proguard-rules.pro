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
