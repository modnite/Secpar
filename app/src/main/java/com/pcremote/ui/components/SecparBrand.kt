package com.pcremote.ui.components

import androidx.compose.foundation.Image
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import com.pcremote.R
import com.pcremote.ui.theme.LocalMidnight

/** Sourced logo letterforms; regular interface text keeps the platform font. */
@Composable
fun SecparWordmark(modifier: Modifier = Modifier) {
    Image(painterResource(if (LocalMidnight.current.isDark) R.drawable.secpar_wordmark_light else R.drawable.secpar_wordmark_dark),
        contentDescription = "Secpar", modifier = modifier)
}
