/**
 * tstrans JVM bindings — MPEG-TS + KLV + codec parsing and SRT/RTP transport.
 *
 * <p>Package layout mirrors the Python binding ({@code tstrans.*}); see the
 * tst-jni design spec §5.1.
 */
module org.tstrans {
    requires java.base;
    exports org.tstrans;
    // exports org.tstrans.mpegts; -- deferred: added in Task 1.1 once the package exists
}
