plugins {
  alias(libs.plugins.android.application)
  alias(libs.plugins.compose.compiler)
  alias(libs.plugins.kotlin.serialization)
}

android {
  namespace = "io.github.yashwanthkamireddi.skykeep"
  compileSdk = 37
  // The NDK the engine is built with: Android's packaging uses its strip to take the symbols
  // (kept for the bindings) out of the app.
  ndkVersion = "30.0.16248370"
  defaultConfig {
    applicationId = "io.github.yashwanthkamireddi.skykeep"
    minSdk = 29
    targetSdk = 36
    versionCode = 1
    versionName = "1.0"
    testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
  }

  buildTypes {
    release {
      isMinifyEnabled = false
      proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
    }
  }
  compileOptions {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
  }
  buildFeatures {
    compose = true
    aidl = false
    buildConfig = false
    shaders = false
  }

  packaging {
    resources {
      excludes += "/META-INF/{AL2.0,LGPL2.1}"
    }
  }
}

kotlin {
  jvmToolchain(17)
}

dependencies {
  // UniFFI's Kotlin bindings call the Rust engine through JNA.
  implementation("net.java.dev.jna:jna:5.19.1@aar")
  val composeBom = platform(libs.androidx.compose.bom)
  implementation(composeBom)
  androidTestImplementation(composeBom)

  // Core Android dependencies
  implementation(libs.androidx.core.ktx)
  implementation(libs.androidx.lifecycle.runtime.ktx)
  implementation(libs.androidx.activity.compose)

  // Arch Components
  implementation(libs.androidx.lifecycle.runtime.compose)
  implementation(libs.androidx.lifecycle.viewmodel.compose)

  // Compose
  implementation(libs.androidx.compose.ui)
  implementation(libs.androidx.compose.ui.tooling.preview)
  implementation(libs.androidx.compose.material3)
  // Tooling
  debugImplementation(libs.androidx.compose.ui.tooling)
  // Instrumented tests
  androidTestImplementation(libs.androidx.compose.ui.test.junit4)
  debugImplementation(libs.androidx.compose.ui.test.manifest)

  // Local tests: jUnit, coroutines, Android runner
  testImplementation(libs.junit)
  testImplementation(libs.kotlinx.coroutines.test)

  // Instrumented tests: jUnit rules and runners
  androidTestImplementation(libs.androidx.test.core)
  androidTestImplementation(libs.androidx.test.ext.junit)
  androidTestImplementation(libs.androidx.test.runner)
  androidTestImplementation(libs.androidx.test.rules)
  androidTestImplementation(libs.androidx.test.espresso.core)

  // Navigation
  implementation(libs.androidx.navigation3.ui)
  implementation(libs.androidx.navigation3.runtime)
  implementation(libs.androidx.lifecycle.viewmodel.navigation3)
}

// The engine, built from the repository's Rust crates before every build: the library for each
// ABI (cargo-ndk), then its Kotlin side generated from that same code (uniffi-bindgen), so the
// two can never be from different versions.
val repo = rootProject.projectDir.parentFile
val engine = tasks.register<Exec>("buildEngine") {
  workingDir = repo
  environment("CARGO_TARGET_DIR", File(repo, "target-android").path)
  // Symbols kept here: the bindings are read from them (UniFFI's metadata), and the app's own
  // packaging strips them from the APK (stripDebugDebugSymbols / its release twin).
  environment("CARGO_PROFILE_RELEASE_STRIP", "debuginfo")
  commandLine(
    "cargo", "ndk", "-t", "arm64-v8a", "-t", "x86_64", "-P", "29",
    "-o", file("src/main/jniLibs").path, "build", "--release", "-p", "quince-mobile",
  )
}
val bindings = tasks.register<Exec>("engineBindings") {
  dependsOn(engine)
  workingDir = repo
  environment("CARGO_TARGET_DIR", File(repo, "target-android/host").path)
  // Read from the phone library just built (the 64-bit Arm one): the tool is its own small
  // crate, so nothing of the engine is built for this computer.
  commandLine(
    "cargo", "run", "-q", "-p", "uniffi-bindgen", "--",
    "generate", "--library", file("src/main/jniLibs/arm64-v8a/libquince_mobile.so").path,
    "--language", "kotlin", "--no-format", "--out-dir", file("src/main/kotlin").path,
  )
}
tasks.named("preBuild") { dependsOn(bindings) }
