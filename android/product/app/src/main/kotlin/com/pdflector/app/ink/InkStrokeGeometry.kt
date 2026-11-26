package com.pdflector.app.ink

import kotlin.math.sqrt

/**
 * Pure (GL-free) triangle geometry for one wet ink segment, in surface pixel coordinates.
 * Kept free of `android.*` so the shape can be checked on the local JVM.
 */
internal object InkStrokeGeometry {
    /** Largest gap allowed between a tessellated round cap and the true circle, in px. */
    const val MAX_SAGITTA_PX = 0.25f

    /** Upper bound of vertices [capsule] writes, for sizing reusable buffers. */
    const val MAX_VERTICES = 6

    /** Floats (x, y per vertex) [capsule] may write. */
    const val MAX_FLOATS = MAX_VERTICES * 2

    fun halfWidth(widthPx: Float): Float = widthPx / 2f

    /**
     * Writes the segment as a `GL_TRIANGLES` list of (x, y) pairs into [out] and returns the
     * number of vertices written.
     */
    fun capsule(x0: Float, y0: Float, x1: Float, y1: Float, widthPx: Float, out: FloatArray): Int {
        require(out.size >= MAX_FLOATS)
        if (!(widthPx > 0f)) return 0
        val halfWidth = halfWidth(widthPx)
        val dx = x1 - x0
        val dy = y1 - y0
        val lengthSquared = dx * dx + dy * dy
        if (lengthSquared == 0f) {
            return quad(
                out,
                x0 - halfWidth, y0 + halfWidth,
                x0 + halfWidth, y0 + halfWidth,
                x0 - halfWidth, y0 - halfWidth,
                x0 + halfWidth, y0 - halfWidth,
            )
        }
        val scale = halfWidth / sqrt(lengthSquared)
        val normalX = -dy * scale
        val normalY = dx * scale
        return quad(
            out,
            x0 + normalX, y0 + normalY,
            x0 - normalX, y0 - normalY,
            x1 + normalX, y1 + normalY,
            x1 - normalX, y1 - normalY,
        )
    }

    /** Strip-ordered quad (a, b, c, d) as the two triangles (a, b, c) and (b, d, c). */
    private fun quad(
        out: FloatArray,
        ax: Float, ay: Float,
        bx: Float, by: Float,
        cx: Float, cy: Float,
        dx: Float, dy: Float,
    ): Int {
        val v = floatArrayOf(ax, ay, bx, by, cx, cy, bx, by, dx, dy, cx, cy)
        v.copyInto(out)
        return 6
    }
}
