import org.gradle.api.file.DuplicatesStrategy

plugins {
    java
}

group = providers.gradleProperty("group").get()
version = providers.gradleProperty("version").get()

repositories {
    mavenCentral()
    maven("https://repo.papermc.io/repository/maven-public/")
    maven("https://repo.extendedclip.com/content/repositories/placeholderapi/")
}

dependencies {
    compileOnly("io.papermc.paper:paper-api:1.21.6-R0.1-SNAPSHOT")
    compileOnly("me.clip:placeholderapi:2.11.6")

    implementation("com.google.code.gson:gson:2.11.0")

    testImplementation(platform("org.junit:junit-bom:5.11.0"))
    testImplementation("org.junit.jupiter:junit-jupiter")
}

java {
    toolchain.languageVersion.set(JavaLanguageVersion.of(21))
}

tasks {
    withType<JavaCompile>().configureEach {
        options.encoding = "UTF-8"
        options.release.set(21)
    }

    test {
        useJUnitPlatform()
    }

    processResources {
        filesMatching(listOf("plugin.yml", "paper-plugin.yml")) {
            expand("version" to project.version)
        }
    }

    jar {
        archiveBaseName.set("sculk-catalyst-paper-bridge")
        duplicatesStrategy = DuplicatesStrategy.EXCLUDE

        // Gson is the only bundled runtime dependency. Paper and PlaceholderAPI stay server-provided.
        from({
            configurations.runtimeClasspath.get()
                .filter { it.name.endsWith(".jar") }
                .map { zipTree(it) }
        }) {
            exclude("META-INF/*.SF", "META-INF/*.DSA", "META-INF/*.RSA")
        }

        manifest {
            attributes[
                "Implementation-Title"
            ] = "Sculk Catalyst Paper Bridge"
            attributes["Implementation-Version"] = project.version.toString()
            attributes["Main-Class"] = "com.sculkcatalyst.paperbridge.PaperBridgePlugin"
        }
    }
}
