package com.pdflector.inkbench

import android.app.Activity
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.PixelFormat
import android.os.Bundle
import android.view.Display
import android.view.MotionEvent
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.View
import android.widget.FrameLayout
import androidx.graphics.lowlatency.GLFrontBufferedRenderer

class MainActivity : Activity() {
    private val metrics = BenchMetrics()
    private val session = StrokeSession(capacity = 512)
    private val commitTracker = StrokeCommitTracker()
    private lateinit var dryView: DryControlView
    private lateinit var wetSurface: SurfaceView
    private var frontRenderer: GLFrontBufferedRenderer<StrokeSegment>? = null
    private var activeToken: Long? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        requestWindowRefreshRate()

        dryView = DryControlView()
        wetSurface = SurfaceView(this).apply {
            setZOrderOnTop(true)
            holder.setFormat(PixelFormat.TRANSLUCENT)
            holder.addCallback(surfaceCallback)
            setBackgroundColor(Color.TRANSPARENT)
            isFocusable = true
        }
        setContentView(FrameLayout(this).apply {
            addView(dryView, FrameLayout.LayoutParams(-1, -1))
            addView(wetSurface, FrameLayout.LayoutParams(-1, -1))
        })
        recordEffectiveRefreshRate()
    }

    override fun onResume() {
        super.onResume()
        requestWindowRefreshRate()
        recordEffectiveRefreshRate()
    }

    override fun onPause() {
        cancelStroke()
        super.onPause()
    }

    override fun onDestroy() {
        cancelStroke()
        releaseRenderer()
        super.onDestroy()
    }

    private fun requestWindowRefreshRate() {
        val attributes = window.attributes
        attributes.preferredRefreshRate = TARGET_REFRESH_RATE
        window.attributes = attributes
        if (
            ::wetSurface.isInitialized &&
            android.os.Build.VERSION.SDK_INT >= 30 &&
            wetSurface.holder.surface.isValid
        ) {
            wetSurface.holder.surface.setFrameRate(
                TARGET_REFRESH_RATE,
                Surface.FRAME_RATE_COMPATIBILITY_FIXED_SOURCE,
                Surface.CHANGE_FRAME_RATE_ONLY_IF_SEAMLESS,
            )
        }
    }

    private fun recordEffectiveRefreshRate() {
        val display: Display? = if (android.os.Build.VERSION.SDK_INT >= 30) {
            display
        } else {
            @Suppress("DEPRECATION")
            windowManager.defaultDisplay
        }
        metrics.setEffectiveRefreshRate(display?.refreshRate ?: 0f)
        dryView.postInvalidateOnAnimation()
    }

    private val surfaceCallback = object : SurfaceHolder.Callback {
        override fun surfaceCreated(holder: SurfaceHolder) {
            releaseRenderer()
            frontRenderer = GLFrontBufferedRenderer(wetSurface, InkGlRenderer(metrics))
            requestWindowRefreshRate()
        }

        override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
            recordEffectiveRefreshRate()
        }

        override fun surfaceDestroyed(holder: SurfaceHolder) {
            cancelStroke()
            releaseRenderer()
        }
    }

    override fun dispatchTouchEvent(event: MotionEvent): Boolean {
        if (event.actionMasked == MotionEvent.ACTION_CANCEL) {
            metrics.input(System.nanoTime())
            cancelStroke()
            return true
        }
        val pointerIndex = event.actionIndex.coerceAtLeast(0)
        if (pointerIndex >= event.pointerCount || event.getToolType(pointerIndex) != MotionEvent.TOOL_TYPE_STYLUS) {
            return super.dispatchTouchEvent(event)
        }
        val timestampNanos = eventTimestampNanos(event)
        metrics.input(timestampNanos)
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                wetSurface.requestUnbufferedDispatch(event)
                beginStroke(event, pointerIndex)
            }
            MotionEvent.ACTION_MOVE -> appendMoveSamples(event, pointerIndex)
            MotionEvent.ACTION_UP -> endStroke(event, pointerIndex)
            MotionEvent.ACTION_CANCEL -> cancelStroke()
        }
        dryView.postInvalidateOnAnimation()
        return true
    }

    private fun beginStroke(event: MotionEvent, pointerIndex: Int) {
        cancelStroke()
        val token = session.begin(eventTimestampNanos(event))
        activeToken = token
        appendSample(token, sampleAt(event, pointerIndex))
    }

    private fun appendMoveSamples(event: MotionEvent, pointerIndex: Int) {
        val token = activeToken ?: return
        // Historical samples are supplied by the real MotionEvent stream; no
        // prediction or extrapolation is performed here.
        for (historyIndex in 0 until event.historySize) {
            appendSample(
                token,
                sampleAt(event, pointerIndex, historyIndex),
            )
        }
        appendSample(token, sampleAt(event, pointerIndex))
    }

    private fun endStroke(event: MotionEvent, pointerIndex: Int) {
        val token = activeToken ?: return
        appendSample(token, sampleAt(event, pointerIndex))
        val result = session.end(token, eventTimestampNanos(event))
        metrics.diagnosticEvictions(result.diagnosticEvictions, System.nanoTime())
        commitTracker.upCommit()
        frontRenderer?.let { renderer ->
            renderer.commit()
            metrics.commit(System.nanoTime())
        } ?: metrics.droppedInput()
        activeToken = null
    }

    private fun appendSample(token: Long, sample: StrokeSample) {
        val segment = session.append(token, sample)
        if (segment != null) {
            frontRenderer?.let { renderer ->
                renderer.renderFrontBufferedLayer(segment)
                commitTracker.frontSegmentSubmitted()
            } ?: metrics.droppedInput()
        } else {
            metrics.droppedInput()
        }
    }

    private fun sampleAt(
        event: MotionEvent,
        pointerIndex: Int,
        historyIndex: Int? = null,
    ): StrokeSample {
        val x = if (historyIndex == null) event.getX(pointerIndex) else event.getHistoricalX(pointerIndex, historyIndex)
        val y = if (historyIndex == null) event.getY(pointerIndex) else event.getHistoricalY(pointerIndex, historyIndex)
        val pressure = if (historyIndex == null) event.getPressure(pointerIndex) else event.getHistoricalPressure(pointerIndex, historyIndex)
        return StrokeSample(x, y, pressure, eventTimestampNanos(event, historyIndex))
    }

    private fun eventTimestampNanos(event: MotionEvent, historyIndex: Int? = null): Long {
        if (android.os.Build.VERSION.SDK_INT >= 34) {
            return if (historyIndex == null) {
                event.eventTimeNanos
            } else {
                event.getHistoricalEventTimeNanos(historyIndex)
            }
        }
        val timeMillis = if (historyIndex == null) {
            event.eventTime
        } else {
            event.getHistoricalEventTime(historyIndex)
        }
        return timeMillis * NANOS_PER_MILLISECOND
    }

    private fun cancelStroke() {
        session.cancel()
        commitTracker.cancel()
        activeToken = null
        frontRenderer?.let { renderer ->
            renderer.cancel()
            renderer.clear()
        }
    }

    private fun releaseRenderer() {
        frontRenderer?.let { renderer ->
            renderer.cancel()
            renderer.clear()
            renderer.release(true)
        }
        frontRenderer = null
    }

    private inner class DryControlView : View(this@MainActivity) {
        private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            typeface = android.graphics.Typeface.MONOSPACE
            textSize = 26f
        }

        override fun onDraw(canvas: Canvas) {
            canvas.drawColor(Color.rgb(18, 22, 26))
            paint.color = Color.rgb(140, 155, 165)
            canvas.drawText("INK BENCH · dry control", 32f, 48f, paint)
            val snapshot = metrics.snapshot()
            paint.color = Color.rgb(105, 120, 130)
            canvas.drawText(
                "refresh=${snapshot.effectiveRefreshRateHz}Hz input=${snapshot.inputEvents} " +
                    "callbacks=${snapshot.callbackCount} commits=${snapshot.commitCount} " +
                    "unsubmittedInput=${snapshot.droppedInput} " +
                    "diagnosticEvictions=${snapshot.diagnosticEvictions}",
                32f,
                84f,
                paint,
            )
        }
    }

    private companion object {
        const val TARGET_REFRESH_RATE = 120f
        const val NANOS_PER_MILLISECOND = 1_000_000L
    }
}
