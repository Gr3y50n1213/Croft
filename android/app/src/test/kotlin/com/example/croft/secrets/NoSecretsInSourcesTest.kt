// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.secrets

import java.io.File
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Grep-style checks on the repository: the database schema has no column or table named for a
 * password or a token (secrets live only in the Keystore), and the Rust credential type is not
 * serializable. The tests run from android/app, so the repository is two folders up.
 */
class NoSecretsInSourcesTest {
    private val repo: File = generateSequence(File("").absoluteFile) { it.parentFile }
        .first { File(it, "core/storage/migrations").isDirectory }

    // An identifier that starts with one of these words. "tokenize" (FTS5 option) is not a token.
    private val secretName = Regex("""\b(password|passwd|passphrase|secret|token(?!ize)|credential|apikey|api_key)\w*""", RegexOption.IGNORE_CASE)

    private fun code(file: File): String =
        file.readLines().joinToString("\n") { it.substringBefore("--") }

    @Test
    fun noTableOrColumnInTheMigrationsIsNamedForASecret() {
        val migrations = File(repo, "core/storage/migrations").listFiles { f -> f.name.endsWith(".sql") }!!
        assertTrue("no migrations found", migrations.isNotEmpty())
        val hits = migrations.flatMap { f ->
            secretName.findAll(code(f)).map { "${f.name}: ${it.value}" }.toList()
        }
        assertTrue("secret-like names in the schema: $hits", hits.isEmpty())
    }

    @Test
    fun theRustCredentialTypeIsNotSerializable() {
        val text = File(repo, "core/ffi/src/credentials.rs").readText()
        assertTrue(text.contains("pub struct FfiCredential"))
        val serde = Regex("""derive\([^)]*Serialize|impl\s+(<[^>]*>\s*)?(serde::)?Serialize\s+for""")
        assertTrue("FfiCredential must not be Serialize", !serde.containsMatchIn(text))
    }
}
