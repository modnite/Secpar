package win.secpar.app;

import android.app.Activity;
import android.content.Intent;
import android.graphics.Color;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.text.InputType;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import com.limelight.Game;
import com.limelight.binding.crypto.AndroidCryptoProvider;
import com.limelight.nvstream.http.ComputerDetails;
import com.limelight.nvstream.http.NvApp;
import com.limelight.nvstream.http.NvHTTP;
import com.limelight.nvstream.http.PairingManager;
import com.limelight.preferences.StreamSettings;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.ByteArrayInputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.security.MessageDigest;
import java.security.cert.CertificateFactory;
import java.security.cert.X509Certificate;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * Secpar's screen: sign in, then the account's computers. Tapping a computer
 * opens the tunnel to it, pairs with it the first time without a PIN to type,
 * and starts the stream.
 */
public class SecparActivity extends Activity {
    private static final int BACKGROUND = Color.rgb(0x15, 0x16, 0x1A);
    private static final int SURFACE = Color.rgb(0x1C, 0x1E, 0x24);
    private static final int BORDER = Color.rgb(0x2F, 0x33, 0x3C);
    private static final int TEXT = Color.rgb(0xF9, 0xFA, 0xFC);
    private static final int DIM = Color.rgb(0x9B, 0xA1, 0xAD);
    private static final int MINT = Color.rgb(0x0A, 0xFF, 0xB6);
    private static final int ORANGE = Color.rgb(0xD9, 0x54, 0x00);

    private final ExecutorService work = Executors.newSingleThreadExecutor();
    // Pairing has two halves that run at the same time.
    private final ExecutorService second = Executors.newSingleThreadExecutor();
    private final Handler main = new Handler(Looper.getMainLooper());
    private LinearLayout page;
    private TextView notice;
    private boolean busy;

    @Override
    protected void onCreate(Bundle saved) {
        super.onCreate(saved);
        ScrollView scroll = new ScrollView(this);
        scroll.setBackgroundColor(BACKGROUND);
        scroll.setFillViewport(true);
        page = new LinearLayout(this);
        page.setOrientation(LinearLayout.VERTICAL);
        int edge = dp(16);
        page.setPadding(edge, edge, edge, edge);
        scroll.addView(page, new ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        setContentView(scroll);
        getWindow().setStatusBarColor(BACKGROUND);
        getWindow().setNavigationBarColor(BACKGROUND);
    }

    @Override
    protected void onResume() {
        super.onResume();
        // Back from a stream, or first shown: nothing stays connected behind this screen.
        run(() -> {
            JSONObject configure = Core.request("configure").put("folder", getFilesDir().getAbsolutePath());
            Core.ask(configure);
            Core.ask(Core.request("disconnect"));
            return Core.ask(Core.request("state"));
        }, state -> {
            if (state.optBoolean("signed_in")) {
                loadComputers(state.optString("handle"));
            } else {
                showSignIn(null);
            }
        });
    }

    private interface Task {
        JSONObject run() throws Exception;
    }

    private interface Done {
        void with(JSONObject answer);
    }

    private void run(Task task, Done done) {
        busy = true;
        work.execute(() -> {
            JSONObject answer;
            try {
                answer = task.run();
            } catch (Exception e) {
                answer = new JSONObject();
            }
            JSONObject result = answer;
            main.post(() -> {
                busy = false;
                if (!isFinishing()) {
                    done.with(result);
                }
            });
        });
    }

    // ---- Sign in

    private void showSignIn(String problem) {
        page.removeAllViews();
        page.setGravity(Gravity.CENTER_VERTICAL);
        page.addView(title("Sign in"));
        EditText email = field("Email", InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS);
        EditText password = field("Password", InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD);
        page.addView(email);
        page.addView(password);
        Button go = button("Sign in", true);
        page.addView(go);
        notice = note(problem == null ? "" : problem, ORANGE);
        page.addView(notice);
        page.addView(note("This device is added to your account when you sign in.", DIM));
        go.setOnClickListener(v -> {
            String address = email.getText().toString().trim();
            String secret = password.getText().toString();
            if (busy || address.isEmpty() || secret.isEmpty()) {
                return;
            }
            password.setText("");
            go.setText("Signing in");
            run(() -> Core.ask(Core.request("sign_in").put("email", address).put("password", secret).put("name", Build.MODEL)), answer -> {
                if (answer.optBoolean("signed_in")) {
                    loadComputers(answer.optString("handle"));
                } else {
                    showSignIn(explain(answer));
                }
            });
        });
    }

    // ---- Computers

    private void loadComputers(String handle) {
        run(() -> Core.ask(Core.request("devices")), answer -> {
            if ("signed_out".equals(answer.optString("error"))) {
                showSignIn(null);
            } else {
                showComputers(handle, answer.optJSONArray("devices"), answer.optBoolean("ok") ? null : explain(answer));
            }
        });
    }

    private void showComputers(String handle, JSONArray devices, String problem) {
        page.removeAllViews();
        page.setGravity(Gravity.TOP);
        page.addView(title("Computers"));
        int shown = 0;
        for (int i = 0; devices != null && i < devices.length(); i++) {
            JSONObject device = devices.optJSONObject(i);
            if (device == null || !device.optBoolean("hosts")) {
                continue;
            }
            shown++;
            boolean online = device.optBoolean("online");
            LinearLayout row = new LinearLayout(this);
            row.setOrientation(LinearLayout.VERTICAL);
            row.setBackground(box(SURFACE, BORDER));
            row.setPadding(dp(14), dp(12), dp(14), dp(12));
            TextView name = text(device.optString("name"), TEXT, 16);
            name.setTypeface(Typeface.DEFAULT_BOLD);
            row.addView(name);
            row.addView(text(online ? "Online" : "Offline", online ? MINT : DIM, 13));
            row.setAlpha(online ? 1f : 0.6f);
            if (online) {
                row.setOnClickListener(v -> connect(handle, device));
            }
            page.addView(row, spaced());
        }
        if (shown == 0) {
            page.addView(note("No computers yet. Sign in on a computer to see it here.", DIM));
        }
        notice = note(problem == null ? "" : problem, ORANGE);
        page.addView(notice);
        Button refresh = button("Refresh", false);
        refresh.setOnClickListener(v -> {
            if (!busy) {
                loadComputers(handle);
            }
        });
        page.addView(refresh, spaced());
        Button settings = button("Stream settings", false);
        settings.setOnClickListener(v -> startActivity(new Intent(this, StreamSettings.class)));
        page.addView(settings, spaced());
        Button out = button("Sign out " + handle, false);
        out.setOnClickListener(v -> {
            if (!busy) {
                run(() -> Core.ask(Core.request("sign_out")), answer -> showSignIn(null));
            }
        });
        page.addView(out, spaced());
    }

    // ---- Connect, pair the first time, stream

    private void connect(String handle, JSONObject device) {
        if (busy) {
            return;
        }
        notice.setTextColor(DIM);
        notice.setText("Connecting to " + device.optString("name"));
        run(() -> {
            JSONObject tunnel = Core.ask(Core.request("connect").put("id", device.optString("id")));
            if (!tunnel.optBoolean("ok")) {
                return tunnel;
            }
            Intent stream = prepare(tunnel, device.optString("name"));
            if (stream == null) {
                Core.ask(Core.request("disconnect"));
                return new JSONObject().put("error", "pairing_failed");
            }
            main.post(() -> startActivity(stream));
            return new JSONObject().put("ok", true);
        }, answer -> notice.setText(answer.optBoolean("ok") ? "" : explainIn(ORANGE, answer)));
    }

    private String explainIn(int colour, JSONObject answer) {
        notice.setTextColor(colour);
        return explain(answer);
    }

    /** Runs off the main thread. Returns the stream to start, or null. */
    private Intent prepare(JSONObject tunnel, String name) throws Exception {
        String address = tunnel.getString("address");
        int port = tunnel.getInt("port");
        File certificateFile = new File(getFilesDir(), "secpar/host-" + tunnel.getString("host") + ".crt");
        for (int attempt = 0; attempt < 2; attempt++) {
            AndroidCryptoProvider crypto = new AndroidCryptoProvider(this);
            // Moonlight's connection code presents this one id to every engine,
            // whatever id it is given, so the computer must be told the same one.
            // What tells devices apart is the certificate.
            String uniqueId = "0123456789ABCDEF";
            ComputerDetails.AddressTuple where = new ComputerDetails.AddressTuple(address, port);
            X509Certificate server = readCertificate(certificateFile);
            NvHTTP http = new NvHTTP(where, 0, uniqueId, server, crypto);
            String info = http.getServerInfo(true);
            ComputerDetails details = http.getComputerDetails(info);
            if (server == null || http.getPairState(info) != PairingManager.PairState.PAIRED) {
                // The app starts the pairing; the computer completes it when asked
                // through the tunnel, where it already knows which device this is.
                String pin = PairingManager.generatePinString();
                String fingerprint = hex(MessageDigest.getInstance("SHA-256").digest(crypto.getClientCertificate().getEncoded()));
                java.util.concurrent.Future<JSONObject> asked = second.submit(() ->
                        Core.ask(Core.request("pair").put("client_id", uniqueId).put("certificate_sha256", fingerprint).put("pin", pin)));
                http = new NvHTTP(where, details.httpsPort, uniqueId, null, crypto);
                PairingManager pairing = http.getPairingManager();
                PairingManager.PairState state = pairing.pair(http.getServerInfo(true), pin);
                String answer = asked.get().optString("answer");
                if ("revoked".equals(answer) && attempt == 0) {
                    // This computer took the app's stream identity away earlier. Make a new one.
                    new File(getFilesDir(), "client.crt").delete();
                    new File(getFilesDir(), "client.key").delete();
                    continue;
                }
                if (state != PairingManager.PairState.PAIRED || pairing.getPairedCert() == null) {
                    return null;
                }
                server = pairing.getPairedCert();
                certificateFile.getParentFile().mkdirs();
                try (FileOutputStream out = new FileOutputStream(certificateFile)) {
                    out.write(server.getEncoded());
                }
                http = new NvHTTP(where, details.httpsPort, uniqueId, server, crypto);
            }
            NvApp desktop = http.getAppByName("Desktop");
            if (desktop == null) {
                return null;
            }
            Intent intent = new Intent(this, Game.class);
            intent.putExtra(Game.EXTRA_HOST, address);
            intent.putExtra(Game.EXTRA_PORT, port);
            intent.putExtra(Game.EXTRA_HTTPS_PORT, details.httpsPort);
            intent.putExtra(Game.EXTRA_APP_NAME, desktop.getAppName());
            intent.putExtra(Game.EXTRA_APP_ID, desktop.getAppId());
            intent.putExtra(Game.EXTRA_APP_HDR, desktop.isHdrSupported());
            intent.putExtra(Game.EXTRA_UNIQUEID, uniqueId);
            intent.putExtra(Game.EXTRA_PC_UUID, details.uuid);
            intent.putExtra(Game.EXTRA_PC_NAME, name);
            intent.putExtra(Game.EXTRA_SERVER_CERT, server.getEncoded());
            return intent;
        }
        return null;
    }

    private static X509Certificate readCertificate(File file) {
        if (!file.isFile()) {
            return null;
        }
        try (FileInputStream in = new FileInputStream(file)) {
            byte[] bytes = new byte[(int) file.length()];
            int read = in.read(bytes);
            return (X509Certificate) CertificateFactory.getInstance("X.509").generateCertificate(new ByteArrayInputStream(bytes, 0, Math.max(read, 0)));
        } catch (Exception e) {
            return null;
        }
    }

    private static String hex(byte[] bytes) {
        StringBuilder out = new StringBuilder();
        for (byte b : bytes) {
            out.append(String.format("%02x", b));
        }
        return out.toString();
    }

    private static String explain(JSONObject answer) {
        String message = answer.optString("message");
        if (!message.isEmpty() && !"null".equals(message)) {
            return message;
        }
        switch (answer.optString("error")) {
            case "unreachable":
                return "The account service did not answer. Check the internet connection and try again.";
            case "offline":
                return "That computer is off or not connected to the internet.";
            case "not_reachable":
                return "That computer did not answer. Check that this device is on the same network.";
            case "pairing_failed":
                return "That computer did not accept this device. Try again.";
            case "":
                return "";
            default:
                return "That did not work (" + answer.optString("error") + "). Try again.";
        }
    }

    // ---- Plain views in Secpar's colours

    private int dp(int value) {
        return Math.round(value * getResources().getDisplayMetrics().density);
    }

    private LinearLayout.LayoutParams spaced() {
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        params.topMargin = dp(10);
        return params;
    }

    private GradientDrawable box(int fill, int stroke) {
        GradientDrawable shape = new GradientDrawable();
        shape.setColor(fill);
        shape.setCornerRadius(dp(6));
        shape.setStroke(dp(1), stroke);
        return shape;
    }

    private TextView text(String value, int colour, int size) {
        TextView view = new TextView(this);
        view.setText(value);
        view.setTextColor(colour);
        view.setTextSize(size);
        return view;
    }

    private TextView title(String value) {
        TextView view = text(value, TEXT, 20);
        view.setTypeface(Typeface.DEFAULT_BOLD);
        view.setPadding(0, 0, 0, dp(6));
        return view;
    }

    private TextView note(String value, int colour) {
        TextView view = text(value, colour, 13);
        view.setPadding(0, dp(10), 0, 0);
        return view;
    }

    private EditText field(String hint, int type) {
        EditText view = new EditText(this);
        view.setHint(hint);
        view.setInputType(type);
        view.setTextColor(TEXT);
        view.setHintTextColor(DIM);
        view.setTextSize(15);
        view.setSingleLine(true);
        view.setBackground(box(SURFACE, BORDER));
        view.setPadding(dp(12), dp(12), dp(12), dp(12));
        view.setLayoutParams(spaced());
        return view;
    }

    private Button button(String label, boolean primary) {
        Button view = new Button(this);
        view.setText(label);
        view.setAllCaps(false);
        view.setTextSize(15);
        view.setTextColor(primary ? BACKGROUND : TEXT);
        view.setBackground(box(primary ? MINT : SURFACE, primary ? MINT : BORDER));
        view.setStateListAnimator(null);
        view.setLayoutParams(spaced());
        return view;
    }

    @Override
    protected void onDestroy() {
        work.shutdown();
        second.shutdown();
        super.onDestroy();
    }
}
