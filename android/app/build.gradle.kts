import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.roborazzi)
}

val spoteVersionName = providers.gradleProperty("spoteVersionName").orElse("0.1.0").get()
val spoteVersionCode = providers.gradleProperty("spoteVersionCode").orElse("1").get().toInt()
val spoteRepo = providers.gradleProperty("spoteRepo").orElse("fabiomode2/spote").get()
val spoteMockData = providers.gradleProperty("spoteMockData").orElse("true").get().toBoolean()

val keystorePath = providers.gradleProperty("spoteKeystore").orNull
val keystorePassword = providers.gradleProperty("spoteKeystorePassword").orNull
val keystoreAlias = providers.gradleProperty("spoteKeyAlias").orNull
val keystoreKeyPassword = providers.gradleProperty("spoteKeyPassword").orNull

android {
    namespace = "dev.spote.app"
    compileSdk = 36

    defaultConfig {
        applicationId = "dev.spote.app"
        minSdk = 26
        targetSdk = 36
        versionCode = spoteVersionCode
        versionName = spoteVersionName

        buildConfigField("String", "GITHUB_REPO", "\"$spoteRepo\"")
        buildConfigField("boolean", "USE_MOCK_DATA", spoteMockData.toString())

        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    signingConfigs {
        if (
            keystorePath != null &&
            keystorePassword != null &&
            keystoreAlias != null &&
            keystoreKeyPassword != null
        ) {
            create("release") {
                storeFile = file(keystorePath)
                storePassword = keystorePassword
                this.keyAlias = keystoreAlias
                this.keyPassword = keystoreKeyPassword
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            isShrinkResources = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            if (keystorePath != null) signingConfig = signingConfigs.getByName("release")
        }
        debug {
            applicationIdSuffix = ".debug"
            versionNameSuffix = "-debug"
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
        isCoreLibraryDesugaringEnabled = true
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }

    testOptions {
        unitTests {
            isIncludeAndroidResources = true
            all { it.systemProperty("robolectric.graphicsMode", "NATIVE") }
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_17)
    }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.navigation.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)

    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons)

    implementation(libs.media3.exoplayer)
    implementation(libs.media3.exoplayer.hls)
    implementation(libs.media3.session)

    implementation(libs.coil.compose)
    implementation(libs.coil.network.okhttp)
    implementation(libs.coroutines.android)
    implementation(libs.annotation)
    implementation(libs.okhttp)
    implementation(libs.jsoup)
    implementation(libs.security.crypto)
    implementation(libs.newpipe)
    implementation(libs.zxing.core)
    implementation(libs.zxing.android.embedded)
    implementation("net.java.dev.jna:jna:${libs.versions.jna.get()}@aar")

    coreLibraryDesugaring(libs.desugar)

    debugImplementation(libs.compose.ui.tooling)
    debugImplementation(libs.compose.ui.test.manifest)

    testImplementation(libs.junit)
    testImplementation(libs.robolectric)
    testImplementation(libs.roborazzi)
    testImplementation(libs.roborazzi.compose)
    testImplementation(platform(libs.compose.bom))
    testImplementation(libs.compose.ui.test.junit4)
}
