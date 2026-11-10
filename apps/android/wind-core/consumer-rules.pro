# CCS/1 RULE:UniFFI/JNA entry points are called through native reflection.
-keep class io.github.prejas3.windconnect.bindings.** { *; }
-keep class com.sun.jna.** { *; }
-keepattributes *Annotation*
# CCS/1 LIMIT:JNA's desktop-only AWT helpers cannot run on Android; UniFFI uses none of them.
-dontwarn java.awt.Component
-dontwarn java.awt.GraphicsEnvironment
-dontwarn java.awt.HeadlessException
-dontwarn java.awt.Window
