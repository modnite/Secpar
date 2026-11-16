package win.secpar.app;

import android.app.Activity;
import android.content.Context;
import android.graphics.Color;
import android.graphics.drawable.Drawable;
import android.os.Handler;
import android.os.Looper;
import android.text.TextUtils;
import android.view.Gravity;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewConfiguration;
import android.view.ViewGroup;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.TextView;

import com.limelight.R;

import androidx.annotation.Keep;

/**
 * Secpar's button and panel over Moonlight's running picture.
 *
 * The button carries the approved triangle in a 40dp translucent square. It
 * starts in the picture's top left corner and can be dragged anywhere inside
 * the picture; the place is kept as a fraction of the free space, so rotating
 * or docking the window leaves the button inside the picture. Tapping it opens
 * a compact panel with the computer's name, the size and frame rate the decoder
 * is using, the stats overlay switch and Disconnect. While the panel is open a
 * full screen view below it takes every touch, so nothing reaches the host.
 *
 * The panel uses the same colours, corners and row heights as the desktop
 * stream menu and the rest of the phone app.
 *
 * Kept from the release shrinker: the stream activity builds this class and its
 * listeners directly, and losing any of it silently removes the feature.
 */
@Keep
public final class SecparStreamOverlay {

    /** What the stream activity tells the panel. All calls are on the UI thread. */
    @Keep
    public interface Host {
        /** The computer the picture comes from. */
        String computerName();

        /** The decoder's size and frame rate, or what it is waiting for. */
        String pictureLine();

        boolean statsVisible();

        void setStatsVisible(boolean visible);

        void disconnect();
    }

    private static final float BUTTON_DP = 40f;
    private static final float TRIANGLE_DP = 24f;
    private static final float PANEL_DP = 250f;
    private static final float ROW_DP = 36f;
    private static final float EDGE_DP = 12f;
    private static final float PANEL_EDGE_DP = 8f;
    private static final float PANEL_GAP_DP = 8f;

    private static final int BUTTON_FILL = 0x8C171D27;
    private static final int BUTTON_OPEN_FILL = 0xF0171D27;
    private static final int BUTTON_BORDER = 0x99313B49;
    private static final int PANEL_FILL = 0xF6171D27;

    private final Context context;
    private final FrameLayout parent;
    private final Host host;
    private final Handler handler = new Handler(Looper.getMainLooper());

    private final View scrim;
    private final LinearLayout panel;
    private final FrameLayout button;
    private TextView nameView;
    private TextView pictureView;
    private TextView statsView;

    private boolean open;
    private float fractionX;
    private float fractionY;

    private final Runnable refresh = new Runnable() {
        @Override
        public void run() {
            if (!open) {
                return;
            }
            updateText();
            handler.postDelayed(this, 1000);
        }
    };

    private final View.OnLayoutChangeListener onLayoutChange = new View.OnLayoutChangeListener() {
        @Override
        public void onLayoutChange(View view, int left, int top, int right, int bottom,
                                   int oldLeft, int oldTop, int oldRight, int oldBottom) {
            // Moonlight re-adds its on-screen controller to this layout when the
            // stream screen rotates or is resized, which would put the pad above
            // the button and swallow taps meant for it.
            keepOnTop();
            if (right - left != oldRight - oldLeft || bottom - top != oldBottom - oldTop) {
                placeButton();
                if (open) {
                    placePanel();
                }
            }
        }
    };

    public SecparStreamOverlay(Activity activity, FrameLayout parent, Host host) {
        this.context = activity;
        this.parent = parent;
        this.host = host;

        // Takes every touch while the panel is open, so the host never sees a
        // tap meant for the panel.
        scrim = new View(activity);
        scrim.setBackgroundColor(Color.TRANSPARENT);
        scrim.setClickable(true);
        scrim.setFocusable(false);
        scrim.setVisibility(View.GONE);
        scrim.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View view) {
                close();
            }
        });
        parent.addView(scrim, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));

        panel = buildPanel();
        parent.addView(panel, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT));

        button = buildButton();
        button.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View view) {
                if (open) {
                    close();
                }
                else {
                    open();
                }
            }
        });
        button.setOnTouchListener(new View.OnTouchListener() {
            private float downX;
            private float downY;
            private int startLeft;
            private int startTop;
            private boolean moved;

            @Override
            public boolean onTouch(View view, MotionEvent event) {
                switch (event.getActionMasked()) {
                    case MotionEvent.ACTION_DOWN:
                        downX = event.getRawX();
                        downY = event.getRawY();
                        startLeft = button.getLeft();
                        startTop = button.getTop();
                        moved = false;
                        return true;

                    case MotionEvent.ACTION_MOVE:
                        float dx = event.getRawX() - downX;
                        float dy = event.getRawY() - downY;
                        if (!moved && Math.hypot(dx, dy) >
                                ViewConfiguration.get(context).getScaledTouchSlop()) {
                            moved = true;
                        }
                        if (moved) {
                            moveButtonTo(startLeft + Math.round(dx), startTop + Math.round(dy));
                        }
                        return true;

                    case MotionEvent.ACTION_UP:
                        if (!moved) {
                            view.performClick();
                        }
                        return true;

                    default:
                        return true;
                }
            }
        });
        parent.addView(button, new FrameLayout.LayoutParams(
                Ui.dp(context, BUTTON_DP), Ui.dp(context, BUTTON_DP)));

        parent.addOnLayoutChangeListener(onLayoutChange);

        // Moonlight adds its on-screen controller to this layout later in
        // onCreate, so the overlay puts itself on top once that has happened.
        parent.post(new Runnable() {
            @Override
            public void run() {
                raise();
            }
        });
    }

    /** Keeps the button, the panel and the touch blocker above everything else. */
    private void raise() {
        scrim.bringToFront();
        panel.bringToFront();
        button.bringToFront();
    }

    /**
     * Raises the overlay again after another view was added on top of it. The
     * check avoids a raise loop, because bringToFront asks for another layout.
     */
    private void keepOnTop() {
        int count = parent.getChildCount();
        if (count == 0 || parent.getChildAt(count - 1) == button) {
            return;
        }
        parent.post(new Runnable() {
            @Override
            public void run() {
                raise();
            }
        });
    }

    public boolean isOpen() {
        return open;
    }

    /** Closes the panel and gives the host its touch back. */
    public void close() {
        if (!open) {
            return;
        }
        open = false;
        handler.removeCallbacks(refresh);
        panel.setVisibility(View.GONE);
        scrim.setVisibility(View.GONE);
        button.setBackground(buttonSurface(false));
    }

    private void open() {
        if (open) {
            return;
        }
        open = true;
        raise();
        updateText();
        placePanel();
        panel.setVisibility(View.VISIBLE);
        scrim.setVisibility(View.VISIBLE);
        button.setBackground(buttonSurface(true));
        handler.removeCallbacks(refresh);
        handler.postDelayed(refresh, 1000);
    }

    private FrameLayout buildButton() {
        FrameLayout view = new FrameLayout(context);
        view.setClickable(true);
        view.setFocusable(true);
        view.setContentDescription("Secpar menu");
        view.setBackground(buttonSurface(false));

        ImageView mark = new ImageView(context);
        mark.setImageResource(R.drawable.secpar_mark);
        mark.setScaleType(ImageView.ScaleType.FIT_CENTER);
        mark.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
        int size = Ui.dp(context, TRIANGLE_DP);
        view.addView(mark, new FrameLayout.LayoutParams(size, size, Gravity.CENTER));
        return view;
    }

    private Drawable buttonSurface(boolean opened) {
        return Ui.press(context, Ui.box(context, opened ? BUTTON_OPEN_FILL : BUTTON_FILL,
                BUTTON_BORDER, 6));
    }

    private LinearLayout buildPanel() {
        LinearLayout card = Ui.column(context);
        card.setBackground(Ui.box(context, PANEL_FILL, Ui.BORDER, 7));
        int pad = Ui.dp(context, 6);
        card.setPadding(pad, pad, pad, pad);
        card.setVisibility(View.GONE);

        nameView = Ui.strong(context, "", 14);
        nameView.setSingleLine(true);
        nameView.setEllipsize(TextUtils.TruncateAt.END);
        nameView.setGravity(Gravity.CENTER_VERTICAL);
        nameView.setPadding(Ui.dp(context, 9), 0, Ui.dp(context, 9), 0);
        nameView.setMinHeight(Ui.dp(context, ROW_DP));
        card.addView(nameView, Ui.fill());

        View line = new View(context);
        line.setBackgroundColor(Ui.BORDER);
        LinearLayout.LayoutParams lineParams = new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, Math.max(1, Ui.dp(context, 0.5f)));
        lineParams.topMargin = Ui.dp(context, 5);
        lineParams.bottomMargin = Ui.dp(context, 5);
        card.addView(line, lineParams);

        LinearLayout picture = row();
        picture.addView(Ui.body(context, "Picture", Ui.DIM), Ui.weighted());
        pictureView = Ui.text(context, "", Ui.TEXT, 13);
        pictureView.setSingleLine(true);
        pictureView.setEllipsize(TextUtils.TruncateAt.END);
        picture.addView(pictureView);
        card.addView(picture, Ui.fill());

        LinearLayout stats = row();
        stats.setBackground(Ui.press(context, Ui.box(context, 0, 0, 4)));
        stats.setClickable(true);
        stats.setFocusable(true);
        stats.addView(Ui.text(context, "Stats overlay", Ui.TEXT, 14), Ui.weighted());
        statsView = Ui.text(context, "", Ui.DIM, 13);
        stats.addView(statsView);
        stats.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View view) {
                host.setStatsVisible(!host.statsVisible());
                updateText();
            }
        });
        card.addView(stats, Ui.fill());

        LinearLayout disconnect = row();
        disconnect.setBackground(Ui.press(context, Ui.box(context, 0, 0, 4)));
        disconnect.setClickable(true);
        disconnect.setFocusable(true);
        disconnect.addView(Ui.text(context, "Disconnect", Ui.BAD, 14), Ui.weighted());
        disconnect.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View view) {
                close();
                host.disconnect();
            }
        });
        card.addView(disconnect, Ui.fill());

        TextView footer = Ui.text(context, "Secpar", Ui.DIM, 10);
        footer.setGravity(Gravity.CENTER_VERTICAL);
        footer.setPadding(Ui.dp(context, 9), 0, Ui.dp(context, 9), 0);
        footer.setMinHeight(Ui.dp(context, 18));
        card.addView(footer, Ui.fill());

        return card;
    }

    private LinearLayout row() {
        LinearLayout row = Ui.row(context);
        row.setGravity(Gravity.CENTER_VERTICAL);
        row.setMinimumHeight(Ui.dp(context, ROW_DP));
        row.setPadding(Ui.dp(context, 9), 0, Ui.dp(context, 9), 0);
        return row;
    }

    private void updateText() {
        String name = host.computerName();
        nameView.setText(name == null || name.isEmpty() ? "Computer" : name);
        pictureView.setText(host.pictureLine());

        boolean visible = host.statsVisible();
        statsView.setText(visible ? "On" : "Off");
        statsView.setTextColor(visible ? Ui.MINT : Ui.DIM);
    }

    private void placeButton() {
        int width = parent.getWidth();
        int height = parent.getHeight();
        if (width <= 0 || height <= 0) {
            return;
        }

        int size = Ui.dp(context, BUTTON_DP);
        int edge = Ui.dp(context, EDGE_DP);
        int spanX = Math.max(0, width - size - 2 * edge);
        int spanY = Math.max(0, height - size - 2 * edge);

        FrameLayout.LayoutParams params = new FrameLayout.LayoutParams(size, size);
        params.gravity = Gravity.LEFT | Gravity.TOP;
        params.leftMargin = edge + Math.round(fractionX * spanX);
        params.topMargin = edge + Math.round(fractionY * spanY);
        button.setLayoutParams(params);
    }

    private void moveButtonTo(int left, int top) {
        int size = Ui.dp(context, BUTTON_DP);
        int edge = Ui.dp(context, EDGE_DP);
        int spanX = Math.max(0, parent.getWidth() - size - 2 * edge);
        int spanY = Math.max(0, parent.getHeight() - size - 2 * edge);

        int x = Math.min(Math.max(left - edge, 0), spanX);
        int y = Math.min(Math.max(top - edge, 0), spanY);
        fractionX = spanX > 0 ? (float) x / spanX : 0f;
        fractionY = spanY > 0 ? (float) y / spanY : 0f;

        placeButton();
        if (open) {
            placePanel();
        }
    }

    /** Opens the panel under the button, or above it when the picture has no room below. */
    private void placePanel() {
        int width = parent.getWidth();
        int height = parent.getHeight();
        if (width <= 0 || height <= 0) {
            return;
        }

        int edge = Ui.dp(context, PANEL_EDGE_DP);
        int panelWidth = Math.min(Ui.dp(context, PANEL_DP), Math.max(Ui.dp(context, 160), width - 2 * edge));
        panel.measure(View.MeasureSpec.makeMeasureSpec(panelWidth, View.MeasureSpec.EXACTLY),
                View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED));
        int panelHeight = panel.getMeasuredHeight();

        int left = Math.min(Math.max(button.getLeft(), edge), Math.max(edge, width - panelWidth - edge));
        int gap = Ui.dp(context, PANEL_GAP_DP);
        int top = button.getTop() + button.getHeight() + gap;
        if (top + panelHeight > height - edge) {
            int above = button.getTop() - panelHeight - gap;
            top = above >= edge ? above : Math.max(edge, height - panelHeight - edge);
        }

        FrameLayout.LayoutParams params = new FrameLayout.LayoutParams(panelWidth,
                ViewGroup.LayoutParams.WRAP_CONTENT);
        params.gravity = Gravity.LEFT | Gravity.TOP;
        params.leftMargin = left;
        params.topMargin = top;
        panel.setLayoutParams(params);
    }
}
