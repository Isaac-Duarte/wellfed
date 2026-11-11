import java.util.Properties

plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

// The release APK is signed with the owner's own keystore, which never sits
// in the repository: WELLFED_ANDROID_KEY_PROPERTIES names a key.properties
// file outside it (storeFile, storePassword, keyAlias, keyPassword). Only a
// release build needs it, and one without it stops before anything is
// built; debug builds keep Android's debug key either way. A lost keystore means
// phones must uninstall the app to take a build signed with a new one.
val repositoryRoot = rootProject.file("../../..").canonicalFile
val releaseKeyVariable = "WELLFED_ANDROID_KEY_PROPERTIES"

class ReleaseKey(val storeFile: File, val storePassword: String, val keyAlias: String, val keyPassword: String)

fun outsideRepository(file: File): Boolean =
    !file.canonicalPath.startsWith(repositoryRoot.path + File.separator)

/** The release key, or why there is none. */
fun loadReleaseKey(): Pair<ReleaseKey?, String?> {
    val path = System.getenv(releaseKeyVariable)
        ?: return null to "$releaseKeyVariable is not set: name the key.properties file that holds the release keystore"
    val propertiesFile = File(path)
    if (!propertiesFile.isAbsolute) return null to "$releaseKeyVariable must be an absolute path, not $path"
    if (!propertiesFile.isFile) return null to "$releaseKeyVariable names $path, which is not a file"
    if (!outsideRepository(propertiesFile)) return null to "$path is inside the repository; keep the signing key outside it"

    val properties = Properties().apply { propertiesFile.inputStream().use { load(it) } }
    val missing = listOf("storeFile", "storePassword", "keyAlias", "keyPassword").filter { properties.getProperty(it).isNullOrEmpty() }
    if (missing.isNotEmpty()) return null to "$path lacks ${missing.joinToString()}"

    val storeFile = File(properties.getProperty("storeFile"))
    if (!storeFile.isAbsolute) return null to "storeFile in $path must be an absolute path"
    if (!storeFile.isFile) return null to "storeFile in $path names ${storeFile.path}, which is not a file"
    if (!outsideRepository(storeFile)) return null to "${storeFile.path} is inside the repository; keep the keystore outside it"

    return ReleaseKey(
        storeFile,
        properties.getProperty("storePassword"),
        properties.getProperty("keyAlias"),
        properties.getProperty("keyPassword"),
    ) to null
}

val (releaseKey, releaseKeyProblem) = loadReleaseKey()

android {
    namespace = "io.github.isaacduarte.wellfed"
    // permission_handler and reactive_ble compile against 37.
    compileSdk = 37
    ndkVersion = flutter.ndkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
        // flutter_local_notifications needs it, for its scheduling API.
        isCoreLibraryDesugaringEnabled = true
    }

    defaultConfig {
        applicationId = "io.github.isaacduarte.wellfed"
        // Android 12: Bluetooth scans without location (neverForLocation).
        minSdk = 31
        targetSdk = flutter.targetSdkVersion
        versionCode = flutter.versionCode
        versionName = flutter.versionName
        ndk {
            // Every advertised ABI needs both the Flutter engine and Rust bridge.
            abiFilters.clear()
            abiFilters.addAll(listOf("arm64-v8a", "x86_64"))
        }
    }

    signingConfigs {
        releaseKey?.let { key ->
            create("release") {
                storeFile = key.storeFile
                storePassword = key.storePassword
                keyAlias = key.keyAlias
                keyPassword = key.keyPassword
            }
        }
    }

    buildTypes {
        release {
            // Never the debug key: without the owner's, the check below
            // stops the build.
            signingConfig = signingConfigs.findByName("release")
            // R8 sees no caller of the TLS verifier's Kotlin half: Rust
            // calls it over JNI.
            proguardFiles("proguard-rules.pro")
        }
    }
}

// Every release variant's build starts with preReleaseBuild, so this stops
// `flutter build apk --release` and `appbundle` alike, and nothing else.
val requireReleaseKey by tasks.registering {
    val problem = releaseKeyProblem
    doFirst {
        if (problem != null) {
            throw GradleException("A release build is signed with the owner's keystore (src/app/README.md#release-signing): $problem")
        }
    }
}
tasks.matching { it.name == "preReleaseBuild" }.configureEach { dependsOn(requireReleaseKey) }

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
    // The periodic catch-up, scheduled from Kotlin rather than through a
    // plugin: the plugins start their Flutter engine with every plugin
    // attached, which takes the Bluetooth plugin's links over from the app.
    implementation("androidx.work:work-runtime-ktx:2.12.0")
    coreLibraryDesugaring("com.android.tools:desugar_jdk_libs:2.1.5")
}
