package com.pcremote.scan

import android.content.Intent
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.pcremote.MainActivity
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** Isolated emulator only: back camera imagefile is the host QR fixture on a monitor. */
@RunWith(AndroidJUnit4::class)
class CameraPairingInstrumentedTest {
    @Test fun cameraReturnsTheDecodedLinkToItsCaller() {
        assumeTrue(InstrumentationRegistry.getArguments().getString("secparCameraFixture") == "true")
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val expected = instrumentation.context.assets.open("host-qr.txt").bufferedReader().use { it.readText() }
        val activity = instrumentation.startActivitySync(Intent(instrumentation.targetContext, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        val received = CountDownLatch(1)
        var text: String? = null
        instrumentation.runOnMainSync {
            // Intercept the result: do not attempt a network connection to the fixture.
            QrScan.start(activity) { text = it; received.countDown() }
        }
        try {
            assertTrue("Camera never returned a decoded pairing link", received.await(60, TimeUnit.SECONDS))
            assertEquals(expected, text)
        } finally {
            instrumentation.runOnMainSync { QrScan.cancel(); activity.finish() }
        }
    }
}
