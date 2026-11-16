package win.secpar.app;

import android.content.Context;
import android.content.res.ColorStateList;
import android.graphics.Typeface;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.RippleDrawable;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.TextView;

import com.limelight.R;

/**
 * Secpar's flat dark surfaces, type sizes and controls. The colours and the
 * 7dp cards are the approved Midnight theme; the spacing matches the 3.x
 * Android screen. Shared by every page so heights and edges line up.
 */
final class Ui {
    static final int BACKGROUND = 0xFF11151C;
    static final int SURFACE = 0xFF171D27;
    static final int SURFACE_HIGH = 0xFF1F2733;
    static final int BORDER = 0xFF313B49;
    static final int CONTROL_BORDER = 0xFF414D5E;
    static final int MINT = 0xFF0AFFB6;
    static final int ON_MINT = 0xFF07261E;
    static final int ORANGE = 0xFFD95400;
    static final int TEXT = 0xFFE6EAF2;
    static final int DIM = 0xFF98A2B8;
    static final int BAD = 0xFFF87171;

    private Ui() {
    }

    static int dp(Context context, float value) {
        return Math.round(value * context.getResources().getDisplayMetrics().density);
    }

    static GradientDrawable box(Context context, int fill, int stroke, float radius) {
        GradientDrawable shape = new GradientDrawable();
        shape.setColor(fill);
        shape.setCornerRadius(dp(context, radius));
        if (stroke != 0) {
            shape.setStroke(dp(context, 1), stroke);
        }
        return shape;
    }

    /** A rounded surface that shows the platform's touch feedback. */
    static Drawable press(Context context, GradientDrawable shape) {
        return new RippleDrawable(ColorStateList.valueOf(0x1FFFFFFF), shape, null);
    }

    static LinearLayout row(Context context) {
        LinearLayout view = new LinearLayout(context);
        view.setOrientation(LinearLayout.HORIZONTAL);
        return view;
    }

    static LinearLayout column(Context context) {
        LinearLayout view = new LinearLayout(context);
        view.setOrientation(LinearLayout.VERTICAL);
        return view;
    }

    static TextView text(Context context, CharSequence value, int colour, float size) {
        TextView view = new TextView(context);
        view.setText(value);
        view.setTextColor(colour);
        view.setTextSize(size);
        return view;
    }

    static TextView strong(Context context, CharSequence value, float size) {
        TextView view = text(context, value, TEXT, size);
        view.setTypeface(Typeface.DEFAULT_BOLD);
        return view;
    }

    static TextView body(Context context, CharSequence value, int colour) {
        TextView view = text(context, value, colour, 13);
        view.setLineSpacing(dp(context, 2), 1f);
        return view;
    }

    static LinearLayout.LayoutParams fill() {
        return new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
    }

    static LinearLayout.LayoutParams top(Context context, float space) {
        LinearLayout.LayoutParams params = fill();
        params.topMargin = dp(context, space);
        return params;
    }

    static LinearLayout.LayoutParams weighted() {
        return new LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f);
    }

    /** A page card: surface, one quiet border, 7dp corners and 12dp inside. */
    static LinearLayout card(Context context) {
        LinearLayout view = column(context);
        view.setBackground(box(context, SURFACE, BORDER, 7));
        int pad = dp(context, 12);
        view.setPadding(pad, pad, pad, pad);
        return view;
    }

    static TextView action(Context context, String label, boolean primary) {
        TextView view = text(context, label, primary ? ON_MINT : TEXT, 14);
        view.setGravity(Gravity.CENTER);
        view.setTypeface(Typeface.DEFAULT_BOLD);
        view.setMinHeight(dp(context, 44));
        view.setPadding(dp(context, 12), dp(context, 10), dp(context, 12), dp(context, 10));
        view.setBackground(press(context, box(context, primary ? MINT : SURFACE_HIGH,
                primary ? MINT : CONTROL_BORDER, 6)));
        view.setClickable(true);
        view.setFocusable(true);
        return view;
    }

    /** A small secondary action that sits under a list instead of spanning it. */
    static TextView quiet(Context context, String label) {
        TextView view = text(context, label, TEXT, 13);
        view.setGravity(Gravity.CENTER);
        view.setTypeface(Typeface.DEFAULT_BOLD);
        view.setMinHeight(dp(context, 40));
        view.setPadding(dp(context, 16), dp(context, 8), dp(context, 16), dp(context, 8));
        view.setBackground(press(context, box(context, SURFACE_HIGH, CONTROL_BORDER, 6)));
        view.setClickable(true);
        view.setFocusable(true);
        return view;
    }

    /** A quiet text action for a row that opens another screen. */
    static TextView link(Context context, String label) {
        TextView view = text(context, label, TEXT, 14);
        view.setGravity(Gravity.CENTER_VERTICAL);
        view.setMinHeight(dp(context, 44));
        view.setBackground(press(context, box(context, 0, 0, 6)));
        view.setClickable(true);
        view.setFocusable(true);
        return view;
    }

    static ImageView icon(Context context, int drawable, int tint, float sizeDp) {
        ImageView view = new ImageView(context);
        view.setImageResource(drawable);
        view.setScaleType(ImageView.ScaleType.FIT_CENTER);
        if (tint != 0) {
            view.setImageTintList(ColorStateList.valueOf(tint));
        }
        int size = dp(context, sizeDp);
        view.setLayoutParams(new LinearLayout.LayoutParams(size, size));
        return view;
    }

    static View gap(Context context, float heightDp) {
        View view = new View(context);
        view.setLayoutParams(new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(context, heightDp)));
        return view;
    }

    /** The approved triangle and wordmark, with the app version beside them. */
    static LinearLayout brand(Context context, String version) {
        LinearLayout line = row(context);
        line.setGravity(Gravity.CENTER_VERTICAL);

        ImageView wordmark = new ImageView(context);
        // The approved wordmark already carries the triangle, so it is not drawn twice.
        wordmark.setImageResource(R.drawable.secpar_wordmark_light);
        wordmark.setAdjustViewBounds(true);
        wordmark.setScaleType(ImageView.ScaleType.FIT_CENTER);
        wordmark.setContentDescription("Secpar");
        line.addView(wordmark, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, dp(context, 20)));

        TextView shown = text(context, version, DIM, 11);
        LinearLayout.LayoutParams versionParams = new LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT);
        versionParams.leftMargin = dp(context, 8);
        line.addView(shown, versionParams);
        return line;
    }

    /** An empty state or a plain answer, on the page rather than in a box. */
    static LinearLayout empty(Context context, String title, String detail) {
        LinearLayout card = card(context);
        card.addView(strong(context, title, 15));
        if (!detail.isEmpty()) {
            card.addView(body(context, detail, DIM), top(context, 4));
        }
        return card;
    }
}
