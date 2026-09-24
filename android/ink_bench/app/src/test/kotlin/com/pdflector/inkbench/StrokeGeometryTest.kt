package com.pdflector.inkbench

import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.FloatBuffer
import org.junit.Assert.assertEquals
import org.junit.Test

class StrokeGeometryTest {
    @Test
    fun segmentQuad_hasFixedWidthAndCausalEndpoints() {
        val from = StrokeSample(10f, 20f, 0.5f, 1L)
        val to = StrokeSample(30f, 20f, 0.5f, 2L)
        val buffer = directBuffer()

        StrokeGeometry.fillQuad(buffer, StrokeSegment(from, to), halfWidthPx = 5f)

        assertEquals(8, buffer.position())
        assertEquals(10f, buffer.get(0), 0.001f)
        assertEquals(25f, buffer.get(1), 0.001f)
        assertEquals(10f, buffer.get(2), 0.001f)
        assertEquals(15f, buffer.get(3), 0.001f)
        assertEquals(30f, buffer.get(4), 0.001f)
        assertEquals(25f, buffer.get(5), 0.001f)
        assertEquals(30f, buffer.get(6), 0.001f)
        assertEquals(15f, buffer.get(7), 0.001f)
    }

    @Test
    fun degenerateDownQuad_isVisibleAndCentered() {
        val sample = StrokeSample(40f, 60f, 0.5f, 1L)
        val buffer = directBuffer()

        StrokeGeometry.fillQuad(buffer, StrokeSegment(sample, sample), halfWidthPx = 5f)

        assertEquals(35f, buffer.get(0), 0.001f)
        assertEquals(65f, buffer.get(1), 0.001f)
        assertEquals(45f, buffer.get(2), 0.001f)
        assertEquals(65f, buffer.get(3), 0.001f)
        assertEquals(35f, buffer.get(4), 0.001f)
        assertEquals(55f, buffer.get(5), 0.001f)
        assertEquals(45f, buffer.get(6), 0.001f)
        assertEquals(55f, buffer.get(7), 0.001f)
    }

    private fun directBuffer(): FloatBuffer = ByteBuffer
        .allocateDirect(8 * Float.SIZE_BYTES)
        .order(ByteOrder.nativeOrder())
        .asFloatBuffer()
}
