# rustls-platform-verifier's Kotlin half is called from Rust over JNI, which
# R8 cannot see.
-keep, includedescriptorclasses class org.rustls.platformverifier.** { *; }
# The JNI hook MainActivity calls at startup.
-keep class io.github.isaacduarte.wellfed.TlsVerifier { *; }
