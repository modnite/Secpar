package win.secpar.gamestream;

import androidx.test.ext.junit.runners.AndroidJUnit4;
import com.limelight.nvstream.jni.MoonBridge;
import org.junit.Test;
import org.junit.runner.RunWith;
import static org.junit.Assert.*;

/** Loads the real upstream JNI library. No network, pairing or capture involved. */
@RunWith(AndroidJUnit4.class)
public class NativeCoreInstrumentedTest {
    @Test public void upstreamNativeLibraryLoadsAndAnswersItsJniBinding() {
        assertNotNull(MoonBridge.getStageName(0));
        assertNotNull(MoonBridge.getLaunchUrlQueryParameters());
        assertTrue(MoonBridge.VIDEO_FORMAT_H264 != 0);
    }
}
