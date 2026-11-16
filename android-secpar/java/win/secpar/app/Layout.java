package win.secpar.app;

/**
 * How the tabs size themselves. Kept free of Android types so the rules can be
 * checked on their own: one column of cards on a phone, two in a docked or
 * landscape window, and a content width that stops growing on very wide screens.
 */
final class Layout {
    /** The two-column layout Moonlight's docked review used. */
    static final int WIDE_DP = 700;
    /** Beyond this width the content stays centred instead of stretching. */
    static final int MAX_CONTENT_DP = 760;
    /** Page margin on each side. */
    static final int EDGE_DP = 16;

    private Layout() {
    }

    static int columns(int widthDp) {
        return widthDp >= WIDE_DP ? 2 : 1;
    }

    /** Side padding, including the extra that centres content on a wide window. */
    static int sideDp(int widthDp) {
        return Math.max(EDGE_DP, (widthDp - MAX_CONTENT_DP) / 2);
    }
}
