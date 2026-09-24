package com.pdflector.app.ink

import android.opengl.GLES20
import androidx.graphics.lowlatency.BufferInfo
import androidx.graphics.lowlatency.GLFrontBufferedRenderer
import androidx.graphics.opengl.egl.EGLManager
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.FloatBuffer
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

/** Converts SurfaceView top-left pixel coordinates to GL clip coordinates. */
internal object InkProjection {
    fun toClip(x: Float, y: Float, width: Float, height: Float): ClipPoint {
        require(width > 0f && height > 0f)
        return ClipPoint(2f * x / width - 1f, 2f * y / height - 1f)
    }

    fun putClip(buffer: FloatBuffer, x: Float, y: Float, width: Float, height: Float) {
        buffer.put(2f * x / width - 1f)
        buffer.put(2f * y / height - 1f)
    }
}

/** Reusable causal-segment renderer; AndroidX pre-rotation is applied by the vertex shader. */
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

    override fun onDrawFrontBufferedLayer(
        eglManager: EGLManager,
        width: Int,
        height: Int,
        bufferInfo: BufferInfo,
        transform: FloatArray,
        param: InkSegment,
    ) {
        prepareGl()
        GLES20.glViewport(0, 0, bufferInfo.width, bufferInfo.height)
        drawSegment(width, height, transform, param)
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
        GLES20.glViewport(0, 0, bufferInfo.width, bufferInfo.height)
        GLES20.glClearColor(0f, 0f, 0f, 0f)
        GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT)
        ledger.commit(params).forEach { drawSegment(width, height, transform, it) }
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

    private fun drawSegment(width: Int, height: Int, transform: FloatArray, segment: InkSegment) {
        if (width <= 0 || height <= 0 || segment.widthPx <= 0f) return
        fillQuad(segment, width.toFloat(), height.toFloat())
        vertices.position(0)

        GLES20.glUseProgram(program)
        GLES20.glUniformMatrix4fv(transformLocation, 1, false, transform, 0)
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

    private fun fillQuad(segment: InkSegment, width: Float, height: Float) {
        val dx = segment.x1 - segment.x0
        val dy = segment.y1 - segment.y0
        val lengthSquared = dx * dx + dy * dy
        val halfWidth = segment.widthPx / 2f
        vertices.position(0)
        if (lengthSquared == 0f) {
            InkProjection.putClip(vertices, segment.x0 - halfWidth, segment.y0 + halfWidth, width, height)
            InkProjection.putClip(vertices, segment.x0 + halfWidth, segment.y0 + halfWidth, width, height)
            InkProjection.putClip(vertices, segment.x0 - halfWidth, segment.y0 - halfWidth, width, height)
            InkProjection.putClip(vertices, segment.x0 + halfWidth, segment.y0 - halfWidth, width, height)
            return
        }

        val scale = halfWidth / sqrt(lengthSquared)
        val normalX = -dy * scale
        val normalY = dx * scale
        InkProjection.putClip(vertices, segment.x0 + normalX, segment.y0 + normalY, width, height)
        InkProjection.putClip(vertices, segment.x0 - normalX, segment.y0 - normalY, width, height)
        InkProjection.putClip(vertices, segment.x1 + normalX, segment.y1 + normalY, width, height)
        InkProjection.putClip(vertices, segment.x1 - normalX, segment.y1 - normalY, width, height)
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
