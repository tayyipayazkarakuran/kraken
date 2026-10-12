plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "dev.kraken.app"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.kraken.app"
        // WebGL 2 in the system WebView.
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    // Stored uncompressed so the asset streams straight out of the APK.
    androidResources {
        noCompress += "wasm"
    }
}

dependencies {
    implementation("androidx.webkit:webkit:1.12.1")
}
