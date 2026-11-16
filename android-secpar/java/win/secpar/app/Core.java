package win.secpar.app;

import org.json.JSONException;
import org.json.JSONObject;

/** Secpar's own part of the app (crates/secpar-android): identity, account, computers and the tunnel. */
public final class Core {
    static {
        System.loadLibrary("secpar_android");
    }

    private Core() {
    }

    private static native String call(String request);

    /** One request, one answer. Blocks, so never call it on the main thread. */
    public static JSONObject ask(JSONObject request) {
        try {
            String answer = call(request.toString());
            return answer == null ? new JSONObject() : new JSONObject(answer);
        } catch (JSONException e) {
            return new JSONObject();
        }
    }

    public static JSONObject request(String what) {
        try {
            return new JSONObject().put("do", what);
        } catch (JSONException e) {
            throw new IllegalStateException(e);
        }
    }
}
