plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.pdflector.inkbench"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.pdflector.inkbench"
        minSdk = 29
        targetSdk = 35
        versionCode = 1
        versionName = "0.1"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.graphics:graphics-core:1.0.4")
    implementation("org.jetbrains.kotlin:kotlin-stdlib:2.0.21")

    testImplementation("junit:junit:4.13.2")
}
