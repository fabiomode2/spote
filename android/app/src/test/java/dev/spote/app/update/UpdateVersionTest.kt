package dev.spote.app.update

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Tests JVM de la comparación semver y de la elección del asset APK. */
class UpdateVersionTest {

    @Test
    fun `remote tag with v prefix is newer`() {
        assertTrue(isNewerVersion("v0.2.0", "0.1.0"))
        assertTrue(isNewerVersion("0.2.0", "0.1.0-debug"))
    }

    @Test
    fun `same version is not newer`() {
        assertFalse(isNewerVersion("0.1.0", "0.1.0"))
        assertFalse(isNewerVersion("v0.1.0", "0.1.0"))
        assertFalse(isNewerVersion("0.1.0-debug", "0.1.0-debug"))
    }

    @Test
    fun `numeric components are compared numerically`() {
        assertTrue(isNewerVersion("0.10.0", "0.9.9"))
        assertTrue(isNewerVersion("1.0.0", "0.99.99"))
        assertTrue(isNewerVersion("0.1.10", "0.1.9"))
    }

    @Test
    fun `debug suffix on current version does not hide stable release`() {
        assertTrue(isNewerVersion("0.1.1", "0.1.1-debug"))
        assertTrue(isNewerVersion("0.1.0", "0.1.0-debug"))
    }

    @Test
    fun `pre release is lower than stable`() {
        assertFalse(isNewerVersion("0.1.0-beta.1", "0.1.0"))
        assertTrue(isNewerVersion("0.1.0", "0.1.0-beta.1"))
        assertTrue(isNewerVersion("0.1.0-beta.2", "0.1.0-beta.1"))
        assertTrue(isNewerVersion("0.1.0-beta.10", "0.1.0-beta.9"))
    }

    @Test
    fun `build metadata is ignored`() {
        assertEquals(0, compareVersions("1.2.3+build.4", "1.2.3"))
        assertFalse(isNewerVersion("1.2.3+build.4", "1.2.3"))
    }

    @Test
    fun `partial versions are padded`() {
        assertTrue(isNewerVersion("1.2.1", "1.2"))
        assertFalse(isNewerVersion("1.2", "1.2.0"))
        assertEquals(0, compareVersions("1.0", "1.0.0"))
    }

    @Test
    fun `unparsable versions never trigger an update`() {
        assertFalse(isNewerVersion("nightly", "0.1.0"))
        assertFalse(isNewerVersion("0.2.0", "dev"))
        assertEquals(0, compareVersions("nightly", "0.1.0"))
    }

    @Test
    fun `apk asset uses first arm64 or universal match`() {
        val selected = selectApkAsset(
            listOf(
                apk("spote-0.2.0-x86_64.apk"),
                apk("spote-0.2.0-arm64-v8a.apk"),
                apk("spote-0.2.0-universal.apk"),
            ),
        )
        assertEquals("spote-0.2.0-arm64-v8a.apk", selected?.name)
    }

    @Test
    fun `apk asset accepts universal when there is no arm64`() {
        val selected = selectApkAsset(
            listOf(
                apk("spote-0.2.0-x86_64.apk"),
                apk("spote-0.2.0-universal.apk"),
            ),
        )
        assertEquals("spote-0.2.0-universal.apk", selected?.name)
    }

    @Test
    fun `apk asset falls back to first apk`() {
        val selected = selectApkAsset(
            listOf(
                apk("spote-0.2.0.apk"),
                apk("spote-0.2.0-x86_64.apk"),
            ),
        )
        assertEquals("spote-0.2.0.apk", selected?.name)
    }

    @Test
    fun `apk asset ignores non apk files`() {
        val selected = selectApkAsset(
            listOf(
                ApkAsset("notes.txt", "https://example.test/notes.txt", null),
                apk("spote.apk"),
            ),
        )
        assertEquals("spote.apk", selected?.name)
    }

    @Test
    fun `apk selection is empty safe`() {
        assertNull(selectApkAsset(emptyList()))
        assertNull(
            selectApkAsset(
                listOf(ApkAsset("spote.zip", "https://example.test/spote.zip", null)),
            ),
        )
    }

    @Test
    fun `sha256 digest is parsed only when valid`() {
        val hex = "9f".repeat(32)
        assertEquals(hex, parseSha256Digest("sha256:$hex"))
        assertEquals(hex, parseSha256Digest("SHA256:${hex.uppercase()}"))
        assertNull(parseSha256Digest(null))
        assertNull(parseSha256Digest(""))
        assertNull(parseSha256Digest("md5:$hex"))
        assertNull(parseSha256Digest("sha256:1234"))
    }

    @Test
    fun `normalize version strips v only with numeric tag`() {
        assertEquals("0.2.0", normalizeVersion("v0.2.0"))
        assertEquals("0.2.0", normalizeVersion("V0.2.0"))
        assertEquals("version-one", normalizeVersion("version-one"))
    }

    private fun apk(name: String) = ApkAsset(
        name = name,
        url = "https://example.test/$name",
        sha256 = null,
    )
}
