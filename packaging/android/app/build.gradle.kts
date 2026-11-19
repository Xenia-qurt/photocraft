// Конфигурация Android-приложения PhotoCraft (GameActivity + Rust cdylib + SAF).
plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.getartcraft.photocraft"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.getartcraft.photocraft"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.23.1"

        ndk {
            // Основная архитектура — arm64-v8a; armeabi-v7a опционально для старых планшетов.
            abiFilters += listOf("arm64-v8a", "armeabi-v7a")
        }
    }

    sourceSets {
        getByName("main") {
            manifest.srcFile("../AndroidManifest.xml")
            jniLibs.srcDirs("src/main/jniLibs")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
        debug {
            isDebuggable = true
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    packaging {
        jniLibs {
            useLegacyPackaging = false
        }
    }
}

dependencies {
    implementation("androidx.games:games-activity:3.0.5")
    implementation("androidx.activity:activity-ktx:1.9.3")
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.appcompat:appcompat:1.7.0")
}
