package com.pdflector.inkbench

import android.opengl.GLES20
import androidx.graphics.lowlatency.BufferInfo
import androidx.graphics.lowlatency.GLFrontBufferedRenderer
import androidx.graphics.opengl.egl.EGLManager
import java.nio.FloatBuffer
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Minimal causal-segment renderer for the wet front buffer and multi buffer. */
class InkGlRenderer(
    private val metrics: BenchMetrics,
) : GLFrontBufferedRenderer.Callback<StrokeSegment> {
    private var program = 0
    private var positionLocation = -1
    private var screenSizeLocation = -1
    private val segmentVertices: FloatBuffer = ByteBuffer
        .allocateDirect(8 * Float.SIZE_BYTES)
        .order(ByteOrder.nativeOrder())
        .asFloatBuffer()
    private val glStatus = IntArray(1)

    override fun onDrawFrontBufferedLayer(
        eglManager: EGLManager,
        width: Int,
        height: Int,
        bufferInfo: BufferInfo,
        transform: FloatArray,
        param: StrokeSegment,
    ) {
        prepareGl()
        drawSegment(width, height, param, clear = false)
        metrics.callback(System.nanoTime())
    }

    override fun onDrawMultiBufferedLayer(
        eglManager: EGLManager,
        width: Int,
        height: Int,
        bufferInfo: BufferInfo,
        transform: FloatArray,
        params: Collection<StrokeSegment>,
    ) {
        prepareGl()
        GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT)
        // AndroidX supplies only parameters queued since the preceding commit.
        // This spike redraws that committed batch, not an accumulated document.
        params.forEach { segment ->
            drawSegment(width, height, segment, clear = false)
        }
        metrics.callback(System.nanoTime())
    }

    private fun prepareGl() {
        if (program != 0) return
        program = linkProgram(VERTEX_SHADER, FRAGMENT_SHADER)
        positionLocation = GLES20.glGetAttribLocation(program, "aPosition")
        screenSizeLocation = GLES20.glGetUniformLocation(program, "uScreenSize")
        GLES20.glEnable(GLES20.GL_BLEND)
        GLES20.glBlendFunc(GLES20.GL_SRC_ALPHA, GLES20.GL_ONE_MINUS_SRC_ALPHA)
        GLES20.glClearColor(0f, 0f, 0f, 0f)
    }

    private fun drawSegment(
        width: Int,
        height: Int,
        segment: StrokeSegment,
        clear: Boolean,
    ) {
        if (clear) GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT)
        if (width <= 0 || height <= 0) return
        segmentVertices.position(0)
        StrokeGeometry.fillQuad(segmentVertices, segment, HALF_WIDTH_PX)
        segmentVertices.position(0)

        GLES20.glUseProgram(program)
        GLES20.glUniform2f(screenSizeLocation, width.toFloat(), height.toFloat())
        GLES20.glEnableVertexAttribArray(positionLocation)
        GLES20.glVertexAttribPointer(
            positionLocation,
            2,
            GLES20.GL_FLOAT,
            false,
            2 * Float.SIZE_BYTES,
            segmentVertices,
        )
        GLES20.glDrawArrays(GLES20.GL_TRIANGLE_STRIP, 0, 4)
        GLES20.glDisableVertexAttribArray(positionLocation)
    }

    private fun linkProgram(vertexSource: String, fragmentSource: String): Int {
        val vertex = compileShader(GLES20.GL_VERTEX_SHADER, vertexSource)
        val fragment = compileShader(GLES20.GL_FRAGMENT_SHADER, fragmentSource)
        val linked = GLES20.glCreateProgram()
        check(linked != 0) { "Unable to create GL program" }
        GLES20.glAttachShader(linked, vertex)
        GLES20.glAttachShader(linked, fragment)
        GLES20.glLinkProgram(linked)
        GLES20.glGetProgramiv(linked, GLES20.GL_LINK_STATUS, glStatus, 0)
        check(glStatus[0] == GLES20.GL_TRUE) {
            "Unable to link GL program: ${GLES20.glGetProgramInfoLog(linked)}"
        }
        GLES20.glDeleteShader(vertex)
        GLES20.glDeleteShader(fragment)
        return linked
    }

    private fun compileShader(type: Int, source: String): Int {
        val shader = GLES20.glCreateShader(type)
        check(shader != 0) { "Unable to create GL shader" }
        GLES20.glShaderSource(shader, source)
        GLES20.glCompileShader(shader)
        GLES20.glGetShaderiv(shader, GLES20.GL_COMPILE_STATUS, glStatus, 0)
        check(glStatus[0] == GLES20.GL_TRUE) {
            "Unable to compile GL shader: ${GLES20.glGetShaderInfoLog(shader)}"
        }
        return shader
    }

    private companion object {
        const val VERTEX_SHADER = """
            uniform vec2 uScreenSize;
            attribute vec2 aPosition;
            void main() {
                vec2 normalizedPosition = aPosition / uScreenSize * 2.0 - 1.0;
                gl_Position = vec4(normalizedPosition, 0.0, 1.0);
            }
        """

        const val FRAGMENT_SHADER = """
            precision mediump float;
            void main() {
                gl_FragColor = vec4(1.0, 0.70, 0.08, 0.95);
            }
        """

        const val HALF_WIDTH_PX = 5f
    }
}
