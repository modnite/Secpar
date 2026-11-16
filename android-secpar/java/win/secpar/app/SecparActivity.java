package win.secpar.app;

import android.app.Activity;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Typeface;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.text.Editable;
import android.text.InputType;
import android.text.TextWatcher;
import android.util.Base64;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.inputmethod.InputMethodManager;
import android.widget.EditText;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import com.limelight.Game;
import com.limelight.R;
import com.limelight.binding.crypto.AndroidCryptoProvider;
import com.limelight.nvstream.http.ComputerDetails;
import com.limelight.nvstream.http.NvApp;
import com.limelight.nvstream.http.NvHTTP;
import com.limelight.nvstream.http.PairingManager;
import com.limelight.preferences.PreferenceConfiguration;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.io.ByteArrayInputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.security.MessageDigest;
import java.security.cert.CertificateFactory;
import java.security.cert.X509Certificate;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * Secpar's three tabs: Computers, Friends and Settings. Tapping a computer
 * opens the tunnel to it, pairs with it the first time without a PIN to type,
 * and starts the stream. Portrait shows one column of cards; a wide or docked
 * window shows two.
 */
public class SecparActivity extends Activity {
    private static final String[] TABS = {"Computers", "Friends", "Settings"};
    /** The still on a card, in dp, beside the name like the monitor glyph it replaces. */
    /** The size a still is kept at: a card's full width on a phone. */
    private static final int STILL_W_DP = 400;
    private static final int STILL_H_DP = 225;
    /** The widest still asked for; a card draws it at about 64 dp. */
    private static final int STILL_WIDTH = 640;
    /** One computer's still is asked for again no sooner than this. */
    private static final long STILL_EVERY = 15_000L;

    private final ExecutorService work = Executors.newSingleThreadExecutor();
    // Pairing has two halves that run at the same time.
    private final ExecutorService second = Executors.newSingleThreadExecutor();
    // Stills wait for a computer on their own thread, so a slow one never holds up
    // the buttons on the page.
    private final ExecutorService stillsWork = Executors.newSingleThreadExecutor();
    private final Handler main = new Handler(Looper.getMainLooper());

    private LinearLayout head;
    private LinearLayout tabsHome;
    private LinearLayout tabs;
    private LinearLayout page;
    private ScrollView scroll;

    private int tab;
    private boolean busy;
    /** The account handle, or null while signed out. */
    private String handle;
    /** Every account device, or null until the list has been fetched. */
    private JSONArray devices;
    /** Every account friend and waiting request, or null before the first fetch. */
    private JSONArray friends;
    private boolean friendsLoaded;
    private boolean friendsBusy;
    private String friendsProblem;
    /** The friend whose permissions are open, by account id. */
    private String openFriend = "";
    /** What is typed in the add-friend field, kept when the page redraws. */
    private String addText = "";
    private String problem;
    private String email = "";
    /** The computer being opened, shown as Connecting on its card. */
    private String connecting;
    /** True while the attempt is waiting for a computer that is only now starting. */
    private boolean starting;
    /** The computer whose stream screen is open, so its answer is read as this one's. */
    private String streaming;
    /** Which attempt is current; one that has been given up stops itself. */
    private int attempts;
    /** The sentence for a stream that did not start, apart from the list's own. */
    private String streamProblem;
    private boolean built;
    /** What this device can play, or null until its decoders have been read. */
    private StreamChoices choices;
    /** Which Settings row has its values open. */
    private String expanded = "";

    /** Each computer's card still, already the size the card draws it at. */
    private final Map<String, Bitmap> stills = new HashMap<>();
    /** The leading view of each card on screen, so a still can appear in place. */
    private final Map<String, ImageView> leads = new HashMap<>();
    /** When each computer's still was last asked for, in uptime milliseconds. */
    private final Map<String, Long> stillAsked = new HashMap<>();
    /** Computers whose still is already on its way. */
    private final Set<String> stillAsking = new HashSet<>();
    /** Whether the still refresh is running, so it is never started twice. */
    private boolean stillLoop;
    /** Whether this screen is in front; nothing is refreshed behind it. */
    private boolean showing;

    @Override
    protected void onCreate(Bundle saved) {
        super.onCreate(saved);
        LinearLayout root = Ui.column(this);
        root.setBackgroundColor(Ui.BACKGROUND);

        head = Ui.row(this);
        head.setGravity(Gravity.CENTER_VERTICAL);
        head.addView(Ui.brand(this, version()));

        tabsHome = Ui.column(this);
        tabs = Ui.row(this);
        tabs.setBackground(Ui.box(this, Ui.SURFACE, Ui.BORDER, 6));
        int inset = Ui.dp(this, 2);
        tabs.setPadding(inset, inset, inset, inset);
        tabsHome.addView(tabs, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, Ui.dp(this, 36)));

        scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        scroll.setBackgroundColor(Ui.BACKGROUND);
        page = Ui.column(this);
        scroll.addView(page, new ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));

        root.addView(head, Ui.fill());
        root.addView(tabsHome, Ui.fill());
        root.addView(scroll, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f));
        setContentView(root);
        // The window draws behind the status and navigation bars. Give the header
        // the status bar's height and the content the bar at the bottom, so the
        // wordmark is never under the clock.
        root.setOnApplyWindowInsetsListener((view, insets) -> {
            int left, top, right, bottom;
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                android.graphics.Insets bars = insets.getInsets(WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
                left = bars.left;
                top = bars.top;
                right = bars.right;
                bottom = bars.bottom;
            } else {
                left = insets.getSystemWindowInsetLeft();
                top = insets.getSystemWindowInsetTop();
                right = insets.getSystemWindowInsetRight();
                bottom = insets.getSystemWindowInsetBottom();
            }
            view.setPadding(left, top, right, bottom);
            return insets;
        });

        getWindow().setStatusBarColor(Ui.BACKGROUND);
        getWindow().setNavigationBarColor(Ui.BACKGROUND);

        // What the stream screen reports about the connection it is making.
        IntentFilter events = new IntentFilter(Game.SECPAR_EVENT);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            registerReceiver(streamEvents, events, Context.RECEIVER_NOT_EXPORTED);
        } else {
            registerReceiver(streamEvents, events);
        }
        // Reading the decoders touches the system's codec list, so it happens
        // once, off the main thread; the Settings rows are drawn when it answers.
        new Thread(() -> {
            StreamChoices read = StreamChoices.read(getApplicationContext());
            main.post(() -> {
                choices = read;
                if (handle != null && tab == 2 && !isFinishing()) {
                    render();
                }
            });
        }, "secpar-stream-choices").start();
        relayout();
    }

    /** Connecting, connected and failed, as the stream screen sees them. */
    private final BroadcastReceiver streamEvents = new BroadcastReceiver() {
        @Override
        public void onReceive(Context context, Intent event) {
            if (streaming == null || event == null) {
                return;
            }
            String state = event.getStringExtra(Game.SECPAR_STATE);
            if ("connected".equals(state)) {
                // The picture is up, so the card behind the stream is no longer opening.
                connecting = null;
                starting = false;
            } else if ("failed".equals(state) || "ended".equals(state)) {
                connecting = null;
                starting = false;
                streaming = null;
                if ("failed".equals(state)) {
                    streamProblem = streamTrouble(event.getStringExtra(Game.SECPAR_REASON));
                }
            } else {
                return;
            }
            if (!isFinishing()) {
                render();
            }
        }
    };

    private void relayout() {
        int side = side();
        head.setPadding(side, Ui.dp(this, 12), side, Ui.dp(this, 10));
        tabsHome.setPadding(side, 0, side, Ui.dp(this, 8));
        page.setPadding(side, Ui.dp(this, 4), side, Ui.dp(this, 16));
        // A width change redraws the pages, but never the sign-in form the person is typing in.
        if (handle != null || !built) {
            render();
        }
    }

    private int side() {
        return Ui.dp(this, Layout.sideDp(widthDp()));
    }

    private int widthDp() {
        return getResources().getConfiguration().screenWidthDp;
    }

    private String version() {
        try {
            String name = getPackageManager().getPackageInfo(getPackageName(), 0).versionName;
            return name == null ? "" : name;
        } catch (Exception e) {
            return "";
        }
    }

    @Override
    protected void onResume() {
        super.onResume();
        showing = true;
        // Back from the stream screen: the attempt behind it is over, whatever it
        // answered, and its card goes back to the list's own state.
        connecting = null;
        starting = false;
        // Back from a stream, or first shown: nothing stays connected behind this screen.
        run(() -> {
            JSONObject configure = Core.request("configure").put("folder", getFilesDir().getAbsolutePath());
            Core.ask(configure);
            Core.ask(Core.request("disconnect"));
            return Core.ask(Core.request("state"));
        }, state -> {
            String account = state.optBoolean("signed_in") ? state.optString("handle") : "";
            handle = account.isEmpty() ? null : account;
            if (handle == null) {
                forgetStills();
                devices = null;
                friends = null;
                friendsLoaded = false;
                render();
            } else {
                loadComputers();
            }
        });
    }

    @Override
    protected void onPause() {
        // A screen nobody is looking at asks for nothing.
        showing = false;
        super.onPause();
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

    // ---- Tabs and pages

    private void render() {
        page.removeAllViews();
        if (handle == null) {
            tabsHome.setVisibility(View.GONE);
            signIn();
            return;
        }
        tabsHome.setVisibility(View.VISIBLE);
        tabs();
        switch (tab) {
            case 0:
                computers();
                break;
            case 1:
                friends();
                break;
            default:
                settings();
                break;
        }
        built = true;
    }

    private void tabs() {
        tabs.removeAllViews();
        for (int i = 0; i < TABS.length; i++) {
            TextView item = Ui.text(this, TABS[i], i == tab ? Ui.MINT : Ui.DIM, 13);
            item.setGravity(Gravity.CENTER);
            item.setTypeface(Typeface.DEFAULT_BOLD);
            item.setBackground(i == tab
                    ? Ui.box(this, Ui.SURFACE_HIGH, 0, 5)
                    : Ui.press(this, Ui.box(this, 0, 0, 5)));
            final int shown = i;
            item.setOnClickListener(v -> {
                if (!busy && tab != shown) {
                    tab = shown;
                    problem = null;
                    friendsProblem = null;
                    expanded = "";
                    // Opening Friends asks the account again, so a new request
                    // from another computer shows up without restarting.
                    if (shown == 1) {
                        friendsLoaded = false;
                    }
                    render();
                }
            });
            tabs.addView(item, new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.MATCH_PARENT, 1f));
        }
    }

    @Override
    public void onConfigurationChanged(android.content.res.Configuration config) {
        super.onConfigurationChanged(config);
        // Rotation or a docked window: same activity, new width.
        relayout();
    }

    @Override
    public void onBackPressed() {
        if (handle != null && tab != 0) {
            tab = 0;
            render();
            return;
        }
        super.onBackPressed();
    }

    // ---- Sign in

    private void signIn() {
        page.addView(Ui.strong(this, "Sign in", 20), Ui.top(this, 4));

        EditText address = field("Email", InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS);
        address.setText(email);
        EditText password = field("Password", InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD);
        page.addView(address, Ui.top(this, 10));
        page.addView(password, Ui.top(this, 8));

        TextView go = Ui.action(this, "Sign in", true);
        page.addView(go, Ui.top(this, 12));

        if (problem != null) {
            page.addView(notice(problem), Ui.top(this, 8));
        }
        page.addView(Ui.body(this, "This device is added to your account when you sign in.", Ui.DIM), Ui.top(this, 10));

        go.setOnClickListener(v -> {
            String account = address.getText().toString().trim();
            String secret = password.getText().toString();
            if (busy || account.isEmpty() || secret.isEmpty()) {
                return;
            }
            email = account;
            password.setText("");
            problem = null;
            hideKeyboard();
            go.setText("Signing in");
            go.setEnabled(false);
            run(() -> Core.ask(Core.request("sign_in").put("email", account).put("password", secret).put("name", Build.MODEL)), answer -> {
                if (answer.optBoolean("signed_in")) {
                    email = "";
                    handle = answer.optString("handle");
                    if (handle.isEmpty()) {
                        handle = account;
                    }
                    devices = null;
                    tab = 0;
                    loadComputers();
                } else {
                    problem = explain(answer);
                    render();
                }
            });
        });
    }

    private EditText field(String hint, int type) {
        EditText view = new EditText(this);
        view.setHint(hint);
        view.setInputType(type);
        view.setTextColor(Ui.TEXT);
        view.setHintTextColor(Ui.DIM);
        view.setTextSize(15);
        view.setSingleLine(true);
        view.setBackground(Ui.box(this, Ui.SURFACE, Ui.CONTROL_BORDER, 6));
        view.setPadding(Ui.dp(this, 12), Ui.dp(this, 10), Ui.dp(this, 12), Ui.dp(this, 10));
        view.setMinHeight(Ui.dp(this, 44));
        return view;
    }

    private void hideKeyboard() {
        View focused = getCurrentFocus();
        if (focused == null) {
            return;
        }
        InputMethodManager keys = (InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
        if (keys != null) {
            keys.hideSoftInputFromWindow(focused.getWindowToken(), 0);
        }
    }

    // ---- Computers

    private void loadComputers() {
        run(() -> Core.ask(Core.request("devices")), answer -> {
            if ("signed_out".equals(answer.optString("error"))) {
                handle = null;
                forgetStills();
                devices = null;
                problem = null;
                render();
                return;
            }
            JSONArray listed = answer.optJSONArray("devices");
            if (listed != null) {
                // A failed refresh keeps the list already on screen.
                devices = listed;
            }
            problem = answer.optBoolean("ok") ? null : explain(answer);
            render();
        });
    }

    private void computers() {
        // The cards are about to be built again, so what is on screen is replaced
        // rather than added to.
        leads.clear();
        List<JSONObject> hosts = new ArrayList<>();
        if (devices == null) {
            if (problem == null) {
                page.addView(Ui.body(this, "Loading computers.", Ui.DIM), Ui.top(this, 12));
            }
        } else {
            for (int i = 0; i < devices.length(); i++) {
                JSONObject device = devices.optJSONObject(i);
                if (device != null && device.optBoolean("hosts")) {
                    hosts.add(device);
                }
            }
            if (hosts.isEmpty()) {
                page.addView(Ui.empty(this, "No computers yet",
                        "Sign in on a Windows or Linux computer with this account to see it here."), Ui.top(this, 8));
            } else {
                List<View> cards = new ArrayList<>();
                for (JSONObject host : hosts) {
                    cards.add(computerCard(host));
                }
                addGrid(cards);
            }
        }
        if (problem != null) {
            page.addView(notice(problem), Ui.top(this, 8));
        }
        if (streamProblem != null) {
            page.addView(notice(streamProblem), Ui.top(this, 8));
        }
        TextView refresh = Ui.quiet(this, "Refresh");
        refresh.setOnClickListener(v -> {
            if (!busy && connecting == null) {
                problem = null;
                loadComputers();
            }
        });
        LinearLayout.LayoutParams refreshParams = new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        refreshParams.topMargin = Ui.dp(this, 12);
        page.addView(refresh, refreshParams);
        watchStills();
    }

    /** Lays cards out one per row in portrait and two in a wide window. */
    private void addGrid(List<View> cards) {
        int columns = Layout.columns(widthDp());
        LinearLayout line = null;
        for (int i = 0; i < cards.size(); i++) {
            if (i % columns == 0) {
                line = Ui.row(this);
                page.addView(line, Ui.top(this, 8));
            }
            LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f);
            if (i % columns != 0) {
                params.leftMargin = Ui.dp(this, 8);
            }
            line.addView(cards.get(i), params);
        }
        if (columns > 1 && cards.size() % columns != 0) {
            View spacer = new View(this);
            LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(0, 1, 1f);
            params.leftMargin = Ui.dp(this, 8);
            line.addView(spacer, params);
        }
    }

    /** A frame as tall as nine sixteenths of its width, for a screen's still. */
    private static final class Screen extends android.widget.FrameLayout {
        Screen(Context context) {
            super(context);
        }

        @Override
        protected void onMeasure(int widthSpec, int heightSpec) {
            int width = MeasureSpec.getSize(widthSpec);
            super.onMeasure(widthSpec, MeasureSpec.makeMeasureSpec(width * 9 / 16, MeasureSpec.EXACTLY));
        }
    }

    /**
     * One computer, as on the desktop: its screen on top, then its name and
     * state. The screen is the latest still, or the monitor glyph until one arrives.
     */
    private View computerCard(JSONObject device) {
        boolean online = device.optBoolean("online");
        boolean opening = device.optString("id").equals(connecting);
        String name = device.optString("name");
        String platform = platformName(device.optString("platform"));

        LinearLayout card = Ui.column(this);
        int edge = Ui.dp(this, 1);
        card.setPadding(edge, edge, edge, edge);
        card.setClipToOutline(true);

        Screen screen = new Screen(this);
        screen.setBackgroundColor(Ui.BACKGROUND);
        ImageView glyph = Ui.icon(this, R.drawable.lucide_monitor, online ? Ui.DIM : Ui.BORDER, 32);
        screen.addView(glyph, new android.widget.FrameLayout.LayoutParams(Ui.dp(this, 32), Ui.dp(this, 32), Gravity.CENTER));
        ImageView picture = new ImageView(this);
        picture.setScaleType(ImageView.ScaleType.CENTER_CROP);
        picture.setVisibility(View.GONE);
        screen.addView(picture, new android.widget.FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        card.addView(screen, Ui.fill());
        if (online) {
            leads.put(device.optString("id"), picture);
            Bitmap still = stills.get(device.optString("id"));
            if (still != null) {
                drawStill(picture, still);
            }
        }

        LinearLayout foot = Ui.row(this);
        foot.setGravity(Gravity.CENTER_VERTICAL);
        foot.setPadding(Ui.dp(this, 12), Ui.dp(this, 10), Ui.dp(this, 8), Ui.dp(this, 10));
        LinearLayout words = Ui.column(this);
        words.addView(Ui.strong(this, name, 15));
        String state = opening ? (starting ? "Starting…" : "Connecting") : online ? "Online" : "Offline";
        words.addView(Ui.body(this, platform + " · " + state, opening || online ? Ui.MINT : Ui.DIM), Ui.top(this, 1));
        foot.addView(words, Ui.weighted());

        if (opening) {
            // The person can give the attempt up without waiting for its answer.
            TextView cancel = Ui.text(this, "Cancel", Ui.MINT, 13);
            cancel.setTypeface(Typeface.DEFAULT_BOLD);
            cancel.setGravity(Gravity.CENTER);
            int pad = Ui.dp(this, 8);
            cancel.setPadding(pad, pad, pad, pad);
            cancel.setBackground(Ui.press(this, Ui.box(this, 0, 0, 6)));
            cancel.setClickable(true);
            cancel.setFocusable(true);
            cancel.setOnClickListener(v -> cancelConnect());
            foot.addView(cancel);
        } else if (online) {
            foot.addView(Ui.icon(this, R.drawable.lucide_chevron_right, Ui.DIM, 16));
        }
        card.addView(foot, Ui.fill());

        if (!online && !opening) {
            // A computer that looks off can still be bringing its hosting up, so
            // it stays dimmed but answers a tap with the attempt below.
            card.setAlpha(0.55f);
        }
        card.setBackground(Ui.press(this, Ui.box(this, Ui.SURFACE, Ui.BORDER, 7)));
        card.setClickable(true);
        card.setFocusable(true);
        card.setOnClickListener(v -> connect(device));
        return card;
    }

    // ---- The stills on the computer cards

    /** Starts the still refresh for the Computers tab; one loop at a time. */
    private void watchStills() {
        if (!stillLoop) {
            stillLoop = true;
            main.post(stillTick);
        }
    }

    /**
     * Asks each online computer for a small picture of its screen, one every 15
     * seconds at most, and only while the Computers tab is the one on screen.
     */
    private final Runnable stillTick = new Runnable() {
        @Override
        public void run() {
            if (!showing || isFinishing() || tab != 0 || handle == null || devices == null) {
                stillLoop = false;
                return;
            }
            long now = SystemClock.elapsedRealtime();
            for (int i = 0; i < devices.length(); i++) {
                JSONObject device = devices.optJSONObject(i);
                if (device == null || !device.optBoolean("hosts") || !device.optBoolean("online")) {
                    continue;
                }
                String id = device.optString("id");
                Long asked = stillAsked.get(id);
                if (stillAsking.contains(id) || (asked != null && now - asked < STILL_EVERY)) {
                    continue;
                }
                stillAsked.put(id, now);
                stillAsking.add(id);
                askStill(id);
            }
            // The next look picks up a computer that has just come online.
            main.postDelayed(stillTick, 5_000);
        }
    };

    /** One request, off the main thread: it blocks, and the JPEG is decoded there too. */
    private void askStill(String id) {
        stillsWork.execute(() -> {
            JSONObject answer = Core.ask(ask("preview", "id", id, "max_width", STILL_WIDTH));
            if (!answer.optBoolean("ok")) {
                android.util.Log.i("SecparStill", "no still: " + answer.optString("err", answer.optString("error", "unknown")));
            }
            Bitmap still = decode(answer);
            main.post(() -> {
                stillAsking.remove(id);
                if (still == null || isFinishing() || tab != 0) {
                    // No still, or nobody looking: the glyph stays.
                    if (still != null) {
                        still.recycle();
                    }
                    return;
                }
                Bitmap before = stills.put(id, still);
                if (before != null && before != still) {
                    before.recycle();
                }
                ImageView lead = leads.get(id);
                if (lead != null) {
                    drawStill(lead, still);
                }
            });
        });
    }

    /** Shows the still over the monitor glyph. */
    private void drawStill(ImageView picture, Bitmap still) {
        picture.setImageBitmap(still);
        picture.setVisibility(View.VISIBLE);
    }

    /**
     * The picture in an answer, decoded and scaled to the card here on the calling
     * thread, so nothing full-size is kept and no decoding happens on the main one.
     */
    private Bitmap decode(JSONObject answer) {
        if (!answer.optBoolean("ok")) {
            return null;
        }
        String jpeg = answer.optString("jpeg");
        if (jpeg.isEmpty()) {
            return null;
        }
        byte[] bytes;
        try {
            bytes = Base64.decode(jpeg, Base64.DEFAULT);
        } catch (IllegalArgumentException e) {
            return null;
        }
        BitmapFactory.Options size = new BitmapFactory.Options();
        size.inJustDecodeBounds = true;
        BitmapFactory.decodeByteArray(bytes, 0, bytes.length, size);
        if (size.outWidth <= 0 || size.outHeight <= 0) {
            return null;
        }
        int boxWidth = Ui.dp(this, STILL_W_DP);
        int boxHeight = Ui.dp(this, STILL_H_DP);
        // Read it at about twice the size it is drawn at.
        int sample = 1;
        while (size.outWidth / (sample * 2) >= boxWidth && size.outHeight / (sample * 2) >= boxHeight) {
            sample *= 2;
        }
        BitmapFactory.Options options = new BitmapFactory.Options();
        options.inSampleSize = sample;
        Bitmap decoded = BitmapFactory.decodeByteArray(bytes, 0, bytes.length, options);
        if (decoded == null) {
            return null;
        }
        double fits = Math.min((double) boxWidth / decoded.getWidth(), (double) boxHeight / decoded.getHeight());
        int width = Math.max(1, (int) Math.round(decoded.getWidth() * fits));
        int height = Math.max(1, (int) Math.round(decoded.getHeight() * fits));
        if (width == decoded.getWidth() && height == decoded.getHeight()) {
            return decoded;
        }
        Bitmap small = Bitmap.createScaledBitmap(decoded, width, height, true);
        if (small != decoded) {
            decoded.recycle();
        }
        return small;
    }

    /** Drops every still, for a change of account. */
    private void forgetStills() {
        for (Bitmap still : stills.values()) {
            still.recycle();
        }
        stills.clear();
        leads.clear();
        stillAsked.clear();
        stillAsking.clear();
    }

    private static String platformName(String platform) {
        switch (platform) {
            case "windows":
                return "Windows";
            case "linux":
                return "Linux";
            case "android":
                return "Android";
            default:
                return "Computer";
        }
    }

    // ---- Friends

    private void loadFriends() {
        if (friendsBusy) {
            return;
        }
        friendsBusy = true;
        run(() -> Core.ask(Core.request("friends")), answer -> {
            friendsBusy = false;
            if ("signed_out".equals(answer.optString("error"))) {
                handle = null;
                forgetStills();
                devices = null;
                friends = null;
                friendsLoaded = false;
                problem = null;
                friendsProblem = null;
                render();
                return;
            }
            JSONArray listed = answer.optJSONArray("friends");
            if (listed != null) {
                // A failed refresh keeps the list already on screen.
                friends = listed;
            }
            friendsProblem = answer.optBoolean("ok") ? null
                    : trouble(answer, "The friend list could not be read. Try again.");
            friendsLoaded = true;
            if (tab == 1) {
                render();
            }
        });
    }

    private void friends() {
        if (!friendsLoaded && !friendsBusy) {
            loadFriends();
        }
        page.addView(addFriend(), Ui.top(this, 4));

        if (friends == null) {
            if (friendsProblem == null) {
                page.addView(Ui.body(this, "Loading friends.", Ui.DIM), Ui.top(this, 12));
            }
        } else if (friends.length() == 0) {
            page.addView(Ui.empty(this, "No friends yet",
                    "Enter an account name above to send a request."), Ui.top(this, 8));
        } else {
            // Requests waiting for an answer come first, then friends, then the
            // requests this account sent.
            List<View> rows = new ArrayList<>();
            for (int kind = 0; kind < 3; kind++) {
                for (int i = 0; i < friends.length(); i++) {
                    JSONObject friend = friends.optJSONObject(i);
                    if (friend != null && friendKind(friend) == kind) {
                        rows.add(friendCard(friend));
                    }
                }
            }
            addGrid(rows);
        }
        if (friendsProblem != null) {
            page.addView(notice(friendsProblem), Ui.top(this, 8));
        }
    }

    /** 0 for a request waiting for an answer, 1 for a friend, 2 for one sent. */
    private static int friendKind(JSONObject friend) {
        if (friend.optBoolean("incoming")) {
            return 0;
        }
        return "accepted".equals(friend.optString("state")) ? 1 : 2;
    }

    private View addFriend() {
        LinearLayout line = Ui.row(this);
        line.setGravity(Gravity.CENTER_VERTICAL);

        EditText field = field("@account name", InputType.TYPE_CLASS_TEXT);
        field.setText(addText);
        TextView send = Ui.action(this, "Send request", true);
        field.addTextChangedListener(new TextWatcher() {
            @Override
            public void beforeTextChanged(CharSequence text, int start, int count, int after) {
            }

            @Override
            public void onTextChanged(CharSequence text, int start, int before, int count) {
            }

            @Override
            public void afterTextChanged(Editable text) {
                addText = text.toString();
                // A request needs a name; an empty field cannot send one.
                boolean ready = !addText.trim().isEmpty();
                send.setEnabled(ready);
                send.setAlpha(ready ? 1f : 0.5f);
            }
        });
        line.addView(field, Ui.weighted());

        send.setEnabled(!addText.trim().isEmpty());
        send.setAlpha(addText.trim().isEmpty() ? 0.5f : 1f);
        LinearLayout.LayoutParams sendParams = new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        sendParams.leftMargin = Ui.dp(this, 8);
        line.addView(send, sendParams);

        send.setOnClickListener(v -> {
            String name = addText.trim();
            if (busy || name.isEmpty()) {
                return;
            }
            hideKeyboard();
            friendAction(ask("add_friend", "handle", name));
        });
        return line;
    }

    private View friendCard(JSONObject friend) {
        int kind = friendKind(friend);
        String account = friend.optString("id");
        String handleText = "@" + friend.optString("handle");
        String name = friend.optString("name");
        if (name.isEmpty()) {
            name = handleText;
        }
        boolean open = kind == 1 && account.equals(openFriend);

        LinearLayout card = Ui.card(this);

        LinearLayout head = Ui.row(this);
        head.setGravity(Gravity.CENTER_VERTICAL);

        ImageView icon = Ui.icon(this, R.drawable.lucide_circle_user, kind == 0 ? Ui.MINT : Ui.DIM, 18);
        LinearLayout.LayoutParams iconParams = new LinearLayout.LayoutParams(icon.getLayoutParams());
        iconParams.rightMargin = Ui.dp(this, 10);
        head.addView(icon, iconParams);

        LinearLayout words = Ui.column(this);
        words.addView(Ui.strong(this, name, 15));
        String second = kind == 0 ? handleText + " · Wants to be your friend"
                : kind == 2 ? handleText + " · Request sent"
                : handleText + " · " + control(friend);
        words.addView(Ui.body(this, second, kind == 0 ? Ui.MINT : Ui.DIM), Ui.top(this, 1));
        head.addView(words, Ui.weighted());

        if (kind == 1) {
            ImageView chevron = Ui.icon(this, R.drawable.lucide_chevron_right, Ui.DIM, 16);
            chevron.setRotation(open ? 90 : 0);
            LinearLayout.LayoutParams chevronParams = new LinearLayout.LayoutParams(chevron.getLayoutParams());
            chevronParams.leftMargin = Ui.dp(this, 8);
            head.addView(chevron, chevronParams);
        }
        card.addView(head);

        if (kind == 0) {
            LinearLayout answer = Ui.row(this);
            TextView accept = Ui.action(this, "Accept", true);
            accept.setOnClickListener(v -> {
                if (!busy) {
                    friendAction(ask("accept_friend", "id", account));
                }
            });
            answer.addView(accept);
            TextView decline = Ui.action(this, "Decline", false);
            LinearLayout.LayoutParams declineParams = new LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT);
            declineParams.leftMargin = Ui.dp(this, 8);
            answer.addView(decline, declineParams);
            decline.setOnClickListener(v -> {
                if (!busy) {
                    friendAction(ask("remove_friend", "id", account));
                }
            });
            card.addView(answer, Ui.top(this, 10));
        } else if (kind == 2) {
            TextView cancel = Ui.quiet(this, "Cancel request");
            cancel.setOnClickListener(v -> {
                if (!busy) {
                    friendAction(ask("remove_friend", "id", account));
                }
            });
            LinearLayout.LayoutParams cancelParams = new LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT);
            cancelParams.topMargin = Ui.dp(this, 10);
            card.addView(cancel, cancelParams);
        } else if (open) {
            // The two permissions, saved as soon as one is chosen.
            boolean full = friend.optBoolean("keyboard") && friend.optBoolean("mouse");
            String connect = "always".equals(friend.optString("connect")) ? "always" : "ask";
            card.addView(permission("Allow to connect",
                    new String[]{"ask", "always"}, new String[]{"Ask each time", "Always"},
                    connect, value -> friendPermissions(account, "connect", value)), Ui.top(this, 8));
            String input = full ? "full" : friend.optBoolean("keyboard") ? "keyboard" : friend.optBoolean("mouse") ? "mouse" : "view";
            card.addView(permission("Control",
                    new String[]{"view", "keyboard", "mouse", "full"},
                    new String[]{"View only", "Keyboard only", "Mouse only", "Keyboard and mouse"},
                    input,
                    value -> friendPermissions(account, "input", value)));

            TextView remove = Ui.quiet(this, "Remove");
            remove.setOnClickListener(v -> {
                if (!busy) {
                    friendAction(ask("remove_friend", "id", account));
                }
            });
            LinearLayout.LayoutParams removeParams = new LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT);
            removeParams.topMargin = Ui.dp(this, 12);
            card.addView(remove, removeParams);
        }

        if (kind == 1) {
            card.setClickable(true);
            card.setFocusable(true);
            card.setOnClickListener(v -> {
                openFriend = open ? "" : account;
                render();
            });
        }
        return card;
    }

    /** One permission as a settings row; its values open underneath it. */
    private View permission(String label, String[] values, String[] labels, String current, Pick pick) {
        List<StreamChoices.Option> options = new ArrayList<>();
        for (int i = 0; i < values.length; i++) {
            options.add(new StreamChoices.Option(values[i], labels[i]));
        }
        LinearLayout holder = Ui.column(this);
        setting(holder, "permission:" + openFriend + ":" + label, label, current, "Some control", options, value -> {
            expanded = "";
            if (!busy && !value.equals(current)) {
                pick.with(value);
            } else {
                render();
            }
        });
        return holder;
    }

    private interface Pick {
        void with(String value);
    }

    /** What the two control permissions add up to, said plainly. */
    private static String control(JSONObject friend) {
        boolean keyboard = friend.optBoolean("keyboard");
        boolean mouse = friend.optBoolean("mouse");
        if (keyboard && mouse) {
            return "Keyboard and mouse";
        }
        if (keyboard) {
            return "Keyboard only";
        }
        if (mouse) {
            return "Mouse only";
        }
        return "View only";
    }

    /** Saves one of the two settings the service knows, then shows its answer. */
    private void friendPermissions(String account, String setting, String value) {
        if ("connect".equals(setting)) {
            friendAction(ask("friend_settings", "id", account, "connect", value));
        } else {
            // Keyboard and mouse are separate, as in Secpar 3; the service
            // works out its own "input" from the two.
            boolean keyboard = "full".equals(value) || "keyboard".equals(value);
            boolean mouse = "full".equals(value) || "mouse".equals(value);
            friendAction(ask("friend_settings", "id", account, "keyboard", keyboard, "mouse", mouse));
        }
    }

    private void friendAction(JSONObject request) {
        if (busy) {
            return;
        }
        friendsProblem = null;
        boolean added = "add_friend".equals(request.optString("do"));
        run(() -> Core.ask(request), answer -> {
            if ("signed_out".equals(answer.optString("error"))) {
                handle = null;
                devices = null;
                friends = null;
                friendsLoaded = false;
                problem = null;
                friendsProblem = null;
                render();
                return;
            }
            friendsProblem = answer.optBoolean("ok") ? null
                    : trouble(answer, "That did not work. Try again.");
            if (answer.optBoolean("ok") && added) {
                addText = "";
            }
            // The service is the one that decides what a change amounts to, so
            // the list is read again rather than assumed.
            friendsLoaded = false;
            render();
        });
    }

    /** A one-request builder; a field that cannot be set leaves an empty request. */
    private static JSONObject ask(String what, Object... pairs) {
        JSONObject request = Core.request(what);
        try {
            for (int i = 0; i + 1 < pairs.length; i += 2) {
                request.put(String.valueOf(pairs[i]), pairs[i + 1]);
            }
        } catch (JSONException e) {
            return new JSONObject();
        }
        return request;
    }

    // ---- Settings

    private void settings() {
        LinearLayout account = group("Account");
        LinearLayout who = Ui.row(this);
        who.setGravity(Gravity.CENTER_VERTICAL);
        ImageView avatar = Ui.icon(this, R.drawable.lucide_circle_user, Ui.MINT, 20);
        LinearLayout.LayoutParams avatarParams = new LinearLayout.LayoutParams(avatar.getLayoutParams());
        avatarParams.rightMargin = Ui.dp(this, 10);
        who.addView(avatar, avatarParams);
        LinearLayout words = Ui.column(this);
        words.addView(Ui.strong(this, handle, 15));
        words.addView(Ui.body(this, "Signed in", Ui.DIM), Ui.top(this, 1));
        who.addView(words, Ui.weighted());
        account.addView(who);

        TextView out = Ui.action(this, "Sign out", false);
        out.setOnClickListener(v -> {
            if (!busy) {
                signOut();
            }
        });
        account.addView(out, Ui.top(this, 10));

        LinearLayout listed = group("Devices");
        if (devices == null) {
            listed.addView(Ui.body(this, "Loading devices.", Ui.DIM), Ui.top(this, 2));
        } else if (devices.length() == 0) {
            listed.addView(Ui.body(this, "No devices listed.", Ui.DIM), Ui.top(this, 2));
        } else {
            for (int i = 0; i < devices.length(); i++) {
                JSONObject device = devices.optJSONObject(i);
                if (device == null) {
                    continue;
                }
                if (i > 0) {
                    listed.addView(Ui.gap(this, 8));
                }
                listed.addView(deviceRow(device));
            }
        }

        // The stream choices, in the app's own rows. Each row writes the
        // preference Moonlight's PreferenceConfiguration reads, and every list
        // holds only what this device can play.
        if (choices == null) {
            LinearLayout reading = group("Streaming");
            reading.addView(Ui.body(this, "Reading this device's decoders and display.", Ui.DIM), Ui.top(this, 2));
        } else {
            LinearLayout picture = group("Picture");
            String resolution = StreamChoices.resolution(this);
            setting(picture, "resolution", "Resolution", resolution, resolution, choices.resolutions, value -> {
                StreamChoices.put(this, StreamChoices.RESOLUTION_KEY, value);
                settleStream();
            });
            String fps = StreamChoices.frameRate(this);
            setting(picture, "fps", "Frame rate", fps, fps + " fps", choices.frameRates, value -> {
                StreamChoices.put(this, StreamChoices.FPS_KEY, value);
                settleStream();
            });
            String bitrate = String.valueOf(StreamChoices.bitrate(this));
            setting(picture, "bitrate", "Bitrate", bitrate, bitrate + " kbps", choices.bitrates, value -> {
                StreamChoices.putBitrate(this, Integer.parseInt(value));
                settleStream();
            });

            LinearLayout video = group("Video");
            String format = StreamChoices.videoFormat(this);
            setting(video, "format", "Video format", format, formatName(format), choices.videoFormats, value -> {
                StreamChoices.put(this, StreamChoices.VIDEO_FORMAT_KEY, value);
                settleStream();
            });
            String pacing = StreamChoices.framePacing(this);
            setting(video, "pacing", "Frame pacing", pacing, pacing, pace(), value -> {
                StreamChoices.put(this, StreamChoices.FRAME_PACING_KEY, value);
                settleStream();
            });

            LinearLayout audio = group("Audio");
            String sound = StreamChoices.audio(this);
            setting(audio, "audio", "Audio output", sound, sound, sound(), value -> {
                StreamChoices.put(this, StreamChoices.AUDIO_KEY, value);
                settleStream();
            });

            LinearLayout display = group("Display");
            String fitting = StreamChoices.stretch(this) ? "stretch" : "fit";
            setting(display, "display", "Display mode", fitting, fitting, displayModes(), value -> {
                StreamChoices.putStretch(this, "stretch".equals(value));
                settleStream();
            });
        }

        LinearLayout about = group("About");
        LinearLayout versionRow = Ui.row(this);
        versionRow.setGravity(Gravity.CENTER_VERTICAL);
        versionRow.addView(Ui.text(this, "Secpar", Ui.TEXT, 14), Ui.weighted());
        versionRow.addView(Ui.text(this, version(), Ui.DIM, 12));
        about.addView(versionRow);

        if (problem != null) {
            page.addView(notice(problem), Ui.top(this, 8));
        }
    }

    private LinearLayout group(String title) {
        LinearLayout card = Ui.card(this);
        card.addView(Ui.strong(this, title, 13));
        page.addView(card, Ui.top(this, 8));
        return card;
    }

    /**
     * One settings row: the label, the saved value, and the values it can be
     * set to. A row with a single possible value shows it without a control,
     * because there would be nothing to choose.
     */
    private void setting(LinearLayout card, String key, String label, String saved, String otherwise,
                         List<StreamChoices.Option> options, Pick pick) {
        LinearLayout line = Ui.row(this);
        line.setGravity(Gravity.CENTER_VERTICAL);
        line.setMinimumHeight(Ui.dp(this, 44));
        line.addView(Ui.text(this, label, Ui.TEXT, 14), Ui.weighted());
        line.addView(Ui.text(this, StreamChoices.label(options, saved, otherwise), Ui.DIM, 13));

        boolean list = options.size() > 1;
        if (list) {
            ImageView chevron = Ui.icon(this, R.drawable.lucide_chevron_right, Ui.DIM, 16);
            chevron.setRotation(key.equals(expanded) ? 90 : 0);
            LinearLayout.LayoutParams chevronParams = new LinearLayout.LayoutParams(chevron.getLayoutParams());
            chevronParams.leftMargin = Ui.dp(this, 6);
            line.addView(chevron, chevronParams);
            line.setBackground(Ui.press(this, Ui.box(this, 0, 0, 5)));
            line.setClickable(true);
            line.setFocusable(true);
            line.setOnClickListener(v -> {
                expanded = key.equals(expanded) ? "" : key;
                render();
            });
        }
        card.addView(line);

        if (list && key.equals(expanded)) {
            for (StreamChoices.Option option : options) {
                card.addView(choice(option, option.value.equals(saved), pick), Ui.top(this, 2));
            }
        }
    }

    /** One value under an open row, with a mint check on the saved one. */
    private View choice(StreamChoices.Option option, boolean current, Pick pick) {
        LinearLayout row = Ui.row(this);
        row.setGravity(Gravity.CENTER_VERTICAL);
        row.setMinimumHeight(Ui.dp(this, 40));
        row.setPadding(Ui.dp(this, 8), 0, Ui.dp(this, 8), 0);
        TextView name = Ui.text(this, option.label, current ? Ui.MINT : Ui.TEXT, 13);
        if (current) {
            name.setTypeface(Typeface.DEFAULT_BOLD);
        }
        row.addView(name, Ui.weighted());
        if (current) {
            row.addView(Ui.icon(this, R.drawable.lucide_check, Ui.MINT, 16));
        }
        row.setBackground(Ui.press(this, Ui.box(this, Ui.SURFACE_HIGH, 0, 5)));
        row.setClickable(true);
        row.setFocusable(true);
        row.setOnClickListener(v -> pick.with(option.value));
        return row;
    }

    /** The frame pacing values Moonlight reads from "frame_pacing". */
    private List<StreamChoices.Option> pace() {
        List<StreamChoices.Option> options = new ArrayList<>();
        options.add(new StreamChoices.Option("latency", "Lowest latency"));
        options.add(new StreamChoices.Option("balanced", "Balanced"));
        options.add(new StreamChoices.Option("cap-fps", "Balanced, capped FPS"));
        options.add(new StreamChoices.Option("smoothness", "Smoothest"));
        return options;
    }

    /** The audio configurations Moonlight reads from "list_audio_config". */
    private List<StreamChoices.Option> sound() {
        List<StreamChoices.Option> options = new ArrayList<>();
        options.add(new StreamChoices.Option("2", "Stereo"));
        options.add(new StreamChoices.Option("51", "5.1 surround"));
        options.add(new StreamChoices.Option("71", "7.1 surround"));
        return options;
    }

    /** The two display modes Moonlight's stretch preference gives. */
    private List<StreamChoices.Option> displayModes() {
        List<StreamChoices.Option> options = new ArrayList<>();
        options.add(new StreamChoices.Option("fit", "Fit to screen"));
        options.add(new StreamChoices.Option("stretch", "Stretch to fill"));
        return options;
    }

    /** The name of a saved video format, for a value this device no longer lists. */
    private static String formatName(String value) {
        switch (value) {
            case "forceav1":
                return "AV1";
            case "forceh265":
                return "HEVC";
            case "neverh265":
                return "H.264";
            default:
                return "Automatic";
        }
    }

    /**
     * After a change: closes the open list, redraws, and reads the saved values
     * back through the class the stream itself uses, so the log shows what a
     * stream would get rather than what was written.
     */
    private void settleStream() {
        expanded = "";
        PreferenceConfiguration config = PreferenceConfiguration.readPreferences(this);
        android.util.Log.i("SecparStream", "read back " + config.width + "x" + config.height
                + " @" + config.fps + " fps, " + config.bitrate + " kbps, " + config.videoFormat
                + ", pacing " + config.framePacing + ", audio " + config.audioConfiguration.channelCount
                + " channels, stretch " + config.stretchVideo);
        render();
    }

    private View deviceRow(JSONObject device) {
        LinearLayout line = Ui.row(this);
        line.setGravity(Gravity.CENTER_VERTICAL);

        boolean mine = device.optBoolean("this");
        boolean hosts = device.optBoolean("hosts");
        ImageView icon = Ui.icon(this, hosts ? R.drawable.lucide_monitor : R.drawable.lucide_monitor_smartphone,
                mine ? Ui.MINT : Ui.DIM, 18);
        LinearLayout.LayoutParams iconParams = new LinearLayout.LayoutParams(icon.getLayoutParams());
        iconParams.rightMargin = Ui.dp(this, 10);
        line.addView(icon, iconParams);

        LinearLayout words = Ui.column(this);
        words.addView(Ui.text(this, device.optString("name"), Ui.TEXT, 14));
        String state = platformName(device.optString("platform"))
                + (hosts ? " · " + (device.optBoolean("online") ? "Online" : "Offline") : "");
        words.addView(Ui.body(this, state, Ui.DIM), Ui.top(this, 1));
        line.addView(words, Ui.weighted());

        if (mine) {
            line.addView(Ui.text(this, "This device", Ui.MINT, 12));
        } else if (!hosts) {
            // Another phone or an old record of this one. Computers are removed
            // from the computer itself, so they are not offered here.
            TextView remove = Ui.quiet(this, "Remove");
            remove.setOnClickListener(v -> {
                if (busy) {
                    return;
                }
                problem = null;
                run(() -> Core.ask(Core.request("remove_device").put("id", device.optString("id"))), answer -> {
                    if (answer.has("devices")) {
                        devices = answer.optJSONArray("devices");
                    } else {
                        problem = explain(answer);
                    }
                    render();
                });
            });
            line.addView(remove);
        }
        return line;
    }

    private void signOut() {
        run(() -> Core.ask(Core.request("sign_out")), answer -> {
            handle = null;
            forgetStills();
            devices = null;
            friends = null;
            friendsLoaded = false;
            friendsProblem = null;
            openFriend = "";
            addText = "";
            problem = null;
            email = "";
            tab = 0;
            if (!answer.optBoolean("ok")) {
                problem = explain(answer);
            }
            render();
        });
    }

    // ---- Connect, pair the first time, stream

    // A computer that has just been signed in spends about half a minute starting
    // its hosting, and until it does the account still lists it as offline. The
    // attempt therefore waits for it instead of calling it broken at the first
    // answer.
    private static final long STARTING_MS = 60_000;
    private static final long STARTING_EVERY_MS = 4_000;

    private void connect(JSONObject device) {
        if (busy || connecting != null) {
            return;
        }
        final String id = device.optString("id");
        final String name = device.optString("name");
        final int attempt = ++attempts;
        connecting = id;
        starting = !device.optBoolean("online");
        problem = null;
        streamProblem = null;
        render();
        reach(id, name, attempt, System.currentTimeMillis());
    }

    /** One try at opening the tunnel to [id], off the main thread. */
    private void reach(String id, String name, int attempt, long began) {
        work.execute(() -> {
            JSONObject answer;
            try {
                answer = Core.ask(Core.request("connect").put("id", id));
                if (answer.optBoolean("ok")) {
                    answer = open(answer, id, name, attempt);
                }
            } catch (Exception e) {
                answer = new JSONObject();
            }
            JSONObject result = answer;
            main.post(() -> reached(id, name, attempt, began, result));
        });
    }

    /** Pairs if this is the first time and opens the stream screen. Null once it is open. */
    private JSONObject open(JSONObject tunnel, String id, String name, int attempt) throws Exception {
        Intent stream = prepare(tunnel, name);
        if (stream == null) {
            Core.ask(Core.request("disconnect"));
            return new JSONObject().put("error", "pairing_failed");
        }
        // The stream screen keeps its own dialogs and toasts out of this flow and
        // reports what it is doing on the broadcast this screen listens for.
        stream.putExtra(Game.EXTRA_SECPAR, true);
        main.post(() -> {
            if (attempt != attempts) {
                return;
            }
            streaming = id;
            startActivity(stream);
        });
        return null;
    }

    private void reached(String id, String name, int attempt, long began, JSONObject answer) {
        if (attempt != attempts) {
            return;
        }
        if (answer == null) {
            // The stream screen has taken over.
            return;
        }
        String error = answer.optString("error");
        boolean waiting = "offline".equals(error) || "computer_not_answering".equals(error);
        if (!answer.optBoolean("ok") && waiting && handle != null && System.currentTimeMillis() - began < STARTING_MS) {
            this.starting = true;
            render();
            long wait = Math.min(STARTING_EVERY_MS, began + STARTING_MS - System.currentTimeMillis());
            main.postDelayed(() -> {
                if (attempt == attempts) {
                    reach(id, name, attempt, began);
                }
            }, Math.max(wait, 0));
            return;
        }
        connecting = null;
        this.starting = false;
        if (handle == null) {
            render();
            return;
        }
        problem = waiting
                ? name + " did not answer. It may still be starting; try again in a moment."
                : trouble(answer, "That did not work. Try again.");
        render();
    }

    /** Gives the attempt up and puts the list back as it was. */
    private void cancelConnect() {
        if (connecting == null) {
            return;
        }
        attempts++;
        connecting = null;
        starting = false;
        problem = null;
        streamProblem = null;
        render();
        // Whatever was opened for this attempt belongs to it and is dropped.
        work.execute(() -> Core.ask(Core.request("disconnect")));
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

    private TextView notice(String value) {
        TextView view = Ui.text(this, value, Ui.BAD, 13);
        view.setLineSpacing(Ui.dp(this, 2), 1f);
        return view;
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
            case "computer_not_answering":
                return "That computer did not answer. Try again in a moment.";
            case "no_internet_path":
                return "This device could not find a path to that computer.";
            case "pairing_failed":
                return "That computer did not accept this device. Try again.";
            case "signed_out":
                return "Sign in again to use this device.";
            case "account_unavailable":
                return "No account with that name.";
            case "cannot_friend_yourself":
                return "That is your own account.";
            case "incoming_request_not_found":
                return "That request is no longer waiting.";
            case "invalid_value":
                return "That setting cannot be saved.";
            case "":
                return "";
            default:
                return "That did not work (" + answer.optString("error") + "). Try again.";
        }
    }

    /** The plain sentence for an answer that failed; never empty. */
    private static String trouble(JSONObject answer, String otherwise) {
        String message = explain(answer);
        return message.isEmpty() ? otherwise : message;
    }

    /** One plain sentence for a stream that did not start, in the app's own words. */
    private static String streamTrouble(String reason) {
        String why = reason == null ? "" : reason;
        if ("no_decoder".equals(why)) {
            return "This device cannot play the stream.";
        }
        if ("connection".equals(why)) {
            return "The connection to that computer ended.";
        }
        if (why.contains("video") || why.contains("audio")) {
            return "That computer started the stream but the picture did not arrive. Try again.";
        }
        return "That computer did not answer. Try again in a moment.";
    }

    @Override
    protected void onDestroy() {
        unregisterReceiver(streamEvents);
        forgetStills();
        work.shutdown();
        second.shutdown();
        stillsWork.shutdown();
        super.onDestroy();
    }
}
