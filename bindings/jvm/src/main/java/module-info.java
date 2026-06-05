/**
 * tstrans JVM bindings — MPEG-TS + KLV + codec parsing and SRT/RTP transport.
 *
 * <p>Package layout mirrors the Python binding ({@code tstrans.*}); see the
 * tst-jni design spec §5.1.
 */
module org.tstrans {
    requires java.base;
    exports org.tstrans;
    exports org.tstrans.codec;
    exports org.tstrans.io;
    exports org.tstrans.klv;
    exports org.tstrans.mpegts;
}
