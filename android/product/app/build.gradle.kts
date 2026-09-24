import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

val signingProperties = Properties().apply {
    val localSigningFile = rootProject.file("signing.local.properties")
    if (localSigningFile.isFile) {
        localSigningFile.inputStream().use(::load)
    }
}
val storePath = signingProperties.getProperty("storeFile")
val storePasswordValue = signingProperties.getProperty("storePassword")
val keyAliasValue = signingProperties.getProperty("keyAlias")
val keyPasswordValue = signingProperties.getProperty("keyPassword")
val hasPersonalSigning = listOf(storePath, storePasswordValue, keyAliasValue, keyPasswordValue)
    .all { !it.isNullOrBlank() }
val workspaceRoot = rootProject.projectDir.resolve("../..").canonicalFile
val generatedRustJniLibs = layout.buildDirectory.dir("generated/rustJniLibs")

android {
    namespace = "com.pdflector.app"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.pdflector.app"
        minSdk = 29
        targetSdk = 35
        versionCode = 16777473
        versionName = "0.2.0"
        ndk { abiFilters += "arm64-v8a" }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }

    signingConfigs {
        if (hasPersonalSigning) {
            create("personal") {
                storeFile = file(storePath!!)
                storePassword = storePasswordValue
                keyAlias = keyAliasValue
                keyPassword = keyPasswordValue
            }
        }
    }

    buildTypes {
        getByName("debug") {
            if (hasPersonalSigning) {
                signingConfig = signingConfigs.getByName("personal")
            }
        }
        getByName("release") {
            if (hasPersonalSigning) {
                signingConfig = signingConfigs.getByName("personal")
            }
        }
    }

    sourceSets.getByName("main").jniLibs.srcDir(generatedRustJniLibs)
}

dependencies {
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.games:games-activity:4.4.0")
    implementation("androidx.graphics:graphics-core:1.0.4")
    testImplementation("junit:junit:4.13.2")
}

val buildRustArm64 by tasks.registering(Exec::class) {
    workingDir(workspaceRoot)
    commandLine("cargo", "build", "-p", "pdf_android", "--lib", "--release", "--target", "aarch64-linux-android")
    doFirst {
        val ndkHome = System.getenv("ANDROID_NDK_HOME")
            ?: throw GradleException("Set ANDROID_NDK_HOME to the Android NDK r28 directory")
        val toolchainBin = file("$ndkHome/toolchains/llvm/prebuilt/linux-x86_64/bin")
        environment("ANDROID_NDK_HOME", ndkHome)
        environment("PATH", "$toolchainBin${File.pathSeparator}${System.getenv("PATH") ?: ""}")
        environment(
            "BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android",
            "--sysroot=$ndkHome/toolchains/llvm/prebuilt/linux-x86_64/sysroot",
        )
    }
    doLast {
        val builtLibrary = workspaceRoot.resolve("target/aarch64-linux-android/release/libpdf_android.so")
        if (!builtLibrary.isFile) throw GradleException("Rust build did not produce $builtLibrary")
        copy {
            from(builtLibrary)
            into(generatedRustJniLibs.get().dir("arm64-v8a"))
        }
    }
}

tasks.register("assembleProductDebug") {
    group = "build"
    description = "Build the Rust ARM64 library and package the product debug APK."
    dependsOn(buildRustArm64, "assembleDebug")
}

// Packaging must never start before the freshly built native library is copied.
tasks.matching { it.name == "mergeDebugJniLibFolders" || it.name == "mergeDebugNativeLibs" }.configureEach {
    dependsOn(buildRustArm64)
}
