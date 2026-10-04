package com.pdflector.app.ink

import android.opengl.GLES20
import androidx.graphics.lowlatency.BufferInfo
import androidx.graphics.lowlatency.GLFrontBufferedRenderer
import androidx.graphics.opengl.egl.EGLManager
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.sqrt

internal data class InkSegment(
    val x0: Float,
    val y0: Float,
    val x1: Float,
    val y1: Float,
    val widthPx: Float,
    val colorArgb: Int,
    val generation: Long,
)

/** Keeps only provisional ink that has not yet been acknowledged by the dry PDF layer. */
internal class InkLedger {
    private var generation = 0L
    private val committed = ArrayList<InkSegment>()

    @Synchronized fun generation(): Long = generation

    @Synchronized fun commit(params: Collection<InkSegment>): List<InkSegment> {
        committed.addAll(params.filter { it.generation == generation })
        return committed.toList()
    }

    @Synchronized fun clear() {
        generation++
        committed.clear()
    }
}

internal data class ClipPoint(val x: Float, val y: Float)

/**
 * Pure matrix math composing AndroidX's `BufferTransformer`-supplied `transform` (a pixel-space
 * rotate+translate, per `androidx.graphics:graphics-core`) with an orthographic projection into
 * clip space. Kept free of `android.*` so it is exercised on the local JVM: `android.opengl.Matrix`
 * is a stub there and throws at runtime.
 *
 * AndroidX hands the callback vertices in the *logical* (pre-rotation) surface size and a
 * `transform` matrix that maps that logical pixel space into the *buffer* pixel space (which is
 * width/height-swapped for 90/270 degree rotations, see `BufferInfo`). It does not itself produce
 * clip-space coordinates. The correct order, matching AndroidX's own samples, is:
 * `clip = ortho(0, bufferWidth, 0, bufferHeight, -1, 1) * transform * pixel`.
 */
internal object InkProjection {
    /** Fills [out] with an orthographic projection mapping pixel space [0, width] x [0, height]
     * to clip space [-1, 1], with y=0 (surface top) mapping to clip -1 so the identity-transform
     * case reproduces the mapping validated on device. */
    fun ortho(out: FloatArray, width: Float, height: Float) {
        require(width > 0f && height > 0f)
        for (i in 0 until 16) out[i] = 0f
        out[0] = 2f / width
        out[5] = 2f / height
        out[10] = -1f
        out[12] = -1f
        out[13] = -1f
        out[15] = 1f
    }

    /** Column-major 4x4 multiply `out = a * b`, matching `android.opengl.Matrix.multiplyMM`. */
    fun multiply(out: FloatArray, a: FloatArray, b: FloatArray) {
        for (col in 0 until 4) {
            for (row in 0 until 4) {
                var sum = 0f
                for (k in 0 until 4) sum += a[k * 4 + row] * b[col * 4 + k]
                out[col * 4 + row] = sum
            }
        }
    }

    /** Fills [out] with `ortho(bufferWidth, bufferHeight) * transform`, reusing [scratch] for the
     * intermediate ortho matrix so no allocation happens per call. */
    fun bufferMvp(
        out: FloatArray,
        scratch: FloatArray,
        transform: FloatArray,
        bufferWidth: Float,
        bufferHeight: Float,
    ) {
        ortho(scratch, bufferWidth, bufferHeight)
        multiply(out, scratch, transform)
    }

    /** Applies [matrix] to the point (x, y, 0, 1) and returns the resulting clip-space x/y. */
    fun apply(matrix: FloatArray, x: Float, y: Float): ClipPoint {
        val clipX = matrix[0] * x + matrix[4] * y + matrix[12]
        val clipY = matrix[1] * x + matrix[5] * y + matrix[13]
        return ClipPoint(clipX, clipY)
    }
}

/** Reusable causal-segment renderer; the AndroidX buffer transform is composed on the CPU into
 * an MVP matrix (see [InkProjection]) rather than assumed to be clip-space already. */
internal class InkGlRenderer(private val ledger: InkLedger) : GLFrontBufferedRenderer.Callback<InkSegment> {
    private var program = 0
    private var positionLocation = -1
    private var transformLocation = -1
    private var colorLocation = -1
    private val vertices = ByteBuffer
        .allocateDirect(8 * Float.SIZE_BYTES)
        .order(ByteOrder.nativeOrder())
        .asFloatBuffer()
    private val status = IntArray(1)
    private val orthoScratch = FloatArray(16)
    private val mvp = FloatArray(16)

    override fun onDrawFrontBufferedLayer(
        eglManager: EGLManager,
        width: Int,
        height: Int,
        bufferInfo: BufferInfo,
        transform: FloatArray,
        param: InkSegment,
    ) {
        prepareGl()
        if (bufferInfo.width <= 0 || bufferInfo.height <= 0) return
        GLES20.glViewport(0, 0, bufferInfo.width, bufferInfo.height)
        InkProjection.bufferMvp(mvp, orthoScratch, transform, bufferInfo.width.toFloat(), bufferInfo.height.toFloat())
        drawSegment(param)
    }

    override fun onDrawMultiBufferedLayer(
        eglManager: EGLManager,
        width: Int,
        height: Int,
        bufferInfo: BufferInfo,
        transform: FloatArray,
        params: Collection<InkSegment>,
    ) {
        prepareGl()
        if (bufferInfo.width <= 0 || bufferInfo.height <= 0) return
        GLES20.glViewport(0, 0, bufferInfo.width, bufferInfo.height)
        GLES20.glClearColor(0f, 0f, 0f, 0f)
        GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT)
        InkProjection.bufferMvp(mvp, orthoScratch, transform, bufferInfo.width.toFloat(), bufferInfo.height.toFloat())
        ledger.commit(params).forEach { drawSegment(it) }
    }

    private fun prepareGl() {
        if (program != 0) return
        val vertexShader = compileShader(GLES20.GL_VERTEX_SHADER, VERTEX_SHADER)
        val fragmentShader = compileShader(GLES20.GL_FRAGMENT_SHADER, FRAGMENT_SHADER)
        val linked = GLES20.glCreateProgram()
        check(linked != 0) { "Unable to create ink GL program" }
        GLES20.glAttachShader(linked, vertexShader)
        GLES20.glAttachShader(linked, fragmentShader)
        GLES20.glLinkProgram(linked)
        GLES20.glGetProgramiv(linked, GLES20.GL_LINK_STATUS, status, 0)
        GLES20.glDeleteShader(vertexShader)
        GLES20.glDeleteShader(fragmentShader)
        check(status[0] == GLES20.GL_TRUE) {
            "Unable to link ink GL program: ${GLES20.glGetProgramInfoLog(linked)}"
        }
        program = linked
        positionLocation = GLES20.glGetAttribLocation(program, "aPosition")
        transformLocation = GLES20.glGetUniformLocation(program, "uBufferTransform")
        colorLocation = GLES20.glGetUniformLocation(program, "uColor")
        GLES20.glEnable(GLES20.GL_BLEND)
        GLES20.glBlendFunc(GLES20.GL_SRC_ALPHA, GLES20.GL_ONE_MINUS_SRC_ALPHA)
        GLES20.glClearColor(0f, 0f, 0f, 0f)
    }

    private fun drawSegment(segment: InkSegment) {
        if (segment.widthPx <= 0f) return
        fillQuad(segment)
        vertices.position(0)

        GLES20.glUseProgram(program)
        GLES20.glUniformMatrix4fv(transformLocation, 1, false, mvp, 0)
        GLES20.glUniform4f(
            colorLocation,
            ((segment.colorArgb ushr 16) and 0xff) / 255f,
            ((segment.colorArgb ushr 8) and 0xff) / 255f,
            (segment.colorArgb and 0xff) / 255f,
            ((segment.colorArgb ushr 24) and 0xff) / 255f,
        )
        GLES20.glEnableVertexAttribArray(positionLocation)
        GLES20.glVertexAttribPointer(
            positionLocation,
            2,
            GLES20.GL_FLOAT,
            false,
            2 * Float.SIZE_BYTES,
            vertices,
        )
        GLES20.glDrawArrays(GLES20.GL_TRIANGLE_STRIP, 0, 4)
        GLES20.glDisableVertexAttribArray(positionLocation)
    }

    /** Writes the quad in surface pixel coordinates (top-left origin, y down); the GL uniform
     * `uBufferTransform` (see [mvp]) carries these into clip space. */
    private fun fillQuad(segment: InkSegment) {
        val dx = segment.x1 - segment.x0
        val dy = segment.y1 - segment.y0
        val lengthSquared = dx * dx + dy * dy
        val halfWidth = segment.widthPx / 2f
        vertices.position(0)
        if (lengthSquared == 0f) {
            putPixel(segment.x0 - halfWidth, segment.y0 + halfWidth)
            putPixel(segment.x0 + halfWidth, segment.y0 + halfWidth)
            putPixel(segment.x0 - halfWidth, segment.y0 - halfWidth)
            putPixel(segment.x0 + halfWidth, segment.y0 - halfWidth)
            return
        }

        val scale = halfWidth / sqrt(lengthSquared)
        val normalX = -dy * scale
        val normalY = dx * scale
        putPixel(segment.x0 + normalX, segment.y0 + normalY)
        putPixel(segment.x0 - normalX, segment.y0 - normalY)
        putPixel(segment.x1 + normalX, segment.y1 + normalY)
        putPixel(segment.x1 - normalX, segment.y1 - normalY)
    }

    private fun putPixel(x: Float, y: Float) {
        vertices.put(x)
        vertices.put(y)
    }

    private fun compileShader(type: Int, source: String): Int {
        val shader = GLES20.glCreateShader(type)
        check(shader != 0) { "Unable to create ink GL shader" }
        GLES20.glShaderSource(shader, source)
        GLES20.glCompileShader(shader)
        GLES20.glGetShaderiv(shader, GLES20.GL_COMPILE_STATUS, status, 0)
        check(status[0] == GLES20.GL_TRUE) {
            "Unable to compile ink GL shader: ${GLES20.glGetShaderInfoLog(shader)}"
        }
        return shader
    }

    private companion object {
        const val VERTEX_SHADER = """
            uniform mat4 uBufferTransform;
            attribute vec2 aPosition;
            void main() {
                gl_Position = uBufferTransform * vec4(aPosition, 0.0, 1.0);
            }
        """

        const val FRAGMENT_SHADER = """
            precision mediump float;
            uniform vec4 uColor;
            void main() {
                gl_FragColor = uColor;
            }
        """
    }
}
