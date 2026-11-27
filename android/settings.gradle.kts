// A plain Gradle project: the Android Gradle Plugin and nothing else. The
// native library is built by cargo-ndk (app/build.gradle.kts runs it).
pluginManagement {
    repositories {
        google {
            content {
                includeGroupByRegex("com\\.android.*")
                includeGroupByRegex("com\\.google.*")
                includeGroupByRegex("androidx.*")
            }
        }
        // (Maven Central through Google's mirror of it first: Maven Central
        // itself answers a busy CI runner 429, too many requests, for minutes;
        // the same artifacts, and Maven Central behind it)
        maven("https://maven-central.storage-download.googleapis.com/maven2/")
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        maven("https://maven-central.storage-download.googleapis.com/maven2/")
        mavenCentral()
    }
}

rootProject.name = "axross-amp-android"
include(":app")
