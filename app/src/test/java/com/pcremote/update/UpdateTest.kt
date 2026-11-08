package com.pcremote.update
import org.junit.Assert.*
import org.junit.Test
class UpdateTest {
    private fun release(tag: String, name: String = "Secpar-preview16.apk", hash: String = "a".repeat(64), url: String = "https://github.com/modnite/Secpar/releases/download/$tag/$name") =
        """{"tag_name":"$tag","assets":[{"name":"$name","browser_download_url":"$url","digest":"sha256:$hash","size":100}]}"""
    @Test fun productAndNewestSelection() {
        val list="[${release("v3.0.3-secpar-preview.9")},${release("v3.0.3-secpar-host-preview.99")},${release("v3.0.3-secpar-preview.16")}]"
        assertEquals("3.0.3-secpar-preview.16",Updater.select(list)?.version)
        assertNull(Updater.select(list,false))
        assertFalse(Updater.isNewer("3.0.3-secpar-preview.15","3.0.3-secpar.16"))
        assertTrue(Updater.isNewer("3.0.3","3.0.3-secpar.16"))
    }
    @Test fun invalidAssetsAreRejected() {
        assertNull(Updater.parse(release("v3.0.3-secpar-preview.16",hash="bad")))
        assertNull(Updater.parse(release("v3.0.3-secpar-preview.16",url="https://example.com/x.apk")))
        assertNull(Updater.parse(release("v3.0.3-secpar-preview.16",name="unrelated.apk")))
        assertNull(Updater.parse(release("v3.0.3-secpar-preview.16").replace("100", "200000001")))
    }

    @Test fun messagesUseOneEllipsisCharacterAndTheCurlyApostrophe() {
        assertEquals("Checking for updates\u2026", CHECKING)
        assertEquals("You\u2019re up to date.", UP_TO_DATE)
        assertEquals("Could not check for updates. Try again.", CHECK_FAILED)
        assertEquals("Installing restarts Secpar and ends any active stream.", UPDATE_WARNING)
        assertEquals(CHECKING, UpdateState().message)
    }
}
