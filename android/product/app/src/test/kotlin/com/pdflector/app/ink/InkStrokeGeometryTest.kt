package com.pdflector.app.ink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.cos
import kotlin.math.hypot
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sin

/**
 * The wet (AndroidX) segments must have the same shape as the dry Rust stroke
 * (`draw_polyline_gpu` in crates/pdf_android/src/gpu/pipeline.rs): a ribbon of half-width
 * `hw = max(widthPx / 2, 0.5)` plus round discs of radius `hw` at the ends and at the turns,
 * i.e. the set of points within `hw` of the polyline (round caps and round joins). The Rust
 * fragment shader fades the edge over `[hw - 1, hw]`, so the hard wet edge must lie in
 * `[hw - MAX_SAGITTA_PX, hw]`: everything closer than `hw - MAX_SAGITTA_PX` is covered and no
 * vertex sticks out beyond `hw`.
 */
class InkStrokeGeometryTest {
    private val out = FloatArray(InkStrokeGeometry.MAX_FLOATS)

    @Test
    fun halfWidthMatchesTheDryRendererClamp() {
        assertEquals(4f, InkStrokeGeometry.halfWidth(8f), 0f)
        assertEquals(0.5f, InkStrokeGeometry.halfWidth(0.4f), 0f)
    }

    @Test
    fun zeroLengthSegmentIsARoundDotOfHalfWidth() {
        for (width in floatArrayOf(0.4f, 1f, 3f, 8f, 40f, 200f, 400f)) {
            val hw = InkStrokeGeometry.halfWidth(width)
            val triangles = capsuleTriangles(10f, 20f, 10f, 20f, width)
            val polyline = listOf(10f to 20f)

            assertNoSpikes(triangles, polyline, hw, "dot w=$width")
            // The outline reaches the dry disc radius (it is not inset).
            val farthest = vertices(triangles).maxOf { (x, y) -> hypot(x - 10f, y - 20f) }
            assertEquals("dot w=$width radius", hw, farthest, 1e-3f * max(1f, hw))
            // A convex fan covering this ring covers the whole disc inside it.
            val r = hw - InkStrokeGeometry.MAX_SAGITTA_PX
            for (i in 0 until 720) {
                val a = Math.toRadians(i / 2.0)
                val px = 10f + (r * cos(a)).toFloat()
                val py = 20f + (r * sin(a)).toFloat()
                assertTrue("dot w=$width misses ($px, $py)", covered(triangles, px, py))
            }
            assertTrue(covered(triangles, 10f, 20f))
        }
    }

    @Test
    fun straightSegmentHasSemicircleCapsOfHalfWidth() {
        val width = 6f
        val hw = InkStrokeGeometry.halfWidth(width)
        val triangles = capsuleTriangles(0f, 0f, 20f, 0f, width)
        val polyline = listOf(0f to 0f, 20f to 0f)

        assertNoSpikes(triangles, polyline, hw, "segment")
        // Round caps extend the stroke past both ends along its axis.
        assertTrue(covered(triangles, -2.5f, 0f))
        assertTrue(covered(triangles, 22.5f, 0f))
        // ...but are round, not square: the corners of a square cap stay empty.
        assertTrue(!covered(triangles, -2.5f, 2.5f))
        assertTrue(!covered(triangles, 22.5f, -2.5f))
        assertCoversNeighbourhood(triangles, polyline, hw, "segment")
    }

    @Test
    fun sharpTurnIsFilledByARoundJoinWithoutSpikes() {
        val width = 6f
        val hw = InkStrokeGeometry.halfWidth(width)
        // ~162 degree turn at (20, 0): the classic case where butt-ended quads leave a notch
        // on the outer side of the corner and miters would spike far beyond hw.
        val polyline = listOf(0f to 0f, 20f to 0f, 2f to 6f)
        val triangles = ArrayList<Triangle>()
        for (i in 0 until polyline.size - 1) {
            val (x0, y0) = polyline[i]
            val (x1, y1) = polyline[i + 1]
            triangles += capsuleTriangles(x0, y0, x1, y1, width)
        }

        assertNoSpikes(triangles, polyline, hw, "join")
        // The outer side of the corner, beyond the tip along the incoming direction.
        assertTrue(covered(triangles, 22.5f, 0f))
        assertCoversNeighbourhood(triangles, polyline, hw, "join")
    }

    @Test
    fun nonPositiveWidthEmitsNothing() {
        assertEquals(0, InkStrokeGeometry.capsule(0f, 0f, 5f, 5f, 0f, out))
        assertEquals(0, InkStrokeGeometry.capsule(0f, 0f, 5f, 5f, -1f, out))
        assertEquals(0, InkStrokeGeometry.capsule(0f, 0f, 5f, 5f, Float.NaN, out))
    }

    @Test
    fun nonFiniteCoordinatesEmitNothing() {
        assertEquals(0, InkStrokeGeometry.capsule(Float.NaN, 0f, 5f, 5f, 4f, out))
        assertEquals(0, InkStrokeGeometry.capsule(0f, Float.POSITIVE_INFINITY, 5f, 5f, 4f, out))
        assertEquals(0, InkStrokeGeometry.capsule(0f, 0f, Float.NEGATIVE_INFINITY, 5f, 4f, out))
        assertEquals(0, InkStrokeGeometry.capsule(0f, 0f, 5f, Float.NaN, 4f, out))
    }

    private data class Triangle(
        val ax: Float, val ay: Float,
        val bx: Float, val by: Float,
        val cx: Float, val cy: Float,
    )

    private fun capsuleTriangles(x0: Float, y0: Float, x1: Float, y1: Float, width: Float): List<Triangle> {
        val count = InkStrokeGeometry.capsule(x0, y0, x1, y1, width, out)
        assertTrue("vertex count $count", count > 0 && count % 3 == 0)
        assertTrue(count <= InkStrokeGeometry.MAX_VERTICES)
        return (0 until count / 3).map { t ->
            val o = t * 6
            Triangle(out[o], out[o + 1], out[o + 2], out[o + 3], out[o + 4], out[o + 5])
        }
    }

    private fun vertices(triangles: List<Triangle>): List<Pair<Float, Float>> =
        triangles.flatMap { listOf(it.ax to it.ay, it.bx to it.by, it.cx to it.cy) }

    private fun covered(triangles: List<Triangle>, px: Float, py: Float): Boolean =
        triangles.any { inside(it, px, py) }

    private fun inside(t: Triangle, px: Float, py: Float): Boolean {
        val e0 = edge(t.ax, t.ay, t.bx, t.by, px, py)
        val e1 = edge(t.bx, t.by, t.cx, t.cy, px, py)
        val e2 = edge(t.cx, t.cy, t.ax, t.ay, px, py)
        val eps = 1e-3f
        return (e0 >= -eps && e1 >= -eps && e2 >= -eps) || (e0 <= eps && e1 <= eps && e2 <= eps)
    }

    /** Signed distance-like edge function (normalised by edge length). */
    private fun edge(ax: Float, ay: Float, bx: Float, by: Float, px: Float, py: Float): Float {
        val len = hypot(bx - ax, by - ay)
        if (len == 0f) return 0f
        return ((bx - ax) * (py - ay) - (by - ay) * (px - ax)) / len
    }

    private fun distanceToPolyline(polyline: List<Pair<Float, Float>>, px: Float, py: Float): Float {
        if (polyline.size == 1) return hypot(px - polyline[0].first, py - polyline[0].second)
        var best = Float.MAX_VALUE
        for (i in 0 until polyline.size - 1) {
            val (ax, ay) = polyline[i]
            val (bx, by) = polyline[i + 1]
            val dx = bx - ax
            val dy = by - ay
            val lenSq = dx * dx + dy * dy
            val t = if (lenSq == 0f) 0f else (((px - ax) * dx + (py - ay) * dy) / lenSq).coerceIn(0f, 1f)
            best = min(best, hypot(px - (ax + t * dx), py - (ay + t * dy)))
        }
        return best
    }

    private fun assertNoSpikes(
        triangles: List<Triangle>,
        polyline: List<Pair<Float, Float>>,
        hw: Float,
        label: String,
    ) {
        for ((x, y) in vertices(triangles)) {
            val d = distanceToPolyline(polyline, x, y)
            assertTrue("$label: vertex ($x, $y) is $d px from the stroke, beyond hw=$hw", d <= hw + 1e-3f * max(1f, hw))
        }
    }

    /** Every grid point within `hw - MAX_SAGITTA_PX` of the polyline is drawn. */
    private fun assertCoversNeighbourhood(
        triangles: List<Triangle>,
        polyline: List<Pair<Float, Float>>,
        hw: Float,
        label: String,
    ) {
        val inner = hw - InkStrokeGeometry.MAX_SAGITTA_PX
        val minX = polyline.minOf { it.first } - hw - 1f
        val maxX = polyline.maxOf { it.first } + hw + 1f
        val minY = polyline.minOf { it.second } - hw - 1f
        val maxY = polyline.maxOf { it.second } + hw + 1f
        val step = 0.1f
        var y = minY
        while (y <= maxY) {
            var x = minX
            while (x <= maxX) {
                if (distanceToPolyline(polyline, x, y) <= inner) {
                    assertTrue("$label misses ($x, $y)", covered(triangles, x, y))
                }
                x += step
            }
            y += step
        }
    }
}
