package win.secpar.app;

import static org.junit.Assert.assertEquals;

import org.junit.Test;

/** The phone and docked window rules from the approved layout. */
public class LayoutTest {
    @Test
    public void aPhoneListsOneColumn() {
        assertEquals(1, Layout.columns(412));
    }

    @Test
    public void aDockedWindowShowsTwoColumns() {
        assertEquals(2, Layout.columns(915));
    }

    @Test
    public void theTwoColumnWidthIsTheThreshold() {
        assertEquals(1, Layout.columns(Layout.WIDE_DP - 1));
        assertEquals(2, Layout.columns(Layout.WIDE_DP));
    }

    @Test
    public void aPhoneKeepsThePlainMargin() {
        assertEquals(Layout.EDGE_DP, Layout.sideDp(412));
    }

    @Test
    public void wideContentStaysCentredRatherThanStretching() {
        int side = Layout.sideDp(1400);
        assertEquals(Layout.MAX_CONTENT_DP, 1400 - 2 * side);
        assertEquals(Layout.EDGE_DP, Layout.sideDp(Layout.MAX_CONTENT_DP));
    }
}
