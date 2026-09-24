package com.pdflector.inkbench

import java.util.concurrent.atomic.AtomicLong

class BenchMetrics {
    private val inputEvents = AtomicLong()
    private val callbackCount = AtomicLong()
    private val commitCount = AtomicLong()
    private val droppedInput = AtomicLong()
    private val diagnosticEvictions = AtomicLong()
    private val lastInputNanos = AtomicLong()
    private val lastCallbackNanos = AtomicLong()
    private val lastCommitNanos = AtomicLong()
    private val lastUnsubmittedInputNanos = AtomicLong()
    private val lastDiagnosticEvictionNanos = AtomicLong()
    private val effectiveRefreshMilliHz = AtomicLong()

    fun input(timestampNanos: Long) {
        inputEvents.incrementAndGet()
        lastInputNanos.set(timestampNanos)
    }

    fun callback(timestampNanos: Long) {
        callbackCount.incrementAndGet()
        lastCallbackNanos.set(timestampNanos)
    }

    fun commit(timestampNanos: Long) {
        commitCount.incrementAndGet()
        lastCommitNanos.set(timestampNanos)
    }

    fun droppedInput(timestampNanos: Long = System.nanoTime()) {
        droppedInput.incrementAndGet()
        lastUnsubmittedInputNanos.set(timestampNanos)
    }

    fun diagnosticEvictions(count: Long, timestampNanos: Long = System.nanoTime()) {
        if (count > 0) {
            diagnosticEvictions.addAndGet(count)
            lastDiagnosticEvictionNanos.set(timestampNanos)
        }
    }

    fun setEffectiveRefreshRate(refreshRateHz: Float) {
        effectiveRefreshMilliHz.set((refreshRateHz * 1000f).toLong())
    }

    fun snapshot(): Snapshot = Snapshot(
        inputEvents = inputEvents.get(),
        callbackCount = callbackCount.get(),
        commitCount = commitCount.get(),
        droppedInput = droppedInput.get(),
        diagnosticEvictions = diagnosticEvictions.get(),
        lastInputNanos = lastInputNanos.get(),
        lastCallbackNanos = lastCallbackNanos.get(),
        lastCommitNanos = lastCommitNanos.get(),
        lastUnsubmittedInputNanos = lastUnsubmittedInputNanos.get(),
        lastDiagnosticEvictionNanos = lastDiagnosticEvictionNanos.get(),
        effectiveRefreshRateHz = effectiveRefreshMilliHz.get() / 1000f,
    )

    data class Snapshot(
        val inputEvents: Long,
        val callbackCount: Long,
        val commitCount: Long,
        val droppedInput: Long,
        val diagnosticEvictions: Long,
        val lastInputNanos: Long,
        val lastCallbackNanos: Long,
        val lastCommitNanos: Long,
        val lastUnsubmittedInputNanos: Long,
        val lastDiagnosticEvictionNanos: Long,
        val effectiveRefreshRateHz: Float,
    )
}
