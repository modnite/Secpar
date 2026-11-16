package com.pcremote.engine;

import org.json.JSONObject;

/**
 * The managed engine's tunnel, from crates/secpar-android (scripts/build-android-core.sh).
 * It carries the engine's ports over one pinned QUIC connection and listens on the
 * phone's loopback, so Moonlight talks to 127.0.0.1:46989 as if the engine were local.
 *
 * <p>Every call blocks and returns JSON, so the app makes them off its main thread.
 */
public final class NativeTunnel {
    private static final boolean LOADED;

    static {
        boolean loaded;
        try {
            System.loadLibrary("secpar_android");
            loaded = true;
        } catch (UnsatisfiedLinkError e) {
            loaded = false;
        }
        LOADED = loaded;
    }

    private NativeTunnel() {}

    /** Whether this APK carries the tunnel library. */
    public static boolean isAvailable() {
        return LOADED;
    }

    /** A tunnel socket and the certificate pin the computer allow-lists. */
    public static final class Ready {
        public final String certificate;
        public final int port;
        public final String seen;

        Ready(String certificate, int port, String seen) {
            this.certificate = certificate;
            this.port = port;
            this.seen = seen;
        }
    }

    /**
     * Opens a tunnel socket. Its UDP port and [Ready.seen] address are what the computer
     * is asked to punch towards. Returns null when the library or a socket is unavailable.
     */
    public static Ready begin() {
        if (!LOADED) return null;
        try {
            JSONObject answer = new JSONObject(start());
            if (!answer.optBoolean("ok")) return null;
            String certificate = answer.optString("certificate_sha256");
            int port = answer.optInt("port");
            if (certificate.isEmpty() || port <= 0) return null;
            String seen = answer.optString("seen");
            return new Ready(certificate, port, seen.isEmpty() || "null".equals(seen) ? null : seen);
        } catch (Exception e) {
            return null;
        }
    }

    /**
     * Connects to the computer's tunnel and offers the engine's ports on the loopback.
     * Returns null on success, otherwise a sentence for the stream screen.
     */
    public static String open(String addressesJson, int port, String hostCertificate) {
        if (!LOADED) return "This phone has no stream tunnel.";
        try {
            JSONObject answer = new JSONObject(connect(addressesJson, port, hostCertificate));
            if (answer.optBoolean("ok")) return null;
            String said = answer.optString("msg");
            return said.isEmpty() || "null".equals(said) ? answer.optString("error", "internal_error") : said;
        } catch (Exception e) {
            return "The stream tunnel did not start.";
        }
    }

    private static native String start();

    private static native String connect(String addresses, int port, String hostCertificate);

    /** Drops the tunnel and its local ports. Safe to call when no tunnel is open. */
    public static native void close();
}
