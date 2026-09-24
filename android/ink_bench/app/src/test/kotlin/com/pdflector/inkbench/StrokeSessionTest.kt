package com.pdflector.inkbench

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class StrokeSessionTest {
    @Test
    fun downMoveUp_commitsOnlyTheActiveGeneration() {
        val session = StrokeSession(capacity = 8)

        val generation = session.begin(10L)
        val down = session.append(generation, StrokeSample(1f, 2f, 0.4f, 11L))
        val move = session.append(generation, StrokeSample(3f, 4f, 0.7f, 12L))

        val committed = session.end(generation, 13L)

        assertEquals(1f, down?.from?.x)
        assertEquals(1f, down?.to?.x)
        assertEquals(1f, move?.from?.x)
        assertEquals(3f, move?.to?.x)
        assertEquals(2L, committed.segmentCount)
        assertEquals(13L, committed.lastTimestampNanos)
        assertFalse(session.hasActiveStroke)
    }

    @Test
    fun cancel_discardsSamplesAndAdvancesGeneration() {
        val session = StrokeSession(capacity = 8)

        val first = session.begin(10L)
        assertTrue(session.append(first, StrokeSample(1f, 2f, 0.4f, 11L)) != null)
        session.cancel()

        val second = session.begin(20L)
        assertEquals(null, session.append(first, StrokeSample(3f, 4f, 0.7f, 21L)))
        assertTrue(session.append(second, StrokeSample(5f, 6f, 0.8f, 22L)) != null)

        val committed = session.end(second, 23L)

        assertEquals(1L, committed.segmentCount)
        assertEquals(2L, session.generation)
    }

    @Test
    fun capacity_isBoundedAndEvictsOldestDiagnosticSegment() {
        val session = StrokeSession(capacity = 2)
        val generation = session.begin(1L)

        assertTrue(session.append(generation, StrokeSample(1f, 1f, 0.1f, 2L)) != null)
        assertTrue(session.append(generation, StrokeSample(2f, 2f, 0.2f, 3L)) != null)
        assertTrue(session.append(generation, StrokeSample(3f, 3f, 0.3f, 4L)) != null)

        val committed = session.end(generation, 5L)

        assertEquals(3L, committed.segmentCount)
        assertEquals(1L, committed.diagnosticEvictions)
        assertEquals(1L, session.diagnosticEvictions)
    }

    @Test
    fun end_from_obsoleteGeneration_doesNotCommit() {
        val session = StrokeSession(capacity = 4)
        val obsolete = session.begin(1L)
        session.cancel()
        val current = session.begin(3L)
        assertTrue(session.append(current, StrokeSample(9f, 9f, 1f, 4L)) != null)

        val committed = session.end(obsolete, 5L)

        assertEquals(0L, committed.segmentCount)
        assertTrue(session.hasActiveStroke)
        assertTrue(session.append(current, StrokeSample(10f, 10f, 1f, 6L)) != null)
    }

    @Test
    fun segments_areCausalAndNeverUseFutureSamples() {
        val session = StrokeSession(capacity = 4)
        val token = session.begin(1L)
        val first = StrokeSample(10f, 10f, 0.1f, 2L)
        val second = StrokeSample(20f, 12f, 0.2f, 3L)
        val third = StrokeSample(30f, 14f, 0.3f, 4L)

        val down = session.append(token, first)
        val move = session.append(token, second)
        val next = session.append(token, third)

        assertEquals(first, down?.from)
        assertEquals(first, down?.to)
        assertEquals(first, move?.from)
        assertEquals(second, move?.to)
        assertEquals(second, next?.from)
        assertEquals(third, next?.to)
    }

    @Test
    fun append_rejectsDuplicateAndEarlierTimestamps() {
        val session = StrokeSession(capacity = 4)
        val token = session.begin(1L)
        val first = StrokeSample(10f, 10f, 0.2f, 4L)
        val later = StrokeSample(30f, 30f, 0.8f, 6L)

        assertEquals(first, session.append(token, first)?.to)
        assertEquals(null, session.append(token, StrokeSample(20f, 20f, 0.5f, 4L)))
        assertEquals(null, session.append(token, StrokeSample(15f, 15f, 0.4f, 3L)))
        val segment = session.append(token, later)

        assertEquals(first, segment?.from)
        assertEquals(later, segment?.to)
    }

    @Test
    fun upCommit_consolidatesFrontSegmentsWithoutMultiReplay() {
        val tracker = StrokeCommitTracker()
        tracker.frontSegmentSubmitted()
        tracker.frontSegmentSubmitted()

        val commit = tracker.upCommit()

        assertEquals(2, commit.frontSegments)
        assertEquals(0, commit.multiReplaySegments)
        assertEquals(0, tracker.upCommit().frontSegments)
    }
}
