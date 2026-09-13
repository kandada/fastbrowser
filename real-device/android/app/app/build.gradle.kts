plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.fastbrowser.device"
    compileSdk = 34
    defaultConfig {
        applicationId = "com.fastbrowser.device"
        minSdk = 24
        targetSdk = 34
        versionCode = 1
        versionName = "0.1"
    }
    buildTypes {
        release { isMinifyEnabled = false }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    // 把 shared/cases 同步进 assets（真机 runner 消费同一份 JSON 用例）
    sourceSets["main"].assets.srcDir("../../../shared/cases")}

dependencies {
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("com.google.code.gson:gson:2.11.0")
}
