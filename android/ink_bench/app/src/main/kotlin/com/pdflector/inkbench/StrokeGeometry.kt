package com.pdflector.inkbench

import java.nio.FloatBuffer
import kotlin.math.sqrt

/** Writes a causal, fixed-width segment quad into caller-owned reusable storage. */
object StrokeGeometry {
    fun fillQuad(
        buffer: FloatBuffer,
        segment: StrokeSegment,
        halfWidthPx: Float,
    ) {
        val from = segment.from
        val to = segment.to
        val dx = to.x - from.x
        val dy = to.y - from.y
        val lengthSquared = dx * dx + dy * dy

        buffer.position(0)
        if (lengthSquared == 0f) {
            putPoint(buffer, from.x - halfWidthPx, from.y + halfWidthPx)
            putPoint(buffer, from.x + halfWidthPx, from.y + halfWidthPx)
            putPoint(buffer, from.x - halfWidthPx, from.y - halfWidthPx)
            putPoint(buffer, from.x + halfWidthPx, from.y - halfWidthPx)
            return
        }

        val scale = halfWidthPx / sqrt(lengthSquared)
        val normalX = -dy * scale
        val normalY = dx * scale
        putPoint(buffer, from.x + normalX, from.y + normalY)
        putPoint(buffer, from.x - normalX, from.y - normalY)
        putPoint(buffer, to.x + normalX, to.y + normalY)
        putPoint(buffer, to.x - normalX, to.y - normalY)
    }

    private fun putPoint(buffer: FloatBuffer, x: Float, y: Float) {
        buffer.put(x)
        buffer.put(y)
    }
}
