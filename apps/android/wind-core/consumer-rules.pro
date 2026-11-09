# CCS/1 RULE:UniFFI/JNA entry points are called through native reflection.
-keep class io.github.prejas3.windconnect.bindings.** { *; }
-keep class com.sun.jna.** { *; }
-keepattributes *Annotation*
