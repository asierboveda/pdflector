package com.pdflector.app.ink

import org.junit.Assert.assertEquals
import org.junit.Test

class InkLedgerTest {
    @Test
    fun retainsConsecutiveCommitsUntilDryAcknowledgement() {
        val ledger = InkLedger()
        val first = segment(ledger.generation(), 1f)
        val second = segment(ledger.generation(), 2f)

        assertEquals(listOf(first), ledger.commit(listOf(first)))
        assertEquals(listOf(first, second), ledger.commit(listOf(second)))

        ledger.clear()
        assertEquals(emptyList<InkSegment>(), ledger.commit(listOf(first)))
        val third = segment(ledger.generation(), 3f)
        assertEquals(listOf(third), ledger.commit(listOf(third)))
    }

    private fun segment(generation: Long, x: Float) =
        InkSegment(x, 0f, x + 1f, 0f, 2f, 0xff000000.toInt(), generation)
}
