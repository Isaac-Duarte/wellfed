plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

android {
    namespace = "io.github.isaacduarte.wellfed"
    // permission_handler and reactive_ble compile against 37.
    compileSdk = 37
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        applicationId = "io.github.isaacduarte.wellfed"
        // Android 12: Bluetooth scans without location (neverForLocation).
        minSdk = 31
        targetSdk = flutter.targetSdkVersion
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    buildTypes {
        release {
            // Releases are not signed yet; debug keys keep `--release` runnable.
            signingConfig = signingConfigs.getByName("debug")
            // R8 sees no caller of the TLS verifier's Kotlin half: Rust
            // calls it over JNI.
            proguardFiles("proguard-rules.pro")
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

flutter {
    source = "../.."
}

// The Kotlin half of rustls-platform-verifier, which the Rust bridge calls to
// check an https server against Android's trust store. Its version must be
// the `rustls-platform-verifier-android` crate's, so it is read from the
// lockfile, and only this one package comes from the project's own Maven
// archive.
repositories {
    maven {
        url = uri("https://github.com/rustls/rustls-platform-verifier/raw/maven-archive/android-release-support/maven/")
        content { includeGroup("org.rustls") }
    }
}

val rustlsVerifierVersion: String =
    rootProject.file("../../../Cargo.lock").readLines().let { lines ->
        val name = lines.indexOfFirst { it.trim() == "name = \"rustls-platform-verifier-android\"" }
        require(name >= 0) { "rustls-platform-verifier-android is not in Cargo.lock" }
        lines[name + 1].trim().removePrefix("version = \"").removeSuffix("\"")
    }

dependencies {
    // The Bluetooth plugin's own RxJava, for its error handler in MainActivity.
    implementation("io.reactivex.rxjava2:rxjava:2.2.21")
    implementation("org.rustls:rustls-platform-verifier:$rustlsVerifierVersion")
}
