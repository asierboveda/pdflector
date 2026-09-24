package com.pdflector.inkbench

data class StrokeSample(
    val x: Float,
    val y: Float,
    val pressure: Float,
    val timeNanos: Long,
)

data class StrokeSegment(
    val from: StrokeSample,
    val to: StrokeSample,
)

class StrokeResult internal constructor(
    val segmentCount: Long,
    val lastTimestampNanos: Long,
    val diagnosticEvictions: Long,
)

data class StrokeCommit(
    val frontSegments: Int,
    val multiReplaySegments: Int,
)

/** Tracks the front segments already submitted; Up only commits them once. */
class StrokeCommitTracker {
    private var pendingFrontSegments = 0

    fun frontSegmentSubmitted() {
        pendingFrontSegments += 1
    }

    fun upCommit(): StrokeCommit {
        val commit = StrokeCommit(
            frontSegments = pendingFrontSegments,
            multiReplaySegments = 0,
        )
        pendingFrontSegments = 0
        return commit
    }

    fun cancel() {
        pendingFrontSegments = 0
    }
}

/**
 * Small bounded state machine for one active stylus stroke.
 *
 * Each accepted real sample immediately produces one causal segment. The
 * bounded ring is diagnostic state only; the segment object passed to AndroidX
 * is never mutated or reused when the ring evicts its oldest reference.
 */
class StrokeSession(capacity: Int) {
    private val ring: Array<StrokeSegment?>
    private var head = 0
    private var retainedCount = 0
    private var submittedSegments = 0L
    private var lastSample: StrokeSample? = null
    private var activeGeneration: Long? = null
    private var evictionCount = 0L
    private var evictionsAtBegin = 0L

    var generation: Long = 0L
        private set

    val hasActiveStroke: Boolean
        get() = activeGeneration != null

    val diagnosticEvictions: Long
        get() = evictionCount

    init {
        require(capacity > 0) { "capacity must be positive" }
        ring = arrayOfNulls(capacity)
    }

    fun begin(_startTimestampNanos: Long): Long {
        generation += 1
        activeGeneration = generation
        evictionsAtBegin = evictionCount
        submittedSegments = 0L
        lastSample = null
        clearRing()
        return generation
    }

    fun append(token: Long, sample: StrokeSample): StrokeSegment? {
        if (activeGeneration != token) return null
        val previous = lastSample
        if (previous != null && sample.timeNanos <= previous.timeNanos) return null
        val start = previous ?: sample
        val segment = StrokeSegment(start, sample)
        lastSample = sample
        submittedSegments += 1
        retain(segment)
        return segment
    }

    fun end(token: Long, endTimestampNanos: Long): StrokeResult {
        if (activeGeneration != token) {
            return StrokeResult(0L, endTimestampNanos, 0L)
        }
        val result = StrokeResult(
            segmentCount = submittedSegments,
            lastTimestampNanos = endTimestampNanos,
            diagnosticEvictions = evictionCount - evictionsAtBegin,
        )
        activeGeneration = null
        lastSample = null
        submittedSegments = 0L
        clearRing()
        return result
    }

    fun cancel() {
        activeGeneration = null
        lastSample = null
        submittedSegments = 0L
        clearRing()
    }

    private fun retain(segment: StrokeSegment) {
        if (retainedCount == ring.size) {
            ring[head] = null
            head = (head + 1) % ring.size
            retainedCount -= 1
            evictionCount += 1
        }
        ring[(head + retainedCount) % ring.size] = segment
        retainedCount += 1
    }

    private fun clearRing() {
        ring.fill(null)
        head = 0
        retainedCount = 0
    }
}
