package com.pdflector.app.ink

import org.junit.Assert.assertEquals
import org.junit.Test

class InkProjectionTest {
    @Test
    fun mapsSurfaceCoordinatesToAndroidXFrontBufferClipSpace() {
        assertEquals(ClipPoint(-1f, -1f), InkProjection.toClip(0f, 0f, 100f, 200f))
        assertEquals(ClipPoint(1f, 1f), InkProjection.toClip(100f, 200f, 100f, 200f))
        assertEquals(ClipPoint(0f, 0f), InkProjection.toClip(50f, 100f, 100f, 200f))
    }

    @Test
    fun androidX90DegreePreRotationProjectsLandscapeCornersIntoBuffer() {
        val transform = floatArrayOf(
            0f, -1f, 0f, 0f,
            1f, 0f, 0f, 0f,
            0f, 0f, 1f, 0f,
            0f, 200f, 0f, 1f,
        )

        assertEquals(ClipPoint(-1f, 1f), InkProjection.toClip(0f, 0f, 100f, 200f, transform))
        assertEquals(ClipPoint(-1f, -1f), InkProjection.toClip(200f, 0f, 100f, 200f, transform))
        assertEquals(ClipPoint(1f, 1f), InkProjection.toClip(0f, 100f, 100f, 200f, transform))
        assertEquals(ClipPoint(1f, -1f), InkProjection.toClip(200f, 100f, 100f, 200f, transform))
    }

    @Test
    fun androidX270DegreePreRotationProjectsLandscapeCornersIntoBuffer() {
        val transform = floatArrayOf(
            0f, 1f, 0f, 0f,
            -1f, 0f, 0f, 0f,
            0f, 0f, 1f, 0f,
            100f, 0f, 0f, 1f,
        )

        assertEquals(ClipPoint(1f, -1f), InkProjection.toClip(0f, 0f, 100f, 200f, transform))
        assertEquals(ClipPoint(1f, 1f), InkProjection.toClip(200f, 0f, 100f, 200f, transform))
        assertEquals(ClipPoint(-1f, -1f), InkProjection.toClip(0f, 100f, 100f, 200f, transform))
        assertEquals(ClipPoint(-1f, 1f), InkProjection.toClip(200f, 100f, 100f, 200f, transform))
    }
}
