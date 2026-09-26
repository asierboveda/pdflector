package com.pdflector.app.ink

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

private const val IDENTITY_MATRIX_INDEX_STEP = 5

private fun identity(): FloatArray {
    val m = FloatArray(16)
    for (i in 0 until 4) m[i * IDENTITY_MATRIX_INDEX_STEP] = 1f
    return m
}

/**
 * `androidx.graphics.lowlatency.BufferTransformer.computeTransform` builds these matrices with
 * `android.opengl.Matrix.setRotateM` (rotate about Z) followed by `translateM` (pixel-space
 * translation), i.e. `M = RotateZ(theta) * Translate(tx, ty, 0)`. `android.opengl.Matrix` is a
 * stub on the local JVM, so these are hand-expanded (column-major, matching AndroidX/GL
 * convention) rather than computed via that API. See BufferTransformer bytecode: rotation 90
 * uses theta=-90 and tx=-logicalWidth; rotation 180 uses theta=180 and tx=-logicalWidth,
 * ty=-logicalHeight; rotation 270 uses theta=90 and ty=-logicalHeight. Buffer dimensions swap
 * width/height for the 90/270 cases only.
 */
private fun rotateThenTranslate(thetaDegrees: Int, tx: Float, ty: Float): FloatArray {
    val theta = Math.toRadians(thetaDegrees.toDouble())
    val cos = Math.cos(theta).toFloat()
    val sin = Math.sin(theta).toFloat()
    val m = FloatArray(16)
    m[0] = cos
    m[1] = sin
    m[4] = -sin
    m[5] = cos
    m[10] = 1f
    m[15] = 1f
    m[12] = cos * tx - sin * ty
    m[13] = sin * tx + cos * ty
    return m
}

private fun rotation90(logicalWidth: Float) = rotateThenTranslate(-90, -logicalWidth, 0f)

private fun rotation180(logicalWidth: Float, logicalHeight: Float) =
    rotateThenTranslate(180, -logicalWidth, -logicalHeight)

private fun rotation270(logicalHeight: Float) = rotateThenTranslate(90, 0f, -logicalHeight)

class InkProjectionTest {
    @Test
    fun identityTransformReproducesThePreviousDirectClipMapping() {
        val width = 100f
        val height = 200f
        val mvp = FloatArray(16)
        val scratch = FloatArray(16)

        InkProjection.bufferMvp(mvp, scratch, identity(), width, height)

        assertEquals(ClipPoint(-1f, -1f), InkProjection.apply(mvp, 0f, 0f))
        assertEquals(ClipPoint(1f, 1f), InkProjection.apply(mvp, width, height))
        assertEquals(ClipPoint(0f, 0f), InkProjection.apply(mvp, width / 2f, height / 2f))
    }

    @Test
    fun rotation90MapsLogicalCornersOntoBufferClipCorners() {
        val logicalWidth = 1440f
        val logicalHeight = 2200f
        // AndroidX swaps buffer width/height for a 90 degree pre-rotation.
        val bufferWidth = logicalHeight
        val bufferHeight = logicalWidth
        val mvp = FloatArray(16)
        val scratch = FloatArray(16)

        InkProjection.bufferMvp(mvp, scratch, rotation90(logicalWidth), bufferWidth, bufferHeight)

        assertClipCorner(mvp, 0f, 0f, -1f, 1f)
        assertClipCorner(mvp, logicalWidth, 0f, -1f, -1f)
        assertClipCorner(mvp, 0f, logicalHeight, 1f, 1f)
        assertClipCorner(mvp, logicalWidth, logicalHeight, 1f, -1f)
        assertCenterStaysInsideViewport(mvp, logicalWidth, logicalHeight)
    }

    @Test
    fun rotation180MapsLogicalCornersOntoBufferClipCorners() {
        val logicalWidth = 1440f
        val logicalHeight = 2200f
        // 180 degrees keeps buffer dimensions equal to the logical ones.
        val mvp = FloatArray(16)
        val scratch = FloatArray(16)

        InkProjection.bufferMvp(mvp, scratch, rotation180(logicalWidth, logicalHeight), logicalWidth, logicalHeight)

        assertClipCorner(mvp, 0f, 0f, 1f, 1f)
        assertClipCorner(mvp, logicalWidth, 0f, -1f, 1f)
        assertClipCorner(mvp, 0f, logicalHeight, 1f, -1f)
        assertClipCorner(mvp, logicalWidth, logicalHeight, -1f, -1f)
        assertCenterStaysInsideViewport(mvp, logicalWidth, logicalHeight)
    }

    @Test
    fun rotation270MapsLogicalCornersOntoBufferClipCorners() {
        val logicalWidth = 1440f
        val logicalHeight = 2200f
        // AndroidX swaps buffer width/height for a 270 degree pre-rotation too.
        val bufferWidth = logicalHeight
        val bufferHeight = logicalWidth
        val mvp = FloatArray(16)
        val scratch = FloatArray(16)

        InkProjection.bufferMvp(mvp, scratch, rotation270(logicalHeight), bufferWidth, bufferHeight)

        assertClipCorner(mvp, 0f, 0f, 1f, -1f)
        assertClipCorner(mvp, logicalWidth, 0f, 1f, 1f)
        assertClipCorner(mvp, 0f, logicalHeight, -1f, -1f)
        assertClipCorner(mvp, logicalWidth, logicalHeight, -1f, 1f)
        assertCenterStaysInsideViewport(mvp, logicalWidth, logicalHeight)
    }

    private fun assertClipCorner(mvp: FloatArray, x: Float, y: Float, expectedClipX: Float, expectedClipY: Float) {
        val clip = InkProjection.apply(mvp, x, y)
        assertEquals(expectedClipX, clip.x, 1e-4f)
        assertEquals(expectedClipY, clip.y, 1e-4f)
    }

    private fun assertCenterStaysInsideViewport(mvp: FloatArray, logicalWidth: Float, logicalHeight: Float) {
        val clip = InkProjection.apply(mvp, logicalWidth / 2f, logicalHeight / 2f)
        assertTrue("clip.x=${clip.x} out of range", clip.x in -1f..1f)
        assertTrue("clip.y=${clip.y} out of range", clip.y in -1f..1f)
    }
}
