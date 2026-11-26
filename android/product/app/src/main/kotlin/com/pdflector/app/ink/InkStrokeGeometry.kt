package com.pdflector.app.ink

import kotlin.math.PI
import kotlin.math.acos
import kotlin.math.atan2
import kotlin.math.ceil
import kotlin.math.cos
import kotlin.math.max
import kotlin.math.sin
import kotlin.math.sqrt

/**
 * Pure (GL-free) triangle geometry for one wet ink segment, in surface pixel coordinates.
 * Kept free of `android.*` so the shape can be checked on the local JVM.
 *
 * It mirrors the dry Rust stroke (`draw_polyline_gpu` in crates/pdf_android/src/gpu/pipeline.rs),
 * so ink does not change shape when the pen lifts and the PDF layer takes over:
 * - half-width `hw = max(widthPx / 2, 0.5)`, the clamp Rust applies to `width * scale / 2`;
 * - a ribbon of half-width `hw` along the segment, plus round discs of radius `hw` at the
 *   ends (Rust draws them at the first/last point, at turns over ~20 degrees and every `hw` of
 *   arc length).
 *
 * Each segment is emitted as a capsule (rectangle + two semicircle caps). Consecutive capsules
 * share their end discs, so their union is exactly the set of points within `hw` of the
 * polyline: round caps and round joins with no miter spikes. Rust draws its discs analytically
 * (distance in the fragment shader); here they are inscribed polygons with enough sides that
 * the gap to the true circle (the sagitta) stays under [MAX_SAGITTA_PX], well inside the
 * `[hw - 1, hw]` band where the dry shader antialiases the edge.
 */
internal object InkStrokeGeometry {
    /** Largest gap allowed between a tessellated round cap and the true circle, in px. */
    const val MAX_SAGITTA_PX = 0.25f

    /** Same floor as the dry renderer's `.max(0.5)` on the half-width. */
    const val MIN_HALF_WIDTH_PX = 0.5f

    /** Sides of the full circle for small widths (each cap uses half of them). */
    const val MIN_CIRCLE_SEGMENTS = 8

    /** Upper bound of circle sides, so the vertex buffer has a fixed size. Keeps the sagitta
     * under [MAX_SAGITTA_PX] up to hw of about 200 px, far above any pen width. */
    const val MAX_CIRCLE_SEGMENTS = 64

    /** Upper bound of vertices [capsule] writes: 2 rectangle triangles + 2 half fans. */
    const val MAX_VERTICES = 6 + 3 * MAX_CIRCLE_SEGMENTS

    /** Floats (x, y per vertex) [capsule] may write. */
    const val MAX_FLOATS = MAX_VERTICES * 2

    fun halfWidth(widthPx: Float): Float = max(widthPx / 2f, MIN_HALF_WIDTH_PX)

    /** Even number of sides of the inscribed circle polygon for [halfWidth], chosen so that
     * `halfWidth * (1 - cos(PI / n)) <= MAX_SAGITTA_PX` (clamped to the bounds above). */
    fun circleSegments(halfWidth: Float): Int {
        val ratio = 1.0 - MAX_SAGITTA_PX / halfWidth.toDouble()
        val needed = if (ratio <= 0.0) MIN_CIRCLE_SEGMENTS.toDouble() else ceil(PI / acos(ratio))
        // Clamp before rounding up to even: both bounds are even, so it stays in range.
        val clamped = needed.coerceIn(MIN_CIRCLE_SEGMENTS.toDouble(), MAX_CIRCLE_SEGMENTS.toDouble()).toInt()
        return clamped + (clamped and 1)
    }

    /**
     * Writes the segment as a `GL_TRIANGLES` list of (x, y) pairs into [out] and returns the
     * number of vertices written (0 for a non-positive or non-finite width).
     */
    fun capsule(x0: Float, y0: Float, x1: Float, y1: Float, widthPx: Float, out: FloatArray): Int {
        require(out.size >= MAX_FLOATS)
        if (!(widthPx > 0f) || widthPx.isInfinite()) return 0
        val halfWidth = halfWidth(widthPx)
        val segments = circleSegments(halfWidth)
        val dx = x1 - x0
        val dy = y1 - y0
        val length = sqrt(dx * dx + dy * dy)
        if (!(length > 1e-3f)) {
            // Dot (pen down without movement): full disc, as Rust's caps at both ends.
            return fan(out, 0, x0, y0, halfWidth, 0.0, segments, segments)
        }
        val scale = halfWidth / length
        val normalX = -dy * scale
        val normalY = dx * scale
        var n = 0
        n = put(out, n, x0 + normalX, y0 + normalY)
        n = put(out, n, x0 - normalX, y0 - normalY)
        n = put(out, n, x1 + normalX, y1 + normalY)
        n = put(out, n, x0 - normalX, y0 - normalY)
        n = put(out, n, x1 - normalX, y1 - normalY)
        n = put(out, n, x1 + normalX, y1 + normalY)
        // The +normal side is the direction rotated +90 degrees: turning from it by -PI sweeps
        // through the forward direction (end cap); by +PI through the backward one (start cap).
        // Both arcs start and end exactly on the rectangle corners, so there is no seam.
        val normalAngle = atan2(normalY.toDouble(), normalX.toDouble())
        val half = segments / 2
        n = fan(out, n, x1, y1, halfWidth, normalAngle, half, -segments)
        n = fan(out, n, x0, y0, halfWidth, normalAngle, half, segments)
        return n
    }

    /** Appends [steps] triangles fanning around (cx, cy) from [startAngle], each spanning
     * `2 * PI / circleSides` (negative [circleSides] turns clockwise in angle). */
    private fun fan(
        out: FloatArray,
        start: Int,
        cx: Float,
        cy: Float,
        radius: Float,
        startAngle: Double,
        steps: Int,
        circleSides: Int,
    ): Int {
        val step = 2.0 * PI / circleSides
        var n = start
        var prevX = cx + (radius * cos(startAngle)).toFloat()
        var prevY = cy + (radius * sin(startAngle)).toFloat()
        for (i in 1..steps) {
            val angle = startAngle + step * i
            val x = cx + (radius * cos(angle)).toFloat()
            val y = cy + (radius * sin(angle)).toFloat()
            n = put(out, n, cx, cy)
            n = put(out, n, prevX, prevY)
            n = put(out, n, x, y)
            prevX = x
            prevY = y
        }
        return n
    }

    private fun put(out: FloatArray, vertex: Int, x: Float, y: Float): Int {
        out[vertex * 2] = x
        out[vertex * 2 + 1] = y
        return vertex + 1
    }
}
