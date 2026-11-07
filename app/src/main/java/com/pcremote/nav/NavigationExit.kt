package com.pcremote.nav

import androidx.navigation.NavBackStackEntry
import androidx.navigation.NavController

/** Late callbacks from an outgoing screen must never pop its replacement. */
internal fun NavController.exitScreen(entry: NavBackStackEntry) {
    // Navigation retains the outgoing composition during its exit animation. A
    // stream ending or Activity.onStop can still invoke that composition's callback.
    if (currentBackStackEntry?.id != entry.id) return
    if (!popBackStack()) navigate(Routes.HOME) { launchSingleTop = true }
}
