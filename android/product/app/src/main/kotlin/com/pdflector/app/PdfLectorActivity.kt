package com.pdflector.app

import android.graphics.Color
import android.graphics.PixelFormat
import android.os.Bundle
import android.util.Log
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.ViewGroup
import com.google.androidgamesdk.GameActivity
import com.pdflector.app.ink.InkGlRenderer
import com.pdflector.app.ink.InkLedger
import com.pdflector.app.ink.InkSegment
import androidx.graphics.lowlatency.GLFrontBufferedRenderer

/** GameActivity host for the native PDF surface and the transparent AndroidX ink surface. */
class PdfLectorActivity : GameActivity() {
    private val inkLock = Any()
    private lateinit var inkSurface: SurfaceView
    private var frontBufferedRenderer: GLFrontBufferedRenderer<InkSegment>? = null
    private val inkLedger = InkLedger()
    private var inkStrokeActive = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        inkSurface = SurfaceView(this).apply {
            setZOrderOnTop(true)
            holder.setFormat(PixelFormat.TRANSLUCENT)
            setBackgroundColor(Color.TRANSPARENT)
            isClickable = false
            isFocusable = false
            holder.addCallback(inkSurfaceCallback)
        }
        addContentView(
            inkSurface,
            ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            ),
        )
        request120Hz()
    }

    override fun onResume() {
        super.onResume()
        request120Hz()
    }

    override fun onPause() {
        cancelInk()
        super.onPause()
    }

    override fun onDestroy() {
        releaseInkRenderer()
        super.onDestroy()
    }

    /** Called from Rust/JNI with one causal segment in surface pixel coordinates. */
    fun renderInkSegment(
        x0: Float,
        y0: Float,
        x1: Float,
        y1: Float,
        widthPx: Float,
        colorArgb: Int,
    ): Boolean {
        synchronized(inkLock) {
            val renderer = frontBufferedRenderer ?: return false
            if (!renderer.isValid()) return false
            inkStrokeActive = true
            renderer.renderFrontBufferedLayer(
                InkSegment(x0, y0, x1, y1, widthPx, colorArgb, inkLedger.generation()),
            )
            return true
        }
    }

    /** Commit only the active gesture; AndroidX batches its submitted front segments once. */
    fun commitInk() {
        synchronized(inkLock) {
            val renderer = frontBufferedRenderer ?: return
            if (!renderer.isValid() || !inkStrokeActive) return
            renderer.commit()
            inkStrokeActive = false
        }
    }

    /** Cancel only the in-progress gesture; earlier committed ink remains until dry ack. */
    fun cancelInk() {
        synchronized(inkLock) {
            inkStrokeActive = false
            frontBufferedRenderer?.takeIf { it.isValid() }?.let {
                it.cancel()
            }
        }
    }

    /** JNI contract: Rust calls this after the committed annotation is visible in the dry layer. */
    fun clearInk() = clearInkIfIdle()

    /** Whether the transparent SurfaceView has a live AndroidX renderer. */
    fun isInkOverlayReady(): Boolean = synchronized(inkLock) {
        ::inkSurface.isInitialized && inkSurface.holder.surface?.isValid == true &&
            frontBufferedRenderer?.isValid() == true
    }

    private fun clearInkIfIdle() {
        synchronized(inkLock) {
            if (inkStrokeActive) return
            inkLedger.clear()
            frontBufferedRenderer?.takeIf { it.isValid() }?.clear()
        }
    }

    private fun releaseInkRenderer() {
        synchronized(inkLock) {
            inkStrokeActive = false
            inkLedger.clear()
            frontBufferedRenderer?.let {
                if (it.isValid()) {
                    it.cancel()
                    it.release(true)
                }
            }
            frontBufferedRenderer = null
        }
    }

    private fun request120Hz() {
        val attributes = window.attributes
        attributes.preferredRefreshRate = TARGET_REFRESH_RATE
        window.attributes = attributes
        if (::inkSurface.isInitialized && inkSurface.holder.surface?.isValid == true &&
            android.os.Build.VERSION.SDK_INT >= 30
        ) {
            inkSurface.holder.surface.setFrameRate(
                TARGET_REFRESH_RATE,
                Surface.FRAME_RATE_COMPATIBILITY_FIXED_SOURCE,
                Surface.CHANGE_FRAME_RATE_ONLY_IF_SEAMLESS,
            )
        }
    }

    private val inkSurfaceCallback = object : SurfaceHolder.Callback {
        override fun surfaceCreated(holder: SurfaceHolder) {
            synchronized(inkLock) {
                releaseInkRenderer()
                try {
                    frontBufferedRenderer = GLFrontBufferedRenderer(inkSurface, InkGlRenderer(inkLedger))
                } catch (error: RuntimeException) {
                    frontBufferedRenderer = null
                    Log.e(TAG, "Unable to create AndroidX ink renderer", error)
                }
            }
            request120Hz()
        }

        override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
            request120Hz()
        }

        override fun surfaceDestroyed(holder: SurfaceHolder) {
            releaseInkRenderer()
        }
    }

    private companion object {
        const val TAG = "PDFLectorInk"
        const val TARGET_REFRESH_RATE = 120f
    }
}
