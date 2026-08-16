plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "org.nightfallcoin.wallet"
    compileSdk = 35
    defaultConfig {
        applicationId = "org.nightfallcoin.wallet"
        minSdk = 26
        targetSdk = 35
        versionCode = 70
        versionName = "0.7.0"
        ndk {
            abiFilters += listOf("arm64-v8a")
        }
    }
    ndkVersion = "29.0.14206865"
    buildTypes {
        // Sideload. Android will not install an APK with no signature at all.
        // This is the debug cert, not a Play Store key — same honesty as
        // "unsigned" in the docs: checksums are the trust, not a store.
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("debug")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    buildFeatures { compose = true }
    packaging {
        jniLibs { useLegacyPackaging = true }
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.compose.ui:ui:1.7.5")
    implementation("androidx.compose.material3:material3:1.3.1")
    implementation("androidx.compose.material:material-icons-extended:1.7.5")
    implementation("net.java.dev.jna:jna:5.14.0@aar")
    implementation("com.google.zxing:core:3.5.3")
}
