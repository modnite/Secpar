package com.pcremote.nav

import android.os.Bundle
import androidx.activity.compose.setContent
import androidx.compose.animation.fadeOut
import androidx.compose.animation.core.tween
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Text
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.lifecycle.Lifecycle
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import com.pcremote.MainActivity
import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.atomic.AtomicInteger

/** Exercises real Navigation Compose transitions without opening a network or media session. */
class NavigationLifecycleInstrumentedTest {
    @Test fun lateStreamExitDoesNotRemoveHomeAcrossBackgroundAndReturn() {
        val fixture = Fixture()
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.onActivity { fixture.install(it) }
            waitUntil { fixture.homeDraws.get() > 0 }
            scenario.onActivity { fixture.nav.navigate(STREAM) }
            waitUntil { fixture.exit != null }
            scenario.onActivity { activity ->
                // A back action starts the exit animation. A hidden-stream callback can
                // still arrive while the outgoing composition is retained by Navigation.
                val lateExit = fixture.exit!!
                lateExit()
                assertEquals(Routes.HOME, fixture.nav.currentDestination?.route)
                assertNotNull("Outgoing stream must still own its hidden callback", activity.onStreamHidden)
                activity.onStreamHidden!!.invoke()
                assertEquals("A late stream exit must not pop the last Home destination",
                    Routes.HOME, fixture.nav.currentDestination?.route)
            }
            scenario.moveToState(Lifecycle.State.CREATED)
            val drawnBeforeReturn = fixture.homeDraws.get()
            scenario.moveToState(Lifecycle.State.RESUMED)
            waitUntil { fixture.homeDraws.get() > drawnBeforeReturn }
            scenario.onActivity { assertEquals(Routes.HOME, fixture.nav.currentDestination?.route) }

            var savedNavigation: Bundle? = null
            scenario.onActivity {
                savedNavigation = fixture.nav.saveState()
                // Clear the synthetic graph before production MainActivity recreates;
                // it deliberately has no fixture destinations in its real graph.
                it.setContent { Text("Recreating") }
            }
            InstrumentationRegistry.getInstrumentation().waitForIdleSync()
            scenario.recreate()
            val drawnBeforeRecreate = fixture.homeDraws.get()
            scenario.onActivity { fixture.install(it, savedNavigation) }
            waitUntil { fixture.homeDraws.get() > drawnBeforeRecreate }
            scenario.onActivity { assertEquals("Saved navigation must restore visible Home",
                Routes.HOME, fixture.nav.currentDestination?.route) }
        }
    }

    @Test fun exitingARootStreamFallsBackToHome() {
        val fixture = Fixture()
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.onActivity { fixture.install(it, start = STREAM) }
            waitUntil { fixture.exit != null }
            scenario.onActivity {
                fixture.exit!!()
                assertEquals(Routes.HOME, fixture.nav.currentDestination?.route)
            }
            waitUntil { fixture.homeDraws.get() > 0 }
        }
    }

    @Test fun oldStreamExitCannotCloseANewerStream() {
        val fixture = Fixture()
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.onActivity { fixture.install(it) }
            waitUntil { fixture.homeDraws.get() > 0 }
            scenario.onActivity { fixture.nav.navigate(STREAM) }
            waitUntil { fixture.exit != null }
            var oldExit: (() -> Unit)? = null
            scenario.onActivity {
                oldExit = fixture.exit
                oldExit!!()
                fixture.nav.navigate(STREAM)
                val newerEntry = fixture.nav.currentBackStackEntry!!.id
                oldExit!!()
                assertEquals("An old callback must not close another entry of the same route",
                    newerEntry, fixture.nav.currentBackStackEntry?.id)
            }
        }
    }

    private class Fixture {
        lateinit var nav: NavHostController
        @Volatile var exit: (() -> Unit)? = null
        val homeDraws = AtomicInteger()

        fun install(activity: MainActivity, restored: Bundle? = null, start: String = Routes.HOME) {
            activity.setContent {
                nav = rememberNavController()
                remember(nav) { restored?.let(nav::restoreState) }
                NavHost(nav, start, exitTransition = { fadeOut(tween(500)) }) {
                    composable(Routes.HOME) {
                        Box(Modifier.fillMaxSize().drawWithContent {
                            drawContent(); homeDraws.incrementAndGet()
                        }) { Text("Your computers") }
                    }
                    composable(STREAM) { entry ->
                        val close = { nav.exitScreen(entry) }
                        DisposableEffect(entry) {
                            exit = close
                            activity.onStreamHidden = close
                            onDispose { activity.onStreamHidden = null }
                        }
                        Text("Stream fixture")
                    }
                }
            }
        }
    }

    private fun waitUntil(condition: () -> Boolean) {
        val deadline = System.currentTimeMillis() + 8000
        while (!condition() && System.currentTimeMillis() < deadline) Thread.sleep(25)
        assertTrue("Navigation fixture did not reach its expected state", condition())
    }

    private companion object { const val STREAM = "stream-fixture" }
}
