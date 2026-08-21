package org.tstrans.srt;

/**
 * Frozen reconnect/gap telemetry snapshot for a managed (auto-reconnect) SRT
 * sender. Mirror of {@code tst_pipeline::ManagedTransportStats} (and the C
 * ABI's {@code TstManagedTransportStats}) — same field order. Returned by
 * {@link ManagedSender#reconnectStats()} / {@link ManagedMuxSender#reconnectStats()}.
 *
 * <p>{@code reconnecting} is only ever {@code true} under
 * {@link ReconnectMode#BACKGROUND} (always {@code false} in {@code BLOCKING}
 * mode, since that mode's reconnect loop runs synchronously inside the call
 * that observed the break rather than on a separate worker this flag could
 * observe as active).
 *
 * @param reconnectAttempts   total {@code factory()} invocations across all
 *     reconnect cycles (successful + failed)
 * @param reconnectSuccesses  factory calls that returned a transport
 *     successfully installed as the new inner connection
 * @param gapLen              messages currently queued in the gap buffer,
 *     awaiting drain once the inner transport reconnects
 * @param gapMessagesDropped  messages evicted by {@code DropOldest} (plus
 *     oversized-after-reconnect drops discovered during drain)
 * @param gapBytesDropped     bytes evicted by the same
 * @param reconnecting        {@code true} while a background reconnect
 *     worker is active
 */
public record ManagedTransportStats(long reconnectAttempts, long reconnectSuccesses,
        long gapLen, long gapMessagesDropped, long gapBytesDropped, boolean reconnecting) {}
