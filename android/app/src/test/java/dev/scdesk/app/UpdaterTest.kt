package dev.scdesk.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class UpdaterTest {
    @Test
    fun the_feed_is_read() {
        val u = Updater.parse(
            """{"version":"0.2.0","versionCode":3,"notes":"Что нового","apk":{"url":"https://github.com/x/y/releases/download/v0.2.0/sc.apk","sha256":"ABCDEF","size":4500000}}""",
        )!!
        assertEquals("0.2.0", u.versionName)
        assertEquals(3, u.versionCode)
        assertEquals("abcdef", u.sha256)
        assertEquals(4_500_000L, u.size)
    }

    @Test
    fun a_broken_feed_is_not_an_update() {
        assertNull(Updater.parse("not json"))
        assertNull(Updater.parse("""{"version":"1"}"""))
    }
}
