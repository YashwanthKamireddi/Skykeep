# What the release build (R8) must keep.

# The engine is reached through JNA, which finds Java classes, fields and methods by name at run
# time: the generated bindings (uniffi.*) and JNA itself are kept whole.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class uniffi.** { *; }
-dontwarn java.awt.**
-dontwarn com.sun.jna.**
