package org.tstrans.rtp;

/**
 * Close-time {@code (reason, detail)} pair {@code nClose} returns on {@link
 * Receiver}, {@link DemuxReceiver}, and {@link H264Receiver}. Package-private
 * — never part of the public API surface, just the carrier `nativeClose`
 * unpacks into each class's cached {@code closedEndReason}/{@code
 * closedEndDetail} fields.
 *
 * <p>Exists because, by the time a subclass's {@code nativeClose(long h)}
 * runs, {@link org.tstrans.NativeHandle}'s own {@code close()} has already
 * zeroed the handle field — there is no live handle left to pass to a
 * follow-up {@code nEndReason}/{@code nEndDetail} call, and the leased native
 * registry entry those natives would query is gone (permanently — ids are
 * never reused). So the {@code nClose} native computes both pieces itself,
 * from the resource it already exclusively owns, and returns them together
 * in this one object. See each class's {@code endReason()} javadoc for the
 * resulting read contract.
 */
final class EndReasonSnapshot {
    final StreamEndReason reason;
    final String detail;

    EndReasonSnapshot(StreamEndReason reason, String detail) {
        this.reason = reason;
        this.detail = detail;
    }
}
