plugins {
    id("com.android.application")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

android {
    namespace = "dev.intelligent_position.ip_app"
    compileSdk = flutter.compileSdkVersion
    ndkVersion = flutter.ndkVersion

    compileOptions {
        // flutter_local_notifications 要用 Java 8 以后的 API 的脱糖版（F-16），没用定时通知也要开。
        isCoreLibraryDesugaringEnabled = true
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        // TODO: Specify your own unique Application ID (https://developer.android.com/studio/build/application-id.html).
        applicationId = "dev.intelligent_position.ip_app"
        // You can update the following values to match your application needs.
        // For more information, see: https://flutter.dev/to/review-gradle-config.
        minSdk = flutter.minSdkVersion
        targetSdk = flutter.targetSdkVersion
        // Uses the version code from pubspec.yaml. When using split APKs, 1000 * ABI_VERSION
        // is added automatically by Flutter. (https://developer.android.com/studio/build/configure-apk-splits#configure-APK-versions)
        // You can force using the value of versionCode by specifying the `-P force-version-code-ignoring-abi=true`
        // flag during build.
        versionCode = flutter.versionCode
        versionName = flutter.versionName
        // 锁屏推送（F-21）用的 Firebase 配置。不放进仓库（没有 google-services.json），构建时用
        // Gradle 属性（-PIP_FCM_APP_ID=…）或同名环境变量给；没给时锁屏推送不可用，App 照常能用。
        for (name in listOf("IP_FCM_APP_ID", "IP_FCM_API_KEY", "IP_FCM_PROJECT_ID", "IP_FCM_SENDER_ID")) {
            val value = (project.findProperty(name) as String?) ?: System.getenv(name) ?: ""
            require(!value.contains('"') && !value.contains('\\')) { "$name 里不能有引号和反斜杠" }
            buildConfigField("String", name, "\"$value\"")
        }
    }

    buildFeatures {
        buildConfig = true
    }

    buildTypes {
        release {
            // TODO: Add your own signing config for the release build.
            // Signing with the debug keys for now, so `flutter run --release` works.
            signingConfig = signingConfigs.getByName("debug")
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

dependencies {
    // Apache-2.0。
    coreLibraryDesugaring("com.android.tools:desugar_jdk_libs:2.1.4")
    // 锁屏推送（F-21）：收 FCM。SDK 本身是 Apache-2.0，运行时要手机上的谷歌 Play 服务（闭源）。
    implementation("com.google.firebase:firebase-messaging:24.1.1")
    // 弹通知（NotificationCompat）。Apache-2.0。
    implementation("androidx.core:core:1.13.1")
    // 解密的 JVM 单元测试（WebPushCryptoTest）。EPL-1.0，只在测试时用，不进 App。
    testImplementation("junit:junit:4.13.2")
}
