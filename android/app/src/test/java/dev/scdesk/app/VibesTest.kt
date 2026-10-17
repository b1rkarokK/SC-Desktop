package dev.scdesk.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** The same checks as the Rust tests of wave_vibe.rs: the two sides read a mood the same way. */
class VibesTest {
    @Test
    fun a_sad_word_lifts_a_track_and_the_opposite_word_sinks_it() {
        assertTrue(Vibes.factor("sad", "Грустная песня про дождь") > 2.5)
        assertTrue(Vibes.factor("sad", "dance Summer party") < 0.2)
        assertEquals(0.5, Vibes.factor("sad", "Untitled 4"), 1e-9)
        assertTrue(Vibes.factor("happy", "dance Summer party") > 2.5)
    }

    @Test
    fun a_word_is_matched_whole_unless_it_is_a_stem() {
        assertEquals(0.5, Vibes.factor("sad", "Blueprint"), 1e-9)
        assertTrue(Vibes.factor("sad", "Тоскую по тебе") > 2.5)
    }

    @Test
    fun no_mood_changes_nothing() {
        assertEquals(1.0, Vibes.factor("any", "Грустная песня"), 1e-9)
    }

    @Test
    fun every_listed_mood_has_words() {
        for ((id, _) in Vibes.all.filter { it.first != "any" }) assertTrue(id, VIBE_WORDS.containsKey(id))
    }
}
